//! Named pipe setup for the FFmpeg video/audio inputs (design.md §4.1).
//!
//! Unix: FIFOs (`mkfifo`). Windows: named-pipe servers (`CreateNamedPipeW`) —
//! Rust writes, FFmpeg connects as the client. `create` runs before the FFmpeg
//! spawn, `open_writer` blocks until the client is connected.

use crate::error::{Error, Result};
#[cfg(unix)]
use std::path::Path;

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
pub fn open_writer(path: &str) -> Result<std::fs::File> {
    #[cfg(unix)]
    {
        std::fs::OpenOptions::new().write(true).open(path).map_err(Into::into)
    }
    #[cfg(windows)]
    server::open_writer(path)
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
