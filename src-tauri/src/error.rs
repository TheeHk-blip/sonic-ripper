use serde::Serialize;
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    YoutubeBotDetected,
    Forbidden,
    TrackNotFound { title: String, artist: String },
    SpotifyParseFailed(String),
    Network(String),
    YtDlpFailed(String),
    FfmpegFailed(String),
    Io(String),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::YoutubeBotDetected => write!(
                f,
                "YouTube bot detection blocked this download. Provide valid cookies and try again."
            ),
            AppError::TrackNotFound { title, artist } => {
                write!(
                    f,
                    "Track not found on YouTube: \"{title}\" by \"{artist}\"."
                )
            }
            AppError::SpotifyParseFailed(msg) => write!(f, "Failed to parse Spotify URL: {msg}"),
            AppError::Network(msg) => write!(f, "Network error: {msg}"),
            AppError::Forbidden => write!(
                f,
                "Authentication error: Provide cookies to authenticate your session"
            ),
            AppError::YtDlpFailed(msg) => write!(f, "yt-dlp failed: {msg}"),
            AppError::FfmpegFailed(msg) => write!(f, "ffmpeg failed: {msg}"),
            AppError::Io(msg) => write!(f, "I/O error: {msg}"),
            AppError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Network(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Other(format!("JSON parse error: {e}"))
    }
}

#[derive(Serialize)]
pub struct ErrorPayload {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl AppError {
    pub fn code(&self) -> Option<&'static str> {
        match self {
            AppError::YoutubeBotDetected => Some("YOUTUBE_BOT_DETECTED"),
            AppError::TrackNotFound { .. } => Some("TRACK_NOT_FOUND"),
            AppError::Forbidden => Some("FORBIDDEN"),
            _ => None,
        }
    }

    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload {
            error: self.to_string(),
            code: self.code().map(|c| c.to_string()),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_payload().serialize(serializer)
    }
}

pub type AppResult<T> = Result<T, AppError>;
