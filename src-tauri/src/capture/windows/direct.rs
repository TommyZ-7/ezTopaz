//! ddagrab direct-input session (C10).
//!
//! ffmpeg captures the fullscreen output itself (`-f lavfi -i ddagrab`,
//! D3D11 frames → `scale_d3d11` → `h264_nvenc`); the video pipe, `VideoSink`
//! and WGC capture are bypassed. Audio still flows through the mixed `f32le`
//! pipe. Window targets stay on the WGC path (`build_direct_args` rejects
//! them). Selected via `StreamConfig.direct_input = "ddagrab"`.

use super::screen::ScreenCapture;

/// What feeds the video leg of a running stream.
pub enum ScreenHandle {
    /// Rust WGC capture → `VideoSink` → pipe (default).
    Wgc(ScreenCapture),
    /// ffmpeg ddagrab device input (no Rust video leg).
    Direct,
}

impl ScreenHandle {
    pub fn stop(&mut self) {
        match self {
            ScreenHandle::Wgc(s) => s.stop(),
            ScreenHandle::Direct => {}
        }
    }
}
