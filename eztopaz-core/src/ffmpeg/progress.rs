//! Parse `ffmpeg -progress pipe:1` output (design.md §4.2).
//!
//! Progress mode emits `key=value` lines, terminated by `progress=continue|end`.

use crate::ipc_types::StreamStatus;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Progress {
    pub frame: u64,
    pub bitrate_kbps: f64,
    pub drop_frames: u64,
}

/// Consume one progress line, updating `status`.
/// Returns true when the line was recognized.
pub fn apply_line(status: &mut StreamStatus, line: &str) -> bool {
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    let value = value.trim();
    match key {
        "frame" => {} // recognized but not surfaced in the UI
        "bitrate" => {
            // e.g. "1536.2kbits/s" or "N/A"
            if let Some(num) = value.strip_suffix("kbits/s") {
                status.bitrate_kbps = num.trim().parse().unwrap_or(status.bitrate_kbps);
            }
        }
        "drop_frames" => {
            status.dropped_frames = value.parse().unwrap_or(status.dropped_frames);
        }
        "out_time_us" => {
            if let Ok(us) = value.parse::<u64>() {
                status.duration_sec = us / 1_000_000;
            }
        }
        "progress" => {
            status.is_live = value != "end";
            return true;
        }
        _ => return false,
    }
    let _ = key;
    true
}

/// Convenience: parse a single line into a fresh Progress snapshot (tests/tools).
pub fn parse_line(line: &str) -> Option<Progress> {
    let mut st = StreamStatus::default();
    if !apply_line(&mut st, line) {
        return None;
    }
    Some(Progress {
        frame: 0,
        bitrate_kbps: st.bitrate_kbps,
        drop_frames: st.dropped_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        let mut st = StreamStatus::default();
        st.is_live = true;

        assert!(apply_line(&mut st, "frame=1234"));
        assert!(apply_line(&mut st, "bitrate=1536.2kbits/s"));
        assert_eq!(st.bitrate_kbps, 1536.2);
        assert!(apply_line(&mut st, "drop_frames=7"));
        assert_eq!(st.dropped_frames, 7);
        assert!(apply_line(&mut st, "out_time_us=62000000"));
        assert_eq!(st.duration_sec, 62);
        assert!(apply_line(&mut st, "progress=continue"));
        assert!(st.is_live);

        assert!(!apply_line(&mut st, "ffmpeg version 8.0"), "non-progress lines ignored");
        assert!(!apply_line(&mut st, "no equals sign here"));

        assert!(apply_line(&mut st, "progress=end"));
        assert!(!st.is_live);
    }

    #[test]
    fn bitrate_na_is_ignored() {
        let mut st = StreamStatus::default();
        assert!(apply_line(&mut st, "bitrate=N/A"));
        assert_eq!(st.bitrate_kbps, 0.0);
    }

    #[test]
    fn parse_line_snapshot() {
        assert_eq!(
            parse_line("bitrate=2000.0kbits/s").unwrap().bitrate_kbps,
            2000.0
        );
        assert!(parse_line("garbage").is_none());
    }
}
