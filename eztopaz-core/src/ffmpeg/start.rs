//! Pure start-stream preparation (validation → pipe paths → ffmpeg argv).
//! Platform capture backends feed the pipes; see src-tauri/src/capture.

use crate::config::{validate_stream_key, ProfilesConfig};
use crate::error::{Error, Result};
use crate::ffmpeg::args::{build_ffmpeg_args_with_transport, transport_for_plan, Transport};
use crate::ffmpeg::pipes;
use crate::ipc_types::StreamConfig;

#[derive(Debug, Clone)]
pub struct StartPlan {
    pub ffmpeg_args: Vec<String>,
    pub video_pipe: String,
    pub audio_pipe: String,
    pub encoder: String,
    pub transport: Transport,
}

/// Validate, build the plan, and create the named pipes FFmpeg reads from.
pub fn prepare(cfg: &StreamConfig, profiles: &ProfilesConfig, usable_encoders: &[String]) -> Result<StartPlan> {
    let plan = build_plan(cfg, profiles, usable_encoders)?;
    pipes::create(&plan.video_pipe)?;
    pipes::create(&plan.audio_pipe)?;
    Ok(plan)
}

/// Pure plan construction (validation → encoder pick → argv). No filesystem side
/// effects, so `cargo test` passes on every OS (design.md §13.1); pipe creation
/// stays in [`prepare`] because Windows pipes are unimplemented yet (design §4.1).
pub fn build_plan(cfg: &StreamConfig, profiles: &ProfilesConfig, usable_encoders: &[String]) -> Result<StartPlan> {
    validate_stream_key(&cfg.stream_key)?;
    let profile = profiles
        .profiles
        .get(&cfg.profile_id)
        .ok_or_else(|| Error::Config(format!("unknown profile: {}", cfg.profile_id)))?;

    let encoder = if cfg.encoder_override == "auto" || cfg.encoder_override.is_empty() {
        crate::ffmpeg::probe::probe_best(usable_encoders).to_string()
    } else {
        if !crate::ffmpeg::probe::MANUAL_ENCODERS.contains(&cfg.encoder_override.as_str()) {
            return Err(Error::EncoderNotAvailable(cfg.encoder_override.clone()));
        }
        if cfg.encoder_override == "auto" {
            crate::ffmpeg::probe::probe_best(usable_encoders).to_string()
        } else {
            cfg.encoder_override.clone()
        }
    };

    let video_pipe = pipes::video_pipe_path();
    let audio_pipe = pipes::audio_pipe_path();

    let transport = transport_for_plan(&encoder, cfg.hw_direct);
    let ffmpeg_args = build_ffmpeg_args_with_transport(
        profile,
        &encoder,
        &cfg.ingest_url,
        &cfg.stream_key,
        &video_pipe,
        &audio_pipe,
        &transport,
    )?;

    Ok(StartPlan { ffmpeg_args, video_pipe, audio_pipe, encoder, transport })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(key: &str, profile_id: &str, encoder: &str) -> StreamConfig {
        StreamConfig {
            stream_key: key.into(),
            profile_id: profile_id.into(),
            encoder_override: encoder.into(),
            ..Default::default()
        }
    }

    #[test]
    fn plan_builds_for_libx264() {
        let profiles = ProfilesConfig::default();
        let plan = build_plan(&cfg("test-key-123", "mid", "auto"), &profiles, &[]).unwrap();
        assert_eq!(plan.encoder, "libx264"); // no usable HW in empty list
        assert!(plan.ffmpeg_args.last().unwrap().ends_with("rtmp://topaz.chat/live/test-key-123"));
    }

    #[test]
    fn plan_picks_nvenc_when_usable() {
        let profiles = ProfilesConfig::default();
        let usable = vec!["h264_nvenc".to_string()];
        let plan = build_plan(&cfg("k3y", "high", "auto"), &profiles, &usable).unwrap();
        assert_eq!(plan.encoder, "h264_nvenc");
    }

    #[test]
    fn hw_direct_opt_in_selects_hwdirect_transport() {
        let profiles = ProfilesConfig::default();
        let mut c = cfg("k3y", "mid", "h264_nvenc");
        c.hw_direct = true;
        let plan = build_plan(&c, &profiles, &["h264_nvenc".to_string()]).unwrap();
        assert_eq!(plan.transport, crate::ffmpeg::args::Transport::HwDirect);
        assert!(plan.ffmpeg_args.contains(&"-init_hw_device".to_string()));
        // default stays on implicit-upload pipe
        let plain = build_plan(&cfg("k3y", "mid", "h264_nvenc"), &profiles, &[]).unwrap();
        assert_eq!(plain.transport, crate::ffmpeg::args::Transport::PipeNv12);
    }

    #[test]
    fn manual_override_is_respected() {
        let profiles = ProfilesConfig::default();
        let plan = build_plan(&cfg("k3y", "mid", "libx264"), &profiles, &[]).unwrap();
        assert_eq!(plan.encoder, "libx264");
    }

    #[cfg(unix)]
    #[test]
    fn prepare_creates_pipes_and_builds() {
        let profiles = ProfilesConfig::default();
        let plan = prepare(&cfg("k3y", "mid", "auto"), &profiles, &[]).unwrap();
        assert_eq!(plan.encoder, "libx264");
        assert!(std::path::Path::new(&plan.video_pipe).exists());
        assert!(std::path::Path::new(&plan.audio_pipe).exists());
    }

    #[cfg(windows)]
    #[test]
    fn prepare_creates_pipe_servers_on_windows() {
        // the named-pipe server (design §4.1) makes prepare() succeed on Windows
        let profiles = ProfilesConfig::default();
        let plan = prepare(&cfg("k3y", "mid", "auto"), &profiles, &[]).unwrap();
        assert!(plan.video_pipe.starts_with(r"\\.\pipe\"));
        assert!(plan.audio_pipe.starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn invalid_key_rejected() {
        let profiles = ProfilesConfig::default();
        assert!(prepare(&cfg("x", "mid", "auto"), &profiles, &[]).is_err());
        assert!(prepare(&cfg("bad key!", "mid", "auto"), &profiles, &[]).is_err());
    }

    #[test]
    fn unknown_profile_rejected() {
        let profiles = ProfilesConfig::default();
        assert!(prepare(&cfg("k3y", "nope", "auto"), &profiles, &[]).is_err());
    }

    #[test]
    fn unknown_encoder_rejected() {
        let profiles = ProfilesConfig::default();
        assert!(prepare(&cfg("k3y", "mid", "h264_magic"), &profiles, &[]).is_err());
    }
}
