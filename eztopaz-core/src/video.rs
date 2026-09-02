//! FramePacer (design.md §3.1.3).
//!
//! WGC/Portal only deliver frames when the content changes. rawvideo needs a
//! steady frame rate, so the pacer holds the last frame and re-emits it at the
//! profile fps. It also owns resolution normalization: pushed frames must be
//! exactly `w*h*4` bytes (BGRA) — capture backends scale before pushing.

use crate::error::{Error, Result};
use std::time::{Duration, Instant};

pub struct FramePacer {
    frame_size: usize,
    interval: Duration,
    last_frame: Option<Vec<u8>>,
    next_emit: Option<Instant>,
}

impl FramePacer {
    pub fn new(w: u32, h: u32, fps: u32) -> Self {
        Self {
            frame_size: (w as usize) * (h as usize) * 4,
            interval: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
            last_frame: None,
            next_emit: None,
        }
    }

    /// Feed a freshly captured frame (already normalized to profile w×h).
    pub fn push(&mut self, frame: &[u8]) -> Result<()> {
        if frame.len() != self.frame_size {
            return Err(Error::Capture(format!(
                "frame size mismatch: got {}, expected {} (resize must be handled upstream)",
                frame.len(),
                self.frame_size
            )));
        }
        let first = self.last_frame.is_none();
        self.last_frame = Some(frame.to_vec());
        if first {
            self.next_emit = Some(Instant::now());
        }
        Ok(())
    }

/// Advance to the next tick at or after `now`, collapsing backlog into one frame.
fn advance(next: Instant, interval: Duration, now: Instant) -> Instant {
    let mut next = next + interval;
    while next <= now {
        next += interval;
    }
    next
}

/// Returns the frame to emit now, if the fps tick has elapsed.
/// Skipped ticks collapse into one emit (never floods the pipe).
pub fn poll(&mut self, now: Instant) -> Option<&[u8]> {
    let next = self.next_emit?;
    if now < next {
        return None;
    }
    self.next_emit = Some(advance(next, self.interval, now));
    self.last_frame.as_deref()
}

    pub fn has_frame(&self) -> bool {
        self.last_frame.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacer() -> FramePacer {
        FramePacer::new(2, 2, 30) // 16-byte frames, 33.3ms interval
    }

    fn frame(byte: u8) -> Vec<u8> {
        vec![byte; 16]
    }

    #[test]
    fn duplicates_last_frame_at_fps() {
        let mut p = pacer();
        assert!(p.poll(Instant::now()).is_none(), "no frame before first push");
        p.push(&frame(1)).unwrap();

        let t0 = Instant::now();
        assert!(p.poll(t0).is_some(), "first frame emits immediately");
        let soon = t0 + Duration::from_millis(10);
        assert!(p.poll(soon).is_none(), "no duplicate within one interval");

        // 5 seconds with zero new captures → ~150 duplicated frames, not a flood
        let mut emitted = 0;
        let mut t = t0;
        let target = t0 + Duration::from_secs(5);
        while t < target {
            if p.poll(t).is_some() {
                emitted += 1;
            }
            t += Duration::from_millis(1);
        }
        assert!(
            (140..=160).contains(&emitted),
            "expected ~150 emits over 5s at 30fps, got {emitted}"
        );
    }

    #[test]
    fn emits_latest_frame_content() {
        let mut p = pacer();
        let t0 = Instant::now();
        p.push(&frame(1)).unwrap();
        p.poll(t0);
        p.push(&frame(2)).unwrap();
        let later = t0 + p.interval * 2;
        assert_eq!(p.poll(later), Some(frame(2).as_slice()));
    }

    #[test]
    fn rejects_size_mismatch() {
        let mut p = pacer();
        assert!(p.push(&vec![0u8; 15]).is_err());
        assert!(p.push(&vec![0u8; 17]).is_err());
        assert!(!p.has_frame());
    }

    #[test]
    fn skipped_ticks_collapse() {
        let mut p = pacer();
        let t0 = Instant::now();
        p.push(&frame(1)).unwrap();
        p.poll(Instant::now());
        // jump far ahead: one poll yields one frame
        let t = t0 + Duration::from_secs(10);
        assert!(p.poll(t).is_some());
        assert!(p.poll(t).is_none(), "same instant: no second emit");
    }

    #[test]
    fn advance_is_monotonic_and_bounded() {
        let interval = Duration::from_millis(33);
        let t0 = Instant::now();
        // far-ahead now: result is the first tick strictly after `now`
        let now = t0 + Duration::from_secs(100);
        let next = advance(t0, interval, now);
        assert!(next > now);
        assert!(next - now < interval);
        // near now: exactly one interval ahead
        let near = t0 + interval + Duration::from_millis(5);
        let next = advance(t0 + interval, interval, near);
        assert_eq!(next - (t0 + interval), interval);
    }
}
