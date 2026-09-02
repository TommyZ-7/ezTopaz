//! Platform capture backends (design.md §3, §14 spike).
//!
//! Real backends land behind the `capture-linux` / `capture-windows` features
//! after the hardware spikes; the default build has no capture so core stays testable.
#![allow(dead_code)] // stub until platform features are enabled

#[cfg(all(unix, feature = "capture-linux"))]
pub mod linux;

#[cfg(all(windows, feature = "capture-windows"))]
pub mod windows;

#[cfg(all(unix, feature = "capture-linux"))]
pub use linux::{AudioCapture, ScreenCapture};
#[cfg(all(windows, feature = "capture-windows"))]
pub use windows::{AudioCapture, ScreenCapture};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("capture backend not available in this build (enable capture-linux / capture-windows)")]
    NotAvailable,
    #[error("X11 session detected; ezTopaz requires Wayland")]
    X11NotSupported,
    #[error("capture failed: {0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, CaptureError>;

/// Encode raw RGBA pixels as PNG (used by the preview pipeline, design §6.4).
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_raw(w, h, rgba.to_vec())?
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

/// A sink writer that swallows frames (preview mode: the capture backend feeds
/// `stream://preview` itself; no pipe/ffmpeg involved).
#[cfg(any(feature = "capture-linux", feature = "capture-windows"))]
pub fn null_video_writer() -> std::fs::File {
    std::fs::File::open(if cfg!(windows) { "NUL" } else { "/dev/null" })
        .expect("null device is always openable")
}
