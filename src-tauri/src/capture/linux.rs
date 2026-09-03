//! Linux capture backend: Portal (ashpd) + PipeWire (design.md §3.1.2, §3.2.2).
//!
//! - Screen/window: xdg-desktop-portal ScreenCast. The OS picker is the only
//!   selection path (no app-side window enumeration); the returned PipeWire
//!   node is captured as BGRA frames → scale to profile → [`VideoSink`].
//! - Audio: PipeWire capture streams. system = default sink monitor,
//!   per-app = capture stream targeted at the app's node, mic = source node.
//!
//! Compile verification happens in CI (`cargo check --features capture-linux`
//! on Ubuntu 24.04 and Arch Linux); runtime needs a Wayland + PipeWire session.

use super::{CaptureError, Result};
use base64::Engine;
use eztopaz_core::audio::AudioSink;
use eztopaz_core::config::{Profile, ScreenTarget, ScreenTargetKind};
use eztopaz_core::ipc_types::{AppAudio, AudioDevices, AudioSelection, DeviceInfo};
use eztopaz_core::video::{bgra_to_rgba, scale_bgra, VideoSink};
use pipewire as pw;
use pw::properties::properties;
use pw::spa::param::ParamType;
use pw::spa::pod::Pod;
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::Emitter;

fn err<E: std::fmt::Display>(e: E) -> CaptureError {
    CaptureError::Failed(e.to_string())
}

// ---------------------------------------------------------------------------
// Portal session state (picker → node id → PipeWire fd)

struct PortalState {
    fd: std::os::fd::OwnedFd,
    node_id: u32,
    kind: ScreenTargetKind,
}

static PORTAL: Mutex<Option<PortalState>> = Mutex::new(None);

/// Open the OS picker and remember the chosen stream (F-SC-02 Wayland path).
pub async fn portal_picker() -> Result<ScreenTarget> {
    use ashpd::desktop::screencast::{CursorMode, PersistMode, Screencast, SourceType};

    let screencast = Screencast::new().await.map_err(err)?;
    let session = screencast.create_session().await.map_err(err)?;
    let request = screencast
        .select_sources(
            &session,
            CursorMode::Hidden,
            SourceType::Monitor | SourceType::Window,
            false,
            None,
            PersistMode::DoNot,
        )
        .await
        .map_err(err)?;
    // ashpd 0.7: select_sources' response is empty; the picker result (streams)
    // is delivered by the Start request (ashpd screencast module docs).
    request.response().map_err(err)?;
    let started = screencast
        .start(&session, &ashpd::WindowIdentifier::default())
        .await
        .map_err(err)?;
    let streams = started.response().map_err(err)?.streams().to_vec();
    let Some(stream) = streams.first() else {
        return Err(CaptureError::Failed("portal picker returned no stream".into()));
    };
    let node_id = stream.pipe_wire_node_id();
    let kind = match stream.source_type() {
        Some(ashpd::desktop::screencast::SourceType::Monitor) => ScreenTargetKind::Display,
        _ => ScreenTargetKind::Window,
    };
    let fd = screencast.open_pipe_wire_remote(&session).await.map_err(err)?;
    // dup: the PipeWire remote fd must outlive the portal proxies
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(libc::dup(fd)) };

    *PORTAL.lock().unwrap() = Some(PortalState { fd: owned, node_id, kind });

    Ok(ScreenTarget { kind, id: format!("portal:{node_id}") })
}

// ---------------------------------------------------------------------------
// screen capture

pub struct ScreenCapture {
    pub sink: VideoSink,
    stop: Arc<AtomicBool>,
    /// Wake channel for the worker's park loop. The worker owns the PipeWire
    /// loop and stops it itself; stop() only signals + joins, never touching
    /// PipeWire from the outside (no raw loop pointer, no cross-thread stop).
    wake: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ScreenCapture {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let (lk, cv) = &*self.wake;
            *lk.lock().unwrap() = true;
            cv.notify_one();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.sink.stop();
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_screen(
    app: tauri::AppHandle,
    profile: &Profile,
    sink: VideoSink,
) -> Result<ScreenCapture> {
    // the portal connection is kept in PORTAL so preview → stream can reuse it
    // without re-picking; the capture thread works on its own dup of the fd
    let (fd, node_id) = {
        let portal = PORTAL.lock().unwrap();
        let portal = portal
            .as_ref()
            .ok_or_else(|| CaptureError::Failed("start_portal_picker() を先に実行してください".into()))?;
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(libc::dup(portal.fd.as_raw_fd())) };
        (fd, portal.node_id)
    };
    let stop = Arc::new(AtomicBool::new(false));
    let wake: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
    let wake2 = wake.clone();
    let stop2 = stop.clone();
    let sink2 = sink.clone();
    let preview_slot: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let preview_slot2 = preview_slot.clone();
    let dst_w = profile.w;
    let dst_h = profile.h;
    // Setup runs on the spawned thread; the result is reported back so the
    // caller fails fast instead of leaking a dead capture. Without this,
    // thread-internal failures were only eprintln! noise while the command
    // returned Ok (dead preview with no error surfaced).
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    let fail_tx = ready_tx.clone();
    let fail = move |msg: String| {
        eprintln!("pw video: {msg}");
        let _ = fail_tx.send(Err(msg));
    };

    let handle = std::thread::Builder::new()
        .name("pw-video".into())
        .spawn(move || {
            let tl = match unsafe { pw::thread_loop::ThreadLoopBox::new(Some("eztopaz-video"), None) } {
                Ok(t) => t,
                Err(e) => {
                    fail(format!("PipeWire loop: {e}"));
                    return;
                }
            };
            // Every PipeWire object call below requires the loop lock when
            // made from outside the loop thread. The guard is held for the
            // whole setup and released before the loop starts parking below.
            let _pw_guard = tl.lock();
            let context = match pw::context::ContextBox::new(tl.loop_(), None) {
                Ok(c) => c,
                Err(e) => {
                    fail(format!("PipeWire context: {e}"));
                    return;
                }
            };
            let core = match context.connect_fd(fd, None) {
                Ok(c) => c,
                Err(e) => {
                    fail(format!("PipeWire connect: {e}"));
                    return;
                }
            };
            let props = properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            };
            let stream = match pw::stream::StreamBox::new(&core, "eztopaz-video", props) {
                Ok(s) => s,
                Err(e) => {
                    fail(format!("PipeWire stream: {e}"));
                    return;
                }
            };

            struct Ud {
                format: pw::spa::param::video::VideoInfoRaw,
                sink: VideoSink,
                stop: Arc<AtomicBool>,
                preview_frame: Arc<Mutex<Option<Vec<u8>>>>,
                dst_w: u32,
                dst_h: u32,
            }
            let ud = Ud {
                format: pw::spa::param::video::VideoInfoRaw::new(),
                sink: sink2,
                stop: stop2.clone(),
                preview_frame: preview_slot2,
                dst_w,
                dst_h,
            };

            let _listener = match stream
                .add_local_listener_with_user_data(ud)
                .state_changed(|_, _, _old, new| {
                    // Error states carry the only visible reason when a
                    // connected stream never delivers (no node, no frames).
                    if let pw::stream::StreamState::Error(e) = new {
                        eprintln!("pw video: stream error: {e}");
                    }
                })
                .param_changed(|_, ud, id, param| {
                    let Some(param) = param else { return };
                    if id != ParamType::Format.as_raw() {
                        return;
                    }
                    let _ = ud.format.parse(param);
                })
                .process(|stream, ud| {
                    if ud.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(mut buffer) = stream.dequeue_buffer() else { return };
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        return;
                    }
                    let data = &mut datas[0];
                    let size = ud.format.size();
                    let (w, h) = (size.width.max(1), size.height.max(1));
                    if let Some(bytes) = data.data() {
                        let frame = scale_bgra(bytes, w, h, ud.dst_w, ud.dst_h);
                        ud.sink.push(frame.clone());
                        // park the newest frame for the 1fps preview thread
                        if let Ok(mut slot) = ud.preview_frame.lock() {
                            if slot.is_none() {
                                *slot = Some(frame);
                            }
                        }
                    }
                })
                .register()
            {
                Ok(l) => l,
                Err(e) => {
                    fail(format!("PipeWire listener: {e}"));
                    return;
                }
            };

            // negotiate BGRA; source size/framerate come back in the negotiated
            // Format (param_changed above). Build the pod with the official macros.
            let obj = pw::spa::pod::object!(
                pw::spa::utils::SpaTypes::ObjectParamFormat,
                ParamType::EnumFormat,
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::MediaType,
                    Id,
                    pw::spa::param::format::MediaType::Video
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::MediaSubtype,
                    Id,
                    pw::spa::param::format::MediaSubtype::Raw
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::VideoFormat,
                    Id,
                    pw::spa::param::video::VideoFormat::BGRA
                ),
            );
            let values: Vec<u8> = match pw::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pw::spa::pod::Value::Object(obj),
            ) {
                Ok(v) => v.0.into_inner(),
                Err(e) => {
                    fail(format!("PipeWire format pod: {e}"));
                    return;
                }
            };
            let mut params = [Pod::from_bytes(&values).unwrap()];

            if let Err(e) = stream.connect(
                pw::spa::utils::Direction::Input,
                Some(node_id),
                pw::stream::StreamFlags::AUTOCONNECT
                    | pw::stream::StreamFlags::MAP_BUFFERS
                    | pw::stream::StreamFlags::RT_PROCESS,
                &mut params,
            ) {
                fail(format!("PipeWire stream connect (node {node_id}): {e}"));
                return;
            }
            // Request dataflow explicitly instead of relying on the default
            // active state; a refusal surfaces through the fail-fast gate.
            if let Err(e) = stream.set_active(true) {
                fail(format!("PipeWire stream activate (node {node_id}): {e}"));
                return;
            }

            let _ = ready_tx.send(Ok(()));
            drop(_pw_guard);
            // A stop() that lands during setup is already recorded in the
            // stop flag / wake channel: skip parking and go straight to a
            // uniform teardown (loop never started: stop() is skipped).
            let parked = !stop2.load(Ordering::Relaxed);
            if parked {
                tl.start();
                // Park on OUR condvar, never tl.wait(): the PipeWire wait must
                // be called with the loop lock held (it is a pthread_cond_wait
                // on the loop mutex — unlocked it returns immediately, which
                // tore the stream down at once: the silent Connecting ->
                // Unconnected with no node and no data). Parking here also
                // avoids a second waiter racing stop()'s internal wait.
                let (lk, cv) = &*wake2;
                let mut stopped = lk.lock().unwrap();
                while !*stopped {
                    stopped = cv.wait(stopped).unwrap();
                }
                // Worker thread (not the loop thread): may block until the
                // loop thread exits. Must run without holding the pw guard.
                tl.stop();
            }
            // Locked teardown: destroying streams/proxies without holding the
            // loop lock trips "called from wrong context" and can leave zombie
            // streams behind (LizardByte/Sunshine#4705 pattern).
            {
                let _guard = tl.lock();
                let _ = stream.set_active(false);
                let _ = stream.disconnect();
                drop(_listener);
                drop(stream);
                drop(core);
                drop(context);
            }
            // tl drops here; the loop is stopped (or never started), so the
            // destroy is clean.
        })
        .map_err(err)?;

    // Fail fast when setup died (or hung): without this gate the command
    // returned Ok while the thread was already gone — dead preview, no error.
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        outcome => {
            let msg = match outcome {
                Ok(Err(e)) => e,
                _ => "PipeWire capture setup timed out".to_string(),
            };
            ScreenCapture { sink, stop, wake, handle: Some(handle) }.stop();
            return Err(CaptureError::Failed(msg));
        }
    }

    // F-SC-03 preview: 1fps 640x360 PNG → `stream://preview` (design §6.4).
    // Runs off the PipeWire RT thread; the process callback parks the newest
    // frame in `preview_slot` and this thread converts/emits at 1fps.
    {
        let slot = preview_slot.clone();
        let app = app.clone();
        let preview_stop = stop.clone();
        let dst_w = dst_w;
        let dst_h = dst_h;
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let mut last = Instant::now() - Duration::from_secs(1);
                loop {
                    if preview_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                    if last.elapsed() < Duration::from_secs(1) {
                        continue;
                    }
                    let Some(frame) = slot.lock().unwrap().take() else { continue };
                    last = Instant::now();
                    let small = scale_bgra(&frame, dst_w, dst_h, 640, 360);
                    let rgba = bgra_to_rgba(&small);
                    let Some(png) = crate::capture::encode_png(&rgba, 640, 360) else { continue };
                    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
                    let _ = app.emit(
                        "stream://preview",
                        eztopaz_core::ipc_types::PreviewFrame {
                            data_url: format!("data:image/png;base64,{b64}"),
                            w: 640,
                            h: 360,
                        },
                    );
                }
            })
            .map_err(err)?;
    }

    Ok(ScreenCapture { sink, stop, wake, handle: Some(handle) })
}

// ---------------------------------------------------------------------------
// audio capture

pub struct AudioCapture {
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub sink: Option<AudioSink>,
}

impl AudioCapture {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let (lk, cv) = &*self.wake;
            *lk.lock().unwrap() = true;
            cv.notify_one();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if let Some(s) = self.sink.as_mut() {
            s.stop();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Per-stream pw properties: (source id, optional TARGET_OBJECT node, capture sink monitor)
fn audio_source_specs(selection: &AudioSelection) -> Vec<(String, Option<String>, bool)> {
    let mut specs = Vec::new();
    if selection.mode == "system" {
        specs.push(("system".into(), None, true));
    }
    if selection.mic.enabled {
        let dev = selection.mic.device.clone();
        let target = if dev == "default" { None } else { Some(dev) };
        specs.push((eztopaz_core::audio::MIC_ID.into(), target, false));
    }
    if selection.mode == "apps" {
        for app in &selection.apps {
            let node = app.rsplit(':').next().unwrap_or("").to_string();
            if node.is_empty() {
                continue;
            }
            specs.push((app.clone(), Some(node), false));
        }
    }
    specs
}

pub fn start_audio(selection: &AudioSelection, sink: AudioSink) -> Result<AudioCapture> {
    let specs = audio_source_specs(selection);
    if specs.is_empty() {
        return Err(CaptureError::Failed("音声ソースが選択されていません".into()));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let wake: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
    let wake2 = wake.clone();
    let stop2 = stop.clone();
    let sink2 = sink.clone();
    // Same fail-fast setup reporting as the video thread: callers must not
    // get Ok for a dead capture.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    let fail_tx = ready_tx.clone();
    let fail = move |msg: String| {
        eprintln!("pw audio: {msg}");
        let _ = fail_tx.send(Err(msg));
    };

    let handle = std::thread::Builder::new()
        .name("pw-audio".into())
        .spawn(move || {
            let tl = match unsafe { pw::thread_loop::ThreadLoopBox::new(Some("eztopaz-audio"), None) } {
                Ok(t) => t,
                Err(e) => {
                    fail(format!("PipeWire loop: {e}"));
                    return;
                }
            };
            // All PipeWire object calls under the loop lock (see video thread).
            let _pw_guard = tl.lock();
            let context = match pw::context::ContextBox::new(tl.loop_(), None) {
                Ok(c) => c,
                Err(e) => {
                    fail(format!("PipeWire context: {e}"));
                    return;
                }
            };
            let core = match context.connect(None) {
                Ok(c) => c,
                Err(e) => {
                    fail(format!("PipeWire connect: {e}"));
                    return;
                }
            };

            let mut streams = Vec::new();
            // Listeners unregister themselves on drop: keep them alive as long
            // as the streams, otherwise no process callback ever fires (dead
            // silence with no error).
            let mut listeners = Vec::new();
            for (id, target, capture_sink) in specs {
                let props = properties! {
                    *pw::keys::MEDIA_TYPE => "Audio",
                    *pw::keys::MEDIA_CATEGORY => "Capture",
                    *pw::keys::MEDIA_ROLE => "Music",
                };
                let props = {
                    let mut p = props;
                    if capture_sink {
                        p.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
                    }
                    if let Some(t) = &target {
                        p.insert(*pw::keys::TARGET_OBJECT, t.as_str());
                    }
                    p
                };
                let stream = match pw::stream::StreamBox::new(&core, &format!("eztopaz-{id}"), props)
                {
                    Ok(s) => s,
                    Err(e) => {
                        fail(format!("PipeWire stream {id}: {e}"));
                        return;
                    }
                };

                struct Ud {
                    sink: AudioSink,
                    id: String,
                    stop: Arc<AtomicBool>,
                }
                let ud = Ud { sink: sink2.clone(), id: id.clone(), stop: stop2.clone() };

                let ud_id = id.clone();
                match stream
                    .add_local_listener_with_user_data(ud)
                    .state_changed(move |_, _, _old, new| {
                        // Error states carry the only visible reason when a
                        // connected stream never delivers (no node, no samples).
                        if let pw::stream::StreamState::Error(e) = new {
                            eprintln!("pw audio: stream error {ud_id}: {e}");
                        }
                    })
                    .process(|stream, ud| {
                        if ud.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let Some(mut buffer) = stream.dequeue_buffer() else { return };
                        let datas = buffer.datas_mut();
                        if datas.is_empty() {
                            return;
                        }
                        let data = &mut datas[0];
                        if let Some(bytes) = data.data() {
                            let n = bytes.len() / 4;
                            let mut samples = Vec::with_capacity(n);
                            for i in 0..n {
                                let b: [u8; 4] =
                                    bytes[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]);
                                samples.push(f32::from_le_bytes(b));
                            }
                            ud.sink.push(&ud.id, samples);
                        }
                    })
                    .register()
                {
                    Ok(l) => listeners.push(l),
                    Err(e) => {
                        fail(format!("PipeWire listener {id}: {e}"));
                        return;
                    }
                }

                let mut info = pw::spa::param::audio::AudioInfoRaw::new();
                info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
                info.set_rate(48_000);
                info.set_channels(2);
                let obj = pw::spa::pod::Object {
                    type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                    id: ParamType::EnumFormat.as_raw(),
                    properties: info.into(),
                };
                let values: Vec<u8> = match pw::spa::pod::serialize::PodSerializer::serialize(
                    std::io::Cursor::new(Vec::new()),
                    &pw::spa::pod::Value::Object(obj),
                ) {
                    Ok(v) => v.0.into_inner(),
                    Err(e) => {
                        fail(format!("PipeWire format pod {id}: {e}"));
                        return;
                    }
                };
                let mut params = [Pod::from_bytes(&values).unwrap()];

                if let Err(e) = stream.connect(
                    pw::spa::utils::Direction::Input,
                    None,
                    pw::stream::StreamFlags::AUTOCONNECT
                        | pw::stream::StreamFlags::MAP_BUFFERS
                        | pw::stream::StreamFlags::RT_PROCESS,
                    &mut params,
                ) {
                    fail(format!("PipeWire stream connect {id}: {e}"));
                    return;
                }
                // Request dataflow explicitly (see the video thread).
                if let Err(e) = stream.set_active(true) {
                    fail(format!("PipeWire stream activate {id}: {e}"));
                    return;
                }
                streams.push(stream);
            }

            let _ = ready_tx.send(Ok(()));
            drop(_pw_guard);
            // Same park/stop discipline as the video thread: never tl.wait()
            // (must hold the loop lock; unlocked it misbehaves), park on our
            // own condvar instead.
            let parked = !stop2.load(Ordering::Relaxed);
            if parked {
                tl.start();
                let (lk, cv) = &*wake2;
                let mut stopped = lk.lock().unwrap();
                while !*stopped {
                    stopped = cv.wait(stopped).unwrap();
                }
                tl.stop();
            }
            // Locked teardown (see the video thread).
            {
                let _guard = tl.lock();
                for s in &streams {
                    let _ = s.set_active(false);
                    let _ = s.disconnect();
                }
                drop(listeners);
                drop(streams);
                drop(core);
                drop(context);
            }
            // tl drops here; stopped (or never started), so destroy is clean.
        })
        .map_err(err)?;

    // Fail fast when setup died (or hung); see start_screen.
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        outcome => {
            let msg = match outcome {
                Ok(Err(e)) => e,
                _ => "PipeWire audio setup timed out".to_string(),
            };
            AudioCapture { stop, wake, handle: Some(handle), sink: Some(sink) }.stop();
            return Err(CaptureError::Failed(msg));
        }
    }

    Ok(AudioCapture { stop, wake, handle: Some(handle), sink: Some(sink) })
}

// ---------------------------------------------------------------------------
// enumeration

pub fn list_displays() -> Result<Vec<eztopaz_core::ipc_types::Display>> {
    // Wayland/Portal: the OS picker is the selection surface (design §3.1.2)
    Ok(Vec::new())
}

pub fn list_audio_devices() -> Result<AudioDevices> {
    let inputs: Vec<DeviceInfo> = vec![DeviceInfo {
        id: "default".into(),
        label: "Default Microphone".into(),
        is_default: true,
    }];

    let tl = match unsafe { pw::thread_loop::ThreadLoopBox::new(Some("eztopaz-probe"), None) } {
        Ok(t) => t,
        Err(e) => return Err(err(e)),
    };
    let context = match pw::context::ContextBox::new(tl.loop_(), None) {
        Ok(c) => c,
        Err(e) => return Err(err(e)),
    };
    let Ok(core) = context.connect(None) else {
        return Ok(AudioDevices { inputs, outputs: Vec::new(), apps: Vec::new() });
    };
    let registry = core.get_registry().map_err(err)?;

    struct Ud {
        apps: Vec<AppAudio>,
    }
    let ud = Arc::new(Mutex::new(Ud { apps: Vec::new() }));
    let ud2 = ud.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = global.props else { return };
            let Some(class) = props.get("media.class") else { return };
            if class != "Stream/Output/Audio" {
                return;
            }
            let label = props
                .get("node.description")
                .or_else(|| props.get("application.process.binary"))
                .unwrap_or("unknown")
                .to_string();
            ud2.lock().unwrap().apps.push(AppAudio {
                id: format!("pw:{}", global.id),
                label,
            });
        })
        .register();

    tl.start();
    std::thread::sleep(Duration::from_millis(500)); // ponytail: fixed probe window; sync-callback when it matters
    tl.stop();
    // wait() releases the loop lock while waiting, so it must be called with
    // the guard held (unlocked it is UB and can return immediately).
    // Same locked teardown as the capture threads: dropping proxies without
    // the loop lock trips "called from wrong context" warnings.
    {
        let _guard = tl.lock();
        tl.wait();
        drop(_listener);
        drop(registry);
        drop(core);
        drop(context);
    }

    let apps = ud.lock().unwrap().apps.clone();
    Ok(AudioDevices { inputs, outputs: Vec::new(), apps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eztopaz_core::audio::{AudioSink, Mixer, SourceState};
    use eztopaz_core::config::MicSource;
    use eztopaz_core::ipc_types::AudioSelection;

    fn pw_dump() -> Option<String> {
        std::process::Command::new("pw-dump")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
    }

    fn eztopaz_nodes() -> Vec<String> {
        pw_dump()
            .map(|out| {
                out.lines()
                    .filter(|l| l.contains("eztopaz-"))
                    .take(5)
                    .map(|l| l.trim().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rapid start/stop cycles against the real daemon (when present):
    /// - setup failures surface as Err (fail-fast, never a silent dead capture)
    /// - teardown leaves no zombie eztopaz-* nodes behind
    /// - process listeners stay registered: mixed PCM reaches the file
    /// Without a daemon (CI) only the fail-fast Err path is asserted.
    #[test]
    fn audio_start_stop_cycle() {
        let daemon = pw_dump().is_some();
        let sink_present = pw_dump()
            .map(|out| out.contains("\"media.class\": \"Audio/Sink\""))
            .unwrap_or(false);
        // Snapshot ambient eztopaz-* nodes (another instance may be running);
        // only nodes created by this test may remain afterwards.
        let before = eztopaz_nodes();
        let mut wrote_bytes = 0u64;
        for i in 0..3 {
            let path = std::env::temp_dir().join(format!("eztopaz-test-audio-{i}.pcm"));
            let _ = std::fs::remove_file(&path);
            let file = std::fs::File::create(&path).unwrap();
            let mixer = Arc::new(Mutex::new(Mixer {
                apps: Default::default(),
                mic: SourceState { gain: 1.0, muted: false, enabled: false },
            }));
            let asink = AudioSink::spawn(file, mixer).unwrap();
            let sel = AudioSelection {
                mode: "system".into(),
                apps: Vec::new(),
                mic: MicSource {
                    device: "default".into(),
                    enabled: false,
                    muted: false,
                    gain: 1.0,
                },
            };
            let mut cap = match start_audio(&sel, asink) {
                Ok(c) => c,
                Err(_) => {
                    assert!(!daemon, "start_audio failed despite a live daemon");
                    return;
                }
            };
            std::thread::sleep(Duration::from_millis(800));
            cap.stop();
            wrote_bytes = wrote_bytes.max(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
            let _ = std::fs::remove_file(&path);
        }
        if daemon {
            let leaked: Vec<_> = eztopaz_nodes()
                .into_iter()
                .filter(|n| !before.contains(n))
                .collect();
            assert!(leaked.is_empty(), "leaked PipeWire nodes: {leaked:?}");
            // Callbacks only fire when a monitor exists; without sinks there is
            // nothing to capture, so data flow is asserted only then.
            if sink_present {
                assert!(wrote_bytes > 0, "no PCM flowed: process listeners dead?");
            }
        }
    }
}
