use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::fs;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::cancel::DownloadRegistry;
use crate::error::{AppError, AppResult};
use crate::logger;
use crate::models::Track;
use crate::settings;
use crate::youtube;

const MAX_STDERR_BYTES: usize = 256 * 1024;
const MAX_COVER_BYTES: usize = 10 * 1024 * 1024;
const CROP_COVERS_TO_SQUARE: bool = true;
const COVER_SQUARE_TOLERANCE: f64 = 0.10;
const MAX_COMPONENT_BYTES: usize = 200;

#[derive(Clone, Copy)]
enum StreamKind {
    Audio,
    Video,
}

const AUDIO_CLIENT_CHAIN: &[Option<&str>] = &[None, Some("android,web"), Some("tv")];
const VIDEO_CLIENT_CHAIN: &[Option<&str>] = &[None, Some("web_embedded,tv"), Some("tv")];
const RETRY_ON_BOT_CHALLENGE: bool = true;

// Last client that produced a download, per stream kind (`Some(None)` = the
// yt-dlp default worked). Next download tries it first. Process-lifetime only.
static LAST_GOOD_AUDIO_CLIENT: Mutex<Option<Option<String>>> = Mutex::new(None);
static LAST_GOOD_VIDEO_CLIENT: Mutex<Option<Option<String>>> = Mutex::new(None);

impl StreamKind {
    fn defaults(self) -> &'static [Option<&'static str>] {
        match self {
            StreamKind::Audio => AUDIO_CLIENT_CHAIN,
            StreamKind::Video => VIDEO_CLIENT_CHAIN,
        }
    }
    fn last_good(self) -> &'static Mutex<Option<Option<String>>> {
        match self {
            StreamKind::Audio => &LAST_GOOD_AUDIO_CLIENT,
            StreamKind::Video => &LAST_GOOD_VIDEO_CLIENT,
        }
    }
    fn stem(self) -> &'static str {
        match self {
            StreamKind::Audio => "audio",
            StreamKind::Video => "video",
        }
    }
}

fn parse_client_spec(spec: &str) -> Vec<Option<String>> {
    let mut chain: Vec<Option<String>> = Vec::new();
    for entry in spec.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let parsed = if entry.eq_ignore_ascii_case("default") {
            Some(None)
        } else if entry
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ',' | '-' | '+'))
        {
            Some(Some(entry.to_string()))
        } else {
            logger::warn(format!("Ignoring invalid yt-dlp client entry \"{entry}\""));
            None
        };
        if let Some(client) = parsed {
            if !chain.contains(&client) {
                chain.push(client);
            }
        }
    }
    chain
}

// Moves the last known good client (if it is part of the chain) to the front.
fn order_chain(
    mut chain: Vec<Option<String>>,
    last_good: Option<Option<String>>,
) -> Vec<Option<String>> {
    if let Some(good) = last_good {
        if let Some(pos) = chain.iter().position(|c| *c == good) {
            let client = chain.remove(pos);
            chain.insert(0, client);
        }
    }
    chain
}

// Override (if valid) else the built-in chain, reordered by the session cache.
fn build_client_chain(kind: StreamKind, spec: Option<&str>) -> Vec<Option<String>> {
    let mut chain = spec.map(parse_client_spec).unwrap_or_default();
    if chain.is_empty() {
        chain = kind
            .defaults()
            .iter()
            .map(|c| c.map(str::to_string))
            .collect();
    }
    let last_good = kind
        .last_good()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    order_chain(chain, last_good)
}

fn remember_good_client(kind: StreamKind, client: &Option<String>) {
    *kind
        .last_good()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(client.clone());
}

// Only failures that a different client could plausibly fix are retried.
// Cancellation, ffmpeg/merge problems, bad URLs, etc. are returned at once.
fn is_client_dependent(e: &AppError) -> bool {
    match e {
        AppError::Forbidden => true,
        AppError::YoutubeBotDetected => RETRY_ON_BOT_CHALLENGE,
        AppError::YtDlpFailed(msg) => {
            let m = error_lines(msg).to_lowercase();
            [
                "requested format is not available",
                "http error",
                "unable to extract",
                "no video formats",
                "nsig",
                "player response",
                "did not get any data blocks",
                "unable to download",
            ]
            .iter()
            .any(|marker| m.contains(marker))
        }
        _ => false,
    }
}

fn append_player_client_arg(args: &mut Vec<String>, client: Option<&str>) {
    if let Some(c) = client {
        args.push("--extractor-args".into());
        args.push(format!("youtube:player_client={c}"));
    }
}

async fn clear_attempt_files(work_dir: &Path, stem: &str) {
    let prefix = format!("{stem}.");
    if let Ok(mut entries) = fs::read_dir(work_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(entry.path()).await;
            }
        }
    }
}

#[derive(Default)]
struct OutputNames(Mutex<HashSet<String>>);

impl OutputNames {
    fn claim(&self, dir: &Path, filename: &str) -> PathBuf {
        let mut claimed = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let first = dir.join(filename);
        if claimed.insert(first.to_string_lossy().to_lowercase()) {
            return first;
        }

        let as_path = Path::new(filename);
        let stem = as_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);
        let ext = as_path.extension().and_then(|e| e.to_str());
        let mut n: u32 = 2;
        loop {
            let name = match ext {
                Some(ext) => format!("{stem} ({n}).{ext}"),
                None => format!("{stem} ({n})"),
            };
            let candidate = dir.join(name);
            if claimed.insert(candidate.to_string_lossy().to_lowercase()) {
                return candidate;
            }
            n += 1;
        }
    }
}

// `/music/Song.mp3` -> `/music/.Song.partial.mp3`. The real extension is kept
// at the end because ffmpeg picks the muxer from it.
fn partial_path(out_path: &Path) -> PathBuf {
    let stem = out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = out_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    out_path.with_file_name(format!(".{stem}.partial.{ext}"))
}

async fn finalize_output(partial: &Path, final_path: &Path) -> AppResult<()> {
    if let Err(e) = fs::rename(partial, final_path).await {
        let _ = fs::remove_file(partial).await;
        return Err(AppError::from(e));
    }
    Ok(())
}

fn validate_media_url(url: &str) -> AppResult<()> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|e| AppError::Other(format!("Invalid media URL: {e}")))?;
    let host_ok = match parsed.host_str() {
        Some(h) => h == "youtu.be" || h == "youtube.com" || h.ends_with(".youtube.com"),
        None => false,
    };
    if matches!(parsed.scheme(), "http" | "https") && host_ok {
        Ok(())
    } else {
        Err(AppError::Other(
            "Unsupported media URL (expected a YouTube link).".to_string(),
        ))
    }
}

fn parse_height_cap(quality: Option<&str>) -> Option<u32> {
    let q = quality?.trim().to_lowercase();
    if q.is_empty() || q == "best" {
        return None;
    }
    match q.as_str() {
        "8k" => return Some(4320),
        "4k" => return Some(2160),
        "2k" => return Some(1440),
        _ => {}
    }
    let digits: String = q.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok().filter(|h| *h >= 144)
}

// Builds the yt-dlp `-f` selector. The old selector ignored the container, so
// choosing WEBM could grab H.264/AAC (un-mergeable into webm) and MP4 could
// grab VP9/Opus. Now we first ask for streams that fit the container, then
// fall back to "anything capped", and finally to "anything" so an unsatisfiable
// quality cap degrades to a download instead of "Requested format is not
// available".
fn video_format_selector(container: &str, height_cap: Option<u32>) -> String {
    let (v_ext, a_ext) = match container {
        "webm" => ("[ext=webm]", "[ext=webm]"),
        "mp4" => ("[ext=mp4]", "[ext=m4a]"),
        _ => ("", ""),
    };
    let cap = height_cap
        .map(|h| format!("[height<={h}]"))
        .unwrap_or_default();
    format!("bestvideo{v_ext}{cap}+bestaudio{a_ext}/bestvideo{cap}+bestaudio/best{cap}/best")
}

fn parse_sample_rate_hz(s: &str) -> Option<u32> {
    let lower = s.trim().to_lowercase();
    let body = lower.trim_end_matches("hz").trim();
    if let Some(k) = body.strip_suffix('k') {
        k.trim()
            .parse::<f64>()
            .ok()
            .map(|v| (v * 1000.0).round() as u32)
    } else {
        body.parse::<f64>().ok().map(|v| v.round() as u32)
    }
}

fn track_tag_value(track: &Track) -> Option<String> {
    if track.track_number == 0 {
        return None;
    }
    Some(if track.total_tracks > 0 {
        format!("{}/{}", track.track_number, track.total_tracks)
    } else {
        track.track_number.to_string()
    })
}

fn looks_like_netscape_cookies(raw: &str) -> bool {
    raw.lines().any(|l| {
        let l = l.trim_end_matches('\r');
        !l.trim().is_empty()
            && (!l.starts_with('#') || l.starts_with("#HttpOnly_"))
            && l.split('\t').count() >= 7
    })
}

fn truncate_to_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn is_windows_reserved_name(s: &str) -> bool {
    let base = s.split('.').next().unwrap_or("").trim_end().to_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.ends_with(|c: char| c.is_ascii_digit()))
}

fn clean_path_component(name: &str) -> Option<String> {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();

    let trimmed = replaced.trim().trim_end_matches(['.', ' ']);
    let truncated = truncate_to_bytes(trimmed, MAX_COMPONENT_BYTES).trim_end_matches(['.', ' ']);
    if truncated.is_empty() || is_windows_reserved_name(truncated) {
        None
    } else {
        Some(truncated.to_string())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackProgressPayload {
    track_id: String,
    phase: &'static str,
    percent: u8,
    stream: &'static str,
}

fn parse_yt_dlp_percent(line: &str) -> Option<u8> {
    let line = line.trim();
    if !line.starts_with("[download]") {
        return None;
    }
    line.split_whitespace()
        .find(|tok| tok.ends_with('%'))
        .and_then(|tok| tok.trim_end_matches('%').parse::<f64>().ok())
        .map(|pct| pct.clamp(0.0, 100.0).round() as u8)
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub format: String,
    pub bitrate: String,
    pub youtube_cookies: Option<String>,
    pub cookies_from_browser: Option<String>,
    pub sample_rate: Option<String>,
    pub video_quality: Option<String>,
    pub ytdlp_clients: Option<String>,
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
}

fn resolve_naming_template(pattern: &str) -> &str {
    match pattern {
        "number_artist_title" => "{trackNumber} - {artist} - {title}",
        "number_title" => "{trackNumber} - {title}",
        "artist_title" => "{artist} - {title}",
        "title_artist" => "{title} - {artist}",
        "title" => "{title}",
        other => other,
    }
}

fn resolve_folder_naming_template(pattern: &str) -> &str {
    match pattern {
        "year_album" => "{year} - {album}",
        "album" => "{album}",
        _ => "{album} - {artist}",
    }
}

fn render_folder_name(
    pattern: &str,
    album: &str,
    artist: &str,
    year: &str,
    default: &str,
) -> String {
    let template = resolve_folder_naming_template(pattern);

    let rendered = template
        .replace("{album}", album.trim())
        .replace("{artist}", artist.trim())
        .replace("{year}", year.trim());

    let mut clean_rendered = rendered;
    loop {
        let trimmed_once = clean_rendered.trim();
        let stripped = trimmed_once
            .trim_start_matches('-')
            .trim_start()
            .trim_end_matches('-')
            .trim_end();
        if stripped == trimmed_once {
            clean_rendered = trimmed_once.to_string();
            break;
        }
        clean_rendered = stripped.to_string();
    }

    if clean_rendered.is_empty() {
        sanitize_path_component(default, "Unknown Album")
    } else {
        sanitize_path_component(&clean_rendered, default)
    }
}

fn render_filename(pattern: &str, track: &Track, extension: &str) -> String {
    let template = resolve_naming_template(pattern);
    let track_number = format!("{:02}", track.track_number);
    let rendered = template
        .replace("{artist}", &track.artist)
        .replace("{title}", &track.title)
        .replace("{album}", &track.album)
        .replace("{year}", &track.year)
        .replace("{trackNumber}", &track_number);

    let fallback = format!("{} - {}", track.artist, track.title);
    let base = sanitize_path_component(&rendered, &fallback);
    format!("{base}.{extension}")
}

fn codec_and_extension(format: &str) -> (&'static str, &'static str) {
    match format.to_lowercase().as_str() {
        "flac" => ("flac", "flac"),
        "wav" => ("pcm_s16le", "wav"),
        "m4a" | "aac" => ("aac", "m4a"),
        "opus" => ("libopus", "opus"),
        _ => ("libmp3lame", "mp3"),
    }
}

// The two audio families YouTube actually serves: Opus (WebM, itag 251) and
// AAC (M4A, itag 140/141). mp3/flac/wav have no native stream, so choosing
// them always means a transcode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NativeCodec {
    Opus,
    Aac,
}

// The native stream an output format can hold WITHOUT re-encoding, if any.
fn native_codec_for_format(format: &str) -> Option<NativeCodec> {
    match format.to_lowercase().as_str() {
        "opus" => Some(NativeCodec::Opus),
        "m4a" | "aac" => Some(NativeCodec::Aac),
        _ => None,
    }
}

fn native_codec_of_file(path: &Path) -> Option<NativeCodec> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "webm" | "opus" | "ogg" => Some(NativeCodec::Opus),
        "m4a" | "aac" => Some(NativeCodec::Aac),
        _ => None,
    }
}

// yt-dlp `(-f selector, -S sort)` for the audio download. With a native
// target we ask for that codec first (highest bitrate), keeping the generic
// fallbacks so a track is never lost just because the preferred stream is
// missing; `run_audio_pipeline` then notices the mismatch and transcodes.
// Transcode targets keep the original m4a-first selection.
fn audio_format_args(target: Option<NativeCodec>) -> (&'static str, &'static str) {
    match target {
        Some(NativeCodec::Opus) => (
            "bestaudio[acodec=opus]/bestaudio[ext=webm]/bestaudio/bestaudio*/best",
            "abr",
        ),
        Some(NativeCodec::Aac) => (
            "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio/bestaudio*/best",
            "abr",
        ),
        None => (
            "bestaudio[ext=m4a]/bestaudio[ext=webm]/bestaudio/bestaudio*/best",
            "aext:m4a:webm,abr",
        ),
    }
}

fn is_video_format(format: &str) -> bool {
    matches!(
        format.to_lowercase().as_str(),
        "mp4" | "webm" | "mkv" | "video"
    )
}

fn video_container_extension(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "webm" => "webm",
        "mkv" => "mkv",
        _ => "mp4",
    }
}

fn is_forbidden(stderr: &str) -> bool {
    let s = error_lines(stderr).to_lowercase();
    s.contains("http error 403") || s.contains("403: forbidden")
}

fn error_lines(stderr: &str) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .filter(|l| l.trim_start().to_ascii_lowercase().starts_with("error:"))
        .collect();
    if errors.is_empty() {
        stderr.to_string()
    } else {
        errors.join("\n")
    }
}

// `https://music.youtube.com/watch?v=ID[&list=..]` -> `https://www.youtube.com/watch?v=ID`
// Same video id, same audio, but on the music host yt-dlp selects the `web_music` client,
// whose HTTPS formats need a GVS PO token we never provide
// so downloads started on a client that was bound to fail or
// to fall back. Anything that is not a plain music watch URL is left alone.
fn canonical_watch_url(url: &str) -> String {
    let Some(rest) = url
        .strip_prefix("https://music.youtube.com/")
        .or_else(|| url.strip_prefix("http://music.youtube.com/"))
    else {
        return url.to_string();
    };
    let Some((path, query)) = rest.split_once('?') else {
        return url.to_string();
    };
    if path != "watch" {
        return url.to_string();
    }
    let id = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("v="))
        .filter(|id| {
            id.len() == 11
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });
    match id {
        Some(id) => format!("https://www.youtube.com/watch?v={id}"),
        None => url.to_string(),
    }
}

fn is_bot_detected(stderr: &str) -> bool {
    error_lines(stderr).to_lowercase().contains("not a bot")
}

fn deno_sidecar_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        "sonic-deno.exe"
    } else {
        "sonic-deno"
    };
    let path = exe_dir.join(name);
    path.exists().then_some(path)
}

fn append_js_runtime_arg(args: &mut Vec<String>) {
    if let Some(deno_path) = deno_sidecar_path() {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", deno_path.to_string_lossy()));
    } else {
        logger::warn(
            "sonic-deno sidecar not found — yt-dlp will fall back to any system JS runtime, if present"
                .to_string(),
        );
    }
}

fn ffmpeg_sidecar_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        "sonic-ffmpeg.exe"
    } else {
        "sonic-ffmpeg"
    };
    let path = exe_dir.join(name);
    path.exists().then_some(path)
}

fn append_ffmpeg_location_arg(args: &mut Vec<String>) {
    if let Some(ffmpeg_path) = ffmpeg_sidecar_path() {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg_path.to_string_lossy().into_owned());
    } else {
        logger::warn(
            "sonic-ffmpeg sidecar not found — yt-dlp will not be able to merge video and audio"
                .to_string(),
        );
    }
}

async fn run_yt_dlp_streaming(
    app: &AppHandle,
    track_id: &str,
    cmd: tauri_plugin_shell::process::Command,
    stream_labels: &'static [&'static str],
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<(String, Option<i32>)> {
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    let (mut rx, child) = cmd
        .spawn()
        .map_err(|e| AppError::YtDlpFailed(format!("failed to spawn yt-dlp sidecar: {e}")))?;
    registry.set_child(track_id, child);

    let mut stderr = String::new();
    let mut exit_code: Option<i32> = None;
    let mut legs_started: usize = 0;
    let mut emitted_transcoding = false;

    loop {
        let event = tokio::select! {
            _ = token.cancelled() => {
                registry.clear_child(track_id);
                return Err(AppError::Cancelled);
            }
            event = rx.recv() => match event {
                Some(event) => event,
                None => break,
            },
        };
        match event {
            CommandEvent::Stdout(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                let trimmed = line.trim();

                if trimmed.starts_with("[download] Destination:") {
                    legs_started += 1;
                    if legs_started > 1 && !emitted_transcoding {
                        emitted_transcoding = true;
                        let _ = app.emit(
                            "track-progress",
                            TrackProgressPayload {
                                track_id: track_id.to_string(),
                                phase: "transcoding",
                                percent: 0,
                                stream: "audio",
                            },
                        );
                    }
                    continue;
                }

                if legs_started <= 1 {
                    if let Some(raw_percent) = parse_yt_dlp_percent(&line) {
                        let stream = stream_labels.first().copied().unwrap_or("audio");
                        let _ = app.emit(
                            "track-progress",
                            TrackProgressPayload {
                                track_id: track_id.to_string(),
                                phase: "downloading",
                                percent: raw_percent,
                                stream,
                            },
                        );
                    }
                }
            }
            CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    logger::proc("yt-dlp:err", trimmed);
                }

                if stderr.len() < MAX_STDERR_BYTES {
                    stderr.push_str(&line);
                    stderr.push('\n');
                }
            }
            CommandEvent::Error(e) => {
                registry.clear_child(track_id);
                logger::error(format!("yt-dlp process error: {e}"));
                return Err(AppError::YtDlpFailed(format!("yt-dlp process error: {e}")));
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
            }
            _ => {}
        }
    }

    registry.clear_child(track_id);
    Ok((stderr, exit_code))
}

#[derive(Clone, Copy)]
struct CookieAuth<'a> {
    cookies_path: Option<&'a str>,
    cookies_from_browser: Option<&'a str>,
}

impl CookieAuth<'_> {
    fn append_to(&self, args: &mut Vec<String>) {
        if let Some(path) = self.cookies_path {
            args.push("--cookies".into());
            args.push(path.to_string());
        } else if let Some(browser) = self.cookies_from_browser {
            args.push("--cookies-from-browser".into());
            args.push(browser.to_string());
        }
    }
}

async fn download_audio(
    app: &AppHandle,
    track_id: &str,
    video_url: &str,
    work_dir: &Path,
    client_spec: Option<&str>,
    target: Option<NativeCodec>,
    cookies: CookieAuth<'_>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    let chain = build_client_chain(StreamKind::Audio, client_spec);
    for (i, client) in chain.iter().enumerate() {
        if token.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if i > 0 {
            clear_attempt_files(work_dir, StreamKind::Audio.stem()).await;
        }
        let result = download_audio_once(
            app,
            track_id,
            video_url,
            work_dir,
            client.as_deref(),
            target,
            cookies,
            registry,
            token,
        )
        .await;
        match result {
            Ok(path) => {
                remember_good_client(StreamKind::Audio, client);
                return Ok(path);
            }
            Err(e) if is_client_dependent(&e) && i + 1 < chain.len() => {
                logger::warn(format!(
                    "yt-dlp audio attempt with client {client:?} failed ({e}); trying next client"
                ));
            }
            Err(e) => return Err(e),
        }
    }
    Err(AppError::Other(
        "no yt-dlp client attempts were made".to_string(),
    ))
}

async fn download_audio_once(
    app: &AppHandle,
    track_id: &str,
    video_url: &str,
    work_dir: &Path,
    client: Option<&str>,
    target: Option<NativeCodec>,
    cookies: CookieAuth<'_>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    let (format_selector, format_sort) = audio_format_args(target);
    let out_template = work_dir.join("audio.%(ext)s");
    let mut cmd = app
        .shell()
        .sidecar("sonic-yt-dlp")
        .map_err(|e| AppError::YtDlpFailed(format!("failed to resolve yt-dlp sidecar: {e}")))?;

    let mut args: Vec<String> = vec![
        "-f".into(),
        format_selector.into(),
        "-S".into(),
        format_sort.into(),
        "-N".into(),
        "8".into(),
        "--newline".into(),
        "--no-playlist".into(),
        "--no-mtime".into(),
        "-o".into(),
        out_template.to_string_lossy().into_owned(),
    ];

    append_player_client_arg(&mut args, client);
    append_js_runtime_arg(&mut args);
    cookies.append_to(&mut args);

    args.push("--".into());
    args.push(video_url.to_string());
    cmd = cmd.args(args);

    let (stderr, exit_code) =
        run_yt_dlp_streaming(app, track_id, cmd, &["audio"], registry, token).await?;

    if exit_code != Some(0) {
        logger::error(format!(
            "yt-dlp exited with {exit_code:?} for track {track_id}"
        ));
        if is_bot_detected(&stderr) {
            return Err(AppError::YoutubeBotDetected);
        }
        if is_forbidden(&stderr) {
            return Err(AppError::Forbidden);
        }
        return Err(AppError::YtDlpFailed(stderr));
    }

    let mut entries = fs::read_dir(work_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some("audio") {
            return Ok(path);
        }
    }
    Err(AppError::YtDlpFailed(
        "yt-dlp reported success but no audio file was produced".to_string(),
    ))
}

async fn download_video(
    app: &AppHandle,
    track_id: &str,
    video_url: &str,
    work_dir: &Path,
    container: &str,
    quality: Option<&str>,
    client_spec: Option<&str>,
    cookies: CookieAuth<'_>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    validate_media_url(video_url)?;

    let chain = build_client_chain(StreamKind::Video, client_spec);
    for (i, client) in chain.iter().enumerate() {
        if token.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if i > 0 {
            clear_attempt_files(work_dir, StreamKind::Video.stem()).await;
        }
        let result = download_video_once(
            app,
            track_id,
            video_url,
            work_dir,
            container,
            quality,
            client.as_deref(),
            cookies,
            registry,
            token,
        )
        .await;
        match result {
            Ok(path) => {
                remember_good_client(StreamKind::Video, client);
                return Ok(path);
            }
            Err(e) if is_client_dependent(&e) && i + 1 < chain.len() => {
                logger::warn(format!(
                    "yt-dlp video attempt with client {client:?} failed ({e}); trying next client"
                ));
            }
            Err(e) => return Err(e),
        }
    }
    Err(AppError::Other(
        "no yt-dlp client attempts were made".to_string(),
    ))
}

async fn download_video_once(
    app: &AppHandle,
    track_id: &str,
    video_url: &str,
    work_dir: &Path,
    container: &str,
    quality: Option<&str>,
    client: Option<&str>,
    cookies: CookieAuth<'_>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    let out_template = work_dir.join("video.%(ext)s");

    let mut cmd = app
        .shell()
        .sidecar("sonic-yt-dlp")
        .map_err(|e| AppError::YtDlpFailed(format!("failed to resolve yt-dlp sidecar: {e}")))?;

    let height_cap = parse_height_cap(quality);
    let format_selector = video_format_selector(container, height_cap);

    let mut args: Vec<String> = vec![
        "-f".into(),
        format_selector,
        "--merge-output-format".into(),
        container.to_string(),
        "-N".into(),
        "8".into(),
        "--newline".into(),
        "--no-playlist".into(),
        "--no-mtime".into(),
        "-o".into(),
        out_template.to_string_lossy().into_owned(),
    ];

    append_player_client_arg(&mut args, client);
    append_js_runtime_arg(&mut args);
    append_ffmpeg_location_arg(&mut args);
    cookies.append_to(&mut args);

    args.push("--".into());
    args.push(video_url.to_string());
    cmd = cmd.args(args);

    let (stderr, exit_code) =
        run_yt_dlp_streaming(app, track_id, cmd, &["video", "audio"], registry, token).await?;

    if exit_code != Some(0) {
        if is_bot_detected(&stderr) {
            return Err(AppError::YoutubeBotDetected);
        }
        if is_forbidden(&stderr) {
            return Err(AppError::Forbidden);
        }
        return Err(AppError::YtDlpFailed(stderr));
    }

    if stderr.to_lowercase().contains("ffmpeg is not installed") {
        return Err(AppError::YtDlpFailed(
            "yt-dlp could not find ffmpeg, so video and audio streams were not merged".to_string(),
        ));
    }

    let mut unmerged: Vec<String> = Vec::new();
    let mut entries = fs::read_dir(work_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == "video" {
            return Ok(path);
        }
        if stem.starts_with("video.f") {
            unmerged.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Err(AppError::YtDlpFailed(if unmerged.is_empty() {
        "yt-dlp reported success but no video file was produced".to_string()
    } else {
        format!(
            "yt-dlp left unmerged streams ({}); ffmpeg merge did not run",
            unmerged.join(", ")
        )
    }))
}

async fn download_cover(cover_url: &str, work_dir: &Path) -> Option<PathBuf> {
    if cover_url.is_empty() {
        return None;
    }
    let client = crate::http::client();
    let res = client
        .get(cover_url)
        .header(reqwest::header::ACCEPT, "image/*,*/*;q=0.8")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    if res
        .content_length()
        .is_some_and(|len| len > MAX_COVER_BYTES as u64)
    {
        return None;
    }
    let header_ext = match res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
    {
        ct if ct.contains("png") => "png",
        ct if ct.contains("webp") => "webp",
        ct if ct.contains("gif") => "gif",
        _ => "jpg",
    };

    let mut res = res;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.ok()? {
        if bytes.len() + chunk.len() > MAX_COVER_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }

    let ext = sniff_image_ext(&bytes).unwrap_or(header_ext);
    let path = work_dir.join(format!("cover.{ext}"));
    fs::write(&path, &bytes).await.ok()?;
    Some(path)
}

fn sniff_image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

fn image_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    let be32 = |o: usize| {
        b.get(o..o + 4)
            .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
    };
    let le16 = |o: usize| {
        b.get(o..o + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]) as u32)
    };
    let le24 = |o: usize| {
        b.get(o..o + 3)
            .map(|s| s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
    };
    match sniff_image_ext(b)? {
        "png" => Some((be32(16)?, be32(20)?)),
        "gif" => Some((le16(6)?, le16(8)?)),
        "webp" => match b.get(12..16)? {
            b"VP8 " => {
                if b.get(23..26)? != [0x9D, 0x01, 0x2A] {
                    return None;
                }
                Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF))
            }
            b"VP8L" => {
                if *b.get(20)? != 0x2F {
                    return None;
                }
                let bits = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        "jpg" => {
            let mut i = 2;
            while i + 4 <= b.len() {
                if b[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let marker = b[i + 1];
                match marker {
                    0xFF => i += 1, // fill byte
                    0x01 | 0xD0..=0xD8 => i += 2,
                    0xD9 => return None,
                    _ => {
                        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
                        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                            let h = u16::from_be_bytes([*b.get(i + 5)?, *b.get(i + 6)?]) as u32;
                            let w = u16::from_be_bytes([*b.get(i + 7)?, *b.get(i + 8)?]) as u32;
                            return Some((w, h));
                        }
                        i += 2 + len;
                    }
                }
            }
            None
        }
        _ => None,
    }
}

async fn prepare_cover(
    app: &AppHandle,
    track_id: &str,
    cover: PathBuf,
    work_dir: &Path,
    needs_jpeg_png: bool,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<Option<PathBuf>> {
    let ext = cover
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let must_convert = needs_jpeg_png && !matches!(ext.as_str(), "jpg" | "jpeg" | "png");

    let dimensions = match fs::read(&cover).await {
        Ok(bytes) => image_dimensions(&bytes),
        Err(_) => None,
    };
    let non_square = CROP_COVERS_TO_SQUARE
        && dimensions.is_some_and(|(w, h)| {
            h > 0 && ((w as f64 / h as f64) - 1.0).abs() > COVER_SQUARE_TOLERANCE
        });
    let crop = CROP_COVERS_TO_SQUARE && (non_square || must_convert);
    if !must_convert && !non_square {
        return Ok(Some(cover));
    }

    let converted = work_dir.join("cover_embed.jpg");
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-i".into(),
        cover.to_string_lossy().into_owned(),
        "-frames:v".into(),
        "1".into(),
    ];
    if crop {
        args.push("-vf".into());
        args.push("crop=w='min(iw,ih)':h='min(iw,ih)'".into());
    }
    args.extend([
        "-q:v".into(),
        "2".into(),
        "-pix_fmt".into(),
        "yuvj420p".into(),
        converted.to_string_lossy().into_owned(),
    ]);
    match run_ffmpeg(app, track_id, &args, registry, token).await {
        Ok(()) => {
            if let Some((w, h)) = dimensions.filter(|_| non_square) {
                let side = w.min(h);
                logger::info(format!(
                    "[Cover] Cropped {w}x{h} cover to a centred {side}x{side} square"
                ));
            }
            Ok(Some(converted))
        }
        Err(AppError::Cancelled) => Err(AppError::Cancelled),
        Err(e) => {
            logger::warn(format!(
                "[Cover] Could not prepare {ext} cover ({e}); continuing without cover art"
            ));
            Ok(None)
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn mime_type_for_cover(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

fn build_opus_picture_metadata(image_bytes: &[u8], mime_type: &str) -> String {
    let mut block = Vec::with_capacity(32 + mime_type.len() + image_bytes.len());
    block.extend_from_slice(&3u32.to_be_bytes());
    block.extend_from_slice(&(mime_type.len() as u32).to_be_bytes());
    block.extend_from_slice(mime_type.as_bytes());
    block.extend_from_slice(&0u32.to_be_bytes());
    block.extend_from_slice(&0u32.to_be_bytes());
    block.extend_from_slice(&0u32.to_be_bytes());
    block.extend_from_slice(&0u32.to_be_bytes());
    block.extend_from_slice(&0u32.to_be_bytes());
    block.extend_from_slice(&(image_bytes.len() as u32).to_be_bytes());
    block.extend_from_slice(image_bytes);
    base64_encode(&block)
}

fn escape_ffmetadata_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

async fn write_ffmetadata_file(
    work_dir: &Path,
    track: &Track,
    opts: &DownloadOptions,
    picture_b64: Option<&str>,
) -> AppResult<PathBuf> {
    let mut content = String::from(";FFMETADATA1\n");
    if opts.embed_id3_tags {
        for (key, value) in [
            ("title", track.title.as_str()),
            ("artist", track.artist.as_str()),
            ("album", track.album.as_str()),
            ("date", track.year.as_str()),
        ] {
            if !value.is_empty() {
                content.push_str(&format!("{key}={}\n", escape_ffmetadata_value(value)));
            }
        }

        if let Some(tag) = track_tag_value(track) {
            content.push_str(&format!("track={tag}\n"));
        }
    }

    if let Some(pic) = picture_b64 {
        content.push_str(&format!(
            "METADATA_BLOCK_PICTURE={}\n",
            escape_ffmetadata_value(pic)
        ));
    }
    let path = work_dir.join("cover.ffmetadata");
    fs::write(&path, content).await?;
    Ok(path)
}

async fn build_ffmpeg_args(
    work_dir: &Path,
    audio_in: &Path,
    cover_in: Option<&Path>,
    out_path: &Path,
    track: &Track,
    opts: &DownloadOptions,
    copy_audio: bool,
) -> AppResult<Vec<String>> {
    let (codec, ext) = codec_and_extension(&opts.format);
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-threads".into(),
        "0".into(),
        "-i".into(),
        audio_in.to_string_lossy().into(),
    ];

    let has_art = opts.embed_id3_tags && cover_in.is_some() && ext != "wav";
    let use_video_stream_art = has_art && ext != "opus";
    let use_metadata_file_art = has_art && ext == "opus";

    if use_video_stream_art {
        args.push("-i".into());
        args.push(cover_in.unwrap().to_string_lossy().into());
        args.push("-map".into());
        args.push("0:a".into());
        args.push("-map".into());
        args.push("1:0".into());
        args.push("-c:v".into());
        args.push("mjpeg".into());
        args.push("-pix_fmt".into());
        args.push("yuvj420p".into());
        args.push("-disposition:v".into());
        args.push("attached_pic".into());
    }

    let mut metadata_file_index: Option<usize> = None;
    if use_metadata_file_art {
        let picture_b64 = cover_in.and_then(|path| {
            std::fs::read(path)
                .ok()
                .map(|bytes| build_opus_picture_metadata(&bytes, mime_type_for_cover(path)))
        });
        let meta_path =
            write_ffmetadata_file(work_dir, track, opts, picture_b64.as_deref()).await?;
        args.push("-i".into());
        args.push(meta_path.to_string_lossy().into());
        metadata_file_index = Some(1); // 0 = audio, 1 = metadata file
    }

    args.push("-c:a".into());
    args.push(if copy_audio { "copy" } else { codec }.to_string());

    if !copy_audio && matches!(codec, "libmp3lame" | "aac" | "libopus") {
        let numeric = opts.bitrate.trim_end_matches(['k', 'K']);
        if numeric.parse::<u32>().is_ok() {
            args.push("-b:a".into());
            args.push(format!("{numeric}k"));
        }
    }
    if let Some(sr) = opts
        .sample_rate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !copy_audio)
    {
        let supported = ext != "opus"
            || parse_sample_rate_hz(sr)
                .is_some_and(|hz| matches!(hz, 8000 | 12000 | 16000 | 24000 | 48000));
        if supported {
            args.push("-ar".into());
            args.push(sr.to_string());
        } else {
            logger::warn(format!(
                "Ignoring sample rate \"{sr}\": Opus supports only 8/12/16/24/48 kHz"
            ));
        }
    }

    if let Some(idx) = metadata_file_index {
        args.push("-map_metadata".into());
        args.push(idx.to_string());
    } else if opts.embed_id3_tags {
        args.push("-id3v2_version".into());
        args.push("3".into());
        for (key, value) in [
            ("title", track.title.as_str()),
            ("artist", track.artist.as_str()),
            ("album", track.album.as_str()),
            ("date", track.year.as_str()),
        ] {
            if !value.is_empty() {
                args.push("-metadata".into());
                args.push(format!("{key}={value}"));
            }
        }
        if let Some(tag) = track_tag_value(track) {
            args.push("-metadata".into());
            args.push(format!("track={tag}"));
        }
    }

    args.push(out_path.to_string_lossy().into());
    Ok(args)
}

fn build_ffmpeg_video_args(
    video_in: &Path,
    out_path: &Path,
    track: &Track,
    opts: &DownloadOptions,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        video_in.to_string_lossy().into(),
    ];
    args.push("-c".into());
    args.push("copy".into());

    if opts.embed_id3_tags {
        for (key, value) in [
            ("title", track.title.as_str()),
            ("artist", track.artist.as_str()),
            ("album", track.album.as_str()),
            ("date", track.year.as_str()),
        ] {
            if !value.is_empty() {
                args.push("-metadata".into());
                args.push(format!("{key}={value}"));
            }
        }
        if let Some(tag) = track_tag_value(track) {
            args.push("-metadata".into());
            args.push(format!("track={tag}"));
        }
    }

    args.push(out_path.to_string_lossy().into());
    args
}

async fn run_ffmpeg(
    app: &AppHandle,
    track_id: &str,
    args: &[String],
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<()> {
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    let sidecar = app
        .shell()
        .sidecar("sonic-ffmpeg")
        .map_err(|e| AppError::FfmpegFailed(format!("failed to resolve ffmpeg sidecar: {e}")))?;

    let (mut rx, child) = sidecar
        .args(args)
        .spawn()
        .map_err(|e| AppError::FfmpegFailed(format!("failed to spawn ffmpeg sidecar: {e}")))?;
    registry.set_child(track_id, child);

    let mut stderr = String::new();
    let mut exit_code: Option<i32> = None;

    loop {
        let event = tokio::select! {
            _ = token.cancelled() => {
                registry.clear_child(track_id);
                return Err(AppError::Cancelled);
            }
            event = rx.recv() => match event {
                Some(event) => event,
                None => break,
            },
        };
        match event {
            CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    logger::proc("ffmpeg", trimmed);
                }
                stderr.push_str(&line);
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
            }
            CommandEvent::Error(e) => {
                registry.clear_child(track_id);
                logger::error(format!("ffmpeg process error: {e}"));
                return Err(AppError::FfmpegFailed(format!("ffmpeg process error: {e}")));
            }
            _ => {}
        }
    }

    registry.clear_child(track_id);
    if exit_code != Some(0) {
        logger::error(format!(
            "ffmpeg exited with {exit_code:?} for track {track_id}"
        ));
        return Err(AppError::FfmpegFailed(stderr));
    }
    Ok(())
}

async fn write_cookies_file(
    youtube_cookies: Option<&str>,
    work_dir: &Path,
) -> AppResult<Option<String>> {
    match youtube_cookies.filter(|s| !s.trim().is_empty()) {
        Some(raw) => {
            if !looks_like_netscape_cookies(raw) {
                return Err(AppError::Other(
                    "YouTube cookies must be in Netscape cookies.txt format (tab-separated)."
                        .to_string(),
                ));
            }
            let p = work_dir.join("cookies.txt");
            fs::write(&p, raw).await?;
            Ok(Some(p.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

async fn run_pipeline(
    app: &AppHandle,
    track: &Track,
    dest_folder: &Path,
    opts: &DownloadOptions,
    names: &OutputNames,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    let preview_url = track
        .preview_url
        .as_deref()
        .ok_or_else(|| AppError::TrackNotFound {
            title: track.title.clone(),
            artist: track.artist.clone(),
        })?;

    let work_dir = tempfile::Builder::new()
        .prefix("sonic-download-")
        .tempdir()
        .map_err(AppError::from)?;
    let cookies_file_path =
        write_cookies_file(opts.youtube_cookies.as_deref(), work_dir.path()).await?;

    if is_video_format(&opts.format) {
        run_video_pipeline(
            app,
            track,
            preview_url,
            work_dir.path(),
            dest_folder,
            opts,
            names,
            cookies_file_path.as_deref(),
            registry,
            token,
        )
        .await
    } else {
        run_audio_pipeline(
            app,
            track,
            work_dir.path(),
            dest_folder,
            opts,
            names,
            cookies_file_path.as_deref(),
            registry,
            token,
        )
        .await
    }
}

async fn run_audio_pipeline(
    app: &AppHandle,
    track: &Track,
    work_dir: &Path,
    dest_folder: &Path,
    opts: &DownloadOptions,
    names: &OutputNames,
    cookies_file_path: Option<&str>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    let music_match = match youtube::source_video_match(track) {
        Some(source) => {
            logger::info(format!(
                "[Match] \"{}\" came from YouTube, using its source video {} directly",
                track.title, source.video_id
            ));
            source
        }
        None => youtube::find_validated_audio_match(
            app,
            &track.title,
            &track.artist,
            Some(track.duration),
            &track.album,
        )
        .await
        .ok_or_else(|| AppError::TrackNotFound {
            title: track.title.clone(),
            artist: track.artist.clone(),
        })?,
    };
    let audio_source_url = canonical_watch_url(&music_match.url);

    let mut track = track.clone();
    if track.album.is_empty() {
        if let Some(meta) =
            youtube::fetch_music_metadata(app, &music_match.video_id, &track.title).await
        {
            if track.cover_url.is_empty() {
                if let Some(cover) = meta.cover_url {
                    track.cover_url = cover;
                }
            }
            if track.year.is_empty() {
                if let Some(year) = meta.year {
                    track.year = year;
                }
            }
            if let Some(album) = meta.album {
                track.album = album;
            }
            if track.album_artist.is_none() {
                if let Some(album_artist) = meta.album_artist {
                    track.album_artist = Some(album_artist);
                }
            }
        }
    }
    let track = &track;

    let cover_url = track.cover_url.clone();
    let cover_dir = work_dir.to_path_buf();
    let cover_handle = tokio::spawn(async move { download_cover(&cover_url, &cover_dir).await });

    let target_codec = native_codec_for_format(&opts.format);
    let audio_path = download_audio(
        app,
        &track.id,
        &audio_source_url,
        work_dir,
        opts.ytdlp_clients.as_deref(),
        target_codec,
        CookieAuth {
            cookies_path: cookies_file_path,
            cookies_from_browser: opts.cookies_from_browser.as_deref(),
        },
        registry,
        token,
    )
    .await?;
    let cover_path = cover_handle.await.ok().flatten();

    let (_, extension) = codec_and_extension(&opts.format);
    let cover_path = match cover_path {
        Some(cover) if opts.embed_id3_tags && extension != "wav" => {
            prepare_cover(
                app,
                &track.id,
                cover,
                work_dir,
                extension == "opus",
                registry,
                token,
            )
            .await?
        }
        other => other,
    };
    let filename = render_filename(&opts.naming_pattern, track, extension);
    fs::create_dir_all(dest_folder).await?;
    let out_path = names.claim(dest_folder, &filename);
    let tmp_path = partial_path(&out_path);
    let _ = app.emit(
        "track-progress",
        TrackProgressPayload {
            track_id: track.id.clone(),
            phase: "tagging",
            percent: 0,
            stream: "audio",
        },
    );

    let copy_audio = target_codec.is_some() && native_codec_of_file(&audio_path) == target_codec;
    if copy_audio {
        logger::info(format!(
            "[Audio] Native {target_codec:?} stream, remuxing without re-encode (bitrate/sample-rate settings do not apply)"
        ));
    } else if let Some(target) = target_codec {
        logger::warn(format!(
            "[Audio] No native {target:?} stream was available (got {:?}), transcoding instead",
            audio_path.extension()
        ));
    }

    let args = build_ffmpeg_args(
        work_dir,
        &audio_path,
        cover_path.as_deref(),
        &tmp_path,
        track,
        opts,
        copy_audio,
    )
    .await?;
    if let Err(e) = run_ffmpeg(app, &track.id, &args, registry, token).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    finalize_output(&tmp_path, &out_path).await?;

    Ok(out_path)
}

async fn run_video_pipeline(
    app: &AppHandle,
    track: &Track,
    preview_url: &str,
    work_dir: &Path,
    dest_folder: &Path,
    opts: &DownloadOptions,
    names: &OutputNames,
    cookies_file_path: Option<&str>,
    registry: &DownloadRegistry,
    token: &CancellationToken,
) -> AppResult<PathBuf> {
    let container = video_container_extension(&opts.format);
    let video_path = download_video(
        app,
        &track.id,
        preview_url,
        work_dir,
        container,
        opts.video_quality.as_deref(),
        opts.ytdlp_clients.as_deref(),
        CookieAuth {
            cookies_path: cookies_file_path,
            cookies_from_browser: opts.cookies_from_browser.as_deref(),
        },
        registry,
        token,
    )
    .await?;

    let filename = render_filename(&opts.naming_pattern, track, container);
    fs::create_dir_all(dest_folder).await?;
    let out_path = names.claim(dest_folder, &filename);
    let tmp_path = partial_path(&out_path);

    let _ = app.emit(
        "track-progress",
        TrackProgressPayload {
            track_id: track.id.clone(),
            phase: "tagging",
            percent: 0,
            stream: "audio",
        },
    );

    let args = build_ffmpeg_video_args(&video_path, &tmp_path, track, opts);
    if let Err(e) = run_ffmpeg(app, &track.id, &args, registry, token).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    finalize_output(&tmp_path, &out_path).await?;

    Ok(out_path)
}

#[tauri::command]
pub async fn download_track(
    app: AppHandle,
    track: Track,
    opts: DownloadTrackArgs,
) -> AppResult<String> {
    logger::info(format!(
        "Download track: {} - {}",
        track.artist, track.title
    ));
    let base_folder = settings::require_download_folder(&app).await?;
    let dest_folder = match opts
        .album_folder
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(fallback_name) => {
            if opts.is_album {
                let pattern = opts
                    .folder_naming_pattern
                    .as_deref()
                    .filter(|p| !p.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(default_folder_naming_pattern);

                let album = opts
                    .album_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&track.album);

                let year = track.year.trim();

                let folder_name =
                    render_folder_name(&pattern, album, &track.artist, year, fallback_name);
                base_folder.join(folder_name)
            } else {
                base_folder.join(sanitize_path_component(fallback_name, "playlist"))
            }
        }
        None => base_folder,
    };
    let mut options = opts.into_download_options();
    options.ytdlp_clients =
        settings::resolve_ytdlp_clients(&app, options.ytdlp_clients.as_deref()).await;

    let saved = settings::load_settings(&app).await?;
    options.youtube_cookies = saved.youtube_cookies;
    options.cookies_from_browser = saved.cookies_from_browser;

    let registry = app.state::<DownloadRegistry>();
    let token = registry.begin_track(&track.id);
    let names = OutputNames::default();
    let result = run_pipeline(
        &app,
        &track,
        &dest_folder,
        &options,
        &names,
        &registry,
        &token,
    )
    .await;
    registry.end_track(&track.id);

    match &result {
        Ok(path) => logger::info(format!("Done: {}", path.to_string_lossy())),
        Err(e) => logger::error(format!("Failed: {} - {e}", track.title)),
    }

    let path = result?;
    Ok(path.to_string_lossy().to_string())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTrackArgs {
    pub format: String,
    pub bitrate: String,
    pub sample_rate: Option<String>,
    pub video_quality: Option<String>,
    #[serde(default)]
    pub ytdlp_clients: Option<String>,
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
    #[serde(default)]
    pub album_folder: Option<String>,
    #[serde(default)]
    pub folder_naming_pattern: Option<String>,
    #[serde(default)]
    pub is_album: bool,
    #[serde(default)]
    pub album_name: Option<String>,
}

impl DownloadTrackArgs {
    fn into_download_options(self) -> DownloadOptions {
        DownloadOptions {
            format: self.format,
            bitrate: self.bitrate,
            sample_rate: self.sample_rate,
            video_quality: self.video_quality,
            ytdlp_clients: self.ytdlp_clients,
            youtube_cookies: None,
            cookies_from_browser: None,
            naming_pattern: self.naming_pattern,
            embed_id3_tags: self.embed_id3_tags,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadBatchArgs {
    pub format: String,
    pub bitrate: String,
    pub playlist_name: String,
    pub sample_rate: Option<String>,
    pub video_quality: Option<String>,
    #[serde(default)]
    pub ytdlp_clients: Option<String>,
    #[serde(default)]
    pub save_in_folder: Option<bool>,
    #[serde(default)]
    pub skip_missing_tracks: bool,
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
    #[serde(default = "default_folder_naming_pattern")]
    pub folder_naming_pattern: String,
    #[serde(default)]
    pub is_album: bool,
    #[serde(default)]
    pub concurrency: Option<usize>,
}

fn default_folder_naming_pattern() -> String {
    "album_artist".to_string()
}

#[tauri::command]
pub async fn download_batch(
    app: AppHandle,
    tracks: Vec<Track>,
    args: DownloadBatchArgs,
) -> AppResult<String> {
    if tracks.is_empty() {
        return Err(AppError::Other("No tracks to download.".to_string()));
    }
    if !args.skip_missing_tracks {
        if let Some(missing) = tracks.iter().find(|t| t.preview_url.is_none()) {
            return Err(AppError::TrackNotFound {
                title: missing.title.clone(),
                artist: missing.artist.clone(),
            });
        }
    }
    logger::info(format!(
        "Download batch: {} track(s) — {}",
        tracks.len(),
        args.playlist_name
    ));
    let registry = app.state::<DownloadRegistry>();
    let base_folder = settings::require_download_folder(&app).await?;

    let collection_display_name = if args.is_album {
        let mut clean_playlist_name = args.playlist_name.trim().to_string();
        for prefix in &["Album - ", "album - ", "Playlist - ", "playlist - "] {
            if clean_playlist_name.starts_with(prefix) {
                clean_playlist_name = clean_playlist_name
                    .strip_prefix(prefix)
                    .unwrap()
                    .trim()
                    .to_string();
            }
        }

        let album_title = tracks
            .iter()
            .find_map(|t| {
                let a = t.album.trim();
                (!a.is_empty()).then(|| a.to_string())
            })
            .unwrap_or(clean_playlist_name.clone());

        let album_artist = tracks
            .iter()
            .find_map(|t| {
                let a = t.album_artist.as_deref().unwrap_or_default().trim();
                (!a.is_empty()).then(|| a.to_string())
            })
            .or_else(|| {
                tracks.iter().find_map(|t| {
                    let a = t.artist.trim();
                    (!a.is_empty()).then(|| a.to_string())
                })
            })
            .unwrap_or_default();

        let year = tracks
            .iter()
            .find_map(|t| {
                let y = t.year.trim();
                (!y.is_empty()).then(|| y.to_string())
            })
            .unwrap_or_default();

        render_folder_name(
            &args.folder_naming_pattern,
            &album_title,
            &album_artist,
            &year,
            &album_title,
        )
    } else {
        let mut clean_name = args.playlist_name.trim().to_string();
        if clean_name.starts_with("Playlist - ") || clean_name.starts_with("playlist - ") {
            clean_name = clean_name[11..].trim().to_string();
        }
        clean_name
    };

    let collection_name = sanitize_path_component(&collection_display_name, "playlist");
    let save_in_folder = args.save_in_folder.unwrap_or(false);
    let saved = settings::load_settings(&app).await?;

    let options = Arc::new(DownloadOptions {
        format: args.format.clone(),
        bitrate: args.bitrate.clone(),
        youtube_cookies: saved.youtube_cookies.clone(),
        cookies_from_browser: saved.cookies_from_browser.clone(),
        sample_rate: args.sample_rate.clone(),
        video_quality: args.video_quality.clone(),
        ytdlp_clients: settings::resolve_ytdlp_clients(&app, args.ytdlp_clients.as_deref()).await,
        naming_pattern: args.naming_pattern.clone(),
        embed_id3_tags: args.embed_id3_tags,
    });

    let batch_work_dir = if save_in_folder {
        None
    } else {
        Some(
            tempfile::Builder::new()
                .prefix("sonic-batch-")
                .tempdir()
                .map_err(AppError::from)?,
        )
    };
    let batch_dir_path = if save_in_folder {
        let dir = base_folder.join(&collection_name);
        fs::create_dir_all(&dir).await?;
        dir
    } else {
        batch_work_dir.as_ref().unwrap().path().to_path_buf()
    };

    let concurrency = settings::resolve_batch_concurrency(&app, args.concurrency)
        .await
        .max(1);
    logger::info(format!("Batch concurrency: {concurrency}"));
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let names = Arc::new(OutputNames::default());

    let mut handles = Vec::with_capacity(tracks.len());
    for track in tracks {
        if track.preview_url.is_none() {
            logger::warn(format!(
                "Skipping \"{}\" by \"{}\" — not found on YouTube",
                track.title, track.artist
            ));
            continue;
        }

        // Registered up front, before the track even waits on a
        // concurrency permit, so cancel_batch/cancel_download can reach
        // a still-queued track — not just one that's already running.
        let token = registry.begin_track(&track.id);
        let track_id = track.id.clone();

        let sem = semaphore.clone();
        let opts = options.clone();
        let names = names.clone();
        let dest = batch_dir_path.clone();
        let app_handle = app.clone();
        handles.push((
            track_id,
            tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore closed");
                let registry = app_handle.state::<DownloadRegistry>();
                let result = if token.is_cancelled() {
                    Err(AppError::Cancelled)
                } else {
                    run_pipeline(&app_handle, &track, &dest, &opts, &names, &registry, &token).await
                };
                registry.end_track(&track.id);
                match &result {
                    Ok(_) => logger::info(format!("✓ {} - {}", track.artist, track.title)),
                    Err(e) if !matches!(e, AppError::Cancelled) => {
                        logger::warn(format!("✗ {} - {}: {e}", track.artist, track.title))
                    }
                    _ => {}
                }
                (track, result)
            }),
        ));
    }

    let mut output_entries: Vec<PathBuf> = Vec::new();
    let mut failures: Vec<(Track, AppError)> = Vec::new();
    let mut join_errors: usize = 0;
    for (track_id, handle) in handles {
        match handle.await {
            Ok((track, result)) => match result {
                Ok(path) => output_entries.push(path),
                Err(e) => failures.push((track, e)),
            },
            Err(e) => {
                join_errors += 1;
                registry.end_track(&track_id);
                logger::error(format!("Download task for track {track_id} crashed: {e}"));
            }
        }
    }

    if failures
        .iter()
        .any(|(_, e)| matches!(e, AppError::Cancelled))
    {
        return Err(AppError::Cancelled);
    }
    if output_entries.is_empty() {
        let reason = failures
            .into_iter()
            .next()
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| "All tracks failed to download.".to_string());
        return Err(AppError::Other(reason));
    }
    if !args.skip_missing_tracks && join_errors > 0 {
        return Err(AppError::Other(format!(
            "{join_errors} download task(s) crashed unexpectedly"
        )));
    }
    if !args.skip_missing_tracks && !failures.is_empty() {
        let (track, err) = failures.into_iter().next().unwrap();
        return Err(AppError::Other(format!(
            "Batch aborted on \"{}\" by \"{}\": {err}",
            track.title, track.artist
        )));
    }
    if !failures.is_empty() {
        logger::warn(format!("{} track(s) skipped due to errors", failures.len()));
    }
    logger::info(format!(
        "Download batch done: {} succeeded",
        output_entries.len()
    ));

    if save_in_folder {
        return Ok(batch_dir_path.to_string_lossy().to_string());
    }

    fs::create_dir_all(&base_folder).await?;
    let zip_path = base_folder.join(format!("{collection_name}.zip"));
    let zip_files = output_entries;
    let zip_target = zip_path.clone();
    let zip_result = tokio::task::spawn_blocking(move || write_zip(&zip_files, &zip_target))
        .await
        .map_err(|e| AppError::Other(format!("Zip task join error: {e}")))?;
    if let Err(e) = zip_result {
        let _ = fs::remove_file(&zip_path).await;
        return Err(e);
    }

    Ok(zip_path.to_string_lossy().to_string())
}

fn sanitize_path_component(name: &str, default: &str) -> String {
    clean_path_component(name)
        .or_else(|| clean_path_component(default))
        .unwrap_or_else(|| "untitled".to_string())
}

fn write_zip(files: &[PathBuf], zip_path: &Path) -> AppResult<()> {
    use std::io::Write;

    let file = std::fs::File::create(zip_path)?;
    let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);

    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("track")
            .to_string();
        writer
            .start_file(name, options)
            .map_err(|e| AppError::Other(format!("zip error: {e}")))?;
        let mut src = std::fs::File::open(path)?;
        std::io::copy(&mut src, &mut writer)
            .map_err(|e| AppError::Other(format!("zip write error: {e}")))?;
    }
    let mut buffered = writer
        .finish()
        .map_err(|e| AppError::Other(format!("zip finish error: {e}")))?;
    buffered.flush()?;
    Ok(())
}
