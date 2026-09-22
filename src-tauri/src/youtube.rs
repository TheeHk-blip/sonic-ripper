use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::Semaphore;
use url::Url;

use crate::models::{ScrapedTrackItem, Track};
use crate::AnalyzeProgress;

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const DIRECT_SEARCH_LIMIT: u32 = 10;

#[derive(Debug, Clone)]
pub struct YoutubeMatch {
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

// Cookie state for every yt-dlp call this module makes (search, direct
// resolve, and the `fetch_full_video_info` verification lookup).
//
// This exists separately from `download.rs`'s per-call `CookieAuth` because
// matching/verification here runs during `analyze` — before a download's
// `DownloadOptions` exists at all — so there's no per-call value to thread
// through. Same pattern as `spotify_client_token`/`set_spotify_client_token`
// in `spotify.rs`: set once from the frontend (or synced in from
// `download.rs` — see `download::download_track`/`download_batch`), read
// internally wherever needed.
//
// `--cookies` (an explicit file) wins over `--cookies-from-browser` when
// both are set, matching `download.rs`'s `CookieAuth::append_to` precedence.
#[derive(Default)]
struct YoutubeCookieState {
    cookies_path: Option<String>,
    cookies_from_browser: Option<String>,
}

static YOUTUBE_COOKIES: Lazy<Mutex<YoutubeCookieState>> =
    Lazy::new(|| Mutex::new(YoutubeCookieState::default()));

pub fn set_youtube_cookies(raw_cookies: Option<String>) {
    let mut state = YOUTUBE_COOKIES.lock().unwrap();
    match raw_cookies.filter(|s| !s.trim().is_empty()) {
        Some(raw) => {
            let path = std::env::temp_dir().join("sonic-youtube-cookies.txt");
            match std::fs::write(&path, &raw) {
                Ok(()) => state.cookies_path = Some(path.to_string_lossy().into_owned()),
                Err(e) => {
                    eprintln!("[YouTube] failed to write cookies file: {e}");
                    state.cookies_path = None;
                }
            }
        }
        None => state.cookies_path = None,
    }
}

pub fn set_cookies_from_browser(browser: Option<String>) {
    YOUTUBE_COOKIES.lock().unwrap().cookies_from_browser = browser.filter(|s| !s.trim().is_empty());
}

// The `--cookies`/`--cookies-from-browser` args to append to every sidecar
// invocation in this module, given whatever was last set via
// `set_youtube_cookies`/`set_cookies_from_browser`. Empty if neither is set
// (the previous, cookie-less behavior).
fn cookie_args() -> Vec<String> {
    let state = YOUTUBE_COOKIES.lock().unwrap();
    let mut args = Vec::new();
    if let Some(path) = &state.cookies_path {
        args.push("--cookies".to_string());
        args.push(path.clone());
    } else if let Some(browser) = &state.cookies_from_browser {
        args.push("--cookies-from-browser".to_string());
        args.push(browser.clone());
    }
    args
}

#[derive(Deserialize)]
struct YtDlpFlatEntry {
    id: Option<String>,
    webpage_url: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    thumbnails: Option<Vec<YtThumb>>,
    title: Option<String>,
    uploader: Option<String>,
    playlist_title: Option<String>,
    playlist_count: Option<u32>,
}

#[derive(Deserialize)]
struct YtThumb {
    url: String,
    width: Option<u32>,
    height: Option<u32>,
}

fn pick_best_thumbnail(
    thumbnails: &Option<Vec<YtThumb>>,
    fallback: &Option<String>,
) -> Option<String> {
    thumbnails
        .as_ref()
        .and_then(|thumbs| {
            thumbs
                .iter()
                .max_by_key(|t| t.width.unwrap_or(0) as u64 * t.height.unwrap_or(0) as u64)
        })
        .map(|t| t.url.clone())
        .or_else(|| fallback.clone())
}

fn best_thumbnail(entry: &YtDlpFlatEntry) -> Option<String> {
    pick_best_thumbnail(&entry.thumbnails, &entry.thumbnail)
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

pub fn looks_like_url(input: &str) -> bool {
    match Url::parse(input.trim()) {
        Ok(u) => u.scheme() == "http" || u.scheme() == "https",
        Err(_) => false,
    }
}

fn extract_youtube_playlist_id(input: &str) -> Option<String> {
    let parsed = Url::parse(input.trim()).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");

    match host {
        "youtube.com" | "music.youtube.com" => {
            if parsed.path() == "/playlist" {
                parsed
                    .query_pairs()
                    .find(|(k, _)| k == "list")
                    .map(|(_, v)| v.into_owned())
                    .filter(|id| !id.is_empty())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn looks_like_youtube_playlist(input: &str) -> bool {
    extract_youtube_playlist_id(input).is_some()
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
            } else if parsed.path().starts_with("/shorts/") || parsed.path().starts_with("/embed/")
            {
                parsed
                    .path_segments()?
                    .nth(1)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

async fn run_yt_dlp_search(
    app: &AppHandle,
    engine: &str,
    query: &str,
    limit: u32,
) -> Vec<YtDlpFlatEntry> {
    let search_spec = format!("{engine}{limit}:{query}");

    let sidecar = match app.shell().sidecar("sonic-yt-dlp") {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[YouTube] failed to resolve yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
    ];
    args.extend(cookie_args());
    args.push(search_spec);

    let output = sidecar.args(args).output().await;

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

async fn run_yt_dlp_music_search(app: &AppHandle, query: &str, limit: u32) -> Vec<YtDlpFlatEntry> {
    let sidecar = match app.shell().sidecar("sonic-yt-dlp") {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[YouTube] failed to resolve yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    let mut search_url = Url::parse("https://music.youtube.com/search")
        .expect("static YouTube Music search URL is always valid");
    search_url.query_pairs_mut().append_pair("q", query);
    let search_url = search_url.to_string();

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--playlist-end".to_string(),
        limit.to_string(),
    ];
    args.extend(cookie_args());
    args.push(search_url);

    let output = sidecar.args(args).output().await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[YouTube] failed to spawn yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "[YouTube] yt-dlp music search exited non-zero for \"{query}\": {}",
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

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
    ];
    args.extend(cookie_args());
    args.push(video_url.to_string());

    let output = sidecar.args(args).output().await;

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

async fn run_yt_dlp_streaming(
    app: &AppHandle,
    mut args: Vec<String>,
    fallback_total: Option<u32>,
    has_second_phase: bool,
) -> Vec<YtDlpFlatEntry> {
    let sidecar = match app.shell().sidecar("sonic-yt-dlp") {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[YouTube] failed to resolve yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    // Cookies inserted here (rather than by each caller) so both
    // `run_yt_dlp_search_streaming` and `run_yt_dlp_direct_streaming` get
    // them for free. Inserted before the trailing search-spec/URL
    // positional rather than appended after it, to stay well clear of any
    // argument-parsing ambiguity between options and positionals.
    let insert_at = args.len().saturating_sub(1);
    for (offset, arg) in cookie_args().into_iter().enumerate() {
        args.insert(insert_at + offset, arg);
    }

    let (mut rx, _child) = match sidecar.args(args).spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[YouTube] failed to spawn yt-dlp sidecar: {e}");
            return Vec::new();
        }
    };

    let mut entries: Vec<YtDlpFlatEntry> = Vec::new();
    let mut total = fallback_total;
    let mut stderr_buf = String::new();
    let phase_multiplier = if has_second_phase { 2 } else { 1 };

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<YtDlpFlatEntry>(line) {
                    if let Some(count) = entry.playlist_count {
                        total = Some(count);
                    }
                    entries.push(entry);
                    let completed = entries.len() as u32;
                    let display_total = total.unwrap_or(completed).max(completed);
                    let _ = app.emit(
                        "analyze-progress",
                        AnalyzeProgress {
                            completed,
                            total: display_total * phase_multiplier,
                        },
                    );
                }
            }
            CommandEvent::Stderr(bytes) => {
                stderr_buf.push_str(&String::from_utf8_lossy(&bytes));
            }
            CommandEvent::Error(e) => {
                eprintln!("[YouTube] yt-dlp process error: {e}");
            }
            CommandEvent::Terminated(payload) => {
                if payload.code != Some(0) {
                    eprintln!(
                        "[YouTube] yt-dlp exited with code {:?}: {stderr_buf}",
                        payload.code
                    );
                }
                break;
            }
            _ => {}
        }
    }

    entries
}

async fn run_yt_dlp_search_streaming(
    app: &AppHandle,
    query: &str,
    limit: u32,
) -> Vec<YtDlpFlatEntry> {
    let search_spec = format!("ytsearch{limit}:{query}");
    run_yt_dlp_streaming(
        app,
        vec![
            "--flat-playlist".to_string(),
            "--dump-json".to_string(),
            "--no-warnings".to_string(),
            search_spec,
        ],
        Some(limit),
        false, // no enrichment follows a disposable search
    )
    .await
}

async fn run_yt_dlp_direct_streaming(app: &AppHandle, video_url: &str) -> Vec<YtDlpFlatEntry> {
    run_yt_dlp_streaming(
        app,
        vec![
            "--flat-playlist".to_string(),
            "--dump-json".to_string(),
            "--no-warnings".to_string(),
            video_url.to_string(),
        ],
        None,
        true, // resolve_youtube_playlist always runs enrichment next
    )
    .await
}

// Fetch up to `limit` YouTube matches for a free-text query.
async fn search_youtube_many(app: &AppHandle, query: &str, limit: u32) -> Vec<YoutubeMatch> {
    run_yt_dlp_search(app, "ytsearch", query, limit)
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

fn fold_letterlike_unicode(c: char) -> char {
    let cp = c as u32;

    // Fullwidth Latin letters/digits (Ａ-Ｚ, ａ-ｚ, ０-９) — fixed offset.
    if (0xFF21..=0xFF3A).contains(&cp) {
        return char::from_u32(cp - 0xFF21 + 'A' as u32).unwrap_or(c);
    }
    if (0xFF41..=0xFF5A).contains(&cp) {
        return char::from_u32(cp - 0xFF41 + 'a' as u32).unwrap_or(c);
    }
    if (0xFF10..=0xFF19).contains(&cp) {
        return char::from_u32(cp - 0xFF10 + '0' as u32).unwrap_or(c);
    }

    // The Mathematical Alphanumeric Symbols block has a few historical
    // "holes" — letters that reuse pre-existing Letterlike Symbols
    // codepoints (e.g. ℎ for italic h, ℂ/ℍ/ℕ/ℙ/ℚ/ℝ/ℤ for double-struck)
    // instead of following the block's regular grid below.
    let hole = match c {
        'ℎ' => Some('h'),
        'ℬ' => Some('B'),
        'ℰ' => Some('E'),
        'ℱ' => Some('F'),
        'ℋ' => Some('H'),
        'ℐ' => Some('I'),
        'ℒ' => Some('L'),
        'ℳ' => Some('M'),
        'ℛ' => Some('R'),
        'ℯ' => Some('e'),
        'ℊ' => Some('g'),
        'ℴ' => Some('o'),
        'ℭ' => Some('C'),
        'ℌ' => Some('H'),
        'ℑ' => Some('I'),
        'ℜ' => Some('R'),
        'ℨ' => Some('Z'),
        'ℂ' => Some('C'),
        'ℍ' => Some('H'),
        'ℕ' => Some('N'),
        'ℙ' => Some('P'),
        'ℚ' => Some('Q'),
        'ℝ' => Some('R'),
        'ℤ' => Some('Z'),
        _ => None,
    };
    if let Some(h) = hole {
        return h;
    }

    // Cyrillic/Greek letters that render pixel-for-pixel identical (or
    // near enough) to a Latin letter in essentially every font
    let homoglyph = match c {
        // Cyrillic lowercase
        'а' => Some('a'),
        'в' => Some('b'),
        'е' => Some('e'),
        'к' => Some('k'),
        'м' => Some('m'),
        'н' => Some('h'),
        'о' => Some('o'),
        'р' => Some('p'),
        'с' => Some('c'),
        'т' => Some('t'),
        'у' => Some('y'),
        'х' => Some('x'),
        'ѕ' => Some('s'),
        'і' => Some('i'),
        'ј' => Some('j'),
        // Cyrillic uppercase
        'А' => Some('A'),
        'В' => Some('B'),
        'Е' => Some('E'),
        'К' => Some('K'),
        'М' => Some('M'),
        'Н' => Some('H'),
        'О' => Some('O'),
        'Р' => Some('P'),
        'С' => Some('C'),
        'Т' => Some('T'),
        'У' => Some('Y'),
        'Х' => Some('X'),
        // Greek
        'ο' => Some('o'),
        'ρ' => Some('p'),
        'υ' => Some('y'),
        'Α' => Some('A'),
        'Β' => Some('B'),
        'Ε' => Some('E'),
        'Ζ' => Some('Z'),
        'Η' => Some('H'),
        'Ι' => Some('I'),
        'Κ' => Some('K'),
        'Μ' => Some('M'),
        'Ν' => Some('N'),
        'Ο' => Some('O'),
        'Ρ' => Some('P'),
        'Τ' => Some('T'),
        'Υ' => Some('Y'),
        'Χ' => Some('X'),
        _ => None,
    };
    if let Some(h) = homoglyph {
        return h;
    }

    // Mathematical Alphanumeric Symbols letters (U+1D400-U+1D6A3): 13
    // style groups of 52 codepoints each — 26 uppercase then 26 lowercase —
    // mapping back to ASCII by position within the group.
    if (0x1D400..=0x1D6A3).contains(&cp) {
        let offset = (cp - 0x1D400) % 52;
        return if offset < 26 {
            char::from_u32('A' as u32 + offset).unwrap_or(c)
        } else {
            char::from_u32('a' as u32 + (offset - 26)).unwrap_or(c)
        };
    }
    // Mathematical digits (bold, double-struck, sans-serif, sans-serif
    // bold, monospace): 5 groups of 10.
    if (0x1D7CE..=0x1D7FF).contains(&cp) {
        let offset = (cp - 0x1D7CE) % 10;
        return char::from_u32('0' as u32 + offset).unwrap_or(c);
    }

    c
}

// Lowercases, strips punctuation, and drops common upload-noise tokens
// ("official video", "lyrics", "ft.", "remaster", ...) so title strings
// from very differently-formatted sources can be compared on their
// actual words rather than incidental formatting.
fn normalize_for_match(s: &str) -> Vec<String> {
    // NOTE: deliberately does NOT strip "lyric"/"lyrics"/"clean" — those are
    // handled as altered-version *signals* below (see
    // `ALTERED_VERSION_SIGNAL_WORDS`), not as noise. Stripping them here
    // used to make them invisible to that check entirely (the check
    // inspects the same normalized word list this function produces), which
    // silently let lyric-video and censored "Clean" edits sail through
    // title/artist scoring unflagged. "explicit" stays noise: it marks the
    // uncensored original, i.e. the version we *want*, not an altered one.
    const NOISE: &[&str] = &[
        "official",
        "video",
        "audio",
        "remaster",
        "remastered",
        "hd",
        "hq",
        "ft",
        "feat",
        "featuring",
        "explicit",
        "visualizer",
        "topic",
        "provided",
        "to",
        "by",
    ];
    s.chars()
        .map(fold_letterlike_unicode)
        .collect::<String>()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .filter(|w| !NOISE.contains(&w.as_str()))
        .collect()
}

// Words that show up in a cover, tribute-act, karaoke backing track,
// instrumental, live-performance, reaction, remix, or otherwise-altered-
// edit video's title/channel name but essentially never in an original
// studio release's — checked as its own reject, independent of the
// title/artist scoring below. That scoring alone doesn't catch these: an
// altered version keeps the original song's title words (so `title_score`
// scores high), can be close enough in length to pass the duration check
// (a live cut, remix, or instrumental is often only a matter of seconds
// off, and very often names the original artist right in its own title or channel
// as attribution enough to satisfy the `artist_hit` check too.
//
// `normalize_for_match` already lowercases/strips punctuation into single tokens, so this
// reuses it rather than re-implementing that on top of a fresh haystack.
const ALTERED_VERSION_SIGNAL_WORDS: &[&str] = &[
    "cover",
    "covers",
    "tribute",
    "karaoke",
    "acapella",
    "cappella",
    "originally",
    "reaction",
    "parody",
    "instrumental",
    "instrumentals",
    "live",
    "performance",
    "remix",
    "rmx",
    "mix",
    "mashup",
    "megamix",
    "slowed",
    "sped",
    "reverb",
    "nightcore",
    "chopped",
    "screwed",
    "extended",
    "without",
    "lyric",
    "lyrics",
    "radio",
    "edit",
    "club",
    "cut",
    "clean",
    "beat",
    "beats",
    "mp4",
    "mp3",
    "wav",
    "webm",
    "mkv",
    "mov",
    "avi",
    "dubplate",
    "plate",
    "raw",
    "acoustic",
    "unplugged",
    "freestyle",
    "cypher",
    "demo",
    "leak",
    "leaked",
    "unreleased",
    "snippet",
    "bootleg",
    "fanmade",
    "trailer",
    "preview",
    "diss",
    "reversed",
    "chipmunk",
    "8d",
    "boosted",
    "vip",
    "flip",
    "ai",
    "zilizopendwa",
    "challenge",
    "choir",
    "zumba",
    "fast",
    "loop",
    "vs",
    "versus",
    "b2b",
    "back2back",
    "medley",
    "blend",
];

// True when a candidate's channel name matches YouTube's auto-generated
// "<Artist> - Topic" format used for Content ID-ingested official audio.
// These channels are generated algorithmically from the rights holder's
// own catalog, not uploaded by a random user, so a hit from one of them is
// about as close to a guaranteed original master as a plain-text search
// can get. Used only as a tie-breaking bonus below, never a requirement —
// plenty of legitimately official audio (Vevo, an artist's own channel)
// isn't a Topic channel at all.
// Major record labels that upload official audio/video directly under
// their own branded channel name rather than an artist-Topic or Vevo
// channel — e.g. "Atlantic Records" for Estelle's "Come Over". Checked as
// a substring match, case-insensitive. Missing this let a random-named
// uploader's re-recording ("Estelle-Come Over (feat. Sean Paul)",
// uploader "Abby Dallas" — actually an acoustic cover, not the original,
// despite an exact-matching duration) narrowly outscore the genuine
// Atlantic Records official upload on tie-break alone.
//
// Organized by parent group so a missing label is easy to spot and slot
// in under the right one. Deliberately only full label names, never a
// bare generic word like "records" or a short acronym prone to appearing
// inside an unrelated channel name — a false negative here just means no
// bonus (harmless), but a false positive would hand the tie-break bonus
// to a channel that isn't actually a major label.
const MAJOR_LABEL_CHANNEL_MARKERS: &[&str] = &[
    // Universal Music Group
    "universal music",
    "umg",
    "republic records",
    "interscope",
    "geffen",
    "a&m records",
    "def jam",
    "island records",
    "motown records",
    "capitol records",
    "capitol music group",
    "virgin records",
    "emi records",
    "polydor",
    "decca records",
    "verve records",
    "mercury records",
    "casablanca records",
    "deutsche grammophon",
    "universal music latino",
    "universal music latin",
    "fonovisa",
    "disa records",
    "machete music",
    "aftermath entertainment",
    "shady records",
    "top dawg entertainment",
    "quality control music",
    "big machine records",
    "cash money records",
    "young money",
    "ovo sound",
    "astralwerks",
    "ingrooves",
    "big loud records",
    "mercury nashville",
    "mca nashville",
    "verve label group",
    "polydor records",
    "spinnin' records",
    "spinnin records",
    // Sony Music Entertainment
    "sony music",
    "columbia records",
    "rca records",
    "epic records",
    "arista records",
    "sony music latin",
    "sony classical",
    "legacy recordings",
    "the orchard",
    "ultra music",
    "so so def",
    "laface records",
    "jive records",
    "zomba",
    "syco music",
    "rca inspiration",
    "provident label group",
    "sony masterworks",
    "rca records nashville",
    "arista nashville",
    "monument records",
    // Warner Music Group
    "warner records",
    "warner music",
    "atlantic records",
    "elektra records",
    "elektra music group",
    "parlophone",
    "asylum records",
    "reprise records",
    "rhino entertainment",
    "300 entertainment",
    "fueled by ramen",
    "roadrunner records",
    "nonesuch records",
    "east west records",
    "warner chappell",
    "warner records nashville",
    "big beat records",
    "canvasback music",
    "ffrr",
    // Major independents / hip-hop & pop labels
    "roc nation",
    "empire",
    "dreamville",
    "concord records",
    "xl recordings",
    "rough trade records",
    "ninja tune",
    "sub pop",
    "domino recording",
    "mad decent",
    "rostrum records",
    "quality control",
    "epitaph records",
    "fearless records",
    "hopeless records",
    "glassnote records",
    "secretly canadian",
    "matador records",
    "merge records",
    "because music",
    "kobalt music",
    "aware records",
    "maybach music group",
    "grand hustle",
    "1017 records",
    "alamo records",
    "generation now",
    // K-pop / other global majors
    "sm entertainment",
    "yg entertainment",
    "jyp entertainment",
    "hybe labels",
    "big hit music",
    "jype",
    "starship entertainment",
    "pledis entertainment",
    "cube entertainment",
    "avex trax",
    //Reggae / dancehall labels
    "vp records",
    "greensleeves records",
    "mixpak records",
    "chimney records",
    "head concussion records",
    "necessary mayhem",
    "penthouse records",
    "jammys records",
    "shocking vibes",
    "kingston stone entertainment",
    "notnice records",
];

// True when `uploader` is a genuine Vevo channel for one of the credited
// artists — i.e. exactly `<ArtistName>VEVO`, matching Vevo's real
// channel-naming convention, with nothing else before the suffix.
// Deliberately stricter than a bare "vevo" substring check: that also
// matched fan channels like "RihannaForVEVO" or "AlexAraujoVEVO" — neither
// an actual Vevo channel, just a name with the word tacked on to look
// official. Both would've collected the same trust bonus as the real
// "RihannaVEVO" under a substring check.
fn is_vevo_channel(uploader: &str, artist: &str) -> bool {
    let u = uploader.trim().to_lowercase();
    let Some(prefix) = u.strip_suffix("vevo") else {
        return false;
    };
    let prefix: String = prefix.chars().filter(|c| c.is_alphanumeric()).collect();
    if prefix.is_empty() {
        return false;
    }
    split_credited_artists(artist).any(|single_artist| {
        let artist_compact: String = single_artist
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        !artist_compact.is_empty() && artist_compact == prefix
    })
}

// Splits a track's credited-artist string into individual artist names.
// Handles both comma-separated lists ("Popcaan, Drake") and the natural-
// language "X and Y" join some sources use for exactly two artists
// instead of a comma. Splitting only on comma
// left "and" as a literal, un-splittable part of the single "artist"
// string, and every downstream check here requires *all* of that
// string's words to appear in the candidate — including the word "and"
// itself, which of course never appears in a real video's title or
// uploader name. That silently rejected an otherwise perfect match
// (identical title, matching duration, real uploader) purely because the
// credited-artist string happened to use "and" instead of a comma.
fn split_credited_artists(artist: &str) -> impl Iterator<Item = &str> {
    artist
        .split(',')
        .flat_map(|part| part.split(" and "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

// Flat set of every credited artist's normalized words, pooled across all
// artists on the track (unlike `artist_hit`'s per-artist check, this
// doesn't need to know which *whole* artist a word belongs to — it's only
// used to decide whether a candidate word is "accounted for" at all, e.g.
// treating "dave" as expected for artist string "Central Cee, Dave").
fn credited_artist_words(artist: &str) -> HashSet<String> {
    split_credited_artists(artist)
        .flat_map(normalize_for_match)
        .collect()
}

// How many extra floor words trigger the compilation/mashup reject in
// `has_unrelated_padding`, and how much that floor scales with a longer
// expected title — see that function's doc comment for the reasoning and
// the worked examples that picked these numbers.
const UNACCOUNTED_WORDS_FLOOR: usize = 4;
const UNACCOUNTED_WORDS_MULTIPLIER: usize = 2;

// True when a candidate's title carries a large amount of content that
// belongs to neither the expected title nor any credited artist — the
// signature of a second, unrelated song bolted onto this one, e.g. a
// mashup/combo upload titled "Gangbiz X Aim For The Moon (ft. Pop Smoke)"
// for an expected title of just "Gangbiz" by "Central Cee" (unaccounted:
// "x", "aim", "for", "the", "moon", "pop", "smoke" — 7 words hanging off a
// single-word expected title).
//
// `conciseness_penalty` in `match_score` already nudges scoring away from
// padded titles as a tie-breaker, but it's deliberately tiny (capped at
// 0.05) so it can never outweigh a short title's perfect `title_score`,
// exactly the gap that let the Gangbiz mashup through as the only
// candidate that survived scoring at all. This is a hard reject instead,
// for the case where the padding is too large to be explained by normal
// channel branding.
//
// Deliberately does NOT fire on a bare joiner word alone a real
// "Central Cee x Dave - Sprinter" collab single only adds one unaccounted
// word ("x", since "dave" is credited) and stays well under the floor.
// The floor and multiplier were picked against this file's own real
// candidate pool: legitimate branding tags (e.g. an album name folded
// into the title, "Wild West" alongside a 2-3 word expected title) sit at
// 1-3 unaccounted words and must keep passing; an unrelated second title
// or a wholly different video (an interview, a "type beat" upload) runs
// well past double digits and must not.
fn has_unrelated_padding(
    candidate_words: &[String],
    expected_words: &[String],
    artist_words: &HashSet<String>,
) -> bool {
    let unaccounted = candidate_words
        .iter()
        .filter(|w| !expected_words.contains(w) && !artist_words.contains(w.as_str()))
        .count();
    unaccounted >= UNACCOUNTED_WORDS_FLOOR
        && unaccounted > expected_words.len() * UNACCOUNTED_WORDS_MULTIPLIER
}

fn is_topic_channel(uploader: &str) -> bool {
    uploader.trim().to_lowercase().ends_with("- topic")
}

// True when the channel looks like an official Topic / major-label
// upload — everything except the Vevo case, which needs the artist name
// to verify (see `is_vevo_channel`) and so is checked separately at the
// call site. Used as a smaller tie-breaking bonus alongside Topic/Vevo.
fn is_official_channel(uploader: &str) -> bool {
    let u = uploader.trim().to_lowercase();
    u.ends_with(" - topic")
        || u.ends_with("- topic")
        || MAJOR_LABEL_CHANNEL_MARKERS
            .iter()
            .any(|marker| u.contains(marker))
}

// Real-world duration can drift a bit between platforms so both the initial flat-data
// filter in `match_score` and the authoritative real-duration recheck in
// `best_scored_match` allow this much slack before a duration mismatch is
// treated as a sign of a genuinely different track.
const DURATION_TOLERANCE_SECS: u32 = 30;

// How well a YouTube Music candidate actually matches the track we
// searched for. Word overlap on the title (candidate must contain most
// of the *expected* title's words — not just any words in common, since
// two songs by the same artist often share a stray word) combined with
// duration closeness when both durations are known. Higher is better;
// `None` means "don't trust this candidate at all".
// Multiple hashtags baked directly into a video's TITLE (as opposed to its
// description, where they're normal) is close to a perfect tell for reuploads
// Checked on the raw title (not the normalized word list, which strips
// punctuation and would lose the `#` entirely).
const HASHTAG_SPAM_THRESHOLD: usize = 2;

fn looks_like_hashtag_spam(title: &str) -> bool {
    title
        .split_whitespace()
        .filter(|w| w.starts_with('#'))
        .count()
        >= HASHTAG_SPAM_THRESHOLD
}

fn is_regional_indicator(c: char) -> bool {
    let cp = c as u32;
    (0x1F1E6..=0x1F1FF).contains(&cp)
}

fn looks_like_flag_emoji_remake(title: &str) -> bool {
    let chars: Vec<char> = title.chars().collect();
    chars
        .windows(2)
        .any(|w| is_regional_indicator(w[0]) && is_regional_indicator(w[1]))
}

// Featured-artist credit markers that show up baked into a track's own
// title text (as opposed to the separate, dedicated `artist` field) —
// "feat.", "ft.", "featuring", "with". Used by `strip_featured_artist_clause`.
const FEATURED_ARTIST_MARKERS: &[&str] = &["feat", "ft", "featuring", "with"];

// Strips a "(feat. X)" / "[ft. X]" / "featuring X" clause out of a track's
// title before it's used for title-*word* matching. Necessary because
// Spotify (and similar sources) bake the featured artist's name directly
// into the title string — e.g. "Gata (feat. Young Miko)" — and without
// this, that featured artist's name silently becomes part of "the song's
// own title words" for overlap scoring.
//
// Confirmed: this is exactly what let a completely unrelated solo "Young
// Miko" video (no relation whatsoever to "Gata") pass the title-overlap
// threshold in both `match_score` and `real_title_confirms` — 2 of the 3
// tokenized "expected words" ("gata", "young", "miko") matched purely by
// virtue of the candidate being *some* Young Miko video, without "gata"
// appearing in it anywhere. For a short/single-word title like this one,
// that's the difference between a 1-of-1 (100%) and a 2-of-3 (67%)
// title score — both clear the same 0.6 gate, so the actual song title
// effectively didn't need to appear at all. The featured artist is still
// correctly checked via the separate `artist` field and `artist_hit` /
// `has_unrelated_padding`'s `artist_words` — this only concerns what
// counts as the song's *own* title.
//
// Two forms are handled:
// - Bracketed: "Title (feat. X)" / "Title [ft. X]" — the whole bracketed
//   clause is dropped when it *opens* with one of the markers (checked
//   against all four markers, since "(with X)" as a bracket-opener is an
//   unambiguous collab credit, not a real title using the word "with").
// - Bare, unbracketed: "Title feat. X" — truncated from the marker
//   onward, checked as a whole word so this never fires mid-word. Only
//   "feat"/"ft"/"featuring" trigger this form (not "with"), since "with"
//   is common enough in genuine, non-collab titles ("Stuck with U") that
//   truncating on it outside of an explicit bracket would be too
//   aggressive.
fn strip_featured_artist_clause(title: &str) -> String {
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(start) = title.find(open) {
            if let Some(rel_end) = title[start..].find(close) {
                let end = start + rel_end;
                let inner = &title[start + 1..end];
                let first_word = inner
                    .split(|c: char| !c.is_alphanumeric())
                    .find(|w| !w.is_empty())
                    .unwrap_or("")
                    .to_lowercase();
                if FEATURED_ARTIST_MARKERS.contains(&first_word.as_str()) {
                    let mut out = String::with_capacity(title.len());
                    out.push_str(&title[..start]);
                    out.push_str(&title[end + 1..]);
                    return out;
                }
            }
        }
    }

    let lower = title.to_lowercase();
    let bytes = lower.as_bytes();
    for marker in ["feat", "ft", "featuring"] {
        let mut search_from = 0;
        while let Some(rel_pos) = lower[search_from..].find(marker) {
            let pos = search_from + rel_pos;
            let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let after = pos + marker.len();
            let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return title[..pos].to_string();
            }
            search_from = after;
        }
    }

    title.to_string()
}

// Counts how many of `expected`'s words are satisfied by `candidate`'s
// words, respecting multiplicity: each candidate word instance can only
// satisfy one expected occurrence of that word, rather than a plain
// "is this word present anywhere in candidate" check.
//
// The bug this fixes: `expected_words.iter().filter(|w|
// candidate_words.contains(w)).count()` treats `.contains()` as a
// presence test, so a repeated word in the expected title is "matched"
// once per occurrence even when the candidate only says it once. A track
// genuinely titled "Na Na Na" (normalizes to `["na","na","na"]`) scored a
// full 3-of-3 (100%) `title_score` against *any* candidate containing the
// single word "na" even once — e.g. an unrelated video whose title is
// just "NA" — because each of the three expected "na" tokens was
// independently satisfied by the same one candidate token. That's
// strictly easier to pass than the two-of-three overlap a real but
// non-repeating title would need, for exactly the short/repetitive
// titles ("Na Na Na", "Hey Hey Hey", "Come On Come On") already flagged
// elsewhere in this file as the riskiest case for word-overlap scoring.
// Consuming each candidate word as it's matched closes that gap without
// changing anything for the (overwhelmingly common) non-repeating case.
fn multiset_overlap(expected: &[String], candidate: &[String]) -> usize {
    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for w in candidate {
        *remaining.entry(w.as_str()).or_insert(0) += 1;
    }
    let mut overlap = 0;
    for w in expected {
        if let Some(count) = remaining.get_mut(w.as_str()) {
            if *count > 0 {
                *count -= 1;
                overlap += 1;
            }
        }
    }
    overlap
}

fn match_score(
    candidate: &YoutubeMatch,
    title: &str,
    artist: &str,
    expected_duration: Option<u32>,
) -> Option<f64> {
    let expected_words = normalize_for_match(&strip_featured_artist_clause(title));
    if expected_words.is_empty() {
        return None;
    }
    let candidate_title = candidate.title.clone().unwrap_or_default();
    let candidate_uploader = candidate.uploader.clone().unwrap_or_default();
    let candidate_words = normalize_for_match(&candidate_title);

    if looks_like_hashtag_spam(&candidate_title) {
        return None;
    }
    if looks_like_flag_emoji_remake(&candidate_title) {
        return None;
    }

    let candidate_uploader_words = normalize_for_match(&candidate_uploader);
    let is_altered_version_signal = candidate_words
        .iter()
        .chain(candidate_uploader_words.iter())
        .any(|w| ALTERED_VERSION_SIGNAL_WORDS.contains(&w.as_str()) && !expected_words.contains(w));
    if is_altered_version_signal {
        return None;
    }

    let overlap = multiset_overlap(&expected_words, &candidate_words);
    let title_score = overlap as f64 / expected_words.len() as f64;

    // Require most of the expected title's words to actually show up —
    // this is what stops a different, more-popular song by the same
    // artist from being accepted just because the artist name matches.
    if title_score < 0.6 {
        return None;
    }

    // Require the *whole* name of at least one credited artist to appear —
    // not just a single word matched anywhere across the combined artist
    // string. The old check (`any single word` from the whole pool of
    // artist words) let a short, common name fragment stand in for an
    // entire artist: "Lil Baby" normalizes to ["lil", "baby"], and "any"
    // matching meant a completely unrelated "Lil Yachty" upload passed
    // this check purely on sharing the word "lil" — nothing else about the
    // two artists has anything in common. `artist` can credit several
    // artists comma-separated (e.g. "Mustard, Roddy Ricch"), so split on
    // that first and require ALL of at least one individual artist's words
    // to be present — a candidate crediting only one of several featured
    // artists still passes, but a lone generic word fragment can no longer
    // substitute for a full name.
    //
    // Checked against tokenized words (`candidate_words` /
    // `candidate_uploader_words`, both already computed above), not a raw
    // substring search on the lowercased title+uploader string. A prior
    // version used `haystack.contains(w.as_str())` on the raw string — an
    // unanchored substring check, not word-boundary matching — which let a
    // short artist-name fragment match against random letters inside an
    // unrelated word. (This is exactly how an "Intro" by "Digga" was
    // resolved to "01 Rah Digga Intro": "digga" is a genuine standalone
    // word in "Rah Digga" too, so tokenizing alone wouldn't have caught
    // that specific collision — but the same raw-substring bug meant an
    // artist credited as e.g. "Digga D" would have matched as soon as any
    // letter 'd' appeared anywhere in the haystack, which is close to
    // guaranteed, silently defeating this whole check for any artist name
    // containing a short word — a middle initial, "Jr", "Lil", "MC", and
    // so on.) The `channel_bonus` tie-break a few lines below already did
    // this the right way, against `candidate_uploader_words`; this just
    // brings the primary accept/reject gate up to the same standard.
    let artist_hit = split_credited_artists(artist).any(|single_artist| {
        let words = normalize_for_match(single_artist);
        !words.is_empty()
            && words
                .iter()
                .all(|w| candidate_words.contains(w) || candidate_uploader_words.contains(w))
    });
    // Only treat a miss here as a real signal when there was uploader data
    // to check it against. YouTube Music's `--flat-playlist` search results
    // never populate `uploader` at all (see `best_scored_match`'s doc
    // comment) — so on that tier this check silently degrades from "the
    // artist appears in the title OR the channel name" to just "... in the
    // title", even though a genuine official Topic-channel upload is
    // routinely titled with nothing but the bare song name ("DC10", not
    // "Central Cee - DC10") precisely because the artist lives in the
    // channel name instead. Hard-rejecting those here meant the exact
    // clean official audio this tier exists to surface got screened out
    // before it was ever scored, silently forcing every such track down to
    // the noisier plain-YouTube fallback. This doesn't weaken the actual
    // guarantee: a candidate let through here on title_score/altered-
    // version/padding checks alone still has to clear `real_title_confirms`
    // against the REAL, resolved uploader in `best_scored_match` before it
    // can become a final pick — this only stops blocking it from reaching
    // that authoritative check in the first place.
    if !artist_hit && !candidate_uploader.trim().is_empty() {
        return None;
    }

    // Reject a candidate whose title is mostly content that belongs to
    // neither the expected title nor the credited artist(s) — a second,
    // unrelated song mashed/combined into this upload. See
    // `has_unrelated_padding` for why this is a hard reject rather than
    // folding into `conciseness_penalty` below.
    let artist_words = credited_artist_words(artist);
    if has_unrelated_padding(&candidate_words, &expected_words, &artist_words) {
        return None;
    }

    let duration_score = match (candidate.duration, expected_duration) {
        (Some(c), Some(e)) => {
            let diff = (c as i64 - e as i64).unsigned_abs();
            // Big enough gap is a strong signal it's the wrong track
            // (different edit, remix, or just a different song entirely).
            // See `DURATION_TOLERANCE_SECS` for why the cutoff sits where
            // it does. This is still only ever checked against flat-search
            // data here — YouTube Music's flat results never populate
            // `duration` at all, so this branch simply never fires for
            // that tier; the authoritative recheck against real duration
            // happens later, in `best_scored_match`.
            if diff > DURATION_TOLERANCE_SECS as u64 {
                return None;
            }
            1.0 - (diff as f64 / DURATION_TOLERANCE_SECS as f64)
        }
        // Duration unknown on one side — don't penalize, but don't reward either.
        _ => 0.5,
    };

    // Small tie-breaking nudge toward Topic / Vevo / official-looking
    // channels when available — see `is_topic_channel` / `is_official_channel`.
    // Doesn't reject anything on its own; only matters when deciding between
    // two candidates that both already cleared every check above.
    let channel_bonus = if is_topic_channel(&candidate_uploader) {
        0.15
    } else if is_vevo_channel(&candidate_uploader, artist) {
        0.15
    } else if is_official_channel(&candidate_uploader) {
        0.10
    } else if !candidate_uploader.trim().is_empty()
        && split_credited_artists(artist).any(|single_artist| {
            let words = normalize_for_match(single_artist);
            !words.is_empty() && words.iter().all(|w| candidate_uploader_words.contains(w))
        })
    {
        // Not a Topic/Vevo channel, but the uploader's own name IS (all of)
        // one of the credited artists — e.g. uploader "Takeoff" for a
        // Takeoff track. A much weaker signal than a Topic channel (any fan
        // channel could name itself after the artist), but still a
        // reasonable nudge over an unrelated lyric/reaction channel when
        // everything else ties. Confirmed useful: "Casper (Lyrics)"
        // (uploader "Cold World") and "Casper" (uploader "Takeoff") scored
        // identically on title/duration alone, with only search-rank order
        // deciding between them.
        0.05
    } else {
        0.0
    };

    // Small penalty for candidate titles padded with words the expected
    // title doesn't have — "Casper" over "Casper (Lyrics)", "Money Trees
    // (feat. Jay Rock)" over "Money Trees ft. Jay Rock - Kendrick Lamar
    // (good kid m.A.A.d city Deluxe)". `title_score` alone can't see this:
    // it only rewards expected words that show up, and is blind to extra
    // words tacked on. Deliberately tiny and capped so it only ever
    // matters as a tie-breaker among candidates that already scored nearly
    // identically on everything else — never enough to override a real
    // title/duration/album difference.
    let extra_words = candidate_words.len().saturating_sub(expected_words.len());
    let conciseness_penalty = (extra_words as f64 * 0.01).min(0.05);

    Some(title_score * 0.7 + duration_score * 0.3 + channel_bonus - conciseness_penalty)
}

// How many of the top-scoring, already-validated candidates get a real,
// non-flat `fetch_full_video_info` lookup — for the real-title recheck,
// the real-duration re-check, and the album check below. Kept small since
// each is a full extra yt-dlp call per candidate, but wide enough that a
// top-ranked candidate rejected by the real-title/duration recheck still
// leaves a few genuine alternatives to fall through to before giving up on
// this tier entirely.
const ALBUM_CHECK_TOP_N: usize = 4;

// True when two album names refer to the same release closely enough to
// use as a match signal. Exact (case/whitespace-insensitive) equality is
// always accepted. A longer variant is only accepted when the *extra*
// tokens are benign catalog suffixes (Deluxe, Expanded, Remaster, …) —
// never when they carry altered-version signals ("Tribute", "Cut",
// "Remix", …). The old plain `contains` check let
// "Ascension (Don't Ever Wonder) The Tribute" match the expected
// "Ascension (Don't Ever Wonder)" and pick the wrong upload.
fn albums_match(a: &str, b: &str) -> bool {
    let a = a.trim().to_lowercase();
    let b = b.trim().to_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }

    // Deliberately does NOT include "pt"/"part"/"vol"/"volume"/"disc"/"cd" —
    // those mark a genuinely different piece of a multi-part release (a
    // different disc, volume, or part), not a cosmetic reissue tag like
    // "Deluxe"/"Remastered". This function's whole reason for existing is
    // to tell e.g. "Gangsteritus" apart from "Gangsteritus Part 2" — two
    // different songs on two different releases that can otherwise tie on
    // title/duration — so treating "Part 2" as benign would silently
    // confirm the exact mismatch this check is supposed to catch. Letting
    // a numbered-part suffix fall through to a real reject (rather than a
    // false confirm) is the same trade-off this file makes everywhere
    // else: a track reported as unconfirmed is strictly better than one
    // wrongly confirmed against the wrong disc/volume.
    const BENIGN_SUFFIX_WORDS: &[&str] = &[
        "deluxe",
        "expanded",
        "remaster",
        "remastered",
        "anniversary",
        "edition",
        "bonus",
        "tracks",
        "track",
    ];

    let (shorter, longer) = if a.len() <= b.len() {
        (a.as_str(), b.as_str())
    } else {
        (b.as_str(), a.as_str())
    };

    // Require the shorter name to appear as a contiguous substring of the
    // longer one (handles "(Deluxe Edition)" / " - Expanded" style suffixes).
    if !longer.contains(shorter) {
        return false;
    }

    // Strip the shorter name out and inspect whatever remains. Any leftover
    // token that isn't a benign catalog word (or pure punctuation/digits)
    // means this is a different release (tribute, cut, remix, karaoke, …).
    let extra = longer.replacen(shorter, " ", 1);
    let extra_words = normalize_for_match(&extra);
    if extra_words.is_empty() {
        return true;
    }
    extra_words
        .iter()
        .all(|w| BENIGN_SUFFIX_WORDS.contains(&w.as_str()) || w.chars().all(|c| c.is_ascii_digit()))
}

// Re-validates title/artist against a candidate's REAL, non-flat title and
// uploader — i.e. against what `candidate.video_id` actually resolves to,
// not what the flat search entry claimed it was. Mirrors `match_score`'s
// title/artist/altered-version gates, without the duration/channel-bonus
// scoring, since this runs as a pass/fail check on a candidate that
// already cleared `match_score` on (possibly mismatched) flat data.
//
// Returns `false` when the real data actively disagrees with what the
// flat entry claimed (wrong song, or an altered-version signal that
// wasn't visible in the flat title/uploader alone) — this is what catches
// a flat-playlist id/metadata mismatch that would otherwise sail through
// on a coincidentally-in-tolerance duration. Returns `true` when there's
// nothing to check against (real title missing) or the real data agrees;
// the duration/album checks remain the primary gate either way.
fn real_title_confirms(
    real_title: Option<&str>,
    real_uploader: Option<&str>,
    expected_title: &str,
    artist: &str,
) -> bool {
    let Some(real_title) = real_title else {
        return true;
    };
    if looks_like_hashtag_spam(real_title) {
        return false;
    }
    if looks_like_flag_emoji_remake(real_title) {
        return false;
    }
    let expected_words = normalize_for_match(&strip_featured_artist_clause(expected_title));
    if expected_words.is_empty() {
        return true;
    }
    let real_words = normalize_for_match(real_title);
    let real_uploader = real_uploader.unwrap_or_default();
    let real_uploader_words = normalize_for_match(real_uploader);

    let is_altered_version_signal = real_words
        .iter()
        .chain(real_uploader_words.iter())
        .any(|w| ALTERED_VERSION_SIGNAL_WORDS.contains(&w.as_str()) && !expected_words.contains(w));
    if is_altered_version_signal {
        return false;
    }

    let overlap = multiset_overlap(&expected_words, &real_words);
    let title_score = overlap as f64 / expected_words.len() as f64;
    if title_score < 0.6 {
        return false;
    }

    // Same fix as `match_score`'s `artist_hit`: checked against tokenized
    // words, not a raw substring search on the lowercased title+uploader
    // string (see that function's comment for why the raw-substring form
    // silently defeats this check for any artist name containing a short
    // word fragment — this is the exact same gate, just run here against
    // the real, resolved title/uploader instead of the flat one).
    let artist_hit = split_credited_artists(artist).any(|single_artist| {
        let words = normalize_for_match(single_artist);
        !words.is_empty()
            && words
                .iter()
                .all(|w| real_words.contains(w) || real_uploader_words.contains(w))
    });
    if !artist_hit {
        return false;
    }

    // Same mashup/compilation reject as `match_score` (see
    // `has_unrelated_padding`), run here against the real, resolved title —
    // catches a flat entry that looked clean but actually resolves to a
    // padded/combined video, not just the reverse.
    let artist_words = credited_artist_words(artist);
    !has_unrelated_padding(&real_words, &expected_words, &artist_words)
}

// Scores every candidate against the expected title/artist/duration (see
// `match_score`) and returns the best-scoring one, or `None` if nothing
// clears the bar. Shared by every lookup tier below so each one applies
// the exact same validation rather than trusting a raw search order.
//
// Two things need a real, non-flat lookup on the shortlisted candidates
// before a final pick is trustworthy — neither can be done from flat
// search data alone:
//
// - Duration. `match_score`'s duration cutoff (`DURATION_TOLERANCE_SECS`)
//   runs against `YoutubeMatch::duration`, which comes straight from the
//   flat search entry — and YouTube Music's flat results never populate
//   that field at all (confirmed: always null), so for that tier the
//   cutoff silently never fires and every candidate falls into the
//   "duration unknown, don't penalize" branch. Two otherwise-similar
//   candidates (e.g. a song and an unrelated "Part 2") can then tie on
//   score with nothing left to separate them but raw search rank. This
//   re-checks the same cutoff against the real duration from
//   `fetch_full_video_info`, and drops a candidate outright on a miss —
//   even one that scored highest on flat data alone.
// - Album, when the track's real album (`expected_album`) is known: a
//   second-pass override, since title/artist/duration alone can't tell
//   "Gangsteritus" apart from "Gangsteritus Part 2" — both share the main
//   title words and can land within the duration tolerance — but they're
//   different releases, and a real album match is decisive where those
//   aren't.
//
// Both checks only pay for a real lookup on the top `ALBUM_CHECK_TOP_N`
// already-validated candidates rather than every candidate in the pool. A
// candidate that fails the real-duration re-check is dropped; among the
// rest, one lacking album info, or with no album match among the top few,
// falls back to plain score order — the album check is a tie-breaker on
// top of the other checks, not a replacement for them.
// The result of `best_scored_match`, plus whether it was chosen because
// its *real* album (from `fetch_full_video_info`) positively matched
// `expected_album` — as opposed to any of the fallback paths (best
// duration-checked candidate, raw flat-data rank, or simply having no
// `expected_duration`/`expected_album` to check at all).
//
// This distinction matters because only a genuine album match is
// independent corroboration strong enough to trust a pick on its own —
// see this struct's use in `find_validated_audio_match`. Every other path
// through `best_scored_match` is no stronger than title+duration
// agreement, which alone can't always tell a genuinely different
// recording apart from the real one under a matching title (see
// `find_validated_audio_match`'s "thin-evidence" cross-check). Concretely:
// a track whose winning candidate is a `music.youtube.com` upload will
// essentially always end up with `album_confirmed: false` even when
// `expected_album` was supplied, because that host's uploads don't expose
// an `album` field to `fetch_full_video_info` at all — only
// `youtube.com`-hosted uploads do.
struct ScoredMatch {
    result: YoutubeMatch,
    album_confirmed: bool,
}

async fn best_scored_match(
    app: &AppHandle,
    candidates: Vec<YoutubeMatch>,
    title: &str,
    artist: &str,
    expected_duration: Option<u32>,
    expected_album: Option<&str>,
) -> Option<ScoredMatch> {
    let mut scored: Vec<(f64, YoutubeMatch)> = candidates
        .into_iter()
        .filter_map(|m| {
            let score = match_score(&m, title, artist, expected_duration)?;
            Some((score, m))
        })
        .collect();
    scored.sort_by(|(a, _), (b, _)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    eprintln!(
        "[Match] \"{title}\" by \"{artist}\" — expected duration {:?}s, expected album {:?}",
        expected_duration, expected_album
    );
    if scored.is_empty() {
        eprintln!("[Match]   no candidates cleared title/artist/duration scoring");
    }
    for (score, m) in &scored {
        eprintln!(
            "[Match]   score {:.3} | title=\"{}\" uploader=\"{}\" duration={:?}s | {}",
            score,
            m.title.as_deref().unwrap_or("?"),
            m.uploader.as_deref().unwrap_or("?"),
            m.duration,
            m.url
        );
    }

    let expected_album = expected_album.map(str::trim).filter(|a| !a.is_empty());

    if expected_duration.is_none() && expected_album.is_none() {
        return scored.into_iter().next().map(|(_, m)| ScoredMatch {
            result: m,
            album_confirmed: false,
        });
    }

    let mut best_within_duration: Option<YoutubeMatch> = None;
    let mut closest_by_duration: Option<(u32, YoutubeMatch)> = None;
    let mut any_lookup_succeeded = false;

    for (score, candidate) in scored.iter().take(ALBUM_CHECK_TOP_N) {
        let info = fetch_full_video_info(app, &candidate.video_id).await;
        if info.is_some() {
            any_lookup_succeeded = true;
        }

        // A candidate that fails this is the
        // wrong video regardless of how close its duration happens to be.
        if !real_title_confirms(
            info.as_ref().and_then(|i| i.title.as_deref()),
            info.as_ref().and_then(|i| i.uploader.as_deref()),
            title,
            artist,
        ) {
            eprintln!(
                "[Match]   real-title check (score {:.3}) \"{}\": resolved video is actually \"{}\" ({}) — rejecting, flat listing didn't match the id",
                score,
                candidate.title.as_deref().unwrap_or("?"),
                info.as_ref().and_then(|i| i.title.as_deref()).unwrap_or("?"),
                info.as_ref().and_then(|i| i.uploader.as_deref()).unwrap_or("?"),
            );
            continue;
        }

        let real_duration = info
            .as_ref()
            .and_then(|i| i.duration)
            .map(|d| d.round() as u32);

        let mut duration_confirmed = expected_duration.is_none();
        match (expected_duration, real_duration) {
            (Some(expected), Some(real)) => {
                let diff = (real as i64 - expected as i64).unsigned_abs() as u32;
                if closest_by_duration
                    .as_ref()
                    .is_none_or(|(best_diff, _)| diff < *best_diff)
                {
                    closest_by_duration = Some((diff, candidate.clone()));
                }
                if diff > DURATION_TOLERANCE_SECS {
                    eprintln!(
                        "[Match]   duration check (score {:.3}) \"{}\": real duration {}s vs expected {}s (diff {}s) — rejecting",
                        score,
                        candidate.title.as_deref().unwrap_or("?"),
                        real,
                        expected,
                        diff
                    );
                    continue;
                }
                duration_confirmed = true;
            }
            (Some(_), None) => {
                eprintln!(
                    "[Match]   duration check (score {:.3}) \"{}\": real duration unavailable (lookup failed) — cannot confirm, rejecting",
                    score,
                    candidate.title.as_deref().unwrap_or("?")
                );
                continue;
            }
            (None, _) => {}
        }

        if duration_confirmed && best_within_duration.is_none() {
            best_within_duration = Some(candidate.clone());
        }

        let Some(expected_album) = expected_album else {
            continue;
        };
        match info.as_ref().and_then(|i| i.album.as_deref()) {
            Some(album) => {
                eprintln!(
                    "[Match]   album check (score {:.3}) \"{}\": found album {:?}",
                    score,
                    candidate.title.as_deref().unwrap_or("?"),
                    album
                );
                if albums_match(album, expected_album) {
                    eprintln!("[Match]   -> album matches, selecting this candidate");
                    return Some(ScoredMatch {
                        result: candidate.clone(),
                        album_confirmed: true,
                    });
                }
            }
            None => {
                eprintln!(
                    "[Match]   album check (score {:.3}) \"{}\": no album info (lookup failed or field empty)",
                    score,
                    candidate.title.as_deref().unwrap_or("?")
                );
            }
        }
    }

    eprintln!(
        "[Match]   no album match among top candidates, falling back to best duration-checked score"
    );

    if let Some(m) = best_within_duration {
        return Some(ScoredMatch {
            result: m,
            album_confirmed: false,
        });
    }
    if let Some((diff, _)) = closest_by_duration {
        eprintln!(
            "[Match]   closest real duration still {diff}s off (hard limit {DURATION_TOLERANCE_SECS}s) — rejecting all top-N candidates"
        );
        return None;
    }
    if any_lookup_succeeded {
        eprintln!("[Match]   every verified top-N candidate failed real-data checks — rejecting");
        return None;
    }
    // No real data for any top-N candidate at all, so there's nothing to have
    // disproven the flat-data rank. Trusting it is still better than
    // returning nothing outright.
    scored.into_iter().next().map(|(_, m)| ScoredMatch {
        result: m,
        album_confirmed: false,
    })
}

// Looks up a track via a YouTube Music search (see `run_yt_dlp_music_search`)
// instead of plain YouTube search. Not used during analyze/enrich —
// `search_youtube` above still picks the general-purpose match every
// track gets shown with. This is the first of the two lookups tried by
// `find_validated_audio_match` right before an *audio* download starts
// (see `download.rs::run_audio_pipeline`), because plain YouTube search
// can land on a live version, a reaction video, or a video-only re-upload
// with intro chatter or extra processing baked into the audio — whereas
// YouTube Music's index skews heavily toward the official "Provided to
// YouTube by ..." / Topic-channel upload for a track, generally the
// cleanest audio source available.
//
// Pulls several candidates rather than trusting Music's #1 result
// outright — for an artist with one much more popular song, that song
// can outrank the actual query match. Each candidate is scored against
// the expected title/artist/duration (see `match_score`) and only a
// good match is used. Returns `None` — rather than a possibly-wrong
// guess — when nothing scores well enough, so the caller falls back to
// scoring plain YouTube search results the same way (see
// `find_validated_audio_match`) instead of trusting an unvalidated pick.
//
// Returns a `ScoredMatch`, not just a `YoutubeMatch` — the caller needs
// `album_confirmed` to decide whether this pick is trustworthy on its own
// or needs cross-checking (see `find_validated_audio_match`), and also
// uses the matched video's id to backfill cover art / release year (see
// `fetch_music_metadata`) when Spotify's per-track resolution failed and
// left the track with generic playlist-level data.
//
// Deliberately doesn't try the "official audio"/"official video" query
// bias that the plain-search fallback below does: Topic-channel uploads
// are auto-ingested from Content ID association, not branded "Official
// Video" content, so they essentially never carry that phrase — biasing
// this query toward it would work against the very upload this search is
// meant to find.
// The flat listing call itself is cheap; only the top `ALBUM_CHECK_TOP_N` survivors
// ever pay for a real per-candidate lookup, so a wider pool here is nearly
// free and meaningfully raises the odds of a genuine official upload
// surviving to be scored at all.
const MUSIC_SEARCH_CANDIDATES: u32 = 50;

async fn find_youtube_music_match(
    app: &AppHandle,
    title: &str,
    artist: &str,
    expected_duration: Option<u32>,
    expected_album: Option<&str>,
) -> Option<ScoredMatch> {
    let query = format!("{title} {artist}");
    let candidates = run_yt_dlp_music_search(app, &query, MUSIC_SEARCH_CANDIDATES)
        .await
        .into_iter()
        .filter_map(entry_to_match)
        .collect();
    if let Some(m) = best_scored_match(
        app,
        candidates,
        title,
        artist,
        expected_duration,
        expected_album,
    )
    .await
    {
        return Some(m);
    }

    // Second chance, only paid for when the bare query above didn't already
    // validate: quoting the title makes YouTube Music weight that exact
    // phrase much more heavily, which can surface the actual official/Topic
    // upload in cases where an artist's more popular, unrelated song (or a
    // remix/deluxe re-issue) crowded it out of the first MUSIC_SEARCH_CANDIDATES
    // results under the bare query. This keeps the pool YT-Music-sourced
    // rather than dropping straight to the noisier plain-YouTube tier.
    let quoted_query = format!("\"{title}\" {artist}");
    let candidates = run_yt_dlp_music_search(app, &quoted_query, MUSIC_SEARCH_CANDIDATES)
        .await
        .into_iter()
        .filter_map(entry_to_match)
        .collect();
    best_scored_match(
        app,
        candidates,
        title,
        artist,
        expected_duration,
        expected_album,
    )
    .await
}

// How many plain-YouTube candidates to score per query tried. Same
// reasoning as `MUSIC_SEARCH_CANDIDATES` (including the widened count):
// pull a wide pool and score them rather than trusting whichever one
// ranks first.
const SEARCH_FALLBACK_CANDIDATES: u32 = 50;

const OFFICIAL_QUERY_SUFFIXES: &[&str] = &["official audio", "official video"];

// Runs a scored plain-YouTube search once per suffix in
// `OFFICIAL_QUERY_SUFFIXES`, in order, returning the first one that
// produces a validated match — before finally trying a bare, unsuffixed
// query. Each of these is a full extra yt-dlp search in the worst case
// (nothing validates until the last, unsuffixed query), which is the
// deliberate trade: more latency on the hard-to-place tracks in exchange
// for a real shot at landing on the actual official upload instead of
// whatever a single bare query happened to rank first.
async fn search_youtube_official_first(
    app: &AppHandle,
    title: &str,
    artist: &str,
    expected_duration: Option<u32>,
    expected_album: Option<&str>,
) -> Option<ScoredMatch> {
    for suffix in OFFICIAL_QUERY_SUFFIXES {
        let query = format!("{title} {artist} {suffix}");
        let candidates = search_youtube_many(app, &query, SEARCH_FALLBACK_CANDIDATES).await;
        if let Some(m) = best_scored_match(
            app,
            candidates,
            title,
            artist,
            expected_duration,
            expected_album,
        )
        .await
        {
            return Some(m);
        }
    }

    let query = format!("{title} {artist}");
    let candidates = search_youtube_many(app, &query, SEARCH_FALLBACK_CANDIDATES).await;
    best_scored_match(
        app,
        candidates,
        title,
        artist,
        expected_duration,
        expected_album,
    )
    .await
}

// The single source of truth for "what audio does this download actually
// use" — called from `download.rs::run_audio_pipeline` in place of the
// track's analyze-time `preview_url`.
//
// Tries YouTube Music first (`find_youtube_music_match`) since its
// index leans toward clean official uploads. If nothing there is
// album-confirmed, falls back to plain YouTube search — official-audio/
// official-video biased first, then bare (see
// `search_youtube_official_first`) — but, critically, scores every
// candidate the exact same way (title/artist/duration via `match_score`)
// instead of trusting a search result's rank outright. That's the
// difference from `search_youtube`/`search_youtube_raw`, which power the
// track's `preview_url` shown during analyze: those never run
// `match_score` at all, just take ytsearch1's #1 result, so a
// `preview_url` can already be a live version, a reaction video, or an
// unrelated song by the same artist. Using it as a silent fallback here is
// exactly what let a download tag the wrong audio with this track's
// metadata.
//
// Returns `None` when nothing on either source clears the bar, so the
// caller can fail the download outright — a "track not found" is a
// better outcome than a track that plays the wrong song.
//
// `expected_album` is the track's real album/single name from its actual
// source metadata (Spotify), passed straight through to the
// album tie-break in `best_scored_match` — pass an empty string when it's
// unknown, matching how the rest of this codebase treats an unset album
// (which is always the case for a track sourced directly from a YouTube
// playlist rather than Spotify). See the branch below for why an
// *unconfirmed* album check is treated the same as no album at all.
pub async fn find_validated_audio_match(
    app: &AppHandle,
    title: &str,
    artist: &str,
    expected_duration: Option<u32>,
    expected_album: &str,
) -> Option<YoutubeMatch> {
    let expected_album = Some(expected_album).filter(|a| !a.trim().is_empty());
    let music_match =
        find_youtube_music_match(app, title, artist, expected_duration, expected_album).await;

    if let Some(m) = &music_match {
        if m.album_confirmed {
            eprintln!(
                "[Match] FINAL PICK (YouTube Music tier, album confirmed): \"{}\" — {}",
                m.result.title.as_deref().unwrap_or("?"),
                m.result.url
            );
            return Some(m.result.clone());
        }
    }

    // Thin-evidence case: either there's no expected_album at all, or
    // there is one but nothing actually confirmed a match against it. A
    // YouTube Music flat search entry never carries its own duration or
    // uploader (both confirmed always null on that tier), so an
    // unconfirmed Music-tier pick here is vouched for by exactly one
    // `fetch_full_video_info` call and nothing else independently
    // corroborating it — title, artist, and duration can all check out on
    // a video that's genuinely a different recording under a copied title
    // (this is what let a "remake" through for a YouTube-playlist-sourced
    // track with no Spotify album to catch it: title matched, artist
    // matched, even duration matched, on a video that just wasn't the
    // original). So here, always also run the plain-YouTube tier — which
    // resolves with real, independently flat-confirmed duration+uploader
    // data straight off the search result, before any enrichment call —
    // and prefer whichever side actually has that stronger evidence. This
    // doubles the yt-dlp work for every track that isn't album-confirmed;
    // deliberate, since this specific gap is exactly what let a
    // wrong-audio pick through with every other check passing.
    if music_match.is_none() {
        eprintln!("[Match] YouTube Music tier produced nothing, trying plain YouTube search");
    } else {
        eprintln!(
            "[Match] no confirmed album match — cross-checking plain YouTube search before committing to the Music-tier pick"
        );
    }
    let plain_match =
        search_youtube_official_first(app, title, artist, expected_duration, expected_album).await;

    // A plain-tier album-confirmed match is just as strong as the
    // Music-tier fast path above, and `youtube.com` (unlike
    // `music.youtube.com`) DOES expose an `album` field — so this is
    // where a genuine album confirmation actually has a real chance to
    // fire for a track that reached this point.
    if let Some(m) = &plain_match {
        if m.album_confirmed {
            eprintln!(
                "[Match] FINAL PICK (plain YouTube tier, album confirmed): \"{}\" — {}",
                m.result.title.as_deref().unwrap_or("?"),
                m.result.url
            );
            return Some(m.result.clone());
        }
    }

    let plain_is_corroborated = plain_match.as_ref().is_some_and(|m| {
        m.result.duration.is_some()
            && m.result
                .uploader
                .as_deref()
                .is_some_and(|u| !u.trim().is_empty())
    });
    let picked_plain = plain_is_corroborated;
    let chosen = if plain_is_corroborated {
        plain_match.map(|m| m.result)
    } else {
        music_match
            .map(|m| m.result)
            .or(plain_match.map(|m| m.result))
    };

    match &chosen {
        Some(m) => eprintln!(
            "[Match] FINAL PICK ({} tier, no confirmed album): \"{}\" — {}",
            if picked_plain {
                "plain YouTube"
            } else {
                "YouTube Music"
            },
            m.title.as_deref().unwrap_or("?"),
            m.url
        ),
        None => eprintln!("[Match] nothing validated on either tier"),
    }
    chosen
}

const FALLBACK_COVER: &str = "";

#[derive(Deserialize)]
struct YtDlpFullEntry {
    release_year: Option<i64>,
    release_date: Option<String>,
    album: Option<String>,
    artist: Option<String>,
    thumbnail: Option<String>,
    thumbnails: Option<Vec<YtThumb>>,
    duration: Option<f64>,
    title: Option<String>,
    uploader: Option<String>,
}

async fn fetch_full_video_info(app: &AppHandle, video_id: &str) -> Option<YtDlpFullEntry> {
    let sidecar = app.shell().sidecar("sonic-yt-dlp").ok()?;
    let url = format!("https://www.youtube.com/watch?v={video_id}");
    let mut args = vec![
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--extractor-args".to_string(),
        "youtube:player_client=web, android".to_string(),
    ];
    args.extend(cookie_args());
    args.push(url);

    let output = sidecar.args(args).output().await.ok()?;
    if !output.status.success() {
        eprintln!(
            "[YouTube] fetch_full_video_info failed for {video_id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| serde_json::from_str::<YtDlpFullEntry>(line).ok())
}

pub struct MusicMetadata {
    pub cover_url: Option<String>,
    pub year: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
}

pub async fn fetch_music_metadata(app: &AppHandle, video_id: &str) -> Option<MusicMetadata> {
    let entry = fetch_full_video_info(app, video_id).await?;
    Some(MusicMetadata {
        cover_url: pick_best_thumbnail(&entry.thumbnails, &entry.thumbnail),
        year: year_from_full_entry(&entry),
        album: entry.album.clone(),
        album_artist: entry.artist.clone(),
    })
}

fn year_from_full_entry(entry: &YtDlpFullEntry) -> Option<String> {
    if let Some(y) = entry.release_year {
        return Some(y.to_string());
    }
    entry
        .release_date
        .as_ref()
        .filter(|d| d.len() >= 4)
        .map(|d| d[..4].to_string())
}

const YOUTUBE_ENRICH_CONCURRENCY: usize = 6;

// Backfills release year (and album, when available) for tracks resolved straight
// from YouTube — i.e. when there's no Spotify metadata to draw on at all, such as a
// pasted YouTube playlist link. Needs one extra, non-flat yt-dlp call per track, so
// it's only worth paying for on tracks that are actually going to be used — not
// disposable search-result candidates (see `direct_search_tracks`, which skips this).
//
// Emits the second half of the combined analyze-progress bar (see
// `run_yt_dlp_streaming`'s `has_second_phase`): listing filled 0–N, this fills N–2N,
// so the bar keeps moving smoothly instead of freezing at "N of N" while this runs.
async fn enrich_tracks_with_youtube_metadata(app: &AppHandle, tracks: &mut [Track]) {
    let listing_total = tracks.len() as u32;
    let overall_total = (listing_total * 2).max(1);
    let semaphore = Arc::new(Semaphore::new(YOUTUBE_ENRICH_CONCURRENCY));
    let mut handles = Vec::with_capacity(tracks.len());
    for (i, track) in tracks.iter().enumerate() {
        let video_id = track.id.clone();
        let sem = semaphore.clone();
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            let info = fetch_full_video_info(&app, &video_id).await;
            (i, info)
        }));
    }

    let mut enrich_completed: u32 = 0;
    for handle in handles {
        if let Ok((i, Some(info))) = handle.await {
            if let Some(track) = tracks.get_mut(i) {
                if track.year.is_empty() {
                    if let Some(year) = year_from_full_entry(&info) {
                        track.year = year;
                    }
                }
                if let Some(album) = info.album.filter(|a| !a.trim().is_empty()) {
                    track.album = album;
                }
                if track.album_artist.is_none() {
                    if let Some(artist) = info.artist.filter(|a| !a.trim().is_empty()) {
                        track.album_artist = Some(artist);
                    }
                }

                if let Some(cover) = pick_best_thumbnail(&info.thumbnails, &info.thumbnail) {
                    track.cover_url = cover;
                }
            }
        }

        enrich_completed += 1;
        let _ = app.emit(
            "analyze-progress",
            AnalyzeProgress {
                completed: listing_total + enrich_completed,
                total: overall_total,
            },
        );
    }
}

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
            album: String::new(),
            album_artist: None,
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

// Multi-result direct search. Returns up to `DIRECT_SEARCH_LIMIT` tracks.
pub async fn direct_search_tracks(app: &AppHandle, query: &str) -> Vec<Track> {
    let entries = run_yt_dlp_search_streaming(app, query, DIRECT_SEARCH_LIMIT).await;
    entries_to_tracks(entries, query)
}

pub async fn resolve_youtube_url(app: &AppHandle, url: &str) -> Vec<Track> {
    let Some(video_id) = extract_youtube_id(url) else {
        return Vec::new();
    };
    let canonical = format!("https://www.youtube.com/watch?v={video_id}");
    let entries = run_yt_dlp_direct(app, &canonical).await;
    let mut tracks = entries_to_tracks(entries, url);
    enrich_tracks_with_youtube_metadata(app, &mut tracks).await;
    tracks
}

pub async fn resolve_youtube_playlist(app: &AppHandle, url: &str) -> (String, bool, Vec<Track>) {
    let Some(playlist_id) = extract_youtube_playlist_id(url) else {
        return (url.to_string(), false, Vec::new());
    };
    let canonical = format!("https://www.youtube.com/playlist?list={playlist_id}");
    let entries = run_yt_dlp_direct_streaming(app, &canonical).await;

    let is_album = playlist_id.starts_with("OLAK5uy");
    let raw_title = entries
        .first()
        .and_then(|e| e.playlist_title.clone())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "YouTube Playlist".to_string());
    let playlist_name = raw_title
        .strip_prefix("Album - ")
        .map(str::to_string)
        .unwrap_or(raw_title);

    let mut tracks = entries_to_tracks(entries, &playlist_name);
    enrich_tracks_with_youtube_metadata(app, &mut tracks).await;
    (playlist_name, is_album, tracks)
}

pub async fn enrich_track(
    app: &AppHandle,
    item: &ScrapedTrackItem,
    id: String,
    track_number: u32,
    total_tracks: u32,
) -> Track {
    let album = item.album.clone().unwrap_or_default();
    let album_artist = item.album_artist.clone();
    let year = item.release_year.clone().unwrap_or_default();

    match search_youtube(app, &item.title, &item.artist).await {
        Some(m) => Track {
            id,
            title: item.title.clone(),
            artist: item.artist.clone(),
            album,
            album_artist,
            year,
            track_number,
            total_tracks,
            duration: item.duration.or(m.duration).unwrap_or(180),
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
            album_artist,
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
