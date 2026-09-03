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
/// Returns `None` for both pipe transports: `PipeBgra` uses CPU swscale,
/// `PipeNv12` is already GPU-friendly system memory that NVENC/QSV/VAAPI/AMF
/// accept without an explicit upload filter (implicit upload). `HwDirect`
/// reserves the upload filter for the future direct-capture path; it also
/// returns `None` until `-init_hw_device` wiring lands, so no broken argv is
/// emitted by default.
pub fn hw_upload_filter(_encoder: &str, _transport: &Transport) -> Option<String> {
    None
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
}
