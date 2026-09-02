//! Platform capture backends (design.md §3, §14 spike).
//!
//! Real backends land behind the `capture-linux` / `capture-windows` features
//! after the hardware spikes; the default build has no capture so core stays testable.
#![allow(dead_code)] // stub until platform features are enabled

#[cfg(all(unix, feature = "capture-linux"))]
pub mod linux;

#[cfg(all(windows, feature = "capture-windows"))]
pub mod windows;

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
