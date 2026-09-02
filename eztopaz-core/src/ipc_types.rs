//! Shared IPC types (design.md §5.3). Serialized over Tauri IPC (camelCase JSON).

pub use crate::config::{MicSource, ScreenTarget, ScreenTargetKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamConfig {
    pub ingest_url: String,
    pub stream_key: String,
    pub screen: ScreenTarget,
    pub audio: AudioSelection,
    pub profile_id: String,
    pub encoder_override: String,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            ingest_url: "rtmp://topaz.chat/live".into(),
            stream_key: String::new(),
            screen: ScreenTarget::default(),
            audio: AudioSelection::default(),
            profile_id: "mid".into(),
            encoder_override: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSelection {
    /// "system" | "apps" (F-AU-01 / F-AU-02a, mutually exclusive)
    pub mode: String,
    pub apps: Vec<String>,
    pub mic: MicSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioMixUpdate {
    pub apps: BTreeMap<String, SourceGain>,
    pub mic: MicUpdate,
}

impl Default for AudioMixUpdate {
    fn default() -> Self {
        Self { apps: BTreeMap::new(), mic: MicUpdate::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceGain {
    pub gain: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MicUpdate {
    pub enabled: bool,
    pub muted: bool,
    pub gain: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub is_live: bool,
    pub duration_sec: u64,
    pub bitrate_kbps: f64,
    pub dropped_frames: u64,
    pub retrying: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VuMeter {
    pub apps: BTreeMap<String, VuLevel>,
    pub mic: Option<VuLevel>,
    pub master: VuLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VuLevel {
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrame {
    /// data URL (`data:image/png;base64,...`)
    pub data_url: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub level: String,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamError {
    pub code: String,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub id: String,
    pub label: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioDevices {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
    pub apps: Vec<AppAudio>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppAudio {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub name: String,
    pub usable: bool,
    pub reason: Option<String>,
}


