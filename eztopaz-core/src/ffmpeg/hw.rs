//! Hardware upload / zero-copy helpers (ffmpeg9 bundle).
//!
//! Default pipe path stays CPU (`None` filter) so existing backends keep
//! working without a GPU. `HwDirect` experimental builders live here and are
//! only used when the caller explicitly opts in (C10 Windows / C11 Linux).

use crate::ffmpeg::args::Transport;

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
/// (FramePacer), so no GPU scaling — upload only.
pub fn hw_upload_chain(backend: &HwBackend) -> Option<String> {
    match backend {
        HwBackend::None => None,
        HwBackend::Cuda => Some("hwupload_cuda".into()),
        HwBackend::D3d11va | HwBackend::D3d12va => Some("hwupload".into()),
        HwBackend::Vaapi => Some("format=nv12,hwupload".into()),
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

/// Experimental direct-capture input argv (C10/C11 future, NOT wired into
/// `StartPlan` yet). Kept here so the ffmpeg9 device syntax is reviewed in
/// one place and unit-tested without touching the live pipe path.
///
/// - Windows: `ddagrab` (fullscreen) is the stable choice today; `gfxcapture`
///   (WGC-based, ffmpeg 8.1+) is the window-aware successor. Both deliver
///   D3D11 frames that `hwupload`/`scale_d3d11` can keep on-GPU.
/// - Linux: PipeWire DMA-BUF import is compositor-dependent; the stable
///   stepping stone is NV12 pipe + `format=nv12,hwupload` (vaapi) above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSource {
    DdagrabFullscreen,
    GfxCaptureWindow,
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
}
