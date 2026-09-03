//! Named pipe setup for the FFmpeg video/audio inputs (design.md §4.1).
//!
//! Unix: FIFOs (`mkfifo`). Windows: named-pipe servers (`CreateNamedPipeW`) —
//! Rust writes, FFmpeg connects as the client. `create` runs before the FFmpeg
//! spawn, `open_writer` blocks until the client is connected.

use crate::error::{Error, Result};
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;

pub fn video_pipe_path() -> String {
    if cfg!(windows) {
        r"\\.\pipe\ezTopaz_video".into()
    } else {
        "/tmp/ezTopaz_video.pipe".into()
    }
}

pub fn audio_pipe_path() -> String {
    if cfg!(windows) {
        r"\\.\pipe\ezTopaz_audio".into()
    } else {
        "/tmp/ezTopaz_audio.pipe".into()
    }
}

/// Create the pipe endpoints. Idempotent (existing pipe of same path is reused).
///
/// Unix: a FIFO (`mkfifo`). Windows: a named-pipe SERVER instance the Rust side
/// writes to; [`open_writer`] later hands the server handle to the sink once
/// FFmpeg (the client) has connected (design §4.1). Re-connects regenerate the
/// instance because a server instance is consumed by one client connection.
pub fn create(path: &str) -> Result<()> {
    #[cfg(unix)]
    {
        let p = Path::new(path);
        if p.exists() {
            return Ok(());
        }
        let cpath = std::ffi::CString::new(path).map_err(|_| Error::Ffmpeg("pipe path NUL".into()))?;
        // 0o660: owner+group RW
        let rc = unsafe { libc::mkfifo(cpath.as_ptr(), 0o660) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(err.into());
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    server::create(path)
}

/// Open a writer to the pipe. On unix this blocks until the reader (FFmpeg) opens
/// the FIFO, so call it from a dedicated thread after FFmpeg spawn; on Windows it
/// blocks in `ConnectNamedPipe` until FFmpeg connects. Same contract, both sides.
///
/// Callers must open video-first and have the video sink feeding before
/// waiting on the audio writer: ffmpeg opens the video input first and reads
/// probe data before it opens the audio input, so an audio wait with no
/// video data flowing never resolves. Prefer [`open_writer_timeout`] per
/// pipe in that order over [`open_writers_concurrently`].
pub fn open_writer(path: &str) -> Result<std::fs::File> {
    #[cfg(unix)]
    {
        std::fs::OpenOptions::new().write(true).open(path).map_err(Into::into)
    }
    #[cfg(windows)]
    server::open_writer(path)
}

/// How long a pipe-writer connect waits for ffmpeg before giving up (covers
/// ffmpeg spawn/init, normally 1-2s). Bounds `start_stream` instead of hanging
/// the main thread forever when ffmpeg never opens its read end.
pub const PIPE_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Open one pipe writer on a helper thread; `Err` on timeout. The helper
/// thread stays blocked (detached) on timeout, so treat timeouts as fatal
/// for the pipeline (kill ffmpeg) rather than retrying the same pipes.
pub fn open_writer_timeout(path: &str, timeout: Duration) -> Result<std::fs::File> {
    let owned = path.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("pipe-open-{owned}"))
        .spawn(move || {
            let _ = tx.send(open_writer(&owned));
        })
        .map_err(|e| Error::Ffmpeg(format!("pipe open thread failed: {e}")))?;
    rx.recv_timeout(timeout)
        .map_err(|_| {
            Error::Ffmpeg(format!("timed out waiting for ffmpeg to open pipe {path}"))
        })?
}

/// Open both pipe writers concurrently; returns `(video, audio)`.
///
/// Only fits readers that open both inputs without reading first (like the
/// test fake below). Real ffmpeg reads video probe data before opening the
/// audio input, so concurrent opens stall when nothing feeds video yet —
/// open video-first with [`open_writer_timeout`], start the video sink, then
/// open audio. The timeout turns a missing ffmpeg reader into a clear error
/// instead of a hang. Total wait is bounded by `timeout` across both pipes.
pub fn open_writers_concurrently(
    video: &str,
    audio: &str,
    timeout: Duration,
) -> Result<(std::fs::File, std::fs::File)> {
    let deadline = std::time::Instant::now() + timeout;
    let (vtx, vrx) = std::sync::mpsc::channel();
    let (atx, arx) = std::sync::mpsc::channel();
    let (vpath, apath) = (video.to_string(), audio.to_string());
    std::thread::Builder::new()
        .name(format!("pipe-open-{vpath}"))
        .spawn(move || {
            let _ = vtx.send(open_writer(&vpath));
        })
        .map_err(|e| Error::Ffmpeg(format!("pipe open thread failed: {e}")))?;
    std::thread::Builder::new()
        .name(format!("pipe-open-{apath}"))
        .spawn(move || {
            let _ = atx.send(open_writer(&apath));
        })
        .map_err(|e| Error::Ffmpeg(format!("pipe open thread failed: {e}")))?;
    // Fast-fail a broken pipe without waiting out the deadline for the other.
    let video_writer = vrx
        .recv_timeout(timeout)
        .map_err(|_| {
            Error::Ffmpeg(format!("timed out waiting for ffmpeg to open video pipe {video}"))
        })??;
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let audio_writer = arx
        .recv_timeout(remaining)
        .map_err(|_| {
            Error::Ffmpeg(format!("timed out waiting for ffmpeg to open audio pipe {audio}"))
        })??;
    Ok((video_writer, audio_writer))
}

#[cfg(windows)]
mod server {
    //! Named-pipe servers (Rust writes, FFmpeg reads as the client).
    use super::{Error, Result};
    use std::collections::HashMap;
    use std::os::windows::io::{FromRawHandle, IntoRawHandle, OwnedHandle, RawHandle};
    use std::sync::Mutex;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_OUTBOUND;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    /// Server instances created by [`super::create`] and consumed by
    /// [`super::open_writer`]. One instance serves exactly one client.
    static SERVERS: Mutex<Option<HashMap<String, OwnedHandle>>> = Mutex::new(None);

    pub(super) fn create(path: &str) -> Result<()> {
        let mut servers = SERVERS.lock().unwrap();
        let map = servers.get_or_insert_with(HashMap::new);
        if map.contains_key(path) {
            return Ok(()); // idempotent, matches the unix exists() shortcut
        }
        unsafe {
            let handle = CreateNamedPipeW(
                &HSTRING::from(path),
                PIPE_ACCESS_OUTBOUND,                                  // server writes…
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,                                                     // single client (ffmpeg)
                1 << 20,                                               // out buffer
                4096,                                                  // in buffer
                0,                                                     // default wait timeout
                None,
            );
            if handle.is_invalid() {
                let err = std::io::Error::last_os_error();
                return Err(Error::Ffmpeg(format!("CreateNamedPipeW({path}) failed: {err}")));
            }
            map.insert(path.to_string(), OwnedHandle::from_raw_handle(handle.0 as RawHandle));
        }
        Ok(())
    }

    pub(super) fn open_writer(path: &str) -> Result<std::fs::File> {
        let raw: RawHandle = {
            let mut servers = SERVERS.lock().unwrap();
            let handle = servers
                .get_or_insert_with(HashMap::new)
                .remove(path)
                .ok_or_else(|| Error::Ffmpeg(format!("pipe server for {path} was not created")))?;
            handle.into_raw_handle()
        };
        unsafe {
            // blocks until ffmpeg opens its read end (ERROR_PIPE_CONNECTED when
            // the client is already there)
            if let Err(e) = ConnectNamedPipe(HANDLE(raw as isize), None) {
                if e.code() != windows::core::HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                    return Err(Error::Ffmpeg(format!("ConnectNamedPipe({path}) failed: {e}")));
                }
            }
            Ok(std::fs::File::from_raw_handle(raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn creates_fifo_idempotently() {
        let dir = std::env::temp_dir().join(format!("eztopaz-pipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.pipe");
        let path_str = path.to_str().unwrap();
        create(path_str).unwrap();
        create(path_str).unwrap(); // second call is a no-op
        let meta = std::fs::metadata(&path).unwrap();
        use std::os::unix::fs::FileTypeExt;
        assert!(meta.file_type().is_fifo(), "created file is a FIFO");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_writers_connect_video_first_without_deadlock() {
        // Regression test for the start_stream freeze: ffmpeg opens the video
        // input before the audio input, so an audio-first sequential open
        // deadlocks (each side waits for the other). Concurrent opens must
        // connect against a video-first reader.
        let dir = std::env::temp_dir().join(format!("eztopaz-pipe-conc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.pipe");
        let audio = dir.join("audio.pipe");
        create(video.to_str().unwrap()).unwrap();
        create(audio.to_str().unwrap()).unwrap();
        // Fake ffmpeg: readers opened video-first, like the ffmpeg argv order.
        let (tx, rx) = std::sync::mpsc::channel();
        let (v, a) = (video.clone(), audio.clone());
        std::thread::spawn(move || {
            let vr = std::fs::OpenOptions::new().read(true).open(&v).unwrap();
            let ar = std::fs::OpenOptions::new().read(true).open(&a).unwrap();
            tx.send(()).unwrap();
            drop((vr, ar));
        });
        let (vw, aw) = open_writers_concurrently(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            Duration::from_secs(10),
        )
        .unwrap();
        drop((vw, aw));
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_open_times_out_without_readers() {
        // No ffmpeg reader: must error, not block forever. (The helper
        // threads stay blocked on the FIFO opens until process exit; the
        // caller must treat this as fatal and kill ffmpeg.)
        let dir = std::env::temp_dir()
            .join(format!("eztopaz-pipe-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.pipe");
        let audio = dir.join("audio.pipe");
        create(video.to_str().unwrap()).unwrap();
        create(audio.to_str().unwrap()).unwrap();
        let err = open_writers_concurrently(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            Duration::from_millis(500),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Ffmpeg(_)), "unexpected error: {err}");
        assert!(open_writer_timeout(video.to_str().unwrap(), Duration::from_millis(200)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_open_stalls_without_video_data() {
        // Faithful ffmpeg contract: the reader opens video, reads probe
        // bytes, and only then opens audio. Concurrent opens stall here
        // because no sink exists yet to feed video — this was the
        // start_stream deadlock (PIPE_OPEN_TIMEOUT + kill). Deterministic:
        // audio can never connect, so a short timeout must fail.
        let dir = std::env::temp_dir()
            .join(format!("eztopaz-pipe-ffmpeg-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.pipe");
        let audio = dir.join("audio.pipe");
        create(video.to_str().unwrap()).unwrap();
        create(audio.to_str().unwrap()).unwrap();
        let (v, a) = (video.clone(), audio.clone());
        std::thread::spawn(move || {
            let mut vr = std::fs::OpenOptions::new().read(true).open(&v).unwrap();
            let mut probe = [0u8; 64];
            use std::io::Read;
            let _ = vr.read_exact(&mut probe); // blocks: nobody feeds video yet
            let ar = std::fs::OpenOptions::new().read(true).open(&a).unwrap();
            drop((vr, ar));
        });
        let err = open_writers_concurrently(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Ffmpeg(_)), "unexpected error: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn video_first_with_feed_connects_both() {
        // The launch_pipeline order: open video, feed it (video sink), then
        // open audio. Against the same probe-reading fake ffmpeg this must
        // connect both writers promptly.
        let dir = std::env::temp_dir()
            .join(format!("eztopaz-pipe-video-first-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.pipe");
        let audio = dir.join("audio.pipe");
        create(video.to_str().unwrap()).unwrap();
        create(audio.to_str().unwrap()).unwrap();
        let (v, a) = (video.clone(), audio.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut vr = std::fs::OpenOptions::new().read(true).open(&v).unwrap();
            let mut probe = [0u8; 64];
            use std::io::Read;
            vr.read_exact(&mut probe).unwrap();
            let ar = std::fs::OpenOptions::new().read(true).open(&a).unwrap();
            tx.send(()).unwrap();
            drop((vr, ar));
        });
        let mut vw = open_writer_timeout(video.to_str().unwrap(), Duration::from_secs(10)).unwrap();
        // video sink starts feeding right after the video connect
        use std::io::Write;
        vw.write_all(&[0u8; 4096]).unwrap();
        vw.flush().unwrap();
        let aw = open_writer_timeout(audio.to_str().unwrap(), Duration::from_secs(10)).unwrap();
        drop((vw, aw));
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn creates_pipe_server_idempotently() {
        // no client connects here; only instance creation + reuse is exercised
        let path = format!(r"\\.\pipe\eztopaz-test-{}", std::process::id());
        create(&path).unwrap();
        create(&path).unwrap(); // second call is a no-op
    }

    #[test]
    fn paths_match_platform() {
        if cfg!(windows) {
            assert!(video_pipe_path().starts_with(r"\\.\pipe\"));
        } else {
            assert!(video_pipe_path().starts_with("/tmp/"));
            assert!(audio_pipe_path().starts_with("/tmp/"));
        }
    }
}
