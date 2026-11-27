use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub year: String,
    pub track_number: u32,
    pub total_tracks: u32,
    pub duration: u32,
    pub cover_url: String,
    pub preview_url: Option<String>,
    #[serde(default)]
    pub not_found_on_youtube: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AnalyzeResult {
    #[serde(rename = "track")]
    Track { track: Track },
    #[serde(rename = "playlist", rename_all = "camelCase")]
    Playlist {
        playlist_name: String,
        is_album: bool,
        tracks: Vec<Track>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ScrapedTrackItem {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<u32>,
    pub cover_url: Option<String>,
    #[allow(dead_code)]
    pub preview_url: Option<String>,
    pub release_year: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ScrapedResult {
    Track(ScrapedTrackItem),
    Playlist {
        playlist_name: String,
        is_album: bool,
        tracks: Vec<ScrapedTrackItem>,
    },
}
