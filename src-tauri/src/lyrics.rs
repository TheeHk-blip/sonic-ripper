use std::path::Path;
use regex::Regex;
use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

use crate::download::CookieAuth;
use crate::models::Track;

#[derive(Debug, Deserialize)]
struct LrclibResponse {
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(default)]
    instrumental: Option<bool>,
}

/// Cleans video artifacts and extra tags from track titles for higher match accuracy.
/// E.g. "One More Time (Official Music Video)" -> "One More Time"
pub fn clean_title(title: &str) -> String {
    let re = Regex::new(
        r"(?i)\s*[\(\[](?:official\s*(?:video|music\s*video|audio|lyric\s*video|visualizer)?|lyrics?|audio|visualizer|remastered|hd|4k|hq)[\)\]]"
    ).unwrap();
    let cleaned = re.replace_all(title, "");
    cleaned.trim().to_string()
}

/// Strips timestamp tags like [01:23.45] from synced lyrics into clean plain text.
pub fn strip_synced_timestamps(synced: &str) -> String {
    let re = Regex::new(r"\[\d+:\d+(?:\.\d+)?\]\s*").unwrap();
    let mut cleaned_lines = Vec::new();
    for line in synced.lines() {
        let stripped = re.replace_all(line, "").trim().to_string();
        if !stripped.is_empty() {
            cleaned_lines.push(stripped);
        }
    }
    cleaned_lines.join("\n")
}

/// Cleans WebVTT subtitles downloaded from YouTube into readable plain text lyrics.
pub fn clean_vtt_subtitles(vtt_content: &str) -> Option<String> {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let bracket_re = Regex::new(r"(?i)^\[[a-záéíóúüñ\s]+\]$").unwrap();

    let mut lines = Vec::new();
    let mut prev_line = String::new();

    for raw_line in vtt_content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("WEBVTT")
            || trimmed.starts_with("NOTE")
            || trimmed.starts_with("Kind:")
            || trimmed.starts_with("Language:")
            || trimmed.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        if trimmed.contains("-->") {
            continue;
        }

        // Strip inline HTML/timestamp tags like <00:00:31.550><c>
        let without_tags = tag_re.replace_all(trimmed, "");
        let clean_line = without_tags.trim().to_string();

        if clean_line.is_empty() {
            continue;
        }

        // Ignore sound annotations like [Music], [Aplausos], [Laughter]
        if bracket_re.is_match(&clean_line) {
            continue;
        }

        // Deduplicate consecutive identical lines from rolling captions
        if clean_line.eq_ignore_ascii_case(&prev_line) {
            continue;
        }

        prev_line = clean_line.clone();
        lines.push(clean_line);
    }

    if lines.len() < 2 {
        return None;
    }

    Some(lines.join("\n"))
}

/// Attempts to fetch lyrics from LRCLIB open lyrics database.
pub async fn fetch_lrclib_lyrics(
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration: Option<u32>,
) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent("SonicRipper/1.0.0 (https://github.com/neokamen/sonic-ripper)")
        .build()
        .ok()?;

    let cleaned_title = clean_title(title);
    let title_to_use = if cleaned_title.is_empty() { title } else { &cleaned_title };

    // 1. Direct get lookup
    let mut get_url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        url::form_urlencoded::byte_serialize(artist.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(title_to_use.as_bytes()).collect::<String>()
    );
    if let Some(alb) = album {
        if !alb.is_empty() && alb != "Spotify Track" && alb != "YouTube" {
            get_url.push_str(&format!(
                "&album_name={}",
                url::form_urlencoded::byte_serialize(alb.as_bytes()).collect::<String>()
            ));
        }
    }
    if let Some(dur) = duration {
        if dur > 0 {
            get_url.push_str(&format!("&duration={dur}"));
        }
    }

    if let Ok(res) = client.get(&get_url).send().await {
        if res.status().is_success() {
            if let Ok(payload) = res.json::<LrclibResponse>().await {
                if payload.instrumental == Some(true) {
                    return None;
                }
                if let Some(plain) = payload.plain_lyrics {
                    let trimmed = plain.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
                if let Some(synced) = payload.synced_lyrics {
                    let cleaned = strip_synced_timestamps(&synced);
                    if !cleaned.trim().is_empty() {
                        return Some(cleaned);
                    }
                }
            }
        }
    }

    // 2. Search fallback
    let search_query = format!("{artist} {title_to_use}");
    let search_url = format!(
        "https://lrclib.net/api/search?q={}",
        url::form_urlencoded::byte_serialize(search_query.as_bytes()).collect::<String>()
    );

    if let Ok(res) = client.get(&search_url).send().await {
        if res.status().is_success() {
            if let Ok(items) = res.json::<Vec<LrclibResponse>>().await {
                for item in items {
                    if item.instrumental == Some(true) {
                        continue;
                    }
                    if let Some(plain) = item.plain_lyrics {
                        let trimmed = plain.trim();
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                    if let Some(synced) = item.synced_lyrics {
                        let cleaned = strip_synced_timestamps(&synced);
                        if !cleaned.trim().is_empty() {
                            return Some(cleaned);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Fallback: Attempts to extract subtitles from YouTube video using sonic-yt-dlp.
pub async fn fetch_youtube_lyrics(
    app: &AppHandle,
    video_url: &str,
    work_dir: &Path,
    auth: CookieAuth<'_>,
) -> Option<String> {
    let out_prefix = work_dir.join("lyrics.%(ext)s");
    let cmd = app.shell().sidecar("sonic-yt-dlp").ok()?;

    let mut args: Vec<String> = vec![
        "--skip-download".into(),
        "--write-subs".into(),
        "--write-auto-subs".into(),
        "--sub-langs".into(),
        "en.*,es.*,ca.*,all".into(),
        "--sub-format".into(),
        "vtt".into(),
        "-o".into(),
        out_prefix.to_string_lossy().to_string(),
    ];
    auth.append_to(&mut args);
    args.push(video_url.to_string());

    let output = cmd.args(&args).output().await.ok()?;
    if !output.status.success() {
        eprintln!(
            "[Lyrics] yt-dlp subtitle download exited with code: {:?}",
            output.status.code()
        );
    }

    // Inspect work_dir for generated lyrics.*.vtt file
    let mut entries = tokio::fs::read_dir(work_dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("lyrics.") && name.ends_with(".vtt") {
            if let Ok(vtt_content) = tokio::fs::read_to_string(entry.path()).await {
                if let Some(clean) = clean_vtt_subtitles(&vtt_content) {
                    return Some(clean);
                }
            }
        }
    }

    None
}

/// Main entrypoint to retrieve lyrics for a track.
/// Tries LRCLIB first (official plain studio lyrics), then falls back to YouTube subtitles.
pub async fn fetch_track_lyrics(
    app: &AppHandle,
    track: &Track,
    preview_url: Option<&str>,
    work_dir: &Path,
    auth: CookieAuth<'_>,
) -> Option<String> {
    // 1. Try LRCLIB
    if let Some(lyrics) = fetch_lrclib_lyrics(
        &track.artist,
        &track.title,
        Some(&track.album),
        Some(track.duration),
    )
    .await
    {
        return Some(lyrics);
    }

    // 2. Fallback to YouTube subtitles if preview_url is available
    if let Some(url) = preview_url {
        if let Some(lyrics) = fetch_youtube_lyrics(app, url, work_dir, auth).await {
            return Some(lyrics);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_title() {
        assert_eq!(
            clean_title("One More Time (Official Video)"),
            "One More Time"
        );
        assert_eq!(
            clean_title("Feel Good Inc. [Official Music Video]"),
            "Feel Good Inc."
        );
        assert_eq!(
            clean_title("Bohemian Rhapsody (Lyrics)"),
            "Bohemian Rhapsody"
        );
        assert_eq!(
            clean_title("Around the World [HQ]"),
            "Around the World"
        );
    }

    #[test]
    fn test_strip_synced_timestamps() {
        let synced = "[00:15.20] One more time\n[00:18.50] We're gonna celebrate\n[00:22.00]";
        let plain = strip_synced_timestamps(synced);
        assert_eq!(plain, "One more time\nWe're gonna celebrate");
    }

    #[test]
    fn test_clean_vtt_subtitles() {
        let vtt = r#"WEBVTT
Kind: captions
Language: en

00:00:00.480 --> 00:00:20.150 align:start position:0%
[Music]

00:00:20.150 --> 00:00:31.150 align:start position:0%
One more time

00:00:31.150 --> 00:00:35.590 align:start position:0%
One more time
We're gonna celebrate
"#;
        let cleaned = clean_vtt_subtitles(vtt);
        assert!(cleaned.is_some());
        let lyrics = cleaned.unwrap();
        assert_eq!(lyrics, "One more time\nWe're gonna celebrate");
    }
}
