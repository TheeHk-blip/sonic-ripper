use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineCoverResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: String,
    pub thumbnail_url: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct ItunesSearchResponse {
    #[serde(default)]
    results: Vec<ItunesItem>,
}

#[derive(Debug, Deserialize)]
struct ItunesItem {
    #[serde(rename = "collectionId")]
    collection_id: Option<u64>,
    #[serde(rename = "trackId")]
    track_id: Option<u64>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeezerSearchResponse {
    #[serde(default)]
    data: Vec<DeezerTrack>,
}

#[derive(Debug, Deserialize)]
struct DeezerTrack {
    id: u64,
    title: Option<String>,
    artist: Option<DeezerArtist>,
    album: Option<DeezerAlbum>,
}

#[derive(Debug, Deserialize)]
struct DeezerArtist {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeezerAlbum {
    title: Option<String>,
    #[serde(rename = "cover_medium")]
    cover_medium: Option<String>,
    #[serde(rename = "cover_xl")]
    cover_xl: Option<String>,
}

pub fn upgrade_itunes_artwork(url: &str, target_size: u32) -> String {
    let needle = "100x100bb";
    if url.contains(needle) {
        url.replace(needle, &format!("{target_size}x{target_size}bb"))
    } else {
        url.replace("60x60bb", &format!("{target_size}x{target_size}bb"))
    }
}

fn encode_query(q: &str) -> String {
    url::form_urlencoded::byte_serialize(q.as_bytes()).collect()
}

fn decode_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

async fn search_itunes(client: &reqwest::Client, query: &str) -> Vec<OnlineCoverResult> {
    let mut results = Vec::new();
    let encoded = encode_query(query);

    // 1. Search Albums first (cleanest high-res album covers)
    let album_url = format!(
        "https://itunes.apple.com/search?term={encoded}&entity=album&limit=25"
    );

    if let Ok(res) = client.get(&album_url).send().await {
        if res.status().is_success() {
            if let Ok(data) = res.json::<ItunesSearchResponse>().await {
                for item in data.results {
                    if let Some(art) = item.artwork_url_100 {
                        let high_res = upgrade_itunes_artwork(&art, 1200);
                        let thumb = upgrade_itunes_artwork(&art, 300);
                        let album = item.collection_name.unwrap_or_default();
                        let artist = item.artist_name.unwrap_or_default();
                        let id = format!("itunes-album-{}", item.collection_id.unwrap_or(0));

                        results.push(OnlineCoverResult {
                            id,
                            title: album.clone(),
                            artist,
                            album,
                            cover_url: high_res,
                            thumbnail_url: thumb,
                            source: "iTunes".to_string(),
                        });
                    }
                }
            }
        }
    }

    // 2. Also search songs if fewer than 15 album results
    if results.len() < 15 {
        let song_url = format!(
            "https://itunes.apple.com/search?term={encoded}&entity=song&limit=15"
        );

        if let Ok(res) = client.get(&song_url).send().await {
            if res.status().is_success() {
                if let Ok(data) = res.json::<ItunesSearchResponse>().await {
                    for item in data.results {
                        if let Some(art) = item.artwork_url_100 {
                            let high_res = upgrade_itunes_artwork(&art, 1200);
                            let thumb = upgrade_itunes_artwork(&art, 300);
                            let title = item.track_name.unwrap_or_default();
                            let album = item.collection_name.unwrap_or_default();
                            let artist = item.artist_name.unwrap_or_default();
                            let id = format!("itunes-song-{}", item.track_id.unwrap_or(0));

                            results.push(OnlineCoverResult {
                                id,
                                title,
                                artist,
                                album,
                                cover_url: high_res,
                                thumbnail_url: thumb,
                                source: "iTunes".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

async fn search_deezer(client: &reqwest::Client, query: &str) -> Vec<OnlineCoverResult> {
    let mut results = Vec::new();
    let encoded = encode_query(query);
    let url = format!("https://api.deezer.com/search?q={encoded}&limit=20");

    if let Ok(res) = client.get(&url).send().await {
        if res.status().is_success() {
            if let Ok(data) = res.json::<DeezerSearchResponse>().await {
                for item in data.data {
                    if let Some(album) = item.album {
                        if let Some(cover_xl) = album.cover_xl {
                            let thumb = album.cover_medium.unwrap_or_else(|| cover_xl.clone());
                            let artist = item.artist.and_then(|a| a.name).unwrap_or_default();
                            let album_title = album.title.unwrap_or_default();
                            let title = item.title.unwrap_or_else(|| album_title.clone());

                            results.push(OnlineCoverResult {
                                id: format!("deezer-{}", item.id),
                                title,
                                artist,
                                album: album_title,
                                cover_url: cover_xl,
                                thumbnail_url: thumb,
                                source: "Deezer".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

async fn search_web_images(client: &reqwest::Client, query: &str) -> Vec<OnlineCoverResult> {
    let mut results = Vec::new();
    let query_str = format!("{query} album cover");
    let encoded = encode_query(&query_str);
    let url = format!("https://www.bing.com/images/search?q={encoded}&form=HDRSC2&first=1");

    let req = client
        .get(&url)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        );

    if let Ok(res) = req.send().await {
        if res.status().is_success() {
            if let Ok(html) = res.text().await {
                let re = regex::Regex::new(r#"m=["'](\{.+?\})["']"#).unwrap();
                for cap in re.captures_iter(&html) {
                    if let Some(m_json) = cap.get(1) {
                        let unescaped = decode_html(m_json.as_str());
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&unescaped) {
                            let murl = val.get("murl").and_then(|v| v.as_str()).unwrap_or("");
                            let turl = val.get("turl").and_then(|v| v.as_str()).unwrap_or(murl);
                            let title = val.get("t").and_then(|v| v.as_str()).unwrap_or("");

                            if (murl.starts_with("http://") || murl.starts_with("https://"))
                                && !murl.contains(".svg")
                            {
                                results.push(OnlineCoverResult {
                                    id: format!("web-{}", results.len()),
                                    title: title.to_string(),
                                    artist: String::new(),
                                    album: title.to_string(),
                                    cover_url: murl.to_string(),
                                    thumbnail_url: turl.to_string(),
                                    source: "Web".to_string(),
                                });
                                if results.len() >= 12 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    results
}

#[tauri::command]
pub async fn search_online_covers(
    query: String,
    source: Option<String>,
) -> AppResult<Vec<OnlineCoverResult>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| AppError::Other(format!("Failed to build HTTP client: {e}")))?;

    // If specifically requested "web", search web images directly
    if source.as_deref() == Some("web") {
        let web_results = search_web_images(&client, trimmed).await;
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for item in web_results {
            if seen.insert(item.cover_url.clone()) {
                deduped.push(item);
                if deduped.len() >= 12 {
                    break;
                }
            }
        }
        return Ok(deduped);
    }

    // Default: Search iTunes
    let mut results = search_itunes(&client, trimmed).await;

    // Supplement with Deezer
    if results.len() < 12 {
        let deezer_results = search_deezer(&client, trimmed).await;
        results.extend(deezer_results);
    }

    // If still very few results (less than 4), supplement with web images
    if results.len() < 4 {
        let web_results = search_web_images(&client, trimmed).await;
        results.extend(web_results);
    }

    // Deduplicate by cover_url or normalized (artist, album)
    let mut seen_covers = HashSet::new();
    let mut seen_pairs = HashSet::new();
    let mut deduplicated = Vec::new();

    for item in results {
        let pair_key = format!(
            "{}:{}",
            item.artist.trim().to_lowercase(),
            item.album.trim().to_lowercase()
        );

        if !seen_covers.insert(item.cover_url.clone()) {
            continue;
        }

        if !item.album.trim().is_empty() && !seen_pairs.insert(pair_key) {
            continue;
        }

        deduplicated.push(item);
        if deduplicated.len() >= 12 {
            break;
        }
    }

    Ok(deduplicated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upgrade_itunes_artwork() {
        let url = "https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/a1/b2/c3/100x100bb.jpg";
        let upgraded = upgrade_itunes_artwork(url, 1200);
        assert_eq!(
            upgraded,
            "https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/a1/b2/c3/1200x1200bb.jpg"
        );

        let thumb = upgrade_itunes_artwork(url, 300);
        assert_eq!(
            thumb,
            "https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/a1/b2/c3/300x300bb.jpg"
        );
    }

    #[test]
    fn test_upgrade_itunes_artwork_fallback() {
        let url = "https://example.com/cover.jpg";
        let upgraded = upgrade_itunes_artwork(url, 1200);
        assert_eq!(upgraded, url);
    }

    #[test]
    fn test_encode_query() {
        assert_eq!(encode_query("Queen Bohemian Rhapsody"), "Queen+Bohemian+Rhapsody");
        assert_eq!(encode_query("AC/DC"), "AC%2FDC");
    }

    #[test]
    fn test_decode_html() {
        assert_eq!(decode_html("&quot;Hello&quot; &amp; &#x27;World&#x27;"), "\"Hello\" & 'World'");
    }
}
