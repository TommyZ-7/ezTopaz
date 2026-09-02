//! Audio mixing in Rust (design.md §3.2.3).
//!
//! WASAPI / PipeWire hand us f32 samples; we sum, gate, scale and clamp in f32
//! and pipe `f32le 48kHz stereo` straight to FFmpeg. VU (peak/rms) is computed
//! per source and for the master in the same pass.

pub mod sink;

pub use sink::{f32le_bytes, AudioSink, MIC_ID};

use crate::error::Result;
use crate::ipc_types::{SourceGain, VuLevel, VuMeter};
use std::collections::BTreeMap;

/// Linear-interpolation resample of interleaved stereo f32 (capture devices
/// whose mix format is not 48kHz; design §3.2.1).
pub fn resample_stereo(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.len() < 4 {
        return samples.to_vec();
    }
    let in_frames = samples.len() / 2;
    let out_frames = ((in_frames as f64) * (to_rate as f64) / (from_rate as f64)).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = (i as f64) * (from_rate as f64) / (to_rate as f64);
        let i0 = (pos as usize).min(in_frames - 1);
        let i1 = (i0 + 1).min(in_frames - 1);
        let t = (pos - i0 as f64) as f32;
        for ch in 0..2 {
            let a = samples[i0 * 2 + ch];
            let b = samples[i1 * 2 + ch];
            out.push(a + (b - a) * t);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceState {
    pub gain: f32,
    pub muted: bool,
    pub enabled: bool,
}

impl Default for SourceState {
    fn default() -> Self {
        Self { gain: 1.0, muted: false, enabled: true }
    }
}

impl From<SourceGain> for SourceState {
    fn from(g: SourceGain) -> Self {
        Self { gain: g.gain, muted: g.muted, enabled: true }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Mixer {
    pub apps: BTreeMap<String, SourceState>,
    pub mic: SourceState,
}

impl Mixer {
    /// Mix stereo-interleaved f32 buffers. Output length = longest input;
    /// shorter sources are silence-padded (dropping samples would glitch).
    pub fn mix(
        &self,
        samples: &BTreeMap<String, Vec<f32>>,
        mic: Option<&[f32]>,
    ) -> (Vec<f32>, VuMeter) {
        let master_len = samples
            .values()
            .map(|s| s.len())
            .chain(mic.map(|s| s.len()))
            .max()
            .unwrap_or(0);

        let mut out = vec![0f32; master_len];
        let mut master_peak = 0f32;
        let mut master_sq = 0f64;

        let mut app_vu = BTreeMap::new();
        for (id, state) in &self.apps {
            let Some(buf) = samples.get(id) else {
                app_vu.insert(id.clone(), VuLevel::default());
                continue;
            };
            let (peak, rms) = accumulate(&mut out, buf, effective_gain(state));
            app_vu.insert(id.clone(), VuLevel { peak, rms });
        }

        let mic_vu = mic.and_then(|buf| {
            let state = &self.mic;
            if !state.enabled {
                return None; // F-AU-03: OFF = not mixed at all
            }
            let out_len = out.len().min(buf.len());
            let (peak, rms) = accumulate(&mut out[..out_len], buf, effective_gain(state));
            Some(VuLevel { peak, rms })
        });

        // master VU over the mixed output
        for &x in &out {
            let a = x.abs();
            if a > master_peak {
                master_peak = a;
            }
            master_sq += (x as f64) * (x as f64);
        }
        let master_rms = if out.is_empty() {
            0.0
        } else {
            (master_sq / out.len() as f64).sqrt() as f32
        };

        (
            out,
            VuMeter {
                apps: app_vu,
                mic: mic_vu,
                master: VuLevel { peak: master_peak, rms: master_rms },
            },
        )
    }
}

/// muted or zero-gain sources contribute nothing but still report VU? No —
/// muted means silent, so VU reads 0. (AC-03: mute → VU 0)
fn effective_gain(s: &SourceState) -> f32 {
    if s.muted || !s.enabled {
        0.0
    } else {
        s.gain.clamp(0.0, 2.0)
    }
}

/// `buf * gain` added into `out`; returns (peak, rms) of the *source* contribution.
fn accumulate(out: &mut [f32], buf: &[f32], gain: f32) -> (f32, f32) {
    let n = out.len().min(buf.len());
    let mut peak = 0f32;
    let mut sq = 0f64;
    for i in 0..n {
        let s = buf[i] * gain;
        let clamped = s.clamp(-1.0, 1.0); // ponytail: hard clamp; soft-knee limiter if mixing distorts audibly
        out[i] += clamped;
        let a = clamped.abs();
        if a > peak {
            peak = a;
        }
        sq += (clamped as f64) * (clamped as f64);
    }
    let rms = if n == 0 { 0.0 } else { (sq / n as f64).sqrt() as f32 };
    (peak, rms)
}

/// Validate a gain coming from the UI slider (0.0 - 2.0).
pub fn sanitize_gain(g: f32) -> Result<f32> {
    if g.is_finite() && (0.0..=2.0).contains(&g) {
        Ok(g)
    } else {
        Err(crate::error::Error::Config(format!("gain out of range: {g}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_state(gain: f32, muted: bool) -> SourceState {
        SourceState { gain, muted, enabled: true }
    }

    #[test]
    fn sums_two_apps() {
        let mut m = Mixer::default();
        m.apps.insert("chrome".into(), app_state(1.0, false));
        m.apps.insert("spotify".into(), app_state(1.0, false));
        let mut samples = BTreeMap::new();
        samples.insert("chrome".into(), vec![0.25, 0.25]);
        samples.insert("spotify".into(), vec![0.25, 0.25]);
        let (out, vu) = m.mix(&samples, None);
        assert_eq!(out, vec![0.5, 0.5]);
        assert!((vu.master.peak - 0.5).abs() < 1e-6);
        assert!((vu.master.rms - 0.5).abs() < 1e-6);
        assert!((vu.apps["chrome"].peak - 0.25).abs() < 1e-6);
    }

    #[test]
    fn muted_app_is_silent_and_vu_zero() {
        let mut m = Mixer::default();
        m.apps.insert("chrome".into(), app_state(1.0, true));
        let mut samples = BTreeMap::new();
        samples.insert("chrome".into(), vec![0.5, 0.5]);
        let (out, vu) = m.mix(&samples, None);
        assert_eq!(out, vec![0.0, 0.0]);
        assert_eq!(vu.apps["chrome"].peak, 0.0);
    }

    #[test]
    fn mic_enabled_false_is_not_mixed() {
        let mut m = Mixer::default();
        m.mic = SourceState { enabled: false, ..Default::default() };
        let samples = BTreeMap::new();
        let (out, vu) = m.mix(&samples, Some(&[0.5, 0.5]));
        assert_eq!(out, vec![0.0, 0.0]);
        assert!(vu.mic.is_none());
    }

    #[test]
    fn mic_muted_reads_zero_vu() {
        let mut m = Mixer::default();
        m.mic = SourceState { enabled: true, muted: true, ..Default::default() };
        let samples = BTreeMap::new();
        let (out, vu) = m.mix(&samples, Some(&[0.5, 0.5]));
        assert_eq!(out, vec![0.0, 0.0]);
        assert_eq!(vu.mic.unwrap().peak, 0.0);
    }

    #[test]
    fn gain_scales_and_clamps() {
        let mut m = Mixer::default();
        m.apps.insert("a".into(), app_state(2.0, false));
        let mut samples = BTreeMap::new();
        samples.insert("a".into(), vec![0.9, -0.9]);
        let (out, _) = m.mix(&samples, None);
        assert_eq!(out, vec![1.0, -1.0]); // 1.8 clamped to 1.0
    }

    #[test]
    fn mismatched_lengths_use_shortest_mix_len() {
        let mut m = Mixer::default();
        m.apps.insert("a".into(), app_state(1.0, false));
        let mut samples = BTreeMap::new();
        samples.insert("a".into(), vec![0.5, 0.5, 0.5, 0.5]);
        let (out, _) = m.mix(&samples, Some(&[0.5, 0.5]));
        assert_eq!(out.len(), 4); // longest input wins, mic padded with silence
    }

    #[test]
    fn gain_sanitizer() {
        assert_eq!(sanitize_gain(1.5).unwrap(), 1.5);
        assert!(sanitize_gain(-0.1).is_err());
        assert!(sanitize_gain(f32::NAN).is_err());
    }

    #[test]
    fn resample_identity_and_rates() {
        let src: Vec<f32> = (0..480).map(|i| i as f32 / 480.0).collect(); // 240 stereo frames
        assert_eq!(resample_stereo(&src, 48000, 48000), src);
        let half = resample_stereo(&src, 48000, 24000); // 240 frames → 120 frames = 240 samples
        assert_eq!(half.len(), 240);
        let dbl = resample_stereo(&src, 24000, 48000); // 240 frames → 480 frames = 960 samples
        assert_eq!(dbl.len(), 960);
        assert!(sanitize_gain(f32::INFINITY).is_err());
    }
}
