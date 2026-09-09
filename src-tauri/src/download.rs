use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::fs;
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::Track;
use crate::settings;

const BATCH_CONCURRENCY: usize = 6;

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
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
}

fn resolve_naming_template(pattern: &str) -> &str {
    match pattern {
        "number_artist_title" => "{trackNumber} - {artist} - {title}",
        "artist_title" => "{artist} - {title}",
        "title_artist" => "{title} - {artist}",
        "title" => "{title}",
        other => other,
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

    let sanitized: String = rendered
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let trimmed = sanitized.trim();
    let base = if trimmed.is_empty() {
        format!("{} - {}", track.artist, track.title)
    } else {
        trimmed.to_string()
    };
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
    let s = stderr.to_lowercase();
    s.contains("forbidden") || s.contains("403")
}

fn is_bot_detected(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("sign in") || s.contains("not bot")
}

async fn run_yt_dlp_streaming(
    app: &AppHandle,
    track_id: &str,
    cmd: tauri_plugin_shell::process::Command,
    stream_labels: &'static [&'static str],
) -> AppResult<(String, Option<i32>)> {
    let (mut rx, _child) = cmd
        .spawn()
        .map_err(|e| AppError::YtDlpFailed(format!("failed to spawn yt-dlp sidecar: {e}")))?;

    let mut stderr = String::new();
    let mut exit_code: Option<i32> = None;
    let mut legs_started: usize = 0;
    let mut emitted_transcoding = false;

    while let Some(event) = rx.recv().await {
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
                stderr.push_str(&String::from_utf8_lossy(&bytes));
                stderr.push('\n');
            }
            CommandEvent::Error(e) => {
                return Err(AppError::YtDlpFailed(format!("yt-dlp process error: {e}")));
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
            }
            _ => {}
        }
    }

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
    cookies: CookieAuth<'_>,
) -> AppResult<PathBuf> {
    let out_template = work_dir.join("audio.%(ext)s");
    let mut cmd = app
        .shell()
        .sidecar("sonic-yt-dlp")
        .map_err(|e| AppError::YtDlpFailed(format!("failed to resolve yt-dlp sidecar: {e}")))?;

    let mut args: Vec<String> = vec![
        "-f".into(),
        // Prefer pure audio-only, then any stream with audio, then best overall
        "bestaudio[ext=m4a]/bestaudio[ext=webm]/bestaudio/bestaudio*/best".into(),
        "-S".into(),
        "aext:m4a:webm,abr".into(),
        "--extractor-args".into(),
        "youtube:player_client=android,web".into(),
        "-N".into(),
        "8".into(),
        "--newline".into(),
        "--no-playlist".into(),
        "--no-mtime".into(),
        "-o".into(),
        out_template.to_string_lossy().into_owned(),
    ];

    cookies.append_to(&mut args);

    args.push(video_url.to_string());
    cmd = cmd.args(args);

    let (stderr, exit_code) = run_yt_dlp_streaming(app, track_id, cmd, &["audio"]).await?;

    if exit_code != Some(0) {
        eprintln!("[yt-dlp stderr]\n{stderr}");
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
    cookies: CookieAuth<'_>,
) -> AppResult<PathBuf> {
    let out_template = work_dir.join("video.%(ext)s");

    let mut cmd = app
        .shell()
        .sidecar("sonic-yt-dlp")
        .map_err(|e| AppError::YtDlpFailed(format!("failed to resolve yt-dlp sidecar: {e}")))?;

    let height_cap = quality
        .map(str::trim)
        .filter(|q| !q.is_empty() && !q.eq_ignore_ascii_case("best"))
        .and_then(|q| q.parse::<u32>().ok());

    let format_selector = match height_cap {
        Some(h) => format!("bestvideo[height<={h}]+bestaudio/best[height<={h}]"),
        None => "bestvideo+bestaudio/best".to_string(),
    };

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

    cookies.append_to(&mut args);

    args.push(video_url.to_string());
    cmd = cmd.args(args);

    let (stderr, exit_code) = run_yt_dlp_streaming(app, track_id, cmd, &["video", "audio"]).await?;

    if exit_code != Some(0) {
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
        if path.file_stem().and_then(|s| s.to_str()) == Some("video") {
            return Ok(path);
        }
    }
    Err(AppError::YtDlpFailed(
        "yt-dlp reported success but no video file was produced".to_string(),
    ))
}

async fn download_cover(cover_url: &str, work_dir: &Path) -> Option<PathBuf> {
    if cover_url.is_empty() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(2)
        .build()
        .ok()?;
    let res = client
        .get(cover_url)
        .header(reqwest::header::ACCEPT, "image/*,*/*;q=0.8")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let ext = match res
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
    let bytes = res.bytes().await.ok()?;
    let path = work_dir.join(format!("cover.{ext}"));
    fs::write(&path, &bytes).await.ok()?;
    Some(path)
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
        content.push_str(&format!(
            "track={}/{}\n",
            track.track_number, track.total_tracks
        ));
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

    let has_art = opts.embed_id3_tags && cover_in.is_some();
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
    args.push(codec.to_string());

    if matches!(codec, "libmp3lame" | "aac" | "libopus") {
        let numeric = opts.bitrate.trim_end_matches(['k', 'K']);
        if numeric.parse::<u32>().is_ok() {
            args.push("-b:a".into());
            args.push(format!("{numeric}k"));
        }
    }
    if let Some(sr) = opts.sample_rate.as_deref() {
        if !sr.is_empty() {
            args.push("-ar".into());
            args.push(sr.to_string());
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
        args.push("-metadata".into());
        args.push(format!(
            "track={}/{}",
            track.track_number, track.total_tracks
        ));
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
        args.push("-metadata".into());
        args.push(format!(
            "track={}/{}",
            track.track_number, track.total_tracks
        ));
    }

    args.push(out_path.to_string_lossy().into());
    args
}

async fn run_ffmpeg(app: &AppHandle, args: &[String]) -> AppResult<()> {
    let sidecar = app
        .shell()
        .sidecar("sonic-ffmpeg")
        .map_err(|e| AppError::FfmpegFailed(format!("failed to resolve ffmpeg sidecar: {e}")))?;

    let output = sidecar
        .args(args)
        .output()
        .await
        .map_err(|e| AppError::FfmpegFailed(format!("failed to spawn ffmpeg sidecar: {e}")))?;

    if !output.status.success() {
        return Err(AppError::FfmpegFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(())
}

async fn write_cookies_file(
    youtube_cookies: Option<&str>,
    work_dir: &Path,
) -> AppResult<Option<String>> {
    match youtube_cookies.filter(|s| !s.trim().is_empty()) {
        Some(raw) => {
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
) -> AppResult<PathBuf> {
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
            cookies_file_path.as_deref(),
        )
        .await
    } else {
        run_audio_pipeline(
            app,
            track,
            preview_url,
            work_dir.path(),
            dest_folder,
            opts,
            cookies_file_path.as_deref(),
        )
        .await
    }
}

async fn run_audio_pipeline(
    app: &AppHandle,
    track: &Track,
    preview_url: &str,
    work_dir: &Path,
    dest_folder: &Path,
    opts: &DownloadOptions,
    cookies_file_path: Option<&str>,
) -> AppResult<PathBuf> {
    let cover_url = track.cover_url.clone();
    let cover_dir = work_dir.to_path_buf();
    let cover_handle = tokio::spawn(async move { download_cover(&cover_url, &cover_dir).await });

    let audio_path = download_audio(
        app,
        &track.id,
        preview_url,
        work_dir,
        CookieAuth {
            cookies_path: cookies_file_path,
            cookies_from_browser: opts.cookies_from_browser.as_deref(),
        },
    )
    .await?;
    let cover_path = cover_handle.await.ok().flatten();

    let (_, extension) = codec_and_extension(&opts.format);
    let filename = render_filename(&opts.naming_pattern, track, extension);
    fs::create_dir_all(dest_folder).await?;
    let out_path = dest_folder.join(&filename);
    let _ = app.emit(
        "track-progress",
        TrackProgressPayload {
            track_id: track.id.clone(),
            phase: "tagging",
            percent: 0,
            stream: "audio",
        },
    );

    let args = build_ffmpeg_args(
        work_dir,
        &audio_path,
        cover_path.as_deref(),
        &out_path,
        track,
        opts,
    )
    .await?;
    run_ffmpeg(app, &args).await?;

    Ok(out_path)
}

async fn run_video_pipeline(
    app: &AppHandle,
    track: &Track,
    preview_url: &str,
    work_dir: &Path,
    dest_folder: &Path,
    opts: &DownloadOptions,
    cookies_file_path: Option<&str>,
) -> AppResult<PathBuf> {
    let container = video_container_extension(&opts.format);
    let video_path = download_video(
        app,
        &track.id,
        preview_url,
        work_dir,
        container,
        opts.video_quality.as_deref(),
        CookieAuth {
            cookies_path: cookies_file_path,
            cookies_from_browser: opts.cookies_from_browser.as_deref(),
        },
    )
    .await?;

    let filename = render_filename(&opts.naming_pattern, track, container);
    fs::create_dir_all(dest_folder).await?;
    let out_path = dest_folder.join(&filename);

    let _ = app.emit(
        "track-progress",
        TrackProgressPayload {
            track_id: track.id.clone(),
            phase: "tagging",
            percent: 0,
            stream: "audio",
        },
    );

    let args = build_ffmpeg_video_args(&video_path, &out_path, track, opts);
    run_ffmpeg(app, &args).await?;

    Ok(out_path)
}

#[tauri::command]
pub async fn download_track(
    app: AppHandle,
    track: Track,
    opts: DownloadTrackArgs,
) -> AppResult<String> {
    let base_folder = settings::require_download_folder(&app).await?;
    let dest_folder = match opts
        .album_folder
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => base_folder.join(sanitize_path_component(name, "Untitled")),
        None => base_folder,
    };
    let options = opts.into_download_options();
    let path = run_pipeline(&app, &track, &dest_folder, &options).await?;
    Ok(path.to_string_lossy().to_string())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTrackArgs {
    pub format: String,
    pub bitrate: String,
    pub youtube_cookies: Option<String>,
    pub cookies_from_browser: Option<String>,
    pub sample_rate: Option<String>,
    pub video_quality: Option<String>,
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
    #[serde(default)]
    pub album_folder: Option<String>,
}

impl DownloadTrackArgs {
    fn into_download_options(self) -> DownloadOptions {
        DownloadOptions {
            format: self.format,
            bitrate: self.bitrate,
            youtube_cookies: self.youtube_cookies,
            cookies_from_browser: self.cookies_from_browser,
            sample_rate: self.sample_rate,
            video_quality: self.video_quality,
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
    pub youtube_cookies: Option<String>,
    pub cookies_from_browser: Option<String>,
    pub sample_rate: Option<String>,
    pub video_quality: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub save_in_folder: Option<bool>,
    #[serde(default)]
    pub skip_missing_tracks: bool,
    pub naming_pattern: String,
    pub embed_id3_tags: bool,
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
    let base_folder = settings::require_download_folder(&app).await?;

    let collection_name = sanitize_path_component(&args.playlist_name, "playlist");
    let save_in_folder = args.save_in_folder.unwrap_or(false);

    let options = Arc::new(DownloadOptions {
        format: args.format.clone(),
        bitrate: args.bitrate.clone(),
        youtube_cookies: args.youtube_cookies.clone(),
        cookies_from_browser: args.cookies_from_browser.clone(),
        sample_rate: args.sample_rate.clone(),
        video_quality: args.video_quality.clone(),
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

    let semaphore = Arc::new(Semaphore::new(BATCH_CONCURRENCY));

    let mut handles = Vec::with_capacity(tracks.len());
    for track in tracks {
        if track.preview_url.is_none() {
            if args.skip_missing_tracks {
                eprintln!(
                    "[Batch] skipping \"{}\" by \"{}\" — not found on YouTube",
                    track.title, track.artist
                );
                continue;
            }
            return Err(AppError::TrackNotFound {
                title: track.title,
                artist: track.artist,
            });
        }

        let sem = semaphore.clone();
        let opts = options.clone();
        let dest = batch_dir_path.clone();
        let app_handle = app.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let result = run_pipeline(&app_handle, &track, &dest, &opts).await;
            (track, result)
        }));
    }

    let mut output_entries: Vec<PathBuf> = Vec::new();
    let mut failures: Vec<(Track, AppError)> = Vec::new();
    for handle in handles {
        let (track, result) = handle
            .await
            .map_err(|e| AppError::Other(format!("Task join error: {e}")))?;
        match result {
            Ok(path) => output_entries.push(path),
            Err(e) => failures.push((track, e)),
        }
    }

    if output_entries.is_empty() {
        let reason = failures
            .into_iter()
            .next()
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| "All tracks failed to download.".to_string());
        return Err(AppError::Other(reason));
    }
    if !args.skip_missing_tracks && !failures.is_empty() {
        let (track, err) = failures.into_iter().next().unwrap();
        return Err(AppError::Other(format!(
            "Batch aborted on \"{}\" by \"{}\": {err}",
            track.title, track.artist
        )));
    }
    if !failures.is_empty() {
        eprintln!("[Batch] {} track(s) skipped due to errors", failures.len());
    }

    if save_in_folder {
        return Ok(batch_dir_path.to_string_lossy().to_string());
    }

    fs::create_dir_all(&base_folder).await?;
    let zip_path = base_folder.join(format!("{collection_name}.zip"));
    write_zip(&output_entries, &zip_path)?;

    Ok(zip_path.to_string_lossy().to_string())
}

fn sanitize_path_component(name: &str, default: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn write_zip(files: &[PathBuf], zip_path: &Path) -> AppResult<()> {
    let file = std::fs::File::create(zip_path)?;
    let mut writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("track")
            .to_string();
        writer
            .start_file(name, options)
            .map_err(|e| AppError::Other(format!("zip error: {e}")))?;
        let bytes = std::fs::read(path)?;
        use std::io::Write;
        writer
            .write_all(&bytes)
            .map_err(|e| AppError::Other(format!("zip write error: {e}")))?;
    }
    writer
        .finish()
        .map_err(|e| AppError::Other(format!("zip finish error: {e}")))?;
    Ok(())
}
