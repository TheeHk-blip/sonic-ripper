use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::{ScrapedResult, ScrapedTrackItem};

const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

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

fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(DEFAULT_UA)
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(AppError::from)
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

    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|b| std::cmp::Reverse(b.0));
    Some(candidates.into_iter().next().unwrap().1)
}

struct TrackPageFallback {
    album: Option<String>,
    year: Option<String>,
    cover_url: Option<String>,
}

async fn fetch_track_page_fallback(track_url: &str) -> TrackPageFallback {
    let empty = TrackPageFallback {
        album: None,
        year: None,
        cover_url: None,
    };
    let c = match client() {
        Ok(c) => c,
        Err(_) => return empty,
    };
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

    let mut album = None;
    let mut year = None;
    if let Some(caps) = OG_DESC_RE.captures(&html) {
        let desc = decode_html(&caps[1]);
        let parts: Vec<&str> = desc
            .split(" · ")
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 3 && parts[1].to_lowercase() != "song" {
            album = Some(parts[1].to_string());
        }
        if let Some(m) = YEAR_RE.find(&desc) {
            year = Some(m.as_str().to_string());
        }
    }
    let cover_url = OG_IMAGE_RE.captures(&html).map(|c| decode_html(&c[1]));

    TrackPageFallback {
        album,
        year,
        cover_url,
    }
}

async fn fetch_album_page_fallback(album_url: &str) -> (Option<String>, Option<String>) {
    let c = match client() {
        Ok(c) => c,
        Err(_) => return (None, None),
    };
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
    let c = client().ok()?;
    let res = c.get(url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let html = res.text().await.ok()?;
    OG_IMAGE_RE.captures(&html).map(|c| decode_html(&c[1]))
}

const COVER_FETCH_CONCURRENCY: usize = 8;

fn track_id_from_entry(t: &Value) -> Option<String> {
    if let Some(uri) = t.get("uri").and_then(|v| v.as_str()) {
        let id = uri.rsplit(':').next().unwrap_or("");
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(id.to_string());
        }
    }
    t.get("id").and_then(|v| v.as_str()).map(String::from)
}

struct TrackEmbedMeta {
    album: Option<String>,
    cover_url: Option<String>,
}

async fn fetch_track_via_web_api(track_id: &str, token: &str) -> Option<TrackEmbedMeta> {
    let c = client().ok()?;
    let url = format!("https://api.spotify.com/v1/tracks/{track_id}");
    let res = c
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let data: Value = res.json().await.ok()?;
    let album = data
        .pointer("/album/name")
        .and_then(|v| v.as_str())
        .map(decode_html);
    // Web API returns album images largest-first.
    let cover_url = data
        .pointer("/album/images")
        .and_then(|v| v.as_array())
        .and_then(|imgs| imgs.first())
        .and_then(|img| img.get("url"))
        .and_then(|v| v.as_str())
        .map(String::from);
    if album.is_some() || cover_url.is_some() {
        Some(TrackEmbedMeta { album, cover_url })
    } else {
        None
    }
}

async fn fetch_track_embed_meta(
    semaphore: Arc<Semaphore>,
    track_id: Option<String>,
    playlist_access_token: Option<Arc<String>>,
) -> Option<TrackEmbedMeta> {
    let track_id = track_id?;
    let _permit = semaphore.acquire_owned().await.ok()?;

    let mut album: Option<String> = None;
    let mut cover_url: Option<String> = None;
    let mut access_token: Option<String> = None;

    let embed_url = format!("https://open.spotify.com/embed/track/{track_id}");
    if let Ok(c) = client() {
        if let Ok(res) = c
            .get(&embed_url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(html) = res.text().await {
                    if let Some(caps) = NEXT_DATA_RE.captures(&html) {
                        if let Ok(next_data) = serde_json::from_str::<Value>(&caps[1]) {
                            access_token = extract_access_token(&next_data);
                            if let Some(entity) =
                                next_data.pointer("/props/pageProps/state/data/entity")
                            {
                                album = entity
                                    .pointer("/album/name")
                                    .and_then(|v| v.as_str())
                                    .map(decode_html);
                                cover_url = best_cover_url(entity);
                            }
                        }
                    }
                }
            }
        }
    }

    let access_token =
        access_token.or_else(|| playlist_access_token.as_deref().map(|t| t.to_string()));

    if (album.is_none() || cover_url.is_none()) && access_token.is_some() {
        if let Some(meta) =
            fetch_track_via_web_api(&track_id, access_token.as_deref().unwrap()).await
        {
            if album.is_none() {
                album = meta.album;
            }
            if cover_url.is_none() {
                cover_url = meta.cover_url;
            }
        }
    }

    if album.is_none() || cover_url.is_none() {
        let open_url = format!("https://open.spotify.com/track/{track_id}");
        let page_fallback = fetch_track_page_fallback(&open_url).await;
        if album.is_none() {
            album = page_fallback.album;
        }
        if cover_url.is_none() {
            cover_url = page_fallback.cover_url;
        }
    }

    if album.is_some() || cover_url.is_some() {
        Some(TrackEmbedMeta { album, cover_url })
    } else {
        None
    }
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
                    if let Ok(c) = client() {
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

/// Pull the anonymous access token Spotify embeds in `__NEXT_DATA__`.
/// Used to call spclient for full playlist track lists (>~100 tracks).
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

/// Full ordered track IDs for a playlist via spclient (no 100-track cap).
async fn fetch_playlist_track_ids_spclient(playlist_id: &str, token: &str) -> Option<Vec<String>> {
    let c = client().ok()?;
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

/// Metadata for one track from its embed page (used for tracks beyond the
/// first ~100 that the playlist embed does not include).
async fn fetch_track_item_from_embed(track_id: String) -> Option<ScrapedTrackItem> {
    let c = client().ok()?;
    let embed_url = format!("https://open.spotify.com/embed/track/{track_id}");
    let res = c
        .get(&embed_url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let html = res.text().await.ok()?;
    let caps = NEXT_DATA_RE.captures(&html)?;
    let next_data: Value = serde_json::from_str(&caps[1]).ok()?;
    let entity = next_data.pointer("/props/pageProps/state/data/entity")?;

    let title = decode_html(
        entity
            .get("name")
            .or_else(|| entity.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Track"),
    );
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
            .get("subtitle")
            .and_then(|v| v.as_str())
            .map(decode_html)
            .unwrap_or_else(|| "Unknown Artist".to_string())
    };
    let album = entity
        .pointer("/album/name")
        .and_then(|v| v.as_str())
        .map(decode_html);
    let duration = entity
        .get("duration")
        .and_then(|v| v.as_u64())
        .map(|ms| (ms as f64 / 1000.0).round() as u32);
    let preview_url = entity
        .pointer("/audioPreview/url")
        .and_then(|v| v.as_str())
        .map(String::from);
    let cover_url = best_cover_url(entity);
    let release_year = extract_release_year(entity);

    Some(ScrapedTrackItem {
        title,
        artist,
        album,
        duration,
        cover_url,
        preview_url,
        release_year,
    })
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
        if let Ok(c) = client() {
            if let Ok(res) = c.head(&url).send().await {
                url = res.url().to_string();
            }
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
    let c = client()?;
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

        let needs_fallback =
            entity_album.is_empty() || entity_cover.is_none() || entity_year.is_none();
        let fallback = if needs_fallback {
            Some(fetch_track_page_fallback(&url).await)
        } else {
            None
        };

        let album = if !entity_album.is_empty() {
            entity_album
        } else {
            fallback
                .as_ref()
                .and_then(|f| f.album.clone())
                .unwrap_or_else(|| title.clone())
        };
        let cover_url =
            entity_cover.or_else(|| fallback.as_ref().and_then(|f| f.cover_url.clone()));
        let release_year = entity_year.or_else(|| fallback.as_ref().and_then(|f| f.year.clone()));
        let preview_url = entity
            .pointer("/audioPreview/url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let duration = entity
            .get("duration")
            .and_then(|v| v.as_u64())
            .map(|ms| (ms as f64 / 1000.0).round() as u32)
            .unwrap_or(180);

        if title.is_empty() {
            return Ok(None);
        }

        return Ok(Some(ScrapedResult::Track(ScrapedTrackItem {
            title,
            artist,
            album: Some(album),
            duration: Some(duration),
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

    let mut tracks = Vec::with_capacity(raw_tracks.len());
    let mut track_ids: Vec<Option<String>> = Vec::with_capacity(raw_tracks.len());
    let mut album_is_placeholder: Vec<bool> = Vec::with_capacity(raw_tracks.len());
    for t in &raw_tracks {
        track_ids.push(track_id_from_entry(t));
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
        let (t_album, is_placeholder_album) = if entity_type == "album" {
            (Some(playlist_name.clone()), false)
        } else if let Some(a) = t.pointer("/album/name").and_then(|v| v.as_str()) {
            (Some(decode_html(a)), false)
        } else {
            (None, true)
        };
        album_is_placeholder.push(is_placeholder_album);
        let duration = t
            .get("duration")
            .and_then(|v| v.as_u64())
            .map(|ms| (ms as f64 / 1000.0).round() as u32)
            .unwrap_or(180);
        let preview_url = t
            .pointer("/audioPreview/url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let cover_url = best_cover_url(t);
        let release_year = if entity_type == "album" {
            playlist_level_year.clone()
        } else {
            extract_release_year(t).or_else(|| playlist_level_year.clone())
        };

        tracks.push(ScrapedTrackItem {
            title: t_title,
            artist: t_artist,
            album: t_album,
            duration: Some(duration),
            cover_url,
            preview_url,
            release_year,
        });
    }

    if tracks.is_empty() {
        return Ok(None);
    }

    // Playlists over ~100 tracks: embed trackList is truncated. Use the
    // anonymous token from __NEXT_DATA__ + spclient for the full ordered
    // URI list, then fill missing tracks via per-track embeds.
    if entity_type == "playlist" {
        if let Some(token) = anon_access_token.as_deref() {
            if let Some(all_ids) = fetch_playlist_track_ids_spclient(&entity_id, token).await {
                if all_ids.len() > tracks.len() {
                    println!(
                        "[Spotify Scraper] Embed had {} tracks; spclient reports {} — fetching remainder",
                        tracks.len(),
                        all_ids.len()
                    );

                    let mut by_id: std::collections::HashMap<String, (ScrapedTrackItem, bool)> =
                        std::collections::HashMap::new();
                    for (i, tid) in track_ids.iter().enumerate() {
                        if let Some(id) = tid {
                            let placeholder = album_is_placeholder.get(i).copied().unwrap_or(false);
                            by_id.insert(id.clone(), (tracks[i].clone(), placeholder));
                        }
                    }

                    let missing: Vec<String> = all_ids
                        .iter()
                        .filter(|id| !by_id.contains_key(id.as_str()))
                        .cloned()
                        .collect();

                    let semaphore = Arc::new(Semaphore::new(COVER_FETCH_CONCURRENCY));
                    let mut handles = Vec::with_capacity(missing.len());
                    for tid in missing {
                        let sem = semaphore.clone();
                        handles.push(tokio::spawn(async move {
                            let _permit = sem.acquire_owned().await.ok();
                            let item = fetch_track_item_from_embed(tid.clone()).await;
                            (tid, item)
                        }));
                    }
                    for handle in handles {
                        if let Ok((tid, Some(item))) = handle.await {
                            let placeholder = item.album.is_none();
                            by_id.insert(tid, (item, placeholder));
                        }
                    }

                    // Rebuild in spclient order
                    let mut ordered = Vec::with_capacity(all_ids.len());
                    let mut ordered_placeholder = Vec::with_capacity(all_ids.len());
                    for id in &all_ids {
                        if let Some((item, placeholder)) = by_id.remove(id) {
                            ordered.push(item);
                            ordered_placeholder.push(placeholder);
                        } else {
                            ordered.push(ScrapedTrackItem {
                                title: format!("Track {id}"),
                                artist: "Unknown Artist".to_string(),
                                album: None,
                                duration: Some(180),
                                cover_url: None,
                                preview_url: None,
                                release_year: playlist_level_year.clone(),
                            });
                            ordered_placeholder.push(true);
                        }
                    }
                    tracks = ordered;
                    album_is_placeholder = ordered_placeholder;
                    track_ids = all_ids.into_iter().map(Some).collect();
                }
            }
        }
    }

    // For playlists: fetch each track's own album name and cover art when
    // either is missing. Album stays blank (None) when recovery fails —
    // never fall back to the playlist name.
    if entity_type != "album" {
        let semaphore = Arc::new(Semaphore::new(COVER_FETCH_CONCURRENCY));
        let mut handles = Vec::with_capacity(track_ids.len());
        for (i, track_id) in track_ids.into_iter().enumerate() {
            let needs_cover = tracks.get(i).and_then(|t| t.cover_url.as_ref()).is_none();
            let needs_album = album_is_placeholder.get(i).copied().unwrap_or(false);
            if !needs_cover && !needs_album {
                handles.push(tokio::spawn(async move { (i, None) }));
                continue;
            }
            let sem = semaphore.clone();
            let token = anon_access_token.clone();
            handles.push(tokio::spawn(async move {
                let meta = fetch_track_embed_meta(sem, track_id, token).await;
                (i, meta)
            }));
        }
        for handle in handles {
            match handle.await {
                Ok((i, Some(meta))) => {
                    if let Some(t) = tracks.get_mut(i) {
                        if t.cover_url.is_none() {
                            if let Some(cover) = meta.cover_url {
                                t.cover_url = Some(cover);
                            }
                        }
                        if album_is_placeholder.get(i).copied().unwrap_or(false) {
                            if let Some(album) = meta.album {
                                t.album = Some(album);
                            } else {
                                t.album = None;
                                println!(
                                    "[Spotify Scraper] Track {i} ({}): per-track fetch \
                                     succeeded but had no album name — leaving album blank",
                                    t.title
                                );
                            }
                        }
                    }
                }
                Ok((i, None)) => {
                    if album_is_placeholder.get(i).copied().unwrap_or(false) {
                        if let Some(t) = tracks.get_mut(i) {
                            t.album = None;
                            println!(
                                "[Spotify Scraper] Track {i} ({}): per-track embed fetch \
                                 failed (network/rate-limit/parse) — leaving album blank",
                                t.title
                            );
                        }
                    }
                }
                Err(_) => {}
            }
        }
    }

    for t in tracks.iter_mut() {
        if t.cover_url.is_none() {
            t.cover_url = entity_cover_url.clone();
        }
    }

    Ok(Some(ScrapedResult::Playlist {
        playlist_name,
        is_album: entity_type == "album",
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
