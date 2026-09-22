use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::error::AppResult;
use crate::http;
use crate::models::{ScrapedResult, ScrapedTrackItem};

static URI_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)spotify:(track|playlist|album|artist):([a-zA-Z0-9]+)").unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)open\.spotify\.com/(?:intl-[a-z]{2,3}(?:-[a-z]{2,3})?/)?(track|playlist|album|artist)/([a-zA-Z0-9]+)").unwrap()
});
static NEXT_DATA_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?s)<script id="__NEXT_DATA__"[^>]*>(.*?)</script>"#).unwrap());
static YEAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(19|20)\d{2}\b").unwrap());
static OG_DESC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+property="og:description"\s+content="([^"]+)""#).unwrap()
});
static OG_IMAGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)<meta\s+property="og:image"\s+content="([^"]+)""#).unwrap());
static MUSIC_DATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:name|property)=["'](?:music|og):release_date["']\s+content=["']([^"']+)["']"#).unwrap()
});
static OG_TITLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+property=["']og:title["']\s+content=["']([^"']+)["']"#).unwrap()
});

pub fn looks_like_spotify_link(input: &str) -> bool {
    URI_RE.is_match(input)
        || URL_RE.is_match(input)
        || input.contains("spotify.link")
        || input.contains("spoti.fi")
}

fn decode_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn extract_release_year(entity: &Value) -> Option<String> {
    let candidates: Vec<Option<&Value>> = vec![
        entity.pointer("/releaseDate/isoString"),
        entity.pointer("/releaseDate/year"),
        entity.get("releaseDate"),
        entity.pointer("/album/releaseDate/isoString"),
        entity.pointer("/album/releaseDate/year"),
        entity.pointer("/album/releaseDate"),
        entity.pointer("/album/date"),
        entity.pointer("/albumOfTrack/date/isoString"),
        entity.pointer("/albumOfTrack/date"),
        entity.get("date"),
        entity.get("year"),
    ];

    for c in candidates.into_iter().flatten() {
        if c.is_null() {
            continue;
        }
        if let Some(n) = c.as_object() {
            if let Some(y) = n.get("year") {
                let y_str = if y.is_string() {
                    y.as_str().unwrap().to_string()
                } else if y.is_number() {
                    y.to_string()
                } else {
                    String::new()
                };
                if y_str.len() == 4 && y_str.chars().all(|c| c.is_ascii_digit()) {
                    return Some(y_str);
                }
            }
            if let Some(iso) = n.get("isoString").and_then(|v| v.as_str()) {
                if let Some(m) = YEAR_RE.find(iso) {
                    return Some(m.as_str().to_string());
                }
            }
            continue;
        }
        let s = if c.is_string() {
            c.as_str().unwrap().to_string()
        } else {
            c.to_string()
        };
        if let Some(m) = YEAR_RE.find(&s) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

fn best_cover_url(entity: &Value) -> Option<String> {
    let mut candidates: Vec<(u64, String)> = Vec::new();

    let mut push_source = |src: &Value| {
        let url = match src.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.to_string(),
            _ => return,
        };
        let dim = src
            .get("width")
            .or_else(|| src.get("maxWidth"))
            .or_else(|| src.get("height"))
            .or_else(|| src.get("maxHeight"))
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        candidates.push((dim, url));
    };

    if let Some(sources) = entity
        .pointer("/coverArt/sources")
        .and_then(|v| v.as_array())
    {
        for s in sources {
            push_source(s);
        }
    }
    if let Some(images) = entity
        .pointer("/visualIdentity/image")
        .and_then(|v| v.as_array())
    {
        for s in images {
            push_source(s);
        }
    }
    // Nested album cover
    if let Some(sources) = entity
        .pointer("/album/coverArt/sources")
        .and_then(|v| v.as_array())
    {
        for s in sources {
            push_source(s);
        }
    }
    // Pathfinder ("albumOfTrack") schema, e.g. fetchPlaylist's itemV2.data.albumOfTrack
    if let Some(sources) = entity
        .pointer("/albumOfTrack/coverArt/sources")
        .and_then(|v| v.as_array())
    {
        for s in sources {
            push_source(s);
        }
    }

    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|b| std::cmp::Reverse(b.0));
    Some(candidates.into_iter().next().unwrap().1)
}

struct TrackPageFallback {
    cover_url: Option<String>,
}

async fn fetch_track_page_fallback(track_url: &str) -> TrackPageFallback {
    let empty = TrackPageFallback { cover_url: None };
    let c = http::client();
    let res = match c
        .get(track_url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return empty,
    };
    let html = match res.text().await {
        Ok(h) => h,
        Err(_) => return empty,
    };
    TrackPageFallback {
        cover_url: OG_IMAGE_RE.captures(&html).map(|c| decode_html(&c[1])),
    }
}

async fn fetch_album_page_fallback(album_url: &str) -> (Option<String>, Option<String>) {
    let c = http::client();
    let res = match c.get(album_url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return (None, None),
    };
    let html = match res.text().await {
        Ok(h) => h,
        Err(_) => return (None, None),
    };

    let mut year = MUSIC_DATE_RE
        .captures(&html)
        .and_then(|c| YEAR_RE.find(&c[1]).map(|m| m.as_str().to_string()));

    if year.is_none() {
        let title = OG_TITLE_RE
            .captures(&html)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let desc = OG_DESC_RE
            .captures(&html)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let blob = format!("{title} {desc}");
        year = YEAR_RE.find(&blob).map(|m| m.as_str().to_string());
    }

    let cover_url = OG_IMAGE_RE.captures(&html).map(|c| decode_html(&c[1]));
    (year, cover_url)
}

async fn fetch_page_og_image(url: &str) -> Option<String> {
    let c = http::client();
    let res = c.get(url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let html = res.text().await.ok()?;
    OG_IMAGE_RE.captures(&html).map(|c| decode_html(&c[1]))
}

const PLAYLIST_CONCURRENCY: usize = 4;

// Artist name(s) from either the embed schema (`artists: [{name}]`) or the
// Pathfinder schema (`artists: {items: [{profile: {name}}]}`).
fn track_artists(data: &Value) -> Option<String> {
    if let Some(items) = data.pointer("/artists/items").and_then(|v| v.as_array()) {
        let names: Vec<&str> = items
            .iter()
            .filter_map(|a| a.pointer("/profile/name").and_then(|v| v.as_str()))
            .collect();
        if !names.is_empty() {
            return Some(decode_html(&names.join(", ")));
        }
    }
    if let Some(arr) = data.get("artists").and_then(|v| v.as_array()) {
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
            .collect();
        if !names.is_empty() {
            return Some(decode_html(&names.join(", ")));
        }
    }
    None
}

fn track_id_from_entry(t: &Value) -> Option<String> {
    if let Some(uri) = t.get("uri").and_then(|v| v.as_str()) {
        let id = uri.rsplit(':').next().unwrap_or("");
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(id.to_string());
        }
    }
    t.get("id").and_then(|v| v.as_str()).map(String::from)
}

fn album_id_from_track_entity(entity: &Value) -> Option<String> {
    if let Some(uri) = entity.pointer("/album/uri").and_then(|v| v.as_str()) {
        let id = uri.rsplit(':').next().unwrap_or("");
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(id.to_string());
        }
    }
    entity
        .pointer("/album/id")
        .and_then(|v| v.as_str())
        .map(String::from)
}

// Picks the track matching `wanted_title` out of a Pathfinder-resolved
// album's track list. Exact (case/whitespace-insensitive) title match wins;
// if nothing matches but the album has exactly one track — the genuine
// "single" case — that lone track is trusted rather than left unmatched,
// since minor formatting drift (e.g. featured-artist punctuation) between
// the embed page's title and Pathfinder's is more likely than a wrong album.
// Anything more ambiguous than that returns None so the caller can fall
// back to the embed path instead of guessing.
fn pick_matching_track(
    tracks: Vec<ScrapedTrackItem>,
    wanted_title: &str,
) -> Option<ScrapedTrackItem> {
    let wanted = wanted_title.trim().to_lowercase();
    if let Some(pos) = tracks
        .iter()
        .position(|t| t.title.trim().to_lowercase() == wanted)
    {
        let mut tracks = tracks;
        return Some(tracks.remove(pos));
    }
    if tracks.len() == 1 {
        let mut tracks = tracks;
        return Some(tracks.remove(0));
    }
    None
}

async fn resolve_track_as_single(track_id: &str) -> Option<ScrapedTrackItem> {
    let mut title = String::new();
    let mut artist = String::new();
    let mut album: Option<String> = None;
    let mut cover_url: Option<String> = None;
    let mut release_year: Option<String> = None;
    let mut duration: Option<u32> = None;
    let mut preview_url: Option<String> = None;

    let embed_url = format!("https://open.spotify.com/embed/track/{track_id}");
    {
        let c = http::client();
        match c
            .get(&embed_url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                if let Ok(html) = res.text().await {
                    if let Some(caps) = NEXT_DATA_RE.captures(&html) {
                        if let Ok(next_data) = serde_json::from_str::<Value>(&caps[1]) {
                            if let Some(entity) =
                                next_data.pointer("/props/pageProps/state/data/entity")
                            {
                                title = decode_html(
                                    entity
                                        .get("name")
                                        .or_else(|| entity.get("title"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(""),
                                );
                                artist = if let Some(artists) =
                                    entity.get("artists").and_then(|v| v.as_array())
                                {
                                    decode_html(
                                        &artists
                                            .iter()
                                            .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                                            .collect::<Vec<_>>()
                                            .join(", "),
                                    )
                                } else {
                                    entity
                                        .get("artist")
                                        .or_else(|| entity.get("subtitle"))
                                        .and_then(|v| v.as_str())
                                        .map(decode_html)
                                        .unwrap_or_default()
                                };
                                // Spotify's embed schema doesn't carry album data at
                                // all for most tracks (confirmed by direct testing) —
                                // this extraction is kept in case that ever changes,
                                // but there's no further fallback for it: it's left
                                // blank rather than guessed.
                                album = entity
                                    .pointer("/album/name")
                                    .and_then(|v| v.as_str())
                                    .map(decode_html)
                                    .filter(|s| !s.is_empty());
                                cover_url = best_cover_url(entity);
                                release_year = extract_release_year(entity);
                                duration = entity
                                    .get("duration")
                                    .and_then(|v| v.as_u64())
                                    .map(|ms| (ms as f64 / 1000.0).round() as u32);
                                preview_url = entity
                                    .pointer("/audioPreview/url")
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                            }
                        }
                    }
                }
            }
            Ok(res) => {
                eprintln!(
                    "[Spotify Scraper] single track embed fetch ({track_id}) HTTP {}",
                    res.status()
                );
            }
            Err(e) => {
                eprintln!(
                    "[Spotify Scraper] single track embed fetch ({track_id}) request error: {e}"
                );
            }
        }
    }

    if cover_url.is_none() {
        let open_url = format!("https://open.spotify.com/track/{track_id}");
        cover_url = fetch_track_page_fallback(&open_url).await.cover_url;
    }

    if title.is_empty() {
        return None;
    }

    Some(ScrapedTrackItem {
        title,
        artist: if artist.is_empty() {
            "Unknown Artist".to_string()
        } else {
            artist
        },
        album,
        album_artist: None,
        duration,
        cover_url,
        preview_url,
        release_year,
    })
}

pub async fn resolve_album_year(
    entity: &Value,
    entity_type: &str,
    entity_id: &str,
) -> Option<String> {
    if let Some(y) = extract_release_year(entity) {
        return Some(y);
    }
    if entity_type != "album" || entity_id.is_empty() {
        return None;
    }

    let open_album_url = format!("https://open.spotify.com/album/{entity_id}");
    let (year, _cover) = fetch_album_page_fallback(&open_album_url).await;
    if let Some(y) = year {
        println!("[Spotify Scraper] Album year recovered via page fallback: {y}");
        return Some(y);
    }

    if let Some(track_list) = entity.get("trackList").and_then(|v| v.as_array()) {
        if let Some(first) = track_list.first() {
            let track_uri = first
                .get("uri")
                .and_then(|v| v.as_str())
                .or_else(|| first.get("id").and_then(|v| v.as_str()));
            if let Some(uri) = track_uri {
                let track_id = uri.rsplit(':').next().unwrap_or("");
                if !track_id.is_empty() && track_id.chars().all(|c| c.is_ascii_alphanumeric()) {
                    let embed_url = format!("https://open.spotify.com/embed/track/{track_id}");
                    {
                        let c = http::client();
                        if let Ok(res) = c
                            .get(&embed_url)
                            .header("Accept-Language", "en-US,en;q=0.9")
                            .send()
                            .await
                        {
                            if res.status().is_success() {
                                if let Ok(html) = res.text().await {
                                    if let Some(caps) = NEXT_DATA_RE.captures(&html) {
                                        if let Ok(next_data) =
                                            serde_json::from_str::<Value>(&caps[1])
                                        {
                                            let track_entity = next_data
                                                .pointer("/props/pageProps/state/data/entity");
                                            if let Some(te) = track_entity {
                                                if let Some(y) = extract_release_year(te) {
                                                    println!(
                                                        "[Spotify Scraper] Album year inherited from first track embed: {y}"
                                                    );
                                                    return Some(y);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

// Pull the anonymous access token Spotify embeds in `__NEXT_DATA__`.
// Used to call spclient for full playlist track lists (>~100 tracks).
fn extract_access_token(next_data: &Value) -> Option<String> {
    const PATHS: &[&str] = &[
        "/props/pageProps/state/settings/session/accessToken",
        "/props/pageProps/settings/session/accessToken",
        "/props/pageProps/session/accessToken",
    ];
    for path in PATHS {
        if let Some(token) = next_data.pointer(path).and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

// Full ordered track IDs for a playlist via spclient (no 100-track cap).
async fn fetch_playlist_track_ids_spclient(playlist_id: &str, token: &str) -> Option<Vec<String>> {
    let c = http::client();
    let url = format!("https://spclient.wg.spotify.com/playlist/v2/playlist/{playlist_id}");
    let res = c
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        eprintln!(
            "[Spotify Scraper] spclient playlist fetch HTTP {}",
            res.status()
        );
        return None;
    }
    let data: Value = res.json().await.ok()?;
    let items = data.pointer("/contents/items")?.as_array()?;
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let uri = item.get("uri").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(id) = uri.strip_prefix("spotify:track:") {
            if !id.is_empty() {
                ids.push(id.to_string());
            }
        }
    }
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

const PATHFINDER_PAGE_SIZE: u32 = 100;
const PATHFINDER_MAX_PAGES: u32 = 50; // safety cap (~5k tracks) against a bad totalCount

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct PathfinderConfig {
    client_token: Option<String>,
    pathfinder_hash: Option<String>,
    get_album_hash: Option<String>,
}

fn pathfinder_config_path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "TheeHk-blip", "sonicripper")?;
    let dir = dirs.config_dir();
    std::fs::create_dir_all(dir).ok()?;
    Some(dir.join("spotify_pathfinder.json"))
}

fn load_pathfinder_config() -> PathfinderConfig {
    pathfinder_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pathfinder_config(cfg: &PathfinderConfig) {
    let Some(path) = pathfinder_config_path() else {
        eprintln!("[Spotify Scraper] couldn't resolve config dir — settings won't persist");
        return;
    };
    match serde_json::to_string_pretty(cfg) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("[Spotify Scraper] failed writing {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[Spotify Scraper] failed serializing pathfinder config: {e}"),
    }
}

static PATHFINDER_CONFIG: Lazy<std::sync::Mutex<PathfinderConfig>> =
    Lazy::new(|| std::sync::Mutex::new(load_pathfinder_config()));

pub fn set_pathfinder_hash(hash: String) {
    let trimmed = hash.trim().to_string();
    let mut guard = PATHFINDER_CONFIG.lock().unwrap();
    guard.pathfinder_hash = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    save_pathfinder_config(&guard);
}

pub fn pathfinder_hash() -> Option<String> {
    PATHFINDER_CONFIG.lock().unwrap().pathfinder_hash.clone()
}

pub fn set_get_album_hash(hash: String) {
    let trimmed = hash.trim().to_string();
    let mut guard = PATHFINDER_CONFIG.lock().unwrap();
    guard.get_album_hash = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    save_pathfinder_config(&guard);
}

pub fn get_album_hash() -> Option<String> {
    PATHFINDER_CONFIG.lock().unwrap().get_album_hash.clone()
}

pub fn set_spotify_client_token(token: String) {
    let trimmed = token.trim().to_string();
    let mut guard = PATHFINDER_CONFIG.lock().unwrap();
    guard.client_token = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    save_pathfinder_config(&guard);
}

pub fn spotify_client_token() -> Option<String> {
    PATHFINDER_CONFIG.lock().unwrap().client_token.clone()
}

async fn fetch_pathfinder_playlist_page(
    playlist_uri: &str,
    offset: u32,
    limit: u32,
    access_token: &str,
    client_token: &str,
) -> Option<Value> {
    let c = http::client();
    let body = serde_json::json!({
        "variables": {
            "uri": playlist_uri,
            "offset": offset,
            "limit": limit,
            "enableWatchFeedEntrypoint": true,
            "includeEpisodeContentRatingsV2": true
        },
        "operationName": "fetchPlaylist",
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": pathfinder_hash()
            }
        }
    });

    let res = c
        .post("https://api-partner.spotify.com/pathfinder/v2/query")
        .header("authorization", format!("Bearer {access_token}"))
        .header("client-token", client_token)
        .header("accept", "application/json")
        .header("content-type", "application/json;charset=UTF-8")
        .header("app-platform", "WebPlayer")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        eprintln!(
            "[Spotify Scraper] pathfinder fetchPlaylist HTTP {} (offset {offset})",
            res.status()
        );
        return None;
    }
    match res.json::<Value>().await {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[Spotify Scraper] pathfinder fetchPlaylist: bad JSON: {e}");
            None
        }
    }
}

struct PathfinderPage {
    total_count: u32,
    tracks: Vec<ScrapedTrackItem>,
}

fn parse_pathfinder_playlist_page(page: &Value) -> Option<PathfinderPage> {
    let content = page.pointer("/data/playlistV2/content")?;
    let total_count = content
        .get("totalCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let items = content.get("items")?.as_array()?;

    let mut tracks = Vec::with_capacity(items.len());
    for item in items {
        let Some(data) = item.pointer("/itemV2/data") else {
            continue;
        };

        if data.get("__typename").and_then(|v| v.as_str()) != Some("Track") {
            continue;
        }

        let title = data
            .get("name")
            .and_then(|v| v.as_str())
            .map(decode_html)
            .unwrap_or_else(|| "Unknown Track".to_string());
        let artist = track_artists(data).unwrap_or_else(|| "Unknown Artist".to_string());
        let album = data
            .pointer("/albumOfTrack/name")
            .and_then(|v| v.as_str())
            .map(decode_html);
        let duration = data
            .pointer("/trackDuration/totalMilliseconds")
            .and_then(|v| v.as_u64())
            .map(|ms| (ms as f64 / 1000.0).round() as u32);
        let cover_url = best_cover_url(data);
        let release_year = extract_release_year(data);

        tracks.push(ScrapedTrackItem {
            title,
            artist,
            album,
            album_artist: None,
            duration,
            cover_url,
            preview_url: None,
            release_year,
        });
    }

    Some(PathfinderPage {
        total_count,
        tracks,
    })
}

async fn resolve_playlist_via_pathfinder(
    entity_id: &str,
    access_token: &str,
    client_token: &str,
) -> Option<Vec<ScrapedTrackItem>> {
    let playlist_uri = format!("spotify:playlist:{entity_id}");
    let mut tracks = Vec::new();
    let mut offset = 0u32;

    for page_num in 0..PATHFINDER_MAX_PAGES {
        let page = fetch_pathfinder_playlist_page(
            &playlist_uri,
            offset,
            PATHFINDER_PAGE_SIZE,
            access_token,
            client_token,
        )
        .await?;
        let parsed = parse_pathfinder_playlist_page(&page)?;

        let got = parsed.tracks.len() as u32;
        tracks.extend(parsed.tracks);
        offset += PATHFINDER_PAGE_SIZE;

        println!(
            "[Spotify Scraper] pathfinder page {page_num}: +{got} tracks ({}/{})",
            tracks.len(),
            parsed.total_count
        );

        if offset >= parsed.total_count || got == 0 {
            break;
        }
    }

    if tracks.is_empty() {
        None
    } else {
        Some(tracks)
    }
}

async fn fetch_pathfinder_album_page(
    album_uri: &str,
    offset: u32,
    limit: u32,
    access_token: &str,
    client_token: &str,
) -> Option<Value> {
    let c = http::client();
    let body = serde_json::json!({
        "variables": {
            "uri": album_uri,
            "locale": "",
            "offset": offset,
            "limit": limit
        },
        "operationName": "getAlbum",
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": get_album_hash()
            }
        }
    });

    let res = c
        .post("https://api-partner.spotify.com/pathfinder/v2/query")
        .header("authorization", format!("Bearer {access_token}"))
        .header("client-token", client_token)
        .header("accept", "application/json")
        .header("content-type", "application/json;charset=UTF-8")
        .header("app-platform", "WebPlayer")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        eprintln!(
            "[Spotify Scraper] pathfinder getAlbum HTTP {} (offset {offset})",
            res.status()
        );
        return None;
    }
    match res.json::<Value>().await {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[Spotify Scraper] pathfinder getAlbum: bad JSON: {e}");
            None
        }
    }
}

struct PathfinderAlbumPage {
    album_name: Option<String>,
    total_count: u32,
    tracks: Vec<ScrapedTrackItem>,
}

fn parse_pathfinder_album_page(page: &Value) -> Option<PathfinderAlbumPage> {
    // "/data/albumUnion" confirmed via a real captured response;
    // the others are kept as fallbacks in case Spotify varies this by
    // album/region/client version.
    const CANDIDATE_ROOTS: &[&str] = &[
        "/data/albumUnion",
        "/data/albumUnionV2",
        "/data/album",
        "/data/albumV2",
    ];

    let mut container = None;
    for path in CANDIDATE_ROOTS {
        if let Some(c) = page.pointer(path) {
            println!("[Spotify Scraper] pathfinder getAlbum: matched root {path}");
            container = Some(c);
            break;
        }
    }
    let Some(container) = container else {
        if let Some(data) = page.get("data") {
            if let Some(obj) = data.as_object() {
                println!(
                    "[Spotify Scraper] pathfinder getAlbum: none of {:?} matched; \
                     top-level keys under /data were: {:?}",
                    CANDIDATE_ROOTS,
                    obj.keys().collect::<Vec<_>>()
                );
            }
        } else {
            println!(
                "[Spotify Scraper] pathfinder getAlbum: response had no /data at all — \
                 raw response: {page}"
            );
        }
        return None;
    };

    let album_name = container
        .get("name")
        .and_then(|v| v.as_str())
        .map(decode_html);
    let album_cover = best_cover_url(container);
    let album_year = extract_release_year(container);
    let album_artist = track_artists(container);

    // Track list container: try a couple of plausible keys/shapes.
    let tracks_container = container
        .get("tracksV2")
        .or_else(|| container.get("tracks"));
    let Some(tracks_container) = tracks_container else {
        println!(
            "[Spotify Scraper] pathfinder getAlbum: matched album root but no tracks/tracksV2 \
             key — album-level keys were: {:?}",
            container.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
        return None;
    };

    let total_count = tracks_container
        .get("totalCount")
        .or_else(|| tracks_container.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let items = tracks_container
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut tracks = Vec::with_capacity(items.len());
    for item in &items {
        // Some Pathfinder list shapes wrap the real object under a "track"
        // (or "itemV2"/"data") key; fall back to the item itself if not.
        let data = item
            .get("track")
            .or_else(|| item.pointer("/itemV2/data"))
            .unwrap_or(item);

        let title = data
            .get("name")
            .and_then(|v| v.as_str())
            .map(decode_html)
            .unwrap_or_else(|| "Unknown Track".to_string());
        let artist = track_artists(data).unwrap_or_else(|| "Unknown Artist".to_string());

        let duration = data
            .pointer("/duration/totalMilliseconds")
            .or_else(|| data.pointer("/trackDuration/totalMilliseconds"))
            .and_then(|v| v.as_u64())
            .or_else(|| data.get("durationMs").and_then(|v| v.as_u64()))
            .or_else(|| data.get("duration_ms").and_then(|v| v.as_u64()))
            .or_else(|| item.get("duration_ms").and_then(|v| v.as_u64()))
            .map(|ms| (ms as f64 / 1000.0).round() as u32);

        let cover_url = best_cover_url(data).or_else(|| album_cover.clone());

        tracks.push(ScrapedTrackItem {
            title,
            artist,
            album: album_name.clone(),
            album_artist: album_artist.clone(),
            duration,
            cover_url,
            preview_url: None,
            release_year: album_year.clone(),
        });
    }

    Some(PathfinderAlbumPage {
        album_name,
        total_count,
        tracks,
    })
}

async fn resolve_album_via_pathfinder(
    entity_id: &str,
    access_token: &str,
    client_token: &str,
) -> Option<(String, Vec<ScrapedTrackItem>)> {
    let album_uri = format!("spotify:album:{entity_id}");
    let mut tracks = Vec::new();
    let mut offset = 0u32;
    let mut album_name = None;

    for page_num in 0..PATHFINDER_MAX_PAGES {
        let page = fetch_pathfinder_album_page(
            &album_uri,
            offset,
            PATHFINDER_PAGE_SIZE,
            access_token,
            client_token,
        )
        .await?;
        let parsed = parse_pathfinder_album_page(&page)?;

        if album_name.is_none() {
            album_name = parsed.album_name.clone();
        }

        let got = parsed.tracks.len() as u32;
        tracks.extend(parsed.tracks);
        offset += PATHFINDER_PAGE_SIZE;

        println!(
            "[Spotify Scraper] pathfinder getAlbum page {page_num}: +{got} tracks ({}/{})",
            tracks.len(),
            parsed.total_count
        );

        if offset >= parsed.total_count || got == 0 {
            break;
        }
    }

    if tracks.is_empty() {
        None
    } else {
        Some((
            album_name.unwrap_or_else(|| "Unknown Album".to_string()),
            tracks,
        ))
    }
}

pub async fn scrape_spotify(input: &str) -> AppResult<Option<ScrapedResult>> {
    let query = input.trim();

    if looks_like_spotify_link(query) {
        scrape_spotify_url(query).await
    } else {
        Ok(None)
    }
}

async fn scrape_spotify_url(input_url: &str) -> AppResult<Option<ScrapedResult>> {
    let mut url = input_url.trim().to_string();

    if url.contains("spotify.link") || url.contains("spoti.fi") {
        let c = http::client();
        if let Ok(res) = c.head(&url).send().await {
            url = res.url().to_string();
        }
    }

    let (entity_type, entity_id) = if let Some(caps) = URI_RE.captures(&url) {
        (caps[1].to_lowercase(), caps[2].to_string())
    } else if let Some(caps) = URL_RE.captures(&url) {
        (caps[1].to_lowercase(), caps[2].to_string())
    } else {
        (String::new(), String::new())
    };

    println!("[Spotify Scraper] Processing entityType=\"{entity_type}\", entityId=\"{entity_id}\" from \"{url}\"");

    if entity_type.is_empty() || entity_id.is_empty() {
        return Ok(None);
    }

    let embed_url = format!("https://open.spotify.com/embed/{entity_type}/{entity_id}");
    let c = http::client();
    let embed_res = match c
        .get(&embed_url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(None),
    };
    let html = embed_res.text().await?;

    let caps = match NEXT_DATA_RE.captures(&html) {
        Some(c) => c,
        None => return Ok(None),
    };
    let next_data: Value = serde_json::from_str(&caps[1])?;
    let entity = match next_data.pointer("/props/pageProps/state/data/entity") {
        Some(e) => e,
        None => return Ok(None),
    };

    let anon_access_token = extract_access_token(&next_data).map(Arc::new);

    if entity_type == "track" {
        let title = decode_html(
            entity
                .get("name")
                .or_else(|| entity.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );

        // Preferred path: Spotify has no dedicated single-track Pathfinder
        // query — a single is just a one-track album — so resolving via the
        // same getAlbum call used for full albums and picking the matching
        // track out of its list gets real album metadata (name/cover/year)
        // that the embed page's track entity mostly can't provide (see the
        // "Left blank rather than guessed" comment below). Falls through to
        // the existing embed-scraping logic on any failure.
        if !title.is_empty() {
            if let Some(album_id) = album_id_from_track_entity(entity) {
                match (anon_access_token.as_deref(), spotify_client_token()) {
                    (Some(token), Some(client_token)) => {
                        match resolve_album_via_pathfinder(&album_id, token, &client_token).await {
                            Some((_album_name, tracks)) => {
                                match pick_matching_track(tracks, &title) {
                                    Some(track) => {
                                        println!(
                                            "[Spotify Scraper] resolved track via pathfinder \
                                             getAlbum (parent album {album_id})"
                                        );
                                        return Ok(Some(ScrapedResult::Track(track)));
                                    }
                                    None => {
                                        println!(
                                            "[Spotify Scraper] pathfinder getAlbum resolved parent \
                                             album {album_id} but no track matched \"{title}\" \
                                             unambiguously — falling back to embed path"
                                        );
                                    }
                                }
                            }
                            None => {
                                println!(
                                    "[Spotify Scraper] pathfinder getAlbum unavailable/failed for \
                                     parent album {album_id} — falling back to embed path"
                                );
                            }
                        }
                    }
                    (None, _) => {
                        println!(
                            "[Spotify Scraper] pathfinder skipped: no anon access token extracted \
                             from embed page __NEXT_DATA__ — falling back to embed path"
                        );
                    }
                    (_, None) => {
                        println!(
                            "[Spotify Scraper] pathfinder skipped: no client token set — \
                             falling back to embed path"
                        );
                    }
                }
            } else {
                println!(
                    "[Spotify Scraper] pathfinder skipped: no album id found on track entity \
                     (tried /album/uri, /album/id) — falling back to embed path"
                );
            }
        }

        let artist = if let Some(artists) = entity.get("artists").and_then(|v| v.as_array()) {
            decode_html(
                &artists
                    .iter()
                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        } else {
            entity
                .get("artist")
                .or_else(|| entity.get("subtitle"))
                .and_then(|v| v.as_str())
                .map(decode_html)
                .unwrap_or_else(|| "Unknown Artist".to_string())
        };
        let entity_album = decode_html(
            entity
                .pointer("/album/name")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        let entity_cover = best_cover_url(entity);
        let entity_year = extract_release_year(entity);

        // Cover-only fallback: album and year aren't recoverable from the full page
        // (no __NEXT_DATA__, no og:description on it, confirmed by direct testing),
        let cover_url = if entity_cover.is_some() {
            entity_cover
        } else {
            fetch_track_page_fallback(&url).await.cover_url
        };

        // Left blank rather than guessed: Spotify's embed schema doesn't carry album
        // data for most tracks, and defaulting to the track title is only right for singles
        let album = if entity_album.is_empty() {
            None
        } else {
            Some(entity_album)
        };
        let release_year = entity_year;
        let preview_url = entity
            .pointer("/audioPreview/url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let duration = entity
            .get("duration")
            .and_then(|v| v.as_u64())
            .map(|ms| (ms as f64 / 1000.0).round() as u32);

        if title.is_empty() {
            return Ok(None);
        }

        return Ok(Some(ScrapedResult::Track(ScrapedTrackItem {
            title,
            artist,
            album,
            album_artist: None,
            duration,
            cover_url,
            preview_url,
            release_year,
        })));
    }

    // Playlist / album path
    let playlist_name = decode_html(
        entity
            .get("name")
            .or_else(|| entity.get("title"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Spotify Collection"),
    );

    // Preferred path for playlists: one paginated Pathfinder call instead of
    // one embed-page fetch per track. Falls through to the existing
    // embed+spclient path below if the token, client-token, or response shape aren't there
    if entity_type == "playlist" {
        match (anon_access_token.as_deref(), spotify_client_token()) {
            (Some(token), Some(client_token)) => {
                match resolve_playlist_via_pathfinder(&entity_id, token, &client_token).await {
                    Some(tracks) => {
                        println!(
                            "[Spotify Scraper] resolved {} tracks via pathfinder fetchPlaylist",
                            tracks.len()
                        );
                        return Ok(Some(ScrapedResult::Playlist {
                            playlist_name,
                            is_album: false,
                            tracks,
                        }));
                    }
                    None => {
                        println!(
                            "[Spotify Scraper] pathfinder fetchPlaylist unavailable/failed — \
                             falling back to embed+spclient path"
                        );
                    }
                }
            }
            (None, _) => {
                println!(
                    "[Spotify Scraper] pathfinder skipped: no anon access token extracted \
                     from embed page __NEXT_DATA__ — falling back to embed+spclient path"
                );
            }
            (_, None) => {
                println!(
                    "[Spotify Scraper] pathfinder skipped: no client token set (paste one in \
                     from devtools) — falling back to embed+spclient path"
                );
            }
        }
    }

    if entity_type == "album" {
        // Preferred path: one paginated Pathfinder getAlbum call. Tried here,
        // before any of the embed-page-derived fallback data (year, cover)
        // below is computed, so a successful resolution never pays for the
        // resolve_album_year/og:image network calls it doesn't need —
        // parse_pathfinder_album_page already sets release_year/cover_url
        // per track from the Pathfinder response itself.
        match (anon_access_token.as_deref(), spotify_client_token()) {
            (Some(token), Some(client_token)) => {
                match resolve_album_via_pathfinder(&entity_id, token, &client_token).await {
                    Some((album_name, tracks)) => {
                        println!(
                            "[Spotify Scraper] resolved {} tracks via pathfinder getAlbum",
                            tracks.len()
                        );
                        return Ok(Some(ScrapedResult::Playlist {
                            playlist_name: album_name,
                            is_album: true,
                            tracks,
                        }));
                    }
                    None => {
                        println!(
                            "[Spotify Scraper] pathfinder getAlbum unavailable/failed — \
                             falling back to embed+page-fallback path"
                        );
                    }
                }
            }
            (None, _) => {
                println!(
                    "[Spotify Scraper] pathfinder skipped: no anon access token extracted \
                     from embed page __NEXT_DATA__ — falling back to embed+page-fallback path"
                );
            }
            (_, None) => {
                println!(
                    "[Spotify Scraper] pathfinder skipped: no client token set — \
                     falling back to embed+page-fallback path"
                );
            }
        }
    }

    let raw_tracks = entity
        .get("trackList")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let playlist_level_year = resolve_album_year(entity, &entity_type, &entity_id).await;

    // Shared cover — largest from coverArt / visualIdentity, else og:image
    let entity_cover_url = match best_cover_url(entity) {
        Some(u) => Some(u),
        None => fetch_page_og_image(&url).await,
    };

    if raw_tracks.is_empty() {
        return Ok(None);
    }

    if entity_type == "album" {
        // Same album-level artist extraction as the pathfinder path above,
        // `track_artists` already handles this exact embed schema shape
        // (`artists: [{name}]`) since the entity itself is the album here.
        let album_artist = track_artists(entity);
        let mut tracks = Vec::with_capacity(raw_tracks.len());
        for t in &raw_tracks {
            let t_title = decode_html(
                t.get("title")
                    .or_else(|| t.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Track"),
            );
            let t_artist = if let Some(sub) = t.get("subtitle").and_then(|v| v.as_str()) {
                decode_html(sub)
            } else if let Some(artists) = t.get("artists").and_then(|v| v.as_array()) {
                decode_html(
                    &artists
                        .iter()
                        .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            } else {
                "Unknown Artist".to_string()
            };
            let duration = t
                .get("duration")
                .and_then(|v| v.as_u64())
                .map(|ms| (ms as f64 / 1000.0).round() as u32);
            let preview_url = t
                .pointer("/audioPreview/url")
                .and_then(|v| v.as_str())
                .map(String::from);
            let cover_url = best_cover_url(t).or_else(|| entity_cover_url.clone());

            tracks.push(ScrapedTrackItem {
                title: t_title,
                artist: t_artist,
                album: Some(playlist_name.clone()),
                album_artist: album_artist.clone(),
                duration,
                cover_url,
                preview_url,
                release_year: playlist_level_year.clone(),
            });
        }

        return Ok(Some(ScrapedResult::Playlist {
            playlist_name,
            is_album: true,
            tracks,
        }));
    }

    let mut track_ids: Vec<Option<String>> = Vec::with_capacity(raw_tracks.len());
    for t in &raw_tracks {
        let track_id = track_id_from_entry(t);
        if track_id.is_none() {
            let t_title = decode_html(
                t.get("title")
                    .or_else(|| t.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Track"),
            );
            println!(
                "[Spotify Scraper] Playlist entry \"{t_title}\": could not extract a track id \
                 (no usable uri/id) — will fall back to playlist-level data for this entry"
            );
        }
        track_ids.push(track_id);
    }

    if let Some(token) = anon_access_token.as_deref() {
        if let Some(all_ids) = fetch_playlist_track_ids_spclient(&entity_id, token).await {
            if all_ids.len() > track_ids.len() {
                println!(
                    "[Spotify Scraper] Embed had {} tracks; spclient reports {} — resolving all of them",
                    track_ids.len(),
                    all_ids.len()
                );
                track_ids = all_ids.into_iter().map(Some).collect();
            }
        }
    }

    let semaphore = Arc::new(Semaphore::new(PLAYLIST_CONCURRENCY));
    let mut handles = Vec::with_capacity(track_ids.len());
    for (i, track_id) in track_ids.iter().cloned().enumerate() {
        let Some(id) = track_id else { continue };
        let sem = semaphore.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            let resolved = resolve_track_as_single(&id).await;
            (i, resolved)
        }));
    }
    let mut single_resolved: Vec<Option<ScrapedTrackItem>> = vec![None; track_ids.len()];
    for handle in handles {
        if let Ok((i, item)) = handle.await {
            single_resolved[i] = item;
        }
    }

    let mut tracks = Vec::with_capacity(track_ids.len());
    for (i, _track_id) in track_ids.iter().enumerate() {
        let raw = raw_tracks.get(i);
        let single = single_resolved[i].take();

        let title = raw
            .and_then(|t| t.get("title").or_else(|| t.get("name")))
            .and_then(|v| v.as_str())
            .map(decode_html)
            .or_else(|| single.as_ref().map(|s| s.title.clone()))
            .unwrap_or_else(|| "Unknown Track".to_string());
        let artist = raw
            .and_then(|t| t.get("subtitle").and_then(|v| v.as_str()))
            .map(decode_html)
            .or_else(|| single.as_ref().map(|s| s.artist.clone()))
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let duration = raw
            .and_then(|t| t.get("duration"))
            .and_then(|v| v.as_u64())
            .map(|ms| (ms as f64 / 1000.0).round() as u32)
            .or_else(|| single.as_ref().and_then(|s| s.duration));
        let preview_url = raw
            .and_then(|t| t.pointer("/audioPreview/url"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| single.as_ref().and_then(|s| s.preview_url.clone()));

        let album = single.as_ref().and_then(|s| s.album.clone());
        let cover_url = single
            .as_ref()
            .and_then(|s| s.cover_url.clone())
            .or_else(|| entity_cover_url.clone());
        let release_year = single
            .as_ref()
            .and_then(|s| s.release_year.clone())
            .or_else(|| playlist_level_year.clone());

        if single.is_none() {
            println!(
                "[Spotify Scraper] Track {i} ({title}): per-track resolution failed — \
                 using playlist-level fallback for cover/year, album left blank"
            );
        }

        tracks.push(ScrapedTrackItem {
            title,
            artist,
            album,
            album_artist: None,
            duration,
            cover_url,
            preview_url,
            release_year,
        });
    }

    if tracks.is_empty() {
        return Ok(None);
    }

    Ok(Some(ScrapedResult::Playlist {
        playlist_name,
        is_album: false,
        tracks,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_next_data(release_date_iso: Option<&str>) -> Value {
        let release_date = match release_date_iso {
            Some(iso) => serde_json::json!({ "isoString": iso }),
            None => Value::Null,
        };
        serde_json::json!({
            "props": {
                "pageProps": {
                    "state": {
                        "data": {
                            "entity": {
                                "name": "Test Song",
                                "artists": [{ "name": "Test Artist" }],
                                "album": { "name": "Test Album" },
                                "releaseDate": release_date,
                                "coverArt": { "sources": [{ "url": "https://example.com/cover.jpg" }] },
                                "audioPreview": { "url": "https://p.scdn.co/preview.mp3" },
                                "duration": 210000
                            }
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn extract_release_year_finds_iso_year() {
        let data = fixture_next_data(Some("2012-11-19"));
        let entity = data.pointer("/props/pageProps/state/data/entity").unwrap();
        assert_eq!(extract_release_year(entity), Some("2012".to_string()));
    }

    #[test]
    fn extract_release_year_none_when_absent() {
        let data = fixture_next_data(None);
        let entity = data.pointer("/props/pageProps/state/data/entity").unwrap();
        assert_eq!(extract_release_year(entity), None);
    }

    #[test]
    fn decode_html_handles_common_entities() {
        assert_eq!(decode_html("Rock &amp; Roll"), "Rock & Roll");
        assert_eq!(decode_html("&quot;Quoted&quot;"), "\"Quoted\"");
    }

    #[test]
    fn best_cover_url_prefers_largest_visual_identity() {
        let entity = serde_json::json!({
            "visualIdentity": {
                "image": [
                    { "url": "https://example.com/300.jpg", "maxWidth": 300, "maxHeight": 300 },
                    { "url": "https://example.com/64.jpg", "maxWidth": 64, "maxHeight": 64 },
                    { "url": "https://example.com/640.jpg", "maxWidth": 640, "maxHeight": 640 }
                ]
            }
        });
        assert_eq!(
            best_cover_url(&entity),
            Some("https://example.com/640.jpg".to_string())
        );
    }

    #[test]
    fn best_cover_url_prefers_largest_cover_art_sources() {
        let entity = serde_json::json!({
            "coverArt": {
                "sources": [
                    { "url": "https://example.com/small.jpg", "width": 64, "height": 64 },
                    { "url": "https://example.com/large.jpg", "width": 640, "height": 640 }
                ]
            }
        });
        assert_eq!(
            best_cover_url(&entity),
            Some("https://example.com/large.jpg".to_string())
        );
    }

    #[test]
    fn best_cover_url_falls_back_when_no_dimensions() {
        let entity = serde_json::json!({
            "coverArt": {
                "sources": [
                    { "url": "https://example.com/only.jpg" }
                ]
            }
        });
        assert_eq!(
            best_cover_url(&entity),
            Some("https://example.com/only.jpg".to_string())
        );
    }
}
