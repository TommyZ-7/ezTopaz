//! FFmpeg command-line construction (design.md §4.3).

use crate::config::{validate_bitrate, Profile};
use crate::error::Result;

/// Per-encoder `-preset` and rate-control flags.
/// `p1` is an NVENC-family preset name; qsv/amf/vaapi reject it (design v0.2 §4.3).
pub fn preset_and_rc(encoder: &str) -> (Option<&'static str>, &'static [&'static str]) {
    match encoder {
        "h264_nvenc" => (Some("p1"), &["-rc", "cbr"]),
        "libx264" => (Some("veryfast"), &["-rc-lookahead", "0"]),
        "h264_qsv" => (Some("veryfast"), &[]),
        "h264_amf" => (Some("speed"), &[]),
        _ => (None, &[]), // vaapi / vulkan: no preset
    }
}

/// Build the full ffmpeg argv. Video = rawvideo named pipe, audio = f32le named pipe.
pub fn build_ffmpeg_args(
    profile: &Profile,
    encoder: &str,
    ingest_url: &str,
    stream_key: &str,
    video_pipe: &str,
    audio_pipe: &str,
) -> Result<Vec<String>> {
    validate_bitrate(profile.v_kbps, profile.a_kbps)?;
    let (preset, rc) = preset_and_rc(encoder);
    let gop = profile.gop(); // F-EN-05: 2 seconds

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "info".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "bgra".into(),
        "-s".into(),
        format!("{}x{}", profile.w, profile.h),
        "-r".into(),
        profile.fps.to_string(),
        "-i".into(),
        video_pipe.into(),
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        "48000".into(),
        "-ac".into(),
        "2".into(),
        "-i".into(),
        audio_pipe.into(),
        "-c:v".into(),
        encoder.into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-profile:v".into(),
        "high".into(),
        "-bf".into(),
        "0".into(),
        "-g".into(),
        gop.to_string(),
        "-b:v".into(),
        format!("{}k", profile.v_kbps),
        "-maxrate".into(),
        format!("{}k", profile.v_kbps),
        "-bufsize".into(),
        format!("{}k", profile.v_kbps * 2),
    ];
    if let Some(p) = preset {
        args.extend(["-preset".into(), p.into()]);
    }
    args.extend(rc.iter().map(|s| s.to_string()));
    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{}k", profile.a_kbps),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        "48000".into(),
        "-f".into(),
        "flv".into(),
        format!("{}/{}", ingest_url.trim_end_matches('/'), stream_key),
    ]);
    Ok(args)
}

pub const LIMITS_NOTE: &str = "Topaz limits: video <= 2000kbps, audio <= 320kbps";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_profiles, MAX_AUDIO_KBPS, MAX_VIDEO_KBPS};
    use crate::error::Error;

    fn profile(id: &str) -> Profile {
        default_profiles().get(id).unwrap().clone()
    }

    fn build(id: &str, enc: &str) -> Vec<String> {
        build_ffmpeg_args(&profile(id), enc, "rtmp://topaz.chat/live", "test-key-123", "vid", "aud")
            .unwrap()
    }

    fn find<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("no {flag}"));
        &args[i + 1]
    }

    #[test]
    fn nvenc_args() {
        let args = build("high", "h264_nvenc"); // 720p60 2000k
        assert_eq!(find(&args, "-c:v"), "h264_nvenc");
        assert_eq!(find(&args, "-preset"), "p1");
        assert_eq!(find(&args, "-rc"), "cbr");
        assert_eq!(find(&args, "-g"), "120"); // 60fps * 2s
        assert_eq!(find(&args, "-bufsize"), "4000k");
        assert_eq!(find(&args, "-s"), "1280x720");
        assert_eq!(find(&args, "-r"), "60");
        assert!(args.contains(&"flv".to_string()));
        assert!(args.last().unwrap().ends_with("rtmp://topaz.chat/live/test-key-123"));
    }

    #[test]
    fn x264_args() {
        let args = build("mid", "libx264"); // 720p30 1500k
        assert_eq!(find(&args, "-preset"), "veryfast");
        assert_eq!(find(&args, "-rc-lookahead"), "0");
        assert_eq!(find(&args, "-g"), "60"); // 30fps * 2s
        assert!(!args.contains(&"-rc".to_string())); // x264: no -rc, ABR via maxrate/bufsize
    }

    #[test]
    fn vaapi_has_no_preset() {
        let args = build("mid", "h264_vaapi");
        assert!(!args.contains(&"-preset".to_string()));
    }

    #[test]
    fn audio_pipe_is_f32le_48k_stereo() {
        let args = build("mid", "libx264");
        assert_eq!(find(&args, "-f"), "rawvideo"); // first -f (video)
        // audio input flags present
        let i = args.iter().position(|a| a == "f32le").unwrap();
        assert!(i > 0);
        assert_eq!(find(&args, "-ar"), "48000");
        assert_eq!(find(&args, "-ac"), "2");
    }

    #[test]
    fn ingest_url_trailing_slash_trimmed() {
        let args = build_ffmpeg_args(
            &profile("mid"),
            "libx264",
            "rtmp://custom.example/live/",
            "k",
            "v",
            "a",
        )
        .unwrap();
        assert!(args.last().unwrap().ends_with("rtmp://custom.example/live/k"));
    }

    #[test]
    fn bitrate_guard_blocks_over_limit() {
        let mut p = profile("mid");
        p.v_kbps = MAX_VIDEO_KBPS + 1;
        let err = build_ffmpeg_args(&p, "libx264", "rtmp://x", "k", "v", "a").unwrap_err();
        assert!(matches!(err, Error::BitrateOver { .. }));

        let mut p = profile("mid");
        p.a_kbps = MAX_AUDIO_KBPS + 1;
        assert!(build_ffmpeg_args(&p, "libx264", "rtmp://x", "k", "v", "a").is_err());
    }
}
