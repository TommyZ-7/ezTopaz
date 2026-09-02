//! Named pipe setup for the FFmpeg video/audio inputs (design.md §4.1).
//!
//! Unix: FIFOs (`mkfifo`). Windows: named pipes are created by the
//! capture-windows backend (spike); until then this module errors out there.

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
    {
        let _ = path;
        Err(Error::NotImplemented("windows named pipe server (capture-windows spike)"))
    }
}

/// Open a writer to the pipe. On unix this blocks until the reader (FFmpeg) opens
/// the FIFO, so call it from a dedicated thread after FFmpeg spawn.
pub fn open_writer(path: &str) -> Result<std::fs::File> {
    #[cfg(unix)]
    {
        std::fs::OpenOptions::new().write(true).open(path).map_err(Into::into)
    }
    #[cfg(windows)]
    {
        let _ = path;
        Err(Error::NotImplemented("windows named pipe writer (capture-windows spike)"))
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
