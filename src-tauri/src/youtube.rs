use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use url::Url;

use crate::models::{ScrapedTrackItem, Track};

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const DIRECT_SEARCH_LIMIT: u32 = 10;

#[derive(Debug, Clone)]
pub struct YoutubeMatch {
    #[allow(dead_code)]
    pub video_id: String,
    pub url: String,
    pub duration: Option<u32>,
    pub thumbnail: Option<String>,
    pub title: Option<String>,
    pub uploader: Option<String>,
}

struct CacheEntry {
    value: Option<YoutubeMatch>,
    inserted_at: Instant,
}

static CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Deserialize)]
struct YtDlpFlatEntry {
    id: Option<String>,
    webpage_url: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    thumbnails: Option<Vec<YtThumb>>,
    title: Option<String>,
    uploader: Option<String>,
}

#[derive(Deserialize)]
struct YtThumb {
    url: String,
}

fn best_thumbnail(entry: &YtDlpFlatEntry) -> Option<String> {
    if let Some(t) = &entry.thumbnail {
        return Some(t.clone());
    }
    entry.thumbnails.as_ref()?.last().map(|t| t.url.clone())
}

fn entry_to_match(entry: YtDlpFlatEntry) -> Option<YoutubeMatch> {
    let video_id = entry.id.clone()?;
    let url = entry
        .webpage_url
        .clone()
        .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={video_id}"));
    Some(YoutubeMatch {
        video_id,
        url,
        duration: entry.duration.map(|d| d.round() as u32),
        thumbnail: best_thumbnail(&entry),
        title: entry.title.clone(),
        uploader: entry.uploader.clone(),
    })
}

pub fn looks_like_youtube_link(input: &str) -> bool {
    extract_youtube_id(input).is_some()
}

fn extract_youtube_id(input: &str) -> Option<String> {
    let parsed = Url::parse(input.trim()).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");

    match host {
        "youtu.be" => parsed
            .path_segments()?
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        "youtube.com" | "music.youtube.com" => {
            if parsed.path() == "/watch" {
                parsed
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned())
            } else if let Some(rest) = parsed.path().strip_prefix("/shorts/") {
                Some(rest.trim_end_matches('/').to_string())
            } else if let Some(rest) = parsed.path().strip_prefix("/embed/") {
                Some(rest.trim_end_matches('/').to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

async fn run_yt_dlp_search(app: &AppHandle, query: &str, limit: u32) -> Vec<YtDlpFlatEntry> {
    let search_spec = format!("ytsearch{limit}:{query}");

    let sidecar = match app.shell().sidecar("sonic-yt-dlp") {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[YouTube] failed to resolve yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    let output = sidecar
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            &search_spec,
        ])
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[YouTube] failed to spawn yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "[YouTube] yt-dlp search exited non-zero for \"{query}\": {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<YtDlpFlatEntry>(line).ok())
        .collect()
}

async fn run_yt_dlp_direct(app: &AppHandle, video_url: &str) -> Vec<YtDlpFlatEntry> {
    let sidecar = match app.shell().sidecar("sonic-yt-dlp") {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[YouTube] failed to resolve yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    let output = sidecar
        .args(["--flat-playlist", "--dump-json", "--no-warnings", video_url])
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[YouTube] failed to spawn yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "[YouTube] yt-dlp direct resolve exited non-zero for \"{video_url}\": {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<YtDlpFlatEntry>(line).ok())
        .collect()
}

/// Fetch up to `limit` YouTube matches for a free-text query.
async fn search_youtube_many(app: &AppHandle, query: &str, limit: u32) -> Vec<YoutubeMatch> {
    run_yt_dlp_search(app, query, limit)
        .await
        .into_iter()
        .filter_map(entry_to_match)
        .collect()
}

// Single best match (cached). Used by Spotify track enrichment.
async fn search_youtube_raw(app: &AppHandle, query: &str) -> Option<YoutubeMatch> {
    let cache_key = query.trim().to_lowercase();

    {
        let cache = CACHE.lock().unwrap();
        if let Some(entry) = cache.get(&cache_key) {
            if entry.inserted_at.elapsed() < CACHE_TTL {
                return entry.value.clone();
            }
        }
    }

    let matched = search_youtube_many(app, query, 1).await.into_iter().next();

    CACHE.lock().unwrap().insert(
        cache_key,
        CacheEntry {
            value: matched.clone(),
            inserted_at: Instant::now(),
        },
    );

    matched
}

pub async fn search_youtube(app: &AppHandle, title: &str, artist: &str) -> Option<YoutubeMatch> {
    search_youtube_raw(app, &format!("{title} {artist}")).await
}

const FALLBACK_COVER: &str =
    "https://images.unsplash.com/photo-1614680376593-902f74fa0d41?w=600&auto=format&fit=crop&q=80";

fn entries_to_tracks(entries: Vec<YtDlpFlatEntry>, fallback_title: &str) -> Vec<Track> {
    let matches: Vec<YoutubeMatch> = entries.into_iter().filter_map(entry_to_match).collect();
    let total = matches.len() as u32;

    matches
        .into_iter()
        .enumerate()
        .map(|(i, m)| Track {
            id: m.video_id.clone(),
            title: m.title.unwrap_or_else(|| fallback_title.to_string()),
            artist: m.uploader.unwrap_or_else(|| "Unknown Artist".to_string()),
            album: "YouTube".to_string(),
            year: String::new(),
            track_number: (i as u32) + 1,
            total_tracks: total,
            duration: m.duration.unwrap_or(180),
            cover_url: m.thumbnail.unwrap_or_else(|| FALLBACK_COVER.to_string()),
            preview_url: Some(m.url),
            not_found_on_youtube: false,
        })
        .collect()
}

/// Multi-result direct search — returns up to `DIRECT_SEARCH_LIMIT` tracks.
pub async fn direct_search_tracks(app: &AppHandle, query: &str) -> Vec<Track> {
    let entries = run_yt_dlp_search(app, query, DIRECT_SEARCH_LIMIT).await;
    entries_to_tracks(entries, query)
}

pub async fn resolve_youtube_url(app: &AppHandle, url: &str) -> Vec<Track> {
    let Some(video_id) = extract_youtube_id(url) else {
        return Vec::new();
    };
    let canonical = format!("https://www.youtube.com/watch?v={video_id}");
    let entries = run_yt_dlp_direct(app, &canonical).await;
    entries_to_tracks(entries, url)
}

pub async fn enrich_track(
    app: &AppHandle,
    item: &ScrapedTrackItem,
    id: String,
    track_number: u32,
    total_tracks: u32,
) -> Track {
    let album = item
        .album
        .clone()
        .unwrap_or_else(|| "Spotify Track".to_string());
    let year = item.release_year.clone().unwrap_or_default();

    match search_youtube(app, &item.title, &item.artist).await {
        Some(m) => Track {
            id,
            title: item.title.clone(),
            artist: item.artist.clone(),
            album,
            year,
            track_number,
            total_tracks,
            duration: m.duration.or(item.duration).unwrap_or(180),
            cover_url: item
                .cover_url
                .clone()
                .or(m.thumbnail)
                .unwrap_or_else(|| FALLBACK_COVER.to_string()),
            preview_url: Some(m.url),
            not_found_on_youtube: false,
        },
        None => Track {
            id,
            title: item.title.clone(),
            artist: item.artist.clone(),
            album,
            year,
            track_number,
            total_tracks,
            duration: item.duration.unwrap_or(180),
            cover_url: item
                .cover_url
                .clone()
                .unwrap_or_else(|| FALLBACK_COVER.to_string()),
            preview_url: None,
            not_found_on_youtube: true,
        },
    }
}
