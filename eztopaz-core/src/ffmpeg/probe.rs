//! Encoder discovery (design.md §8.1).
//!
//! `ffmpeg -encoders` only lists *compiled-in* encoders; driver presence is not
//! reflected. So discovery is two steps:
//!   1. parse `-encoders` output for candidates,
//!   2. verify each candidate with a 1-frame encode test (`testsrc` → null).
//! Results are cached at first launch. `h264_vulkan` is manual-select only
//! (requirements F-EN-03).

/// Candidates for auto-selection, in priority order (Win/Linux filtered at runtime).
pub const AUTO_CANDIDATES: &[&str] = &["h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi"];

/// Manual-select list shown in the UI (vulkan included, design §5.3/§15).
pub const MANUAL_ENCODERS: &[&str] = &[
    "auto",
    "libx264",
    "h264_nvenc",
    "h264_qsv",
    "h264_amf",
    "h264_vaapi",
    "h264_vulkan",
];

/// Parse the output of `ffmpeg -encoders`.
/// Lines look like: ` VFINLT h264_nvenc           NVIDIA NVENC H.264 encoder`.
pub fn parse_encoders(output: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim_start();
        // flag field: letters and dots, at least 4 chars, followed by the name
        let mut chars = trimmed.char_indices();
        let mut name_start = None;
        let mut flags_end = 0;
        let mut saw_letter = false;
        for (i, c) in chars.by_ref() {
            if c.is_ascii_uppercase() {
                saw_letter = true;
                flags_end = i + 1;
            } else if c == ' ' && saw_letter {
                name_start = Some(i);
                break;
            } else if c == '.' {
                flags_end = i + 1;
            } else if c.is_ascii_lowercase() {
                break; // description start without flag field
            }
        }
        if let Some(start) = name_start {
            let rest = &trimmed[start..];
            if let Some(name) = rest.split_whitespace().next() {
                if !found.iter().any(|f: &String| f == name) {
                    found.push(name.to_string());
                    let _ = flags_end;
                }
            }
        }
    }
    found
}

/// Command args for the 1-frame functional test of an encoder.
pub fn test_encode_args(encoder: &str) -> Vec<String> {
    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "testsrc=duration=0.1:size=320x240:rate=30".into(),
        "-c:v".into(),
        encoder.into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]
}

/// `encoders.json` next to `profiles.json`: avoids re-running the 1-frame
/// functional tests on every launch. Keyed by ffmpeg binary identity
/// (path + mtime + size) plus the `ffmpeg -version` first line, so driver
/// or binary updates invalidate the cache.
pub fn encoder_cache_path() -> std::path::PathBuf {
    crate::config::config_dir().join("encoders.json")
}

/// (mtime_secs, size_bytes) for cache invalidation; (0, 0) when unknown
/// (e.g. `ffmpeg` resolved via PATH).
pub fn ffmpeg_file_id(ffmpeg: &std::path::Path) -> (u64, u64) {
    let meta = std::fs::metadata(ffmpeg);
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (mtime, size)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct EncoderCacheFile {
    ffmpeg: String,
    version: String,
    mtime: u64,
    size: u64,
    encoders: Vec<crate::ipc_types::EncoderInfo>,
}

/// Load a previous probe result when the ffmpeg identity still matches.
/// Returns `None` on any mismatch or I/O/parse failure (caller re-probes).
pub fn load_cached_encoders(
    cache: &std::path::Path,
    ffmpeg: &std::path::Path,
    version: &str,
) -> Option<Vec<crate::ipc_types::EncoderInfo>> {
    let raw = std::fs::read_to_string(cache).ok()?;
    let cached: EncoderCacheFile = serde_json::from_str(&raw).ok()?;
    if cached.ffmpeg != ffmpeg.to_string_lossy() || cached.version != version {
        return None;
    }
    let (mtime, size) = ffmpeg_file_id(ffmpeg);
    if cached.mtime != mtime || cached.size != size {
        return None;
    }
    Some(cached.encoders)
}

/// Best-effort cache store; failures are ignored by the caller.
pub fn save_cached_encoders(
    cache: &std::path::Path,
    ffmpeg: &std::path::Path,
    version: &str,
    encoders: &[crate::ipc_types::EncoderInfo],
) -> bool {
    if let Some(parent) = cache.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let (mtime, size) = ffmpeg_file_id(ffmpeg);
    let file = EncoderCacheFile {
        ffmpeg: ffmpeg.to_string_lossy().into_owned(),
        version: version.to_string(),
        mtime,
        size,
        encoders: encoders.to_vec(),
    };
    let Ok(json) = serde_json::to_string(&file) else { return false };
    let tmp = cache.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_err() {
        return false;
    }
    std::fs::rename(&tmp, cache).is_ok()
}

/// Auto-selection over a list of *functionally verified* encoders.
/// Platform-pure so it can be unit-tested for both OSes.
pub fn probe_best_in(usable: &[String], is_windows: bool, is_linux: bool) -> &'static str {
    let has = |e: &str| usable.iter().any(|u| u == e);
    if has("h264_nvenc") {
        return "h264_nvenc";
    }
    if has("h264_qsv") {
        return "h264_qsv";
    }
    if is_windows && has("h264_amf") {
        return "h264_amf";
    }
    if is_linux && has("h264_vaapi") {
        return "h264_vaapi";
    }
    "libx264"
}

pub fn probe_best(usable: &[String]) -> &'static str {
    probe_best_in(usable, cfg!(target_os = "windows"), cfg!(target_os = "linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
ffmpeg version 8.0 Copyright (c) 2000-2026 the FFmpeg developers
Hyper fast Audio and Video encoder
usage: ffmpeg [options] [[infile options] -i infile]... {[outfile options] outfile}...

Video encoders:
 A..... = Supported
 V..... = Supported
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10
 V....D libx264rgb           libx264 H.264, 4:2:0, 4:2:2, 4:4:4
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D h264_qsv             QuickSync H.264 Encoder
 V....D h264_amf             AMD AMF H.264 Encoder
 V....D h264_vaapi           VAAPI H.264 Encoder
 V....D h264_vulkan          Vulkan H.264 Encoder
 V....D mpeg4                MPEG-4 part 2.
------------------
Text encoders:
";

    #[test]
    fn parses_encoder_names() {
        let enc = parse_encoders(SAMPLE);
        for name in ["libx264", "h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi", "h264_vulkan"] {
            assert!(enc.iter().any(|e| e == name), "missing {name}");
        }
        assert!(!enc.iter().any(|e| e == "usage:"), "should not pick non-encoder lines");
    }

    #[test]
    fn auto_priority() {
        let all: Vec<String> = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(probe_best_in(&all, true, true), "h264_nvenc");

        let no_nvenc = all[1..].to_vec();
        assert_eq!(probe_best_in(&no_nvenc, true, true), "h264_qsv");

        // amf is windows-only, vaapi is linux-only
        let amf_only: Vec<String> = ["h264_amf"].iter().map(|s| s.to_string()).collect();
        assert_eq!(probe_best_in(&amf_only, true, false), "h264_amf");
        assert_eq!(probe_best_in(&amf_only, false, false), "libx264");

        let vaapi_only: Vec<String> = ["h264_vaapi"].iter().map(|s| s.to_string()).collect();
        assert_eq!(probe_best_in(&vaapi_only, false, true), "h264_vaapi");
        assert_eq!(probe_best_in(&vaapi_only, true, false), "libx264");

        // vulkan never selected in auto (manual only)
        let vulkan_only: Vec<String> = ["h264_vulkan"].iter().map(|s| s.to_string()).collect();
        assert_eq!(probe_best_in(&vulkan_only, true, true), "libx264");

        assert_eq!(probe_best_in(&[], true, true), "libx264");
    }

    #[test]
    fn test_encode_args_shape() {
        let args = test_encode_args("h264_nvenc");
        assert_eq!(args[2], "-i");
        assert!(args.iter().any(|a| a == "h264_nvenc"));
    }

    fn cache_test_encoders() -> Vec<crate::ipc_types::EncoderInfo> {
        vec![crate::ipc_types::EncoderInfo {
            name: "h264_nvenc".into(),
            usable: true,
            reason: None,
        }]
    }

    #[test]
    fn encoder_cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("eztopaz-enc-cache-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let fake_ffmpeg = dir.join("ffmpeg");
        std::fs::write(&fake_ffmpeg, b"fake").unwrap();
        let cache = dir.join("encoders.json");
        let encoders = cache_test_encoders();

        assert!(save_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 8.0", &encoders));
        assert_eq!(
            load_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 8.0"),
            Some(encoders)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encoder_cache_rejects_stale_identity() {
        let dir =
            std::env::temp_dir().join(format!("eztopaz-enc-stale-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let fake_ffmpeg = dir.join("ffmpeg");
        std::fs::write(&fake_ffmpeg, b"fake").unwrap();
        let cache = dir.join("encoders.json");

        assert!(save_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 8.0", &cache_test_encoders()));
        // version bump invalidates
        assert_eq!(load_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 9.0"), None);
        // binary change invalidates
        std::fs::write(&fake_ffmpeg, b"fake-new-binary").unwrap();
        assert_eq!(
            load_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 8.0"),
            None
        );
        // corrupt file falls back to re-probe
        std::fs::write(&cache, "{ not json").unwrap();
        assert_eq!(load_cached_encoders(&cache, &fake_ffmpeg, "ffmpeg version 8.0"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
