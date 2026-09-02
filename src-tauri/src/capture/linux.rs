//! Linux capture backend: Portal (ashpd) + PipeWire (design.md §3.1.2, §3.2.2).
//!
//! - Screen/window: xdg-desktop-portal ScreenCast. The OS picker is the only
//!   selection path (no app-side window enumeration); the returned PipeWire
//!   node is captured as BGRA frames → scale to profile → [`VideoSink`].
//! - Audio: PipeWire capture streams. system = default sink monitor,
//!   per-app = capture stream targeted at the app's node, mic = source node.
//!
//! Compile verification happens in CI (`cargo check --features capture-linux`
//! with libpipewire-dev); runtime needs a Wayland + PipeWire session.

use super::{CaptureError, Result};
use eztopaz_core::audio::AudioSink;
use eztopaz_core::config::{Profile, ScreenTarget, ScreenTargetKind};
use eztopaz_core::ipc_types::{AppAudio, AudioDevices, AudioSelection, DeviceInfo};
use eztopaz_core::video::{scale_bgra, VideoSink};
use pipewire as pw;
use pw::properties::properties;
use pw::spa::param::ParamType;
use pw::spa::pod::Pod;
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// pw_thread_loop raw pointer; pw_thread_loop_stop is callable cross-thread
    loop_ptr: Arc<AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ScreenCapture {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let ptr = self.loop_ptr.swap(0, Ordering::SeqCst);
        if ptr != 0 {
            unsafe {
                pw::sys::pw_thread_loop_stop(ptr as *mut pw::sys::pw_thread_loop);
            }
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
    _app: tauri::AppHandle,
    target: &ScreenTarget,
    profile: &Profile,
    sink: VideoSink,
) -> Result<ScreenCapture> {
    let state = PORTAL.lock().unwrap().take().ok_or_else(|| {
        CaptureError::Failed("start_portal_picker() を先に実行してください".into())
    })?;
    let node_id = state.node_id;
    let stop = Arc::new(AtomicBool::new(false));
    let loop_ptr = Arc::new(AtomicUsize::new(0));
    let loop_ptr2 = loop_ptr.clone();
    let dst_w = profile.w;
    let dst_h = profile.h;

    let handle = std::thread::Builder::new()
        .name("pw-video".into())
        .spawn(move || {
            let tl = match unsafe { pw::thread_loop::ThreadLoop::new(Some("eztopaz-video"), None) } {
                Ok(t) => t,
                Err(e) => return eprintln!("pw video: {e}"),
            };
            loop_ptr2.store(tl.as_raw_ptr() as usize, Ordering::SeqCst);
            let context = match pw::context::Context::new(&tl) {
                Ok(c) => c,
                Err(e) => return eprintln!("pw video: {e}"),
            };
            let core = match context.connect_fd(state.fd, None) {
                Ok(c) => c,
                Err(e) => return eprintln!("pw video: {e}"),
            };
            let props = properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            };
            let stream = match pw::stream::Stream::new(&core, "eztopaz-video", props) {
                Ok(s) => s,
                Err(e) => return eprintln!("pw video: {e}"),
            };

            struct Ud {
                format: pw::spa::param::video::VideoInfoRaw,
                sink: VideoSink,
                stop: Arc<AtomicBool>,
                dst_w: u32,
                dst_h: u32,
            }
            let ud = Ud {
                format: pw::spa::param::video::VideoInfoRaw::new(),
                sink,
                stop: stop.clone(),
                dst_w,
                dst_h,
            };

            let _listener = stream
                .add_local_listener_with_user_data(ud)
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
                        ud.sink.push(frame);
                    }
                })
                .register();
            if let Err(e) = _listener {
                return eprintln!("pw video: {e}");
            }

            // negotiate BGRA; source size/framerate come back in the negotiated
            // Format (param_changed above). libspa 0.8 has no From<VideoInfoRaw>
            // impl for Vec<Property>, so build the pod with the official macros.
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
                Err(e) => return eprintln!("pw video: {e}"),
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
                return eprintln!("pw video: {e}");
            }

            tl.start();
            tl.wait();
        })
        .map_err(err)?;

    Ok(ScreenCapture { sink, stop, loop_ptr, handle: Some(handle) })
}

// ---------------------------------------------------------------------------
// audio capture

pub struct AudioCapture {
    stop: Arc<AtomicBool>,
    loop_ptr: Arc<AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub sink: Option<AudioSink>,
}

impl AudioCapture {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let ptr = self.loop_ptr.swap(0, Ordering::SeqCst);
        if ptr != 0 {
            unsafe {
                pw::sys::pw_thread_loop_stop(ptr as *mut pw::sys::pw_thread_loop);
            }
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
    let loop_ptr = Arc::new(AtomicUsize::new(0));
    let loop_ptr2 = loop_ptr.clone();
    let stop2 = stop.clone();

    let handle = std::thread::Builder::new()
        .name("pw-audio".into())
        .spawn(move || {
            let tl = match unsafe { pw::thread_loop::ThreadLoop::new(Some("eztopaz-audio"), None) } {
                Ok(t) => t,
                Err(e) => return eprintln!("pw audio: {e}"),
            };
            loop_ptr2.store(tl.as_raw_ptr() as usize, Ordering::SeqCst);
            let context = match pw::context::Context::new(&tl) {
                Ok(c) => c,
                Err(e) => return eprintln!("pw audio: {e}"),
            };
            let core = match context.connect(None) {
                Ok(c) => c,
                Err(e) => return eprintln!("pw audio: {e}"),
            };

            let mut streams = Vec::new();
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
                let stream = match pw::stream::Stream::new(&core, &format!("eztopaz-{id}"), props)
                {
                    Ok(s) => s,
                    Err(e) => return eprintln!("pw audio: {e}"),
                };

                struct Ud {
                    sink: AudioSink,
                    id: String,
                    stop: Arc<AtomicBool>,
                }
                let ud = Ud { sink: sink.clone(), id: id.clone(), stop: stop2.clone() };

                let listener = stream
                    .add_local_listener_with_user_data(ud)
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
                    .register();
                if let Err(e) = listener {
                    return eprintln!("pw audio: {e}");
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
                    Err(e) => return eprintln!("pw audio: {e}"),
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
                    return eprintln!("pw audio: {e}");
                }
                streams.push(stream);
            }

            tl.start();
            tl.wait();
        })
        .map_err(err)?;

    Ok(AudioCapture { stop, loop_ptr, handle: Some(handle), sink: Some(sink) })
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

    let tl = match unsafe { pw::thread_loop::ThreadLoop::new(Some("eztopaz-probe"), None) } {
        Ok(t) => t,
        Err(e) => return Err(err(e)),
    };
    let context = match pw::context::Context::new(&tl) {
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
    tl.wait();

    let apps = ud.lock().unwrap().apps.clone();
    Ok(AudioDevices { inputs, outputs: Vec::new(), apps })
}
