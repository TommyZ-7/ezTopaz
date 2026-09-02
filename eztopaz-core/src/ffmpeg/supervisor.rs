//! FFmpeg process lifecycle (design.md §4.1, §9): spawn, progress feed, stop, retry.
//!
//! The capture backends (feature-gated) own the named-pipe writer threads; the
//! supervisor owns the process and its `-progress` stream. Kill-on-drop is the
//! safety net required by requirements §6 (クラッシュ時FFmpeg確実kill).

use crate::error::{Error, Result};
use crate::ipc_types::StreamStatus;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Resolve the bundled ffmpeg binary. Env override wins, then the resource dir
/// next to the executable, then PATH.
pub fn ffmpeg_path() -> PathBuf {
    if let Ok(p) = std::env::var("EZTOPAZ_FFMPEG") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        let dir = exe.parent().unwrap();
        let candidates = [
            dir.join("resources/ffmpeg").join(exe_name), // bundled (design §4.4)
            dir.join(exe_name),
        ];
        for c in candidates {
            if c.exists() {
                return c;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

/// F-ST-04: 3 retries with exponential backoff.
pub const MAX_RETRIES: u32 = 3;

pub fn retry_backoff_ms(retry: u32) -> u64 {
    1000u64.saturating_mul(1 << retry.min(4)) // 1s, 2s, 4s, ...
}

#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    pub lines: Arc<Mutex<Vec<String>>>,
    pub stop: Arc<AtomicBool>,
}

/// `logs/ezTopaz-YYYY-MM-DD.log` under the config dir (design §7.1).
pub fn open_log_file() -> Option<Arc<Mutex<std::fs::File>>> {
    let dir = crate::config::config_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("ezTopaz-{}.log", today_iso()));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    Some(Arc::new(Mutex::new(file)))
}

/// Days-since-epoch → ISO date (Howard Hinnant's civil algorithm; no chrono dep).
fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    civil_from_days((secs / 86_400) as i64)
}

fn civil_from_days(z: i64) -> String {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Debug)]
pub struct StreamProcess {
    child: Child,
    pub status: Arc<Mutex<StreamStatus>>,
    pub stderr_log: LogBuffer,
    started_at: Instant,
    pub retry_count: u32,
}

impl StreamProcess {
    /// Spawn ffmpeg. `args` comes from args::build_ffmpeg_args.
    pub fn spawn(ffmpeg: &Path, args: &[String]) -> Result<Self> {
        let mut child = Command::new(ffmpeg)
            .args(args)
            .stdout(Stdio::piped()) // -progress pipe:1
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Ffmpeg(format!("spawn failed: {e}")))?;

        let status = Arc::new(Mutex::new(StreamStatus { is_live: true, ..Default::default() }));
        let stderr_log = LogBuffer::default();

        // stderr → log buffer + log file (design §7.1, §9)
        if let Some(stderr) = child.stderr.take() {
            let log = stderr_log.clone();
            let file = open_log_file();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                    if log.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(f) = &file {
                        use std::io::Write;
                        if let Ok(mut f) = f.lock() {
                            let _ = writeln!(f, "{line}");
                        }
                    }
                    let mut buf = log.lines.lock().unwrap();
                    if buf.len() >= 1000 {
                        buf.drain(..500); // ponytail: cap log buffer; rotate file in a later pass
                    }
                    buf.push(line);
                }
            });
        }

        // stdout → StreamStatus (design §4.2)
        let st = status.clone();
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut local = StreamStatus { is_live: true, ..Default::default() };
                for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                    let mut s = st.lock().unwrap();
                    let _ = crate::ffmpeg::progress::apply_line(&mut local, &line);
                    *s = local;
                }
                // progress stream closed = process ending
                st.lock().unwrap().is_live = false;
            });
        }

        Ok(Self {
            child,
            status,
            stderr_log,
            started_at: Instant::now(),
            retry_count: 0,
        })
    }

    pub fn status(&self) -> StreamStatus {
        let mut st = self.status.lock().unwrap().clone();
        st.duration_sec = self.started_at.elapsed().as_secs();
        st
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Non-blocking exit check: Some(status) if ffmpeg has exited.
    pub fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Kill and reap. Safe to call multiple times.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stderr_log.stop.store(true, Ordering::Relaxed);
    }

    pub fn tail_stderr(&self, n: usize) -> Vec<String> {
        let buf = self.stderr_log.lines.lock().unwrap();
        let start = buf.len().saturating_sub(n);
        buf[start..].to_vec()
    }
}

impl Drop for StreamProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_capped() {
        assert_eq!(retry_backoff_ms(0), 1000);
        assert_eq!(retry_backoff_ms(1), 2000);
        assert_eq!(retry_backoff_ms(2), 4000);
        assert_eq!(retry_backoff_ms(9), 16000); // capped at shift 4
    }

    #[test]
    fn ffmpeg_path_env_override() {
        std::env::set_var("EZTOPAZ_FFMPEG", "/usr/local/bin/ffmpeg-test");
        assert_eq!(ffmpeg_path(), PathBuf::from("/usr/local/bin/ffmpeg-test"));
        std::env::remove_var("EZTOPAZ_FFMPEG");
    }

    #[test]
    fn spawn_missing_binary_errors() {
        let fake = if cfg!(windows) { r"C:\definitely\missing.exe" } else { "/definitely/missing/ffmpeg".into() };
        let err = StreamProcess::spawn(Path::new(&fake), &["-version".to_string()]).unwrap_err();
        assert!(matches!(err, Error::Ffmpeg(_)));
    }

    #[test]
    fn civil_date_known_values() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(19_000), "2022-01-08");
        assert_eq!(civil_from_days(20_628), "2026-06-24");
    }
}
