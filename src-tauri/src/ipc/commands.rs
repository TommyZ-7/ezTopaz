//! Tauri IPC commands (design.md §5.1).
//!
//! Capture-dependent commands return a clear error on builds without the
//! platform feature; they come alive when `capture-linux` / `capture-windows`
//! land (design.md §14 spike).

use arboard::Clipboard;
use eztopaz_core::config::{
    self, validate_bitrate, ProfilesConfig, MAX_AUDIO_KBPS, MAX_VIDEO_KBPS,
};
use eztopaz_core::error::Error;
use eztopaz_core::ffmpeg::probe;
use eztopaz_core::ffmpeg::start::prepare;
use eztopaz_core::ffmpeg::supervisor::{ffmpeg_path, StreamProcess};
use eztopaz_core::ipc_types::*;
use std::sync::Mutex;
use std::time::Duration;
use tauri::State;

pub struct AppState {
    pub stream: Mutex<Option<StreamProcess>>,
    pub last_mix: Mutex<AudioMixUpdate>,
}

impl Default for AppState {
    fn default() -> Self {
        Self { stream: Mutex::new(None), last_mix: Mutex::new(AudioMixUpdate::default()) }
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
pub fn start_portal_picker() -> CmdResult<ScreenTarget> {
    #[cfg(feature = "capture-linux")]
    return crate::capture::linux::portal_picker().map_err(err);
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
        validate_bitrate(p.v_kbps, p.a_kbps)
            .map_err(|_| format!("{id}: video {}/audio {}kbps exceeds limits ({}k/{}k)", p.v_kbps, p.a_kbps, MAX_VIDEO_KBPS, MAX_AUDIO_KBPS))?;
    }
    if !cfg.ingest_url.starts_with("rtmp://") {
        return Err("ingest URL must start with rtmp://".into());
    }
    let path = config::config_path();
    config::save(&path, &cfg).map_err(err)
}

// ---------- encoders ----------

/// `ffmpeg -encoders` parse + 1-frame functional test per candidate (design §8.1).
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
    let output = std::process::Command::new(&ffmpeg)
        .arg("-encoders")
        .output()
        .map_err(|e| format!("ffmpeg -encoders failed: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let listed = probe::parse_encoders(&text);

    let mut result = Vec::new();
    for name in probe::AUTO_CANDIDATES {
        if !listed.iter().any(|e| e == name) {
            continue; // not compiled in; UI simply won't show it as auto candidate
        }
        let usable = test_encode_1frame(&ffmpeg, name);
        result.push(EncoderInfo {
            name: name.to_string(),
            usable,
            reason: (!usable).then(|| "1-frame encode test failed (driver missing?)".into()),
        });
    }
    result.push(EncoderInfo { name: "libx264".into(), usable: true, reason: None });
    Ok(result)
}

/// Run the 1-frame encode test with a crude timeout (a hung HW encoder must not
/// freeze the probe).
fn test_encode_1frame(ffmpeg: &std::path::Path, encoder: &str) -> bool {
    let args = probe::test_encode_args(encoder);
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.args(&args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    run_with_timeout(cmd, Duration::from_secs(5)).unwrap_or(false)
}

fn run_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Option<bool> {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let child = cmd.spawn().ok()?;
    std::thread::spawn(move || {
        let mut child = child;
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let _ = tx.send(ok);
    });
    match rx.recv_timeout(timeout) {
        Ok(ok) => Some(ok),
        Err(_) => {
            // best-effort kill
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            None
        }
    }
}

// ---------- stream lifecycle ----------

#[tauri::command]
pub fn start_stream(state: State<'_, AppState>, cfg: StreamConfig) -> CmdResult<StreamStatus> {
    #[allow(unused_mut)] // mut only used by feature builds
    let mut guard = state.stream.lock().unwrap();
    if guard.is_some() {
        return Err("stream already running".into());
    }
    let profiles = config::load(&config::config_path()).map_err(err)?;
    let usable: Vec<String> = probe::AUTO_CANDIDATES.iter().map(|s| s.to_string()).collect();
    // ponytail: assumes all compiled-in candidates work; cache probe_encoders() result instead
    let plan = prepare(&cfg, &profiles, &usable).map_err(err)?;
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

    // Capture backends own the pipe writers; without one, starting would just
    // freeze on empty pipes, so refuse here.
    #[cfg(not(any(feature = "capture-linux", feature = "capture-windows")))]
    {
        let _ = plan;
        Err(Error::NotImplemented("capture backend (build with capture-linux/capture-windows)")).map_err(err)
    }
    #[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
    {
        let ffmpeg = ffmpeg_path();
        let proc = StreamProcess::spawn(&ffmpeg, &plan.ffmpeg_args).map_err(err)?;
        let status = proc.status();
        *guard = Some(proc);
        Ok(status)
    }
}

#[tauri::command]
pub fn stop_stream(state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(mut p) = state.stream.lock().unwrap().take() {
        p.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> StreamStatus {
    let mut guard = state.stream.lock().unwrap();
    match guard.as_mut() {
        Some(p) => {
            let mut st = p.status();
            if let Some(exited) = p.try_wait() {
                if !exited.success() {
                    st.is_live = false;
                }
            }
            st
        }
        None => StreamStatus::default(),
    }
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
