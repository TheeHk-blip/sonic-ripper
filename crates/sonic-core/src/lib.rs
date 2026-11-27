//! Shared engine for Sonic Ripper.
//!
//! Everything that does real work — URL analysis, YouTube matching, the
//! yt-dlp/ffmpeg pipelines, settings and naming — lives here so the Tauri
//! desktop app and the CLI can use exactly the same code. Platform-specific
//! concerns (sidecar processes, event delivery, data dir) are supplied by a
//! [`Host`] implementation.

pub mod analyze;
pub mod cancel;
pub mod download;
pub mod error;
pub mod host;
pub mod http;
pub mod logger;
pub mod models;
pub mod settings;
pub mod spotify;
pub mod youtube;

pub use analyze::{
    analyze, analyze_with_hints, analyze_with_hints_and_exclusions, AnalyzeProgress,
};
pub use download::{download_batch, download_track, DownloadBatchArgs, DownloadTrackArgs};
pub use error::{AppError, AppResult};
pub use host::{Host, HostRef};
pub use models::{AnalyzeResult, Track};
