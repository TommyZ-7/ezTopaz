//! Video pipe sink: capture frames → FramePacer → rawvideo FIFO.
//!
//! Capture backends push frames through [`VideoSink::push`]; the pump thread
//! emits them at the profile fps (static screens keep flowing) and writes to
//! the named pipe. Clonable so capture callbacks can hold a handle.

use super::FramePacer;
use crate::error::{Error, Result};
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct VideoSink {
    tx: Arc<mpsc::Sender<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl VideoSink {
    pub fn spawn(writer: File, w: u32, h: u32, fps: u32) -> Result<Self> {
        let pacer = FramePacer::new(w, h, fps);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let interval = Duration::from_secs_f64(1.0 / fps.max(1) as f64);

        let handle = std::thread::Builder::new()
            .name("video-sink".into())
            .spawn(move || {
                let mut pacer = pacer;
                let mut writer = writer;
                loop {
                    if stop2.load(Ordering::Relaxed) {
                        return;
                    }
                    // wait for a frame (or tick), keeping only the newest
                    match rx.recv_timeout(interval / 4) {
                        Ok(frame) => {
                            let _ = pacer.push(&frame);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    while let Ok(frame) = rx.try_recv() {
                        let _ = pacer.push(&frame); // newest frame wins
                    }
                    let now = Instant::now();
                    while let Some(frame) = pacer.poll(now) {
                        if writer.write_all(frame).is_err() {
                            return; // reader (ffmpeg) gone
                        }
                    }
                }
            })
            .map_err(|e| Error::Capture(format!("video sink thread: {e}")))?;

        Ok(Self {
            tx: Arc::new(tx),
            stop,
            handle: Arc::new(Mutex::new(Some(handle))),
        })
    }

    /// Push a freshly captured (already scaled) frame. Returns false when stopped.
    pub fn push(&self, frame: Vec<u8>) -> bool {
        self.tx.send(frame).is_ok()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.handle.lock() {
            if let Some(h) = slot.take() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for VideoSink {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Nearest-neighbor BGRA scale + letterbox into `dst_w x dst_h` (design §3.1.3:
/// all frames are normalized to the profile size before the pipe).
pub fn scale_bgra(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    scale_bgra_into(&mut dst, src, src_w, src_h, dst_w, dst_h);
    dst
}

pub fn scale_bgra_into(dst: &mut [u8], src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) {
    let dst_len = (dst_w as usize) * (dst_h as usize) * 4;
    if dst.len() < dst_len {
        return;
    }
    // letterbox: opaque black everywhere first
    for px in dst[..dst_len].chunks_exact_mut(4) {
        px[0] = 0;
        px[1] = 0;
        px[2] = 0;
        px[3] = 255;
    }
    if src_w == 0 || src_h == 0 || src.len() < (src_w as usize) * (src_h as usize) * 4 {
        return;
    }
    let scale = (dst_w as f64 / src_w as f64).min(dst_h as f64 / src_h as f64);
    let out_w = (((src_w as f64) * scale) as u32).max(1).min(dst_w);
    let out_h = (((src_h as f64) * scale) as u32).max(1).min(dst_h);
    let off_x = (dst_w - out_w) / 2;
    let off_y = (dst_h - out_h) / 2;

    let x_ratio = src_w as f64 / out_w as f64;
    let y_ratio = src_h as f64 / out_h as f64;
    let sw = src_w as usize;
    let sh = src_h as usize;
    for oy in 0..out_h {
        let sy = (((oy as f64 + 0.5) * y_ratio) as usize).min(sh - 1);
        let dst_row = ((oy + off_y) as usize) * (dst_w as usize) * 4;
        for ox in 0..out_w {
            let sx = (((ox as f64 + 0.5) * x_ratio) as usize).min(sw - 1);
            let s = (sy * sw + sx) * 4;
            let d = dst_row + ((ox + off_x) as usize) * 4;
            dst[d] = src[s];
            dst[d + 1] = src[s + 1];
            dst[d + 2] = src[s + 2];
            dst[d + 3] = 255;
        }
    }
}

/// BGRA → RGBA in place (for PNG preview encoding).
pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4)
        .flat_map(|px| [px[2], px[1], px[0], px[3]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_identity() {
        let src = vec![7u8; 4 * 4 * 4]; // 4x4 BGRA
        let dst = scale_bgra(&src, 4, 4, 4, 4);
        assert_eq!(dst.len(), 64);
        assert!(dst.chunks(4).all(|px| px == [7, 7, 7, 255]));
    }

    #[test]
    fn scale_downsamples_with_letterbox() {
        // 4x4 source into 8x4 dest: fits to 4x4, centered with 2px bars each side
        let src = vec![10u8; 4 * 4 * 4];
        let dst = scale_bgra(&src, 4, 4, 8, 4);
        assert_eq!(dst.len(), 8 * 4 * 4);
        // first pixel is letterbox black, center pixels carry the source color
        assert_eq!(&dst[0..4], &[0, 0, 0, 255]);
        assert_eq!(&dst[4 * 4..4 * 4 + 4], &[10, 10, 10, 255]);
        assert_eq!(&dst[7 * 4..8 * 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn scale_rejects_bad_source() {
        let dst = scale_bgra(&[0u8; 3], 4, 4, 4, 4);
        assert_eq!(dst.len(), 64);
        assert!(dst.chunks(4).all(|px| px == [0, 0, 0, 255]));
    }

    #[test]
    fn bgra_rgba_swap() {
        let rgba = bgra_to_rgba(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(rgba, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
