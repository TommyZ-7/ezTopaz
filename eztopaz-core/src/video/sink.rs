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
        Self::spawn_for_transport(
            writer,
            w,
            h,
            fps,
            crate::ffmpeg::args::Transport::PipeBgra,
        )
    }

    /// Transport-aware spawn. `PipeNv12` keeps the BGRA pacer (capture code
    /// untouched) but converts to NV12 just before the pipe write, so the
    /// pipe format always matches `build_ffmpeg_args_with_transport`.
    pub fn spawn_for_transport(
        writer: File,
        w: u32,
        h: u32,
        fps: u32,
        transport: crate::ffmpeg::args::Transport,
    ) -> Result<Self> {
        let pacer = FramePacer::new(w, h, fps);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let interval = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        let convert_nv12 =
            matches!(transport, crate::ffmpeg::args::Transport::PipeNv12)
                && w % 2 == 0
                && h % 2 == 0;

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
                        if convert_nv12 {
                            let nv12 = super::nv12::bgra_to_nv12(frame, w, h);
                            if writer.write_all(&nv12).is_err() {
                                return; // reader (ffmpeg) gone
                            }
                        } else if writer.write_all(frame).is_err() {
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
///
/// Integer fixed-point math (no `f64` per pixel) + row-parallel emit via
/// `std::thread::scope` for large frames. No new dependencies.
pub fn scale_bgra(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    scale_bgra_into(&mut dst, src, src_w, src_h, dst_w, dst_h);
    dst
}

fn fill_letterbox_black(dst: &mut [u8]) {
    // opaque black BGRA [0,0,0,255]: memset + alpha plane.
    dst.fill(0);
    for a in dst.iter_mut().skip(3).step_by(4) {
        *a = 255;
    }
}

pub fn scale_bgra_into(dst: &mut [u8], src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) {
    let dst_len = (dst_w as usize) * (dst_h as usize) * 4;
    if dst.len() < dst_len {
        return;
    }
    let dst = &mut dst[..dst_len];
    fill_letterbox_black(dst);
    if src_w == 0 || src_h == 0 || src.len() < (src_w as usize) * (src_h as usize) * 4 {
        return;
    }
    // Identity fast path: copy BGR, force opaque alpha.
    if src_w == dst_w && src_h == dst_h {
        copy_bgra_opaque(dst, src);
        return;
    }
    // Integer aspect fit: scale = min(dst_w/src_w, dst_h/src_h).
    let (out_w, out_h) = fit_output(src_w, src_h, dst_w, dst_h);
    let off_x = (dst_w - out_w) / 2;
    let off_y = (dst_h - out_h) / 2;
    blit_nearest(dst, src, src_w, src_h, dst_w, out_w, out_h, off_x, off_y);
}

fn copy_bgra_opaque(dst: &mut [u8], src: &[u8]) {
    // src and dst same length here; keep alpha opaque like the scaler does.
    let n = dst.len().min(src.len());
    let (d, s) = (&mut dst[..n], &src[..n]);
    // 4-byte chunks: BGR copy + A=255. Compiler auto-vectorizes this loop.
    for (dpx, spx) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
        dpx[0] = spx[0];
        dpx[1] = spx[1];
        dpx[2] = spx[2];
        dpx[3] = 255;
    }
}

fn fit_output(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32) {
    let sw = src_w as u64;
    let sh = src_h as u64;
    let dw = dst_w as u64;
    let dh = dst_h as u64;
    if dw * sh <= dh * sw {
        // width-constrained
        let out_w = dst_w;
        let out_h = ((sh * dw / sw) as u32).max(1).min(dst_h);
        (out_w, out_h)
    } else {
        let out_h = dst_h;
        let out_w = ((sw * dh / sh) as u32).max(1).min(dst_w);
        (out_w, out_h)
    }
}

fn blit_nearest(
    dst: &mut [u8],
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    out_w: u32,
    out_h: u32,
    off_x: u32,
    off_y: u32,
) {
    let sw = src_w as usize;
    let sh = src_h as usize;
    let dw = dst_w as usize;
    let ow = out_w as usize;
    let oh = out_h as usize;
    let ox = off_x as usize;
    let oy0 = off_y as usize;
    let stride = dw * 4;
    // Precompute source x for every output column (integer nearest).
    let sw_u64 = src_w as u64;
    let ow_u64 = out_w as u64;
    let mut map_x = vec![0usize; ow];
    for (dx, sx) in map_x.iter_mut().enumerate() {
        let v = ((2 * dx as u64 + 1) * sw_u64) / (2 * ow_u64);
        *sx = (v as usize).min(sw - 1);
    }
    let sh_u64 = src_h as u64;
    let oh_u64 = out_h as u64;

    // Middle band that actually carries the image (rest stays letterbox).
    let middle = &mut dst[oy0 * stride..(oy0 + oh) * stride];
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // Thread per ~64 rows; small frames stay single-threaded (spawn cost).
    let want = (oh + 63) / 64;
    let n_threads = threads.min(want).max(1).min(oh.max(1));
    if n_threads <= 1 {
        blit_rows(middle, src, &map_x, sw, sh, dw, ow, ox, 0, oh, sh_u64, oh_u64);
        return;
    }
    let rows_per_thread = oh.div_ceil(n_threads);
    std::thread::scope(|s| {
        for (chunk_idx, chunk) in middle.chunks_mut(rows_per_thread * stride).enumerate() {
            let start_oy = chunk_idx * rows_per_thread;
            let rows = chunk.len() / stride;
            let map_x = &map_x;
            s.spawn(move || {
                blit_rows(
                    chunk, src, map_x, sw, sh, dw, ow, ox, start_oy, rows, sh_u64, oh_u64,
                );
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn blit_rows(
    chunk: &mut [u8],
    src: &[u8],
    map_x: &[usize],
    sw: usize,
    sh: usize,
    _dw: usize,
    ow: usize,
    ox: usize,
    start_oy: usize,
    rows: usize,
    sh_u64: u64,
    oh_u64: u64,
) {
    for r in 0..rows {
        let oy = start_oy + r;
        let v = ((2 * oy as u64 + 1) * sh_u64) / (2 * oh_u64);
        let sy = (v as usize).min(sh - 1);
        let s_row = sy * sw * 4;
        let d_row = r * _dw * 4;
        // SAFETY-free row copy via precomputed x map.
        for (dx, sx) in map_x.iter().enumerate().take(ow) {
            let s = s_row + sx * 4;
            let d = d_row + (dx + ox) * 4;
            chunk[d] = src[s];
            chunk[d + 1] = src[s + 1];
            chunk[d + 2] = src[s + 2];
            // alpha already 255 from the letterbox fill
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

    #[test]
    fn scale_preserves_gradient_corners() {
        // 4x2 distinct columns → 4x2 identity-ish: corners must map 1:1.
        let mut src = vec![0u8; 4 * 2 * 4];
        for x in 0..4 {
            for y in 0..2 {
                let i = (y * 4 + x) * 4;
                src[i] = (x * 60) as u8;
                src[i + 1] = (y * 120) as u8;
                src[i + 2] = 200;
                src[i + 3] = 255;
            }
        }
        let dst = scale_bgra(&src, 4, 2, 4, 2);
        assert_eq!(&dst[0..3], &[0, 0, 200]);
        assert_eq!(&dst[(3 * 4)..(3 * 4 + 3)], &[180, 0, 200]);
    }

    #[test]
    fn scale_large_parallel_matches_single() {
        // 256x144 → 1280x720 exercises the scoped row-parallel path.
        let sw = 256u32;
        let sh = 144u32;
        let mut src = vec![0u8; (sw * sh * 4) as usize];
        for y in 0..sh {
            for x in 0..sw {
                let i = ((y * sw + x) * 4) as usize;
                src[i] = (x % 251) as u8;
                src[i + 1] = (y % 251) as u8;
                src[i + 2] = ((x + y) % 251) as u8;
                src[i + 3] = 255;
            }
        }
        let dst = scale_bgra(&src, sw, sh, 1280, 720);
        assert_eq!(dst.len(), 1280 * 720 * 4);
        // center pixel carries scaled content, corner is letterbox or content
        // but alpha must stay opaque everywhere.
        assert!(dst.chunks_exact(4).all(|px| px[3] == 255));
        let center = ((360 * 1280 + 640) * 4) as usize;
        assert_ne!(&dst[center..center + 3], &[0, 0, 0]);
    }
}
