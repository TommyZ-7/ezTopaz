//! Windows capture backend: WGC (screen) + WASAPI process loopback (audio, 2004+).
//! Implemented under the `capture-windows` feature after the hardware spike
//! (design.md §14). Function signatures are the IPC contract (commands.rs).

use super::{CaptureError, Result};
use eztopaz_core::ipc_types::{AudioDevices, Display, WindowInfo};

pub fn list_displays() -> Result<Vec<Display>> {
    Err(CaptureError::NotAvailable)
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    Err(CaptureError::NotAvailable)
}

pub fn list_audio_devices() -> Result<AudioDevices> {
    Err(CaptureError::NotAvailable)
}
