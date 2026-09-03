//! Hardware upload / zero-copy helpers (ffmpeg9 bundle).
//!
//! Default pipe path stays CPU (`None` filter) so existing backends keep
//! working without a GPU. `HwDirect` (explicit `-init_hw_device` + upload
//! filter) and the Windows `ddagrab` direct-input path are opt-in
//! (`StreamConfig::{hw_direct,direct_input}`).
//!
//! Linux DMA-BUF note (C11): true fd zero-copy cannot cross into the sidecar
//! ffmpeg CLI — named pipes cannot transport DMA-BUF fds (`SCM_RIGHTS` needs
//! a unix socket, which ffmpeg has no input protocol for) and ffmpeg ships
//! no PipeWire capture input. The implemented C11 path is therefore NV12
//! pipe + explicit VAAPI upload (`format=nv12,hwupload,scale_vaapi`), which
//! keeps every post-upload step on-GPU.

use crate::config::{Profile, ScreenTarget, ScreenTargetKind};
use crate::error::{Error, Result};
use crate::ffmpeg::args::{self, Transport};
use crate::ipc_types::StreamConfig;

/// GPU backend selected by the caller (probe result, not auto-detected here
/// so `cargo test` stays platform-independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HwBackend {
    #[default]
    None,
    Cuda,
    D3d11va,
    D3d12va,
    Vaapi,
    Vulkan,
    Qsv,
    Amf,
}

impl HwBackend {
    /// Encoder → preferred backend for the zero-copy path.
    pub fn for_encoder(encoder: &str) -> Self {
        match encoder {
            "h264_nvenc" => Self::Cuda,
            "h264_qsv" => Self::Qsv,
            "h264_vaapi" => Self::Vaapi,
            "h264_amf" => Self::Amf,
            "h264_vulkan" => Self::Vulkan,
            _ => Self::None,
        }
    }
}

/// Optional `-vf hwupload*` chain for pipe transports.
///
/// - `PipeBgra` → `None` (CPU swscale).
/// - `PipeNv12` → `None` (NV12 system memory; encoders upload implicitly).
/// - `HwDirect` → explicit upload filter (C10 Windows / C11 Linux stepping
///   stone: pipe stays, but frames are uploaded to GPU before encode so the
///   scaler/encoder run on hardware). Requires the matching
///   `-init_hw_device` prefix emitted by `build_ffmpeg_args_with_transport`.
pub fn hw_upload_filter(encoder: &str, transport: &Transport) -> Option<String> {
    if *transport != Transport::HwDirect {
        return None;
    }
    hw_upload_chain(&HwBackend::for_encoder(encoder))
}

/// Explicit upload chain per backend. Frames are already profile-sized
/// (FramePacer), so no GPU scaling — upload only, except VAAPI where
/// `scale_vaapi` normalizes the uploaded frames to `nv12` on-GPU.
pub fn hw_upload_chain(backend: &HwBackend) -> Option<String> {
    match backend {
        HwBackend::None => None,
        HwBackend::Cuda => Some("hwupload_cuda".into()),
        HwBackend::D3d11va | HwBackend::D3d12va => Some("hwupload".into()),
        HwBackend::Vaapi => Some("format=nv12,hwupload,scale_vaapi=format=nv12".into()),
        HwBackend::Vulkan => Some("hwupload".into()),
        HwBackend::Qsv => Some("hwupload=extra_hw_frames=64".into()),
        HwBackend::Amf => Some("hwupload".into()),
    }
}

/// `-init_hw_device` argv prefix for the direct-capture path.
/// `None` means "no device init" (current pipe default).
pub fn init_hw_device_args(backend: &HwBackend) -> Vec<String> {
    match backend {
        HwBackend::None => vec![],
        HwBackend::Cuda => vec!["-init_hw_device".into(), "cuda=cu:0".into()],
        HwBackend::D3d11va => vec!["-init_hw_device".into(), "d3d11va=d3d11:0".into()],
        HwBackend::D3d12va => vec!["-init_hw_device".into(), "d3d12va=d3d12:0".into()],
        HwBackend::Vaapi => vec![
            "-init_hw_device".into(),
            "vaapi=va:/dev/dri/renderD128".into(),
        ],
        HwBackend::Vulkan => vec!["-init_hw_device".into(), "vulkan=vk:0".into()],
        HwBackend::Qsv => vec!["-init_hw_device".into(), "qsv=qs:0".into()],
        HwBackend::Amf => vec![],
    }
}

/// Whether the running OS could support direct capture (compile-time gate).
/// Runtime device presence is still verified by the 1-frame probe.
pub fn direct_capture_supported() -> bool {
    cfg!(target_os = "windows") || cfg!(target_os = "linux")
}

/// Direct device video input (C10 Windows). `ddagrab` captures a whole
/// output on-GPU (D3D11 frames); only fullscreen targets are supported —
/// window capture stays on the Rust WGC → pipe path. `gfxcapture`
/// (WGC-based, ffmpeg 8.1+) remains experimental and is NOT selectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSource {
    DdagrabFullscreen,
    GfxCaptureWindow,
}

/// `direct_input` selection strings accepted from `StreamConfig`.
/// Only `"ddagrab"` is wired; anything else (including `"gfxcapture"`)
/// returns `None` so the caller falls back to the pipe path.
pub fn parse_direct_input(s: Option<&str>) -> Option<DirectSource> {
    match s {
        Some("ddagrab") => Some(DirectSource::DdagrabFullscreen),
        _ => None,
    }
}

pub fn direct_capture_input_args(source: &DirectSource, fps: u32) -> Vec<String> {
    match source {
        DirectSource::DdagrabFullscreen => vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("ddagrab=framerate={fps}"),
        ],
        DirectSource::GfxCaptureWindow => vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("gfxcapture=framerate={fps}"),
        ],
    }
}

/// Fullscreen ddagrab input bound to the selected display (`screen.id`
/// `"display:<N>"`, same suffix convention as the WGC backend).
/// Window targets are rejected: ddagrab is output-based.
pub fn direct_video_input_args(
    profile: &Profile,
    screen: &ScreenTarget,
    source: &DirectSource,
) -> Result<Vec<String>> {
    if screen.kind != ScreenTargetKind::Display {
        return Err(Error::Config(
            "direct input (ddagrab) supports fullscreen displays only; use the pipe path for windows"
                .into(),
        ));
    }
    let idx: usize = screen
        .id
        .rsplit(':')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let input = match source {
        DirectSource::DdagrabFullscreen => {
            format!("ddagrab=output_idx={idx}:framerate={}", profile.fps)
        }
        DirectSource::GfxCaptureWindow => {
            return Err(Error::Config("gfxcapture direct input is experimental".into()));
        }
    };
    Ok(vec![
        "-fflags".into(),
        "nobuffer".into(),
        "-probesize".into(),
        "32".into(),
        "-analyzeduration".into(),
        "0".into(),
        "-thread_queue_size".into(),
        "1024".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        input,
    ])
}

/// GPU scale + format for ddagrab D3D11 frames: profile size, `nv12`,
/// consumable by `h264_nvenc` without ever touching the CPU.
pub fn direct_video_filter(profile: &Profile) -> String {
    format!("scale_d3d11={}:{}:format=nv12", profile.w, profile.h)
}

/// Full ffmpeg argv for the direct-input session: device video in, mixed
/// `f32le` audio still via pipe, same encoder/muxer block as the pipe path.
/// Only `h264_nvenc` is accepted (D3D11 frames need an encoder that takes
/// them; QSV/AMF would need an extra `hwmap`, which is unwired).
pub fn build_direct_args(
    profile: &Profile,
    encoder: &str,
    cfg: &StreamConfig,
    audio_pipe: &str,
) -> Result<Vec<String>> {
    let source = parse_direct_input(cfg.direct_input.as_deref()).ok_or_else(|| {
        Error::Config(format!(
            "unsupported direct_input: {:?} (only \"ddagrab\" is wired)",
            cfg.direct_input
        ))
    })?;
    if encoder != "h264_nvenc" {
        return Err(Error::EncoderNotAvailable(format!(
            "{encoder} has no direct-input path (use h264_nvenc or the pipe path)"
        )));
    }
    let transport = Transport::HwDirect;
    let mut argv: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "info".into(),
        "-progress".into(),
        "pipe:1".into(),
    ];
    argv.extend(init_hw_device_args(&HwBackend::D3d11va));
    argv.extend(direct_video_input_args(profile, &cfg.screen, &source)?);
    argv.extend(args::pipe_audio_input_args(audio_pipe));
    argv.extend(args::encoder_output_args(
        profile,
        encoder,
        &cfg.ingest_url,
        &cfg.stream_key,
        &transport,
        Some(&direct_video_filter(profile)),
    )?);
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_transports_need_no_upload_filter() {
        assert_eq!(
            hw_upload_filter("h264_nvenc", &Transport::PipeBgra),
            None
        );
        assert_eq!(
            hw_upload_filter("h264_nvenc", &Transport::PipeNv12),
            None
        );
    }

    #[test]
    fn hwdirect_emits_upload_chain() {
        assert_eq!(
            hw_upload_filter("h264_nvenc", &Transport::HwDirect),
            Some("hwupload_cuda".into())
        );
        assert_eq!(
            hw_upload_filter("h264_qsv", &Transport::HwDirect),
            Some("hwupload=extra_hw_frames=64".into())
        );
        assert_eq!(
            hw_upload_filter("libx264", &Transport::HwDirect),
            None
        );
    }

    #[test]
    fn backend_mapping() {
        assert_eq!(HwBackend::for_encoder("h264_nvenc"), HwBackend::Cuda);
        assert_eq!(HwBackend::for_encoder("h264_qsv"), HwBackend::Qsv);
        assert_eq!(HwBackend::for_encoder("libx264"), HwBackend::None);
    }

    #[test]
    fn no_device_init_by_default() {
        assert!(init_hw_device_args(&HwBackend::None).is_empty());
        assert!(!init_hw_device_args(&HwBackend::Cuda).is_empty());
    }

    #[test]
    fn direct_capture_inputs_shape() {
        let dda = direct_capture_input_args(&DirectSource::DdagrabFullscreen, 60);
        assert_eq!(dda[0], "-f");
        assert!(dda.join(" ").contains("ddagrab"));
        let gfx = direct_capture_input_args(&DirectSource::GfxCaptureWindow, 30);
        assert!(gfx.join(" ").contains("gfxcapture"));
    }

    #[test]
    fn vaapi_chain_normalizes_on_gpu() {
        assert_eq!(
            hw_upload_filter("h264_vaapi", &Transport::HwDirect),
            Some("format=nv12,hwupload,scale_vaapi=format=nv12".into())
        );
    }

    fn direct_cfg(screen_kind: ScreenTargetKind, id: &str, direct: Option<&str>) -> StreamConfig {
        StreamConfig {
            stream_key: "test-key-123".into(),
            screen: ScreenTarget { kind: screen_kind, id: id.into() },
            direct_input: direct.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn parse_direct_input_gates_experimental() {
        assert_eq!(parse_direct_input(Some("ddagrab")), Some(DirectSource::DdagrabFullscreen));
        assert_eq!(parse_direct_input(Some("gfxcapture")), None);
        assert_eq!(parse_direct_input(None), None);
    }

    #[test]
    fn direct_video_input_binds_display() {
        let profiles = crate::config::default_profiles();
        let p = &profiles["mid"];
        let args = direct_video_input_args(
            p,
            &direct_cfg(ScreenTargetKind::Display, "display:1", None).screen,
            &DirectSource::DdagrabFullscreen,
        )
        .unwrap()
        .join(" ");
        assert!(args.contains("ddagrab=output_idx=1:framerate=30"), "{args}");
        // window targets rejected
        assert!(direct_video_input_args(
            p,
            &direct_cfg(ScreenTargetKind::Window, "window:123", None).screen,
            &DirectSource::DdagrabFullscreen,
        )
        .is_err());
    }

    #[test]
    fn build_direct_args_shape() {
        let profiles = crate::config::default_profiles();
        let p = &profiles["mid"]; // 1280x720@30
        let cfg = direct_cfg(ScreenTargetKind::Display, "display:0", Some("ddagrab"));
        let argv = build_direct_args(p, "h264_nvenc", &cfg, "aud").unwrap().join(" ");
        assert!(argv.contains("-init_hw_device d3d11va=d3d11:0"), "{argv}");
        assert!(argv.contains("ddagrab=output_idx=0:framerate=30"), "{argv}");
        assert!(argv.contains("scale_d3d11=1280:720:format=nv12"), "{argv}");
        assert!(argv.contains("-c:v h264_nvenc"), "{argv}");
        assert!(argv.contains("f32le"), "{argv}");
        assert!(!argv.contains("rawvideo"), "{argv}");
        assert!(argv.ends_with("rtmp://topaz.chat/live/test-key-123"), "{argv}");
    }

    #[test]
    fn build_direct_args_rejects_non_nvenc_and_unset() {
        let profiles = crate::config::default_profiles();
        let p = &profiles["mid"];
        let cfg = direct_cfg(ScreenTargetKind::Display, "display:0", Some("ddagrab"));
        assert!(build_direct_args(p, "libx264", &cfg, "aud").is_err());
        let unset = direct_cfg(ScreenTargetKind::Display, "display:0", None);
        assert!(build_direct_args(p, "h264_nvenc", &unset, "aud").is_err());
    }
}
