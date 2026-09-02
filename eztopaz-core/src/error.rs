use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config error: {0}")]
    Config(String),

    #[error("bitrate over Topaz limit (video {v_kbps}k > 2000k or audio {a_kbps}k > 320k)")]
    BitrateOver { v_kbps: u32, a_kbps: u32 },

    #[error("encoder not available: {0}")]
    EncoderNotAvailable(String),

    #[error("stream key invalid: {0}")]
    StreamKey(String),

    #[error("ffmpeg error: {0}")]
    Ffmpeg(String),

    #[error("capture error: {0}")]
    Capture(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
