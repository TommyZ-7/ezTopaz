//! Windows capture backend (design.md §3.1.1, §3.2.1).
//!
//! - Screen/window: WGC (free-threaded frame pool) → BGRA staging readback →
//!   scale to profile → [`VideoSink`].
//! - Audio: WASAPI shared-mode. System = render-device loopback (polling),
//!   mic = capture device, per-app = process loopback (Win10 2004+).
//!
//! Compile-verified with `cargo check --target x86_64-pc-windows-msvc
//! --features capture-windows`; runtime behavior needs a Windows machine.

mod audio;
mod enumerate;
mod screen;

pub use audio::start_audio;
pub use enumerate::{list_audio_devices, list_displays, list_windows};
pub use screen::{start_screen, ScreenCapture};

use super::CaptureError;
use eztopaz_core::audio::AudioSink;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub type Result<T> = std::result::Result<T, CaptureError>;

/// Running audio capture threads; stopping kills the loops and the sink.
pub struct AudioCapture {
    stops: Vec<Arc<AtomicBool>>,
    handles: Vec<JoinHandle<()>>,
    pub sink: Option<AudioSink>,
}

impl AudioCapture {
    pub fn new(sink: AudioSink) -> Self {
        Self { stops: Vec::new(), handles: Vec::new(), sink: Some(sink) }
    }

    fn add(&mut self, stop: Arc<AtomicBool>, handle: JoinHandle<()>) {
        self.stops.push(stop);
        self.handles.push(handle);
    }

    pub fn stop(&mut self) {
        for s in &self.stops {
            s.store(true, Ordering::Relaxed);
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
        if let Some(s) = self.sink.as_mut() {
            s.stop();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn err<E: std::fmt::Display>(e: E) -> CaptureError {
    CaptureError::Failed(e.to_string())
}

/// COM apartment per capture thread.
pub(super) fn co_init() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}
