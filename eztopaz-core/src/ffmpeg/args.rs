//! FFmpeg command-line construction (design.md §4.3).

use crate::config::{validate_bitrate, Profile};
use crate::error::Result;

/// Per-encoder `-preset` and rate-control / low-latency flags.
/// `p1` is an NVENC-family preset name; qsv/amf/vaapi reject it.
/// ffmpeg9 notes:
/// - old NVENC aliases (`ll`, `llhq`, …) are removed; use `p1` + `-tune ll`.
/// - `zerolatency` tune is intentionally NOT used whole (gray-screen
///   regression); x264 gets sliced-threads/sync-lookahead/scene-cut off only.
pub fn preset_and_rc(encoder: &str) -> (Option<&'static str>, &'static [&'static str]) {
    match encoder {
        "h264_nvenc" => (
            Some("p1"),
            &[
                "-rc",
                "cbr",
                "-tune",
                "ll",
                "-rc-lookahead",
                "0",
                "-no-scenecut",
                "1",
                "-forced-idr",
                "1",
                "-delay",
                "0",
            ],
        ),
        "libx264" => (
            Some("veryfast"),
            &[
                "-rc-lookahead",
                "0",
                "-x264-params",
                "sliced-threads=1:sync-lookahead=0:scenecut=0",
            ],
        ),
        "h264_qsv" => (Some("veryfast"), &["-async_depth", "1"]),
        "h264_amf" => (Some("speed"), &["-usage", "lowlatency"]),
        _ => (None, &[]), // vaapi / vulkan: no preset
    }
}

/// Build the full ffmpeg argv. Video = rawvideo named pipe, audio = f32le named pipe.
///
/// Low-latency / robustness additions (ffmpeg9 bundle):
/// - `-thread_queue_size` + `-probesize 32 -analyzeduration 0` + `-fflags nobuffer`
///   on both pipe inputs (drops / AV drift under load).
/// - `-sc_threshold 0 -keyint_min <gop>` (strict 2s GOP, no scene-cut bursts).
/// - `-sws_flags fast_bilinear` (BGRA→YUV420p is the CPU hot spot).
/// - `-fps_mode cfr` (steady rate from FramePacer).
/// - `-flvflags no_duration_filesize -flush_packets 1` (RTMP latency).
pub fn build_ffmpeg_args(
    profile: &Profile,
    encoder: &str,
    ingest_url: &str,
    stream_key: &str,
    video_pipe: &str,
    audio_pipe: &str,
) -> Result<Vec<String>> {
    build_ffmpeg_args_with_transport(
        profile,
        encoder,
        ingest_url,
        stream_key,
        video_pipe,
        audio_pipe,
        &Transport::PipeBgra,
    )
}

/// Video transport into ffmpeg. `PipeBgra` is the historical default;
/// `PipeNv12` carries `w*h*3/2`-byte frames (see `video::nv12`) so HW
/// encoders skip the BGRA→YUV swscale. `HwDirect` is reserved for the
/// zero-copy capture path (`ffmpeg::hw`) and currently falls back to pipe
/// args until the capture backend is wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    PipeBgra,
    PipeNv12,
    HwDirect,
}

impl Transport {
    fn input_pix_fmt(self) -> &'static str {
        match self {
            Transport::PipeBgra => "bgra",
            Transport::PipeNv12 | Transport::HwDirect => "nv12",
        }
    }

    fn input_frame_desc(self) -> &'static str {
        match self {
            Transport::PipeBgra => "rawvideo",
            Transport::PipeNv12 | Transport::HwDirect => "rawvideo",
        }
    }
}

/// Output pixel format: HW encoders take `nv12` directly (no yuv420p
/// re-convert in-driver); software stays on `yuv420p`.
pub fn output_pix_fmt(encoder: &str, transport: &Transport) -> &'static str {
    match *transport {
        Transport::PipeNv12 | Transport::HwDirect => match encoder {
            "libx264" => "yuv420p",
            _ => "nv12",
        },
        Transport::PipeBgra => "yuv420p",
    }
}

/// HW encoders get NV12 pipes (GPU-friendly, 3/8 bandwidth); software keeps
/// BGRA (no conversion cost when no GPU upload is possible).
pub fn transport_for_encoder(encoder: &str) -> Transport {
    match encoder {
        "h264_nvenc" | "h264_qsv" | "h264_amf" | "h264_vaapi" | "h264_vulkan" => {
            Transport::PipeNv12
        }
        _ => Transport::PipeBgra,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_ffmpeg_args_with_transport(
    profile: &Profile,
    encoder: &str,
    ingest_url: &str,
    stream_key: &str,
    video_pipe: &str,
    audio_pipe: &str,
    transport: &Transport,
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
        "-fflags".into(),
        "nobuffer".into(),
        "-probesize".into(),
        "32".into(),
        "-analyzeduration".into(),
        "0".into(),
        "-thread_queue_size".into(),
        "1024".into(),
        "-f".into(),
        transport.input_frame_desc().into(),
        "-pix_fmt".into(),
        transport.input_pix_fmt().into(),
        "-s".into(),
        format!("{}x{}", profile.w, profile.h),
        "-r".into(),
        profile.fps.to_string(),
        "-i".into(),
        video_pipe.into(),
        "-thread_queue_size".into(),
        "512".into(),
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
        output_pix_fmt(encoder, transport).into(),
        "-profile:v".into(),
        "high".into(),
        "-bf".into(),
        "0".into(),
        "-g".into(),
        gop.to_string(),
        "-keyint_min".into(),
        gop.to_string(),
        "-sc_threshold".into(),
        "0".into(),
        "-sws_flags".into(),
        "fast_bilinear".into(),
        "-fps_mode".into(),
        "cfr".into(),
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
    // Optional GPU upload when the caller selected a HW backend explicitly
    // (see `ffmpeg::hw::hw_upload_filter`; default pipe path stays CPU).
    if let Some(hw_filter) = crate::ffmpeg::hw::hw_upload_filter(encoder, transport) {
        args.extend(["-vf".into(), hw_filter]);
    }
    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{}k", profile.a_kbps),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        "48000".into(),
        "-flvflags".into(),
        "no_duration_filesize".into(),
        "-flush_packets".into(),
        "1".into(),
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

    #[test]
    fn pipe_robustness_flags() {
        let args = build("mid", "libx264");
        assert_eq!(find(&args, "-thread_queue_size"), "1024");
        assert!(args.contains(&"512".to_string()));
        assert_eq!(find(&args, "-probesize"), "32");
        assert_eq!(find(&args, "-analyzeduration"), "0");
        assert_eq!(find(&args, "-fflags"), "nobuffer");
    }

    #[test]
    fn strict_gop_no_scenecut() {
        let args = build("mid", "libx264"); // gop 60
        assert_eq!(find(&args, "-g"), "60");
        assert_eq!(find(&args, "-keyint_min"), "60");
        assert_eq!(find(&args, "-sc_threshold"), "0");
    }

    #[test]
    fn swscale_and_cfr_and_muxer_flags() {
        let args = build("mid", "libx264");
        assert_eq!(find(&args, "-sws_flags"), "fast_bilinear");
        assert_eq!(find(&args, "-fps_mode"), "cfr");
        assert_eq!(find(&args, "-flvflags"), "no_duration_filesize");
        assert_eq!(find(&args, "-flush_packets"), "1");
    }

    #[test]
    fn nvenc_low_latency_flags() {
        let args = build("high", "h264_nvenc");
        assert_eq!(find(&args, "-tune"), "ll");
        assert_eq!(find(&args, "-rc-lookahead"), "0");
        assert_eq!(find(&args, "-no-scenecut"), "1");
        assert_eq!(find(&args, "-forced-idr"), "1");
    }

    #[test]
    fn x264_sliced_threads_no_full_zerolatency() {
        let args = build("mid", "libx264");
        let params = find(&args, "-x264-params");
        assert!(params.contains("sliced-threads=1"));
        assert!(params.contains("sync-lookahead=0"));
        assert!(params.contains("scenecut=0"));
        // full `-tune zerolatency` is banned (gray-screen regression)
        assert!(!args.contains(&"zerolatency".to_string()));
    }

    #[test]
    fn qsv_and_amf_low_latency() {
        let qsv = build("mid", "h264_qsv");
        assert_eq!(find(&qsv, "-async_depth"), "1");
        let amf = build("mid", "h264_amf");
        assert_eq!(find(&amf, "-usage"), "lowlatency");
    }

    #[test]
    fn nv12_transport_uses_nv12_for_hw() {
        let p = profile("mid");
        let hw = build_ffmpeg_args_with_transport(
            &p,
            "h264_nvenc",
            "rtmp://topaz.chat/live",
            "test-key-123",
            "vid",
            "aud",
            &Transport::PipeNv12,
        )
        .unwrap();
        assert_eq!(find(&hw, "-pix_fmt"), "nv12"); // input
        // output pix_fmt for HW is nv12 (last -pix_fmt before -profile:v)
        let out_pix = hw
            .windows(2)
            .find(|w| w[0] == "-pix_fmt" && (w[1] == "nv12" || w[1] == "yuv420p"))
            .unwrap()[1]
            .clone();
        assert_eq!(out_pix, "nv12");
        // x264 stays on yuv420p even with nv12 input
        let sw = build_ffmpeg_args_with_transport(
            &p,
            "libx264",
            "rtmp://topaz.chat/live",
            "test-key-123",
            "vid",
            "aud",
            &Transport::PipeNv12,
        )
        .unwrap();
        let pix_fmts: Vec<&str> = sw
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "-pix_fmt")
            .map(|(i, _)| sw[i + 1].as_str())
            .collect();
        assert_eq!(pix_fmts, vec!["nv12", "yuv420p"]);
    }
}
