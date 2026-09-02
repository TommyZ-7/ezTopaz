//! profiles.json management (requirements.md §8.2, design.md §7).
//!
//! Path: Win `%APPDATA%/ezTopaz/`, Linux `~/.config/ezTopaz/`.
//! Save is atomic (tmp + rename; `std::fs::rename` replaces existing files on
//! Windows too via MOVEFILE_REPLACE_EXISTING).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_DIR_NAME: &str = "ezTopaz";
pub const CONFIG_FILE_NAME: &str = "profiles.json";
pub const SCHEMA_VERSION: u32 = 2;

pub const MAX_VIDEO_KBPS: u32 = 2000;
pub const MAX_AUDIO_KBPS: u32 = 320;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    #[serde(rename = "v_kbps")]
    pub v_kbps: u32,
    #[serde(rename = "a_kbps")]
    pub a_kbps: u32,
    #[serde(default = "default_encoder")]
    pub encoder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
}

fn default_encoder() -> String {
    "auto".into()
}

impl Profile {
    /// F-EN-05: GOP = 2 seconds.
    pub fn gop(&self) -> u32 {
        self.fps * 2
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicSource {
    pub device: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "one")]
    pub gain: f32,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

impl Default for MicSource {
    fn default() -> Self {
        Self { device: "default".into(), enabled: true, muted: false, gain: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ScreenTarget {
    #[serde(rename = "type")]
    pub kind: ScreenTargetKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTargetKind {
    #[default]
    Display,
    Window,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LastSources {
    #[serde(default)]
    pub screen: ScreenTarget,
    #[serde(default, rename = "includeApps")]
    pub include_apps: Vec<String>,
    #[serde(default)]
    pub mic: MicSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfilesConfig {
    pub version: u32,
    pub locale: String,
    pub ingest_url: String,
    pub active_profile: String,
    pub profiles: std::collections::BTreeMap<String, Profile>,
    pub last_stream_key: String,
    pub last_sources: LastSources,
    pub encoder_override: String,
}

impl Default for ProfilesConfig {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            locale: default_locale(),
            ingest_url: "rtmp://topaz.chat/live".into(),
            active_profile: "mid".into(),
            profiles: default_profiles(),
            last_stream_key: String::new(),
            last_sources: LastSources::default(),
            encoder_override: "auto".into(),
        }
    }
}

fn default_locale() -> String {
    // F-CF-05: follow OS language, ja/en only.
    let lang = std::env::var("LANG").unwrap_or_default().to_lowercase();
    if lang.starts_with("ja") { "ja".into() } else { "en".into() }
}

/// Built-in profiles (requirements.md §5.3). Profile names are i18n keys, not literals.
pub fn default_profiles() -> std::collections::BTreeMap<String, Profile> {
    let list = [
        ("low", "profile.low", 854, 480, 30, 800, 128, None),
        ("mid", "profile.mid", 1280, 720, 30, 1500, 192, None),
        ("high", "profile.high", 1280, 720, 60, 2000, 320, None),
        (
            "1080p",
            "profile.1080p",
            1920,
            1080,
            30,
            2000,
            320,
            Some("warn.topaz1080p"),
        ),
    ];
    list.into_iter()
        .map(|(id, name, w, h, fps, v, a, warn)| {
            (
                id.to_string(),
                Profile {
                    name: name.into(),
                    w,
                    h,
                    fps,
                    v_kbps: v,
                    a_kbps: a,
                    encoder: "auto".into(),
                    warn: warn.map(Into::into),
                },
            )
        })
        .collect()
}

/// F-EN-04: guard against Topaz hard limits (2000k / 320k). Reject, do not clamp,
/// so the UI can show the offending numbers in red.
pub fn validate_bitrate(v_kbps: u32, a_kbps: u32) -> Result<()> {
    if v_kbps > MAX_VIDEO_KBPS || a_kbps > MAX_AUDIO_KBPS {
        return Err(Error::BitrateOver { v_kbps, a_kbps });
    }
    Ok(())
}

/// F-ST-01: 3-64 chars, alphanumeric / hyphen / underscore.
pub fn validate_stream_key(key: &str) -> Result<()> {
    let ok = (3..=64).contains(&key.len())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::StreamKey(key.to_string()))
    }
}

pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var("APPDATA")
            .map(|d| PathBuf::from(d).join(CONFIG_DIR_NAME))
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
        base.join(CONFIG_DIR_NAME)
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE_NAME)
}

/// Load config; missing or corrupt file yields defaults (design.md §7.1).
pub fn load(path: &Path) -> Result<ProfilesConfig> {
    if !path.exists() {
        return Ok(ProfilesConfig::default());
    }
    let raw = std::fs::read_to_string(path)?;
    match serde_json::from_str(&raw) {
        Ok(cfg) => Ok(cfg),
        // ponytail: corrupt config regenerates defaults; backup-and-migrate if users ever complain
        Err(_) => Ok(ProfilesConfig::default()),
    }
}

pub fn save(path: &Path, cfg: &ProfilesConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(cfg)?)?;
    std::fs::rename(&tmp, path)?; // atomic replace (unix + windows)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_limits() {
        for (id, p) in default_profiles() {
            assert!(validate_bitrate(p.v_kbps, p.a_kbps).is_ok(), "{id}");
            assert_eq!(p.gop(), p.fps * 2, "{id} GOP=2s");
        }
    }

    #[test]
    fn bitrate_guard_rejects_over_limit() {
        assert!(matches!(
            validate_bitrate(2001, 192),
            Err(Error::BitrateOver { v_kbps: 2001, a_kbps: 192 })
        ));
        assert!(matches!(
            validate_bitrate(1500, 321),
            Err(Error::BitrateOver { v_kbps: 1500, a_kbps: 321 })
        ));
    }

    #[test]
    fn stream_key_validation() {
        assert!(validate_stream_key("my-event-123").is_ok());
        assert!(validate_stream_key("ab").is_err()); // < 3
        assert!(validate_stream_key(&"a".repeat(65)).is_err()); // > 64
        assert!(validate_stream_key("bad key!").is_err());
        assert!(validate_stream_key("ok_key-1").is_ok());
    }

    #[test]
    fn roundtrip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("eztopaz-test-{}", std::process::id()));
        let path = dir.join(CONFIG_FILE_NAME);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(load(&path).unwrap(), ProfilesConfig::default());

        let mut cfg = ProfilesConfig::default();
        cfg.last_stream_key = "test-key-123".into();
        cfg.ingest_url = "rtmp://custom.example/live".into();
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path).unwrap(), cfg);

        // atomic save leaves no tmp file
        assert!(!dir.join("profiles.json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("eztopaz-test-corrupt-{}", std::process::id()));
        let path = dir.join(CONFIG_FILE_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path).unwrap(), ProfilesConfig::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_matches_design_schema() {
        let cfg = ProfilesConfig::default();
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["version"], 2);
        assert!(json.get("ingestUrl").is_some());
        assert!(json.get("activeProfile").is_some());
        assert!(json.get("lastStreamKey").is_some());
        assert!(json.get("encoderOverride").is_some());
        let mid = &json["profiles"]["mid"];
        assert_eq!(mid["v_kbps"], 1500);
        assert_eq!(mid["w"], 1280);
    }
}
