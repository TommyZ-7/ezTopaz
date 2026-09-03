//! Tauri IPC commands (design.md §5.1).
//!
//! Capture-dependent commands come alive with the `capture-linux` /
//! `capture-windows` features; builds without a platform feature return a
//! clear error (and `start_stream` refuses to start so pipes don't freeze).

use arboard::Clipboard;
use eztopaz_core::config::{self, validate_bitrate, Profile, ProfilesConfig, MAX_AUDIO_KBPS, MAX_VIDEO_KBPS};
// Error is only constructed on the non-capture fallback paths
#[cfg_attr(feature = "capture-linux", allow(unused_imports))]
use eztopaz_core::error::Error;
use eztopaz_core::ffmpeg::probe;
use eztopaz_core::ffmpeg::start::{prepare, StartPlan};
use eztopaz_core::ffmpeg::supervisor::{ffmpeg_path, StreamProcess};
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
use eztopaz_core::ffmpeg::supervisor::{retry_backoff_ms, MAX_RETRIES};
use eztopaz_core::ipc_types::*;
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
use eztopaz_core::video::VideoSink;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;

pub struct AppState {
    pub stream: Mutex<Option<StreamProcess>>,
    /// F-ST-04: everything needed to respawn the pipeline after an abnormal exit
    pub session: Mutex<Option<StreamSession>>,
    /// F-ST-04: retry attempt in backoff (Some(n) = "再接続中 n/3")
    pub retrying: Mutex<Option<u32>>,
    pub last_mix: Mutex<AudioMixUpdate>,
    /// live mixer of the running stream (update_audio_mix targets this)
    pub active_mixer: Mutex<Option<Arc<Mutex<eztopaz_core::audio::Mixer>>>>,
    /// pre-stream preview capture (F-SC-03); stopped by start/stop_stream
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    pub preview: Mutex<Option<crate::capture::ScreenCapture>>,
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    pub preview: Mutex<Option<()>>,
    /// Live video leg: plain WGC capture on Linux, `ScreenHandle`
    /// (WGC or ddagrab-direct) on Windows.
    #[cfg(feature = "capture-windows")]
    pub screen: Mutex<Option<crate::capture::windows::ScreenHandle>>,
    #[cfg(feature = "capture-linux")]
    pub screen: Mutex<Option<crate::capture::linux::ScreenCapture>>,
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    pub audio_cap: Mutex<Option<crate::capture::AudioCapture>>,
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    pub screen: Mutex<Option<()>>,
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    pub audio_cap: Mutex<Option<()>>,
}

/// F-ST-04: the live stream's inputs, kept so a retry can respawn the whole
/// pipeline (new pipe servers + capture + ffmpeg) with the same settings.
#[derive(Clone)]
pub struct StreamSession {
    pub cfg: StreamConfig,
    pub plan: StartPlan,
    pub profile: Profile,
    pub mixer: Arc<Mutex<eztopaz_core::audio::Mixer>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            stream: Mutex::new(None),
            session: Mutex::new(None),
            retrying: Mutex::new(None),
            last_mix: Mutex::new(AudioMixUpdate::default()),
            active_mixer: Mutex::new(None),
            preview: Mutex::new(None),
            screen: Mutex::new(None),
            audio_cap: Mutex::new(None),
        }
    }
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn ping() -> String {
    "ezTopaz".into()
}

// ---------- capture (feature-gated backends) ----------

#[tauri::command]
pub fn get_displays() -> CmdResult<Vec<Display>> {
    #[cfg(feature = "capture-windows")]
    return crate::capture::windows::list_displays().map_err(err);
    #[cfg(feature = "capture-linux")]
    return crate::capture::linux::list_displays().map_err(err);
    #[cfg(not(any(feature = "capture-windows", feature = "capture-linux")))]
    Err(Error::NotImplemented("display enumeration (capture feature)")).map_err(err)
}

#[tauri::command]
pub fn get_windows() -> CmdResult<Vec<WindowInfo>> {
    #[cfg(feature = "capture-windows")]
    return crate::capture::windows::list_windows().map_err(err);
    #[cfg(feature = "capture-linux")]
    // Portal environments: no app-side window list; UI shows the picker button.
    return Ok(vec![]);
    #[cfg(not(any(feature = "capture-windows", feature = "capture-linux")))]
    Err(Error::NotImplemented("window enumeration (capture feature)")).map_err(err)
}

#[tauri::command]
pub async fn start_portal_picker() -> CmdResult<ScreenTarget> {
    #[cfg(feature = "capture-linux")]
    return crate::capture::linux::portal_picker().await.map_err(err);
    #[cfg(not(feature = "capture-linux"))]
    Err(Error::NotImplemented("portal picker (capture-linux feature)")).map_err(err)
}

#[tauri::command]
pub fn get_audio_devices() -> CmdResult<AudioDevices> {
    #[cfg(feature = "capture-windows")]
    return crate::capture::windows::list_audio_devices().map_err(err);
    #[cfg(feature = "capture-linux")]
    return crate::capture::linux::list_audio_devices().map_err(err);
    #[cfg(not(any(feature = "capture-windows", feature = "capture-linux")))]
    Err(Error::NotImplemented("audio device enumeration (capture feature)")).map_err(err)
}

// ---------- config ----------

#[tauri::command]
pub fn get_profiles() -> CmdResult<ProfilesConfig> {
    config::load(&config::config_path()).map_err(err)
}

#[tauri::command]
pub fn save_profiles(cfg: ProfilesConfig) -> CmdResult<()> {
    // F-EN-04: guard every profile against Topaz limits before persisting
    for (id, p) in &cfg.profiles {
        validate_bitrate(p.v_kbps, p.a_kbps).map_err(|_| {
            format!("{id}: video {}/audio {}kbps exceeds limits ({}k/{}k)", p.v_kbps, p.a_kbps, MAX_VIDEO_KBPS, MAX_AUDIO_KBPS)
        })?;
    }
    if !cfg.ingest_url.starts_with("rtmp://") {
        return Err("ingest URL must start with rtmp://".into());
    }
    let path = config::config_path();
    config::save(&path, &cfg).map_err(err)
}

// ---------- encoders ----------

/// `ffmpeg -encoders` parse + 1-frame functional test per candidate (design §8.1).
///
/// Second and later launches hit the `encoders.json` cache (keyed by ffmpeg
/// identity + `-version`), so startup costs a single fast `-version` spawn
/// instead of up to 4 slow GPU-init test encodes. The functional tests run
/// in parallel via `thread::scope`, bounding a cold start by one timeout
/// instead of the sum.
#[tauri::command]
pub fn probe_encoders() -> CmdResult<Vec<EncoderInfo>> {
    let ffmpeg = ffmpeg_path();
    if ffmpeg.as_os_str() != "ffmpeg" && !ffmpeg.exists() {
        return Ok(vec![EncoderInfo {
            name: "libx264".into(),
            usable: false,
            reason: Some("ffmpeg binary not found".into()),
        }]);
    }
    let version = ffmpeg_version(&ffmpeg);
    let cache_path = probe::encoder_cache_path();
    if let Some(cached) = probe::load_cached_encoders(&cache_path, &ffmpeg, &version) {
        return Ok(cached);
    }
    let output = ffmpeg_cmd(&ffmpeg)
        .arg("-encoders")
        .output()
        .map_err(|e| format!("ffmpeg -encoders failed: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let listed = probe::parse_encoders(&text);

    let candidates: Vec<&str> = probe::AUTO_CANDIDATES
        .iter()
        .copied()
        .filter(|name| listed.iter().any(|e| e == name))
        .collect();
    // Parallel functional tests; cold start costs max(1 test), not the sum.
    let mut checked: Vec<(usize, bool)> = std::thread::scope(|s| {
        candidates
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let ffmpeg = ffmpeg.clone();
                s.spawn(move || (i, test_encode_1frame(&ffmpeg, name)))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap_or((0, false)))
            .collect()
    });
    checked.sort_by_key(|(i, _)| *i);
    let mut result: Vec<EncoderInfo> = checked
        .into_iter()
        .map(|(i, usable)| EncoderInfo {
            name: candidates[i].to_string(),
            usable,
            reason: (!usable).then(|| "1-frame encode test failed (driver missing?)".into()),
        })
        .collect();
    result.push(EncoderInfo { name: "libx264".into(), usable: true, reason: None });
    let _ = probe::save_cached_encoders(&cache_path, &ffmpeg, &version, &result);
    Ok(result)
}

/// Hidden ffmpeg spawn: no console window flashes on Windows (B-1).
fn ffmpeg_cmd(ffmpeg: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// First line of `ffmpeg -version` for cache invalidation; "" when unknown
/// (falls through to a fresh probe instead of trusting a stale cache).
fn ffmpeg_version(ffmpeg: &std::path::Path) -> String {
    ffmpeg_cmd(ffmpeg)
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .unwrap_or_default()
}

fn test_encode_1frame(ffmpeg: &std::path::Path, encoder: &str) -> bool {
    let args = probe::test_encode_args(encoder);
    let mut cmd = ffmpeg_cmd(ffmpeg);
    cmd.args(&args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    run_with_timeout(cmd, Duration::from_secs(5)).unwrap_or(false)
}

fn run_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Option<bool> {
    let mut child = cmd.spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    // Covers Windows too (the old mpsc version only killed on unix,
                    // leaking hung ffmpeg test processes + their consoles).
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// Kill a pipeline ffmpeg that failed mid-launch and report its stderr tail
/// with the context. A bare pipe-timeout hides the real cause (e.g. an
/// unusable HW encoder dying at init); the tail shows it.
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
fn pipeline_error(proc: &mut StreamProcess, ctx: impl std::fmt::Display) -> String {
    // grace period for the stderr-drain thread to flush pending lines —
    // without it the tail below (and the log file) is often empty.
    std::thread::sleep(Duration::from_millis(300));
    let tail = proc.tail_stderr(20);
    proc.stop();
    if tail.is_empty() {
        ctx.to_string()
    } else {
        format!("{ctx}; ffmpeg stderr:\n{}", tail.join("\n"))
    }
}

/// Fail fast when ffmpeg already exited (bad argv, missing encoder, ...):
/// report its stderr instead of waiting out the pipe timeout.
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
fn check_ffmpeg_alive(proc: &mut StreamProcess) -> Result<(), String> {
    if proc.try_wait().is_some() {
        return Err(pipeline_error(proc, "ffmpeg exited before opening pipes"));
    }
    Ok(())
}

// ---------- stream lifecycle ----------

/// Spawn ffmpeg and wire the capture backends to it (design §4.1). Used by
/// `start_stream` and the F-ST-04 retry loop; a retry regenerates the pipe
/// servers because one named-pipe server instance serves exactly one client.
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
fn launch_pipeline(
    app: &tauri::AppHandle,
    state: &AppState,
    sess: &StreamSession,
    retry: u32,
) -> Result<StreamProcess, String> {
    use eztopaz_core::audio::AudioSink;
    use eztopaz_core::ffmpeg::pipes;

    // (re)generate the pipe servers/clients before the ffmpeg spawn
    pipes::create(&sess.plan.video_pipe).map_err(err)?;
    pipes::create(&sess.plan.audio_pipe).map_err(err)?;

    let ffmpeg = ffmpeg_path();

    // C10: ddagrab direct video input (Windows fullscreen only). The video
    // pipe/sink/capture are bypassed; audio still flows through its pipe.
    #[cfg(feature = "capture-windows")]
    let direct = eztopaz_core::ffmpeg::hw::parse_direct_input(sess.cfg.direct_input.as_deref());

    #[cfg(feature = "capture-windows")]
    let mut proc = match &direct {
        Some(_) => {
            let argv = eztopaz_core::ffmpeg::hw::build_direct_args(
                &sess.profile,
                &sess.plan.encoder,
                &sess.cfg,
                &sess.plan.audio_pipe,
            )
            .map_err(err)?;
            StreamProcess::spawn(&ffmpeg, &argv).map_err(err)?
        }
        None => StreamProcess::spawn(&ffmpeg, &sess.plan.ffmpeg_args).map_err(err)?,
    };
    #[cfg(feature = "capture-linux")]
    let mut proc = StreamProcess::spawn(&ffmpeg, &sess.plan.ffmpeg_args).map_err(err)?;

    // Pipe open order: ffmpeg opens the video input first and reads probe
    // data before it opens the audio input, so the video leg must be
    // connected AND feeding before the audio writer can connect. Waiting
    // for both writers up front deadlocks (no sink exists yet to feed
    // video) and ends in PIPE_OPEN_TIMEOUT + kill. Timeout/error kills the
    // spawned ffmpeg and returns its stderr tail with the context.
    #[cfg(feature = "capture-linux")]
    {
        check_ffmpeg_alive(&mut proc)?;
        let video_writer =
            pipes::open_writer_timeout(&sess.plan.video_pipe, pipes::PIPE_OPEN_TIMEOUT)
                .map_err(|e| pipeline_error(&mut proc, err(e)))?;
        // Pipe format (BGRA/NV12) always matches ffmpeg argv via plan.transport.
        let vsink = VideoSink::spawn_for_transport(
            video_writer,
            sess.profile.w,
            sess.profile.h,
            sess.profile.fps,
            sess.plan.transport,
        )
        .map_err(|e| pipeline_error(&mut proc, err(e)))?;
        let screen =
            crate::capture::linux::start_screen(app.clone(), &sess.profile, vsink)
                .map_err(|e| pipeline_error(&mut proc, err(e)))?;
        *state.screen.lock().unwrap() = Some(screen);
        // video data now flows; ffmpeg proceeds to the audio input.
        if let Err(e) = check_ffmpeg_alive(&mut proc) {
            stop_capture_backends(state);
            return Err(e);
        }
        let audio_writer =
            pipes::open_writer_timeout(&sess.plan.audio_pipe, pipes::PIPE_OPEN_TIMEOUT)
                .map_err(|e| {
                    stop_capture_backends(state);
                    pipeline_error(&mut proc, err(e))
                })?;
        let asink = AudioSink::spawn(audio_writer, sess.mixer.clone()).map_err(|e| {
            stop_capture_backends(state);
            pipeline_error(&mut proc, err(e))
        })?;
        let audio_cap =
            crate::capture::linux::start_audio(&sess.cfg.audio, asink).map_err(|e| {
                stop_capture_backends(state);
                pipeline_error(&mut proc, err(e))
            })?;
        *state.audio_cap.lock().unwrap() = Some(audio_cap);
    }
    #[cfg(feature = "capture-windows")]
    {
        use crate::capture::windows::ScreenHandle;
        if direct.is_some() {
            // ddagrab: no video pipe; the single audio connect is still
            // bounded so a missing ffmpeg reader can't freeze start_stream.
            check_ffmpeg_alive(&mut proc)?;
            let audio_writer =
                pipes::open_writer_timeout(&sess.plan.audio_pipe, pipes::PIPE_OPEN_TIMEOUT)
                    .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            let asink = AudioSink::spawn(audio_writer, sess.mixer.clone())
                .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            let audio_cap = crate::capture::windows::start_audio(&sess.cfg.audio, asink)
                .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            *state.screen.lock().unwrap() = Some(ScreenHandle::Direct);
            *state.audio_cap.lock().unwrap() = Some(audio_cap);
        } else {
            // Same video-first order as Linux: the video leg must feed
            // before ffmpeg opens the audio input (see above).
            check_ffmpeg_alive(&mut proc)?;
            let video_writer =
                pipes::open_writer_timeout(&sess.plan.video_pipe, pipes::PIPE_OPEN_TIMEOUT)
                    .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            // Pipe format (BGRA/NV12) always matches ffmpeg argv via plan.transport.
            let vsink = VideoSink::spawn_for_transport(
                video_writer,
                sess.profile.w,
                sess.profile.h,
                sess.profile.fps,
                sess.plan.transport,
            )
            .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            let screen = crate::capture::windows::start_screen(
                app.clone(),
                &sess.cfg.screen,
                &sess.profile,
                vsink,
                false,
            )
            .map_err(|e| pipeline_error(&mut proc, err(e)))?;
            *state.screen.lock().unwrap() = Some(ScreenHandle::Wgc(screen));
            if let Err(e) = check_ffmpeg_alive(&mut proc) {
                stop_capture_backends(state);
                return Err(e);
            }
            let audio_writer =
                pipes::open_writer_timeout(&sess.plan.audio_pipe, pipes::PIPE_OPEN_TIMEOUT)
                    .map_err(|e| {
                        stop_capture_backends(state);
                        pipeline_error(&mut proc, err(e))
                    })?;
            let asink = AudioSink::spawn(audio_writer, sess.mixer.clone()).map_err(|e| {
                stop_capture_backends(state);
                pipeline_error(&mut proc, err(e))
            })?;
            let audio_cap = crate::capture::windows::start_audio(&sess.cfg.audio, asink)
                .map_err(|e| {
                    stop_capture_backends(state);
                    pipeline_error(&mut proc, err(e))
                })?;
            *state.audio_cap.lock().unwrap() = Some(audio_cap);
        }
    }

    proc.retry_count = retry;
    Ok(proc)
}

fn stop_capture_backends(state: &AppState) {
    let _ = state;
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    {
        if let Some(mut s) = state.screen.lock().unwrap().take() {
            s.stop();
        }
        if let Some(mut a) = state.audio_cap.lock().unwrap().take() {
            a.stop();
        }
    }
}

/// Take the preview slot's mutex guard across stop so concurrent
/// start_preview/stop_preview commands serialize (Tauri invokes run on a
/// thread pool): stop + replace + store is atomic, never interleaved.
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
fn take_preview_slot(
    state: &AppState,
) -> std::sync::MutexGuard<'_, Option<crate::capture::ScreenCapture>> {
    let mut slot = state.preview.lock().unwrap();
    if let Some(mut p) = slot.take() {
        p.stop();
    }
    slot
}

fn stop_preview_impl(state: &AppState) {
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    {
        // take() + stop() under one guard; the old capture is fully joined
        // before this returns, so a following start cannot overlap it.
        let _slot = take_preview_slot(state);
    }
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    let _ = state;
}

#[tauri::command]
pub fn start_stream(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    cfg: StreamConfig,
) -> CmdResult<StreamStatus> {
    // sync commands run serialized; the check + store below is race-free
    if state.stream.lock().unwrap().is_some() {
        return Err("stream already running".into());
    }
    stop_preview_impl(&state); // preview must release the capture backends

    let profiles = config::load(&config::config_path()).map_err(err)?;
    let profile = profiles
        .profiles
        .get(&cfg.profile_id)
        .cloned()
        .ok_or_else(|| format!("unknown profile: {}", cfg.profile_id))?;
    let usable: Vec<String> = if cfg.encoder_override == "auto" || cfg.encoder_override.is_empty() {
        // Auto mode resolves against functionally verified encoders only.
        // Assuming all compiled-in candidates work picks e.g. h264_nvenc on
        // driverless machines, whose ffmpeg then dies before opening the
        // pipes (timeout + kill). probe_encoders() is cache-backed, so this
        // costs a single `-version` spawn on warm starts.
        probe_encoders()
            .map(|infos| infos.into_iter().filter(|i| i.usable).map(|i| i.name).collect())
            .unwrap_or_default() // probe failed: probe_best falls back to libx264
    } else {
        // manual override is used as-is by build_plan; usability is the user's choice
        Vec::new()
    };
    let plan = prepare(&cfg, &profiles, &usable).map_err(err)?;

    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    {
        // no capture backend in this build: refuse rather than stream frozen pipes
        let _ = (&app, &state, &profile, &plan);
        return Err(Error::NotImplemented("streaming (capture feature)")).map_err(err);
    }

    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    {
        // ddagrab direct input exists only on Windows; fail loudly instead
        // of silently falling back to the pipe path.
        #[cfg(feature = "capture-linux")]
        if cfg.direct_input.is_some() {
            return Err("direct input (ddagrab) is Windows-only".into());
        }
        // initial mixer state from the UI selection
        let mixer = Arc::new(Mutex::new(eztopaz_core::audio::Mixer {
            apps: cfg
                .audio
                .apps
                .iter()
                .map(|a| {
                    (
                        a.clone(),
                        eztopaz_core::audio::SourceState { gain: 1.0, muted: false, enabled: true },
                    )
                })
                .collect(),
            mic: eztopaz_core::audio::SourceState {
                gain: cfg.audio.mic.gain,
                muted: cfg.audio.mic.muted,
                enabled: cfg.audio.mic.enabled,
            },
        }));
        *state.last_mix.lock().unwrap() = AudioMixUpdate {
            apps: cfg
                .audio
                .apps
                .iter()
                .map(|a| (a.clone(), SourceGain { gain: 1.0, muted: false }))
                .collect(),
            mic: MicUpdate {
                enabled: cfg.audio.mic.enabled,
                muted: cfg.audio.mic.muted,
                gain: cfg.audio.mic.gain,
            },
        };

        *state.session.lock().unwrap() = Some(StreamSession {
            cfg,
            plan,
            profile: profile.clone(),
            mixer: mixer.clone(),
        });
        let sess = state.session.lock().unwrap();
        let proc = launch_pipeline(&app, &state, sess.as_ref().expect("session just set"), 0)?;
        drop(sess);
        *state.active_mixer.lock().unwrap() = Some(mixer);
        *state.retrying.lock().unwrap() = None;

        let status = proc.status();
        *state.stream.lock().unwrap() = Some(proc);
        Ok(status)
    }
}

#[tauri::command]
pub fn stop_stream(state: State<'_, AppState>) -> CmdResult<()> {
    *state.retrying.lock().unwrap() = None; // cancels a pending F-ST-04 retry
    stop_capture_backends(&state);
    *state.active_mixer.lock().unwrap() = None;
    if let Some(mut p) = state.stream.lock().unwrap().take() {
        p.stop();
    }
    *state.session.lock().unwrap() = None;
    stop_preview_impl(&state);
    Ok(())
}

#[tauri::command]
pub fn get_status(app: tauri::AppHandle, state: State<'_, AppState>) -> StreamStatus {
    let retrying = state.retrying.lock().unwrap().clone();
    let mut guard = state.stream.lock().unwrap();
    let Some(p) = guard.as_mut() else {
        // between retries (backoff) or fully stopped
        return StreamStatus { retrying, ..Default::default() };
    };
    let mut st = p.status();
    let mut give_up = false;
    if let Some(exited) = p.try_wait() {
        if exited.success() {
            st.is_live = false;
        } else {
            #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
            {
                // F-ST-04: abnormal exit → retry with exponential backoff
                if p.retry_count < MAX_RETRIES && state.session.lock().unwrap().is_some() {
                    if retrying.is_none() {
                        let next = p.retry_count + 1;
                        *state.retrying.lock().unwrap() = Some(next);
                        p.status.lock().unwrap().retrying = Some(next);
                        st.retrying = Some(next);
                        spawn_retry_thread(app, next);
                    }
                } else if retrying.is_none() {
                    // retries exhausted (or nothing to retry with) → give up
                    give_up = true;
                }
            }
            #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
            {
                let _ = app;
                give_up = true;
            }
        }
    }
    if give_up {
        st.is_live = false;
        *state.session.lock().unwrap() = None;
        *guard = None; // drop the dead process; stop_stream still works
    }
    st
}

/// F-ST-04: respawn the whole pipeline (pipe servers → capture → ffmpeg) with
/// exponential backoff, at most MAX_RETRIES times; 「再接続中 n/3」 via retrying.
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
fn spawn_retry_thread(app: tauri::AppHandle, first_retry: u32) {
    std::thread::Builder::new()
        .name("stream-retry".into())
        .spawn(move || {
            use tauri::Manager;
            let state = app.state::<AppState>();
            for n in first_retry..=MAX_RETRIES {
                std::thread::sleep(Duration::from_millis(retry_backoff_ms(n - 1)));
                // user stop cancels the pending retry
                if state.retrying.lock().unwrap().is_none() {
                    return;
                }
                let Some(sess) = state.session.lock().unwrap().clone() else { return };
                // the broken writers died with the old ffmpeg; rebuild everything
                stop_capture_backends(&state);
                match launch_pipeline(&app, &state, &sess, n) {
                    Ok(proc) => {
                        *state.stream.lock().unwrap() = Some(proc);
                        *state.retrying.lock().unwrap() = None;
                        return;
                    }
                    Err(e) => {
                        eprintln!("stream retry {n}/{MAX_RETRIES} failed: {e}");
                    }
                }
            }
            // retries exhausted → stop (design §9: 3回失敗で停止)
            *state.retrying.lock().unwrap() = None;
            stop_capture_backends(&state);
            *state.session.lock().unwrap() = None;
            *state.stream.lock().unwrap() = None;
        })
        .ok();
}

// ---------- pre-stream preview (F-SC-03) ----------

/// Capture-only preview: no ffmpeg, no pipes — the capture backends emit
/// `stream://preview` (640x360 PNG @1fps, design §6.4) themselves. Runs until
/// `stop_preview` / `start_stream` / `stop_stream`.
#[tauri::command]
pub fn start_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    cfg: StreamConfig,
) -> CmdResult<()> {
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    {
        let _ = (app, state, cfg);
        return Err(Error::NotImplemented("preview (capture feature)")).map_err(err);
    }
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    {
        if state.stream.lock().unwrap().is_some() {
            return Err("stream already running".into());
        }
        // Hold the preview slot across stop + replace + store so a concurrent
        // start_preview/stop_preview cannot interleave and leak a capture.
        let mut slot = take_preview_slot(&state);
        let profiles = config::load(&config::config_path()).map_err(err)?;
        let profile = profiles
            .profiles
            .get(&cfg.profile_id)
            .cloned()
            .ok_or_else(|| format!("unknown profile: {}", cfg.profile_id))?;
        let preview_profile = Profile { w: 640, h: 360, fps: 1, ..profile };
        let vsink = VideoSink::spawn(
            crate::capture::null_video_writer(),
            preview_profile.w,
            preview_profile.h,
            preview_profile.fps,
        )
        .map_err(err)?;
        let screen = {
            #[cfg(feature = "capture-linux")]
            {
                crate::capture::linux::start_screen(app, &preview_profile, vsink).map_err(err)?
            }
            #[cfg(feature = "capture-windows")]
            {
                crate::capture::windows::start_screen(
                    app,
                    &cfg.screen,
                    &preview_profile,
                    vsink,
                    false,
                )
                .map_err(err)?
            }
        };
        *slot = Some(screen);
        Ok(())
    }
}

#[tauri::command]
pub fn stop_preview(state: State<'_, AppState>) -> CmdResult<()> {
    stop_preview_impl(&state);
    Ok(())
}

#[tauri::command]
pub fn get_vu(state: State<'_, AppState>) -> VuMeter {
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    let _ = state;
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    if let Some(cap) = state.audio_cap.lock().unwrap().as_ref() {
        if let Some(sink) = &cap.sink {
            return sink.last_vu.lock().unwrap().clone();
        }
    }
    VuMeter::default()
}

#[tauri::command]
pub fn update_audio_mix(state: State<'_, AppState>, mix: AudioMixUpdate) -> CmdResult<()> {
    for g in mix.apps.values() {
        if !(0.0..=2.0).contains(&g.gain) {
            return Err(format!("gain out of range: {}", g.gain));
        }
    }
    if !(0.0..=2.0).contains(&mix.mic.gain) {
        return Err(format!("gain out of range: {}", mix.mic.gain));
    }
    if let Some(mixer) = state.active_mixer.lock().unwrap().as_ref() {
        let mut m = mixer.lock().unwrap();
        for (id, g) in &mix.apps {
            m.apps.insert(
                id.clone(),
                eztopaz_core::audio::SourceState {
                    gain: g.gain,
                    muted: g.muted,
                    enabled: true,
                },
            );
        }
        m.mic = eztopaz_core::audio::SourceState {
            gain: mix.mic.gain,
            muted: mix.mic.muted,
            enabled: mix.mic.enabled,
        };
    }
    *state.last_mix.lock().unwrap() = mix;
    Ok(())
}

// ---------- misc ----------

#[tauri::command]
pub fn copy_to_clipboard(text: String) -> CmdResult<()> {
    let mut cb = Clipboard::new().map_err(err)?;
    cb.set_text(text).map_err(err)
}

#[tauri::command]
pub fn open_logs_dir() -> CmdResult<()> {
    let dir = config::config_dir().join("logs");
    std::fs::create_dir_all(&dir).map_err(err)?;
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(&dir).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&dir).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&dir).spawn();
    r.map_err(err)?;
    Ok(())
}
