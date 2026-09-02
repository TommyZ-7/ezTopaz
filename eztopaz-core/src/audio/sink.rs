//! Audio pipe sink: per-source blocks → Rust Mixer → f32le FIFO (design §3.2.3).
//!
//! Capture backends push interleaved stereo f32 blocks tagged by source id
//! (`"mic"` for the microphone). The sink thread mixes what is available and
//! writes continuously; sources that stop pushing drop out (their device closed)
//! instead of stalling the stream.

use super::Mixer;
use crate::error::{Error, Result};
use crate::ipc_types::VuMeter;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const MIC_ID: &str = "__mic__";
/// ~10ms of stereo @48kHz
const BLOCK: usize = 960;

#[derive(Clone)]
pub struct AudioSink {
    tx: mpsc::Sender<(String, Vec<f32>)>,
    stop: Arc<AtomicBool>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub mixer: Arc<Mutex<Mixer>>,
    pub last_vu: Arc<Mutex<VuMeter>>,
}

impl AudioSink {
    pub fn spawn(writer: File, mixer: Arc<Mutex<Mixer>>) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<(String, Vec<f32>)>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let last_vu = Arc::new(Mutex::new(VuMeter::default()));
        let last_vu2 = last_vu.clone();
        let mixer_thread = mixer.clone();

        let handle = std::thread::Builder::new()
            .name("audio-sink".into())
            .spawn(move || {
                let mut writer = writer;
                let mut queues: HashMap<String, VecDeque<f32>> = HashMap::new();
                loop {
                    if stop2.load(Ordering::Relaxed) {
                        return;
                    }
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok((id, block)) => {
                            auto_register(&mixer_thread, &id);
                            queues.entry(id).or_default().extend(block);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    while let Ok((id, block)) = rx.try_recv() {
                        auto_register(&mixer_thread, &id);
                        queues.entry(id).or_default().extend(block);
                    }

                    // mix the shortest available run across sources that have data
                    let available: Vec<String> = queues
                        .iter()
                        .filter(|(_, q)| !q.is_empty())
                        .map(|(id, _)| id.clone())
                        .collect();
                    if available.is_empty() {
                        continue;
                    }
                    let n = available
                        .iter()
                        .map(|id| queues[id].len())
                        .min()
                        .unwrap_or(0)
                        .min(BLOCK);

                    let mut samples: BTreeMap<String, Vec<f32>> = BTreeMap::new();
                    for id in &available {
                        let q = queues.get_mut(id).unwrap();
                        samples.insert(id.clone(), q.drain(..n).collect());
                    }
                    let mic_slice = samples.get(MIC_ID).map(|v| v.as_slice());
                    let apps: BTreeMap<String, Vec<f32>> = samples
                        .iter()
                        .filter(|(id, _)| id.as_str() != MIC_ID)
                        .map(|(id, v)| (id.clone(), v.clone()))
                        .collect();

                    let (out, vu) = mixer_thread.lock().unwrap().mix(&apps, mic_slice);
                    if writer.write_all(&f32le_bytes(&out)).is_err() {
                        return; // reader (ffmpeg) gone
                    }
                    *last_vu2.lock().unwrap() = vu;
                }
            })
            .map_err(|e| Error::Capture(format!("audio sink thread: {e}")))?;

        Ok(Self {
            tx,
            stop,
            handle: Arc::new(Mutex::new(Some(handle))),
            mixer,
            last_vu,
        })
    }

    /// Push an interleaved stereo f32 block for a source. `"mic"` is the microphone.
    pub fn push(&self, source_id: &str, block: Vec<f32>) -> bool {
        self.tx.send((source_id.to_string(), block)).is_ok()
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

impl Drop for AudioSink {
    fn drop(&mut self) {
        self.stop();
    }
}

fn auto_register(mixer: &Arc<Mutex<Mixer>>, id: &str) {
    if id == MIC_ID {
        return; // mixer.mic always exists
    }
    let mut m = mixer.lock().unwrap();
    m.apps.entry(id.to_string()).or_default();
}

/// f32 → little-endian bytes (the pipe format ffmpeg reads with `-f f32le`).
pub fn f32le_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_f32s(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    #[test]
    fn mixes_two_sources_into_file() {
        let dir = std::env::temp_dir().join(format!("eztopaz-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mix.f32");
        let file = File::create(&path).unwrap();
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let sink = AudioSink::spawn(file, mixer).unwrap();

        sink.push("chrome", vec![0.5; 960 * 3]);
        sink.push("spotify", vec![0.25; 960]);
        std::thread::sleep(Duration::from_millis(300));
        sink.stop();

        let bytes = std::fs::read(&path).unwrap();
        let samples = read_f32s(&bytes);
        assert!(!samples.is_empty(), "sink wrote mixed samples");
        // both mixed while spotify's block was queued; chrome alone once it starves
        assert!(samples.iter().any(|s| (s - 0.75).abs() < 1e-6), "both sources mixed");
        assert!(samples.iter().any(|s| (s - 0.5).abs() < 1e-6), "chrome alone after join ends");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stopped_source_drops_out() {
        let dir = std::env::temp_dir().join(format!("eztopaz-audio2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mix.f32");
        let file = File::create(&path).unwrap();
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let sink = AudioSink::spawn(file, mixer).unwrap();

        sink.push("a", vec![0.5; 960 * 3]);
        std::thread::sleep(Duration::from_millis(100));
        sink.push("b", vec![0.5; 960]); // b stops after one block
        std::thread::sleep(Duration::from_millis(100));
        sink.push("a", vec![0.5; 960]); // a keeps going → mixed alone afterwards
        std::thread::sleep(Duration::from_millis(100));
        sink.stop();

        let samples = read_f32s(&std::fs::read(&path).unwrap());
        assert!(!samples.is_empty());
        // while both flow: 1.0; after b starves: 0.5 (order depends on arrival timing)
        assert!(samples.iter().any(|s| (s - 1.0).abs() < 1e-6), "both mixed while b flows");
        assert!(
            samples.iter().any(|s| (s - 0.5).abs() < 1e-6),
            "b dropping out leaves a alone"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn f32le_bytes_roundtrip() {
        let v = vec![0.5f32, -1.0, 0.0];
        let b = f32le_bytes(&v);
        assert_eq!(b.len(), 12);
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 0.5);
    }
}
