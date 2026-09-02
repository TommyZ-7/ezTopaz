//! Linux capture backend: Portal (ashpd) + PipeWire.
//! Implemented under the `capture-linux` feature after the hardware spike
//! (design.md §14). Function signatures are the IPC contract (commands.rs).

use super::{CaptureError, Result};
use eztopaz_core::ipc_types::{AudioDevices, Display, ScreenTarget, WindowInfo};

pub fn list_displays() -> Result<Vec<Display>> {
    Err(CaptureError::NotAvailable)
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    // Portal environments have no app-side window list; the UI shows the picker.
    Ok(vec![])
}

pub fn portal_picker() -> Result<ScreenTarget> {
    Err(CaptureError::NotAvailable)
}

pub fn list_audio_devices() -> Result<AudioDevices> {
    Err(CaptureError::NotAvailable)
}
