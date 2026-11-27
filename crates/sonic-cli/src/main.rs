mod progress;

use std::collections::HashMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use directories::ProjectDirs;
use progress::Progress;
use serde_json::Value;
use sonic_core::cancel::DownloadRegistry;
use sonic_core::host::{
    BoxFuture, Host, HostRef, ProcessChild, ProcessEvent, ProcessOutput, SpawnedProcess,
};
use sonic_core::{
    analyze_with_hints_and_exclusions, download_batch, download_track, AnalyzeResult, AppError,
    AppResult,
};
use sonic_core::{DownloadBatchArgs, DownloadTrackArgs};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

const BINARIES_DIR_ENV: &str = "SONIC_BINARIES_DIR";

#[derive(Parser, Debug)]
#[command(
    name = "sonic-cli",
    version,
    about = "Download audio/video from Spotify or YouTube with Sonic Ripper's engine"
)]
struct Cli {
    /// Spotify/YouTube URL, or a plain search query
    #[arg(short, long, value_name = "URL")]
    url: Option<String>,

    /// Extra metadata tag to embed; repeat for multiple tags (KEY=VALUE)
    #[arg(long = "tag", value_name = "KEY=VALUE")]
    tags: Vec<String>,

    /// Per-track metadata override: VIDEO_ID=KEY=VALUE (repeatable)
    #[arg(long = "track-tag", value_name = "VIDEO_ID=KEY=VALUE", hide = true)]
    track_tags: Vec<String>,

    /// Performer credit that must survive YTM writer/producer filtering.
    #[arg(long = "performer-hint", value_name = "NAME", hide = true)]
    performer_hints: Vec<String>,

    /// Exact video id already owned by an earlier release; never download it.
    #[arg(long = "exclude-track", value_name = "VIDEO_ID", hide = true)]
    exclude_tracks: Vec<String>,

    /// Rewrite tags on an existing file without downloading (use with --tag)
    #[arg(long = "tag-file", value_name = "FILE", hide = true)]
    tag_file: Option<PathBuf>,

    /// Output format: flac, mp3, m4a, wav, opus, mp4, webm, mkv
    #[arg(short, long, default_value = "flac", value_name = "FORMAT")]
    format: String,

    /// Bitrate for lossy formats (128k/192k/256k/320k/lossless)
    #[arg(long, default_value = "lossless", value_name = "BITRATE")]
    bitrate: String,

    /// Lossless sample rate, e.g. 44100 or 48000
    #[arg(long, value_name = "HZ")]
    sample_rate: Option<String>,

    /// Video resolution cap when format is mp4 (360p..2160p/best)
    #[arg(long, value_name = "QUALITY")]
    video_quality: Option<String>,

    /// Netscape-format cookies.txt file (for YouTube bot checks)
    #[arg(long, value_name = "PATH", conflicts_with = "cookies_from_browser")]
    cookies: Option<PathBuf>,

    /// Use cookies from a browser profile, e.g. chrome, firefox, brave
    #[arg(long, value_name = "BROWSER")]
    cookies_from_browser: Option<String>,

    /// Directory to save files into (required unless already configured)
    #[arg(short, long, value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// folder = one directory per playlist/album; zip = single .zip archive
    #[arg(long, default_value = "folder", value_parser = ["folder", "zip"])]
    mode: String,

    /// File naming pattern
    #[arg(long, default_value = "number_artist_title", value_parser = [
        "number_artist_title", "number_title", "artist_title", "title_artist", "title"
    ])]
    naming: String,

    /// Album folder naming pattern
    #[arg(long, default_value = "album_artist", value_parser = [
        "album_artist", "year_album", "album"
    ])]
    folder_pattern: String,

    /// Treat the source as an album (uses folder naming + album metadata)
    #[arg(long)]
    is_album: bool,

    /// Override the playlist/album name used for the containing folder or ZIP
    #[arg(long, value_name = "NAME")]
    playlist_name: Option<String>,

    /// Concurrent downloads (1-16)
    #[arg(long, default_value_t = 6)]
    concurrency: usize,

    /// Continue when individual tracks fail
    #[arg(long)]
    skip_missing: bool,

    /// Do not embed ID3 tags and cover art
    #[arg(long)]
    no_id3: bool,

    /// yt-dlp client chain, e.g. "web_music;default;tv"
    #[arg(long, value_name = "CLIENTS")]
    ytdlp_clients: Option<String>,

    /// Directory for the CLI's own settings.json (default: OS app data dir)
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Use the CLI flags as settings for this process only (used by get-catalog)
    #[arg(long, hide = true)]
    in_memory_settings: bool,

    /// Directory containing sonic-yt-dlp / sonic-ffmpeg / sonic-deno
    #[arg(long, value_name = "DIR")]
    binaries_dir: Option<PathBuf>,

    /// Show every engine log line (match scoring, yt-dlp/ffmpeg output) above the bars
    #[arg(short, long)]
    verbose: bool,
}

struct CliProcessChild(Arc<Mutex<Option<Child>>>);

impl ProcessChild for CliProcessChild {
    fn kill(&self) -> io::Result<()> {
        let mut guard = self.0.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            child.start_kill()?;
        }
        Ok(())
    }
}

struct CliHost {
    data_dir: PathBuf,
    binaries_dir: PathBuf,
    registry: Arc<DownloadRegistry>,
    progress: Arc<Progress>,
}

impl CliHost {
    fn new(cli: &Cli, progress: Arc<Progress>) -> Self {
        let data_dir = cli.config_dir.clone().unwrap_or_else(|| {
            ProjectDirs::from("com", "TheeHk-blip", "sonic-ripper-cli")
                .map(|dirs| dirs.data_dir().to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".sonic-ripper-cli"))
        });
        let binaries_dir = cli
            .binaries_dir
            .clone()
            .or_else(|| env::var_os(BINARIES_DIR_ENV).map(PathBuf::from))
            .unwrap_or_else(|| {
                env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(Path::to_path_buf))
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        Self {
            data_dir,
            binaries_dir,
            registry: Arc::new(DownloadRegistry::default()),
            progress,
        }
    }
}

impl Host for CliHost {
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn app_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn emit(&self, event: &str, payload: Value) {
        self.progress.handle(event, &payload);
    }

    fn registry(&self) -> Arc<DownloadRegistry> {
        self.registry.clone()
    }

    fn run_output(
        &self,
        name: &str,
        args: Vec<String>,
    ) -> BoxFuture<'_, io::Result<ProcessOutput>> {
        let binary = self.resolve_binary(name);
        let name = name.to_string();
        Box::pin(async move {
            let binary = require_binary(binary, &name)?;
            let output = Command::new(binary)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await?;
            Ok(ProcessOutput {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })
    }

    fn spawn_stream(&self, name: &str, args: Vec<String>) -> io::Result<SpawnedProcess> {
        let binary = require_binary(self.resolve_binary(name), name)?;
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let (tx, events) = mpsc::channel(64);

        let out_tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if out_tx
                    .send(ProcessEvent::Stdout(line.into_bytes()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let err_tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if err_tx
                    .send(ProcessEvent::Stderr(line.into_bytes()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let child_handle = Arc::new(Mutex::new(Some(child)));
        let wait_handle = child_handle.clone();
        let done_tx = tx.clone();
        tokio::spawn(async move {
            // Poll with try_wait so the child stays in the shared handle and
            // can still be killed if a cancel path needs it.
            loop {
                let outcome = {
                    let mut guard = wait_handle.lock().unwrap();
                    match guard.as_mut() {
                        Some(child) => match child.try_wait() {
                            Ok(Some(status)) => Some(Ok(status.code().unwrap_or(-1))),
                            Ok(None) => None,
                            Err(e) => Some(Err(e.to_string())),
                        },
                        None => None,
                    }
                };
                match outcome {
                    Some(Ok(code)) => {
                        let _ = done_tx.send(ProcessEvent::Terminated(Some(code))).await;
                        return;
                    }
                    Some(Err(e)) => {
                        let _ = done_tx.send(ProcessEvent::Error(e)).await;
                        return;
                    }
                    None => {}
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        });

        Ok(SpawnedProcess {
            events,
            child: Arc::new(CliProcessChild(child_handle)),
        })
    }

    fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
        let bundled = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        let local = self.binaries_dir.join(&bundled);
        if local.is_file() {
            return Some(local);
        }

        let system_name = match name {
            "sonic-yt-dlp" => "yt-dlp",
            "sonic-ffmpeg" => "ffmpeg",
            "sonic-deno" => "deno",
            _ => name,
        };
        let system_file = if cfg!(windows) {
            format!("{system_name}.exe")
        } else {
            system_name.to_string()
        };
        env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths)
                .map(|dir| dir.join(&system_file))
                .find(|path| path.is_file())
        })
    }
}

fn require_binary(path: Option<PathBuf>, name: &str) -> io::Result<PathBuf> {
    path.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Could not find '{name}'. Put it in --binaries-dir, set {BINARIES_DIR_ENV}, \
                 or install '{name}' on PATH."
            ),
        )
    })
}

fn config_error(err: AppError) -> ExitCode {
    eprintln!("error: {err}");
    ExitCode::from(1)
}

async fn apply_settings(cli: &Cli, host: &HostRef) -> AppResult<()> {
    let cookies = match &cli.cookies {
        Some(path) => Some(
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| AppError::Other(format!("Could not read cookies file: {e}")))?,
        ),
        None => None,
    };

    if cli.in_memory_settings {
        let folder = if let Some(dir) = &cli.output_dir {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| AppError::Other(format!("Invalid output directory: {e}")))?;
            tokio::fs::canonicalize(dir)
                .await
                .map_err(|e| AppError::Other(format!("Invalid output directory: {e}")))?
        } else {
            sonic_core::settings::require_download_folder(host).await?
        };
        let clients = match &cli.ytdlp_clients {
            Some(raw) => sonic_core::settings::normalize_ytdlp_clients(raw)
                .map_err(AppError::Other)?,
            None => None,
        };
        let settings = sonic_core::settings::AppSettings {
            download_folder: Some(folder.to_string_lossy().into_owned()),
            youtube_cookies: cookies.clone(),
            cookies_from_browser: cli.cookies_from_browser.clone(),
            download_concurrency: Some(cli.concurrency.clamp(
                sonic_core::settings::MIN_BATCH_CONCURRENCY,
                sonic_core::settings::MAX_BATCH_CONCURRENCY,
            )),
            ytdlp_clients: clients,
        };
        sonic_core::settings::use_in_memory_settings(settings);
        sonic_core::settings::sync_persisted_youtube_cookies(host).await;
        return Ok(());
    }

    sonic_core::settings::set_download_concurrency(host.clone(), Some(cli.concurrency)).await?;

    if let Some(dir) = &cli.output_dir {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| AppError::Other(format!("Invalid output directory: {e}")))?;
        let dir = tokio::fs::canonicalize(dir)
            .await
            .map_err(|e| AppError::Other(format!("Invalid output directory: {e}")))?;
        sonic_core::settings::set_download_folder(host.clone(), dir.to_string_lossy().into_owned())
            .await?;
    } else {
        sonic_core::settings::require_download_folder(host).await?;
    }

    if cookies.is_some() {
        sonic_core::settings::set_youtube_cookies(host.clone(), cookies).await?;
    }
    if let Some(browser) = &cli.cookies_from_browser {
        sonic_core::settings::set_cookies_from_browser(host.clone(), Some(browser.clone())).await?;
    }
    if let Some(clients) = &cli.ytdlp_clients {
        sonic_core::settings::set_ytdlp_clients(host.clone(), Some(clients.clone())).await?;
    }

    Ok(())
}

fn split_once(value: &str, label: &str) -> AppResult<(String, String)> {
    let (left, right) = value
        .split_once('=')
        .ok_or_else(|| AppError::Other(format!("{label} must be KEY=VALUE, got {value:?}")))?;
    let left = left.trim();
    if left.is_empty() {
        return Err(AppError::Other(format!("{label} has an empty key")));
    }
    Ok((left.to_string(), right.to_string()))
}

fn parse_tags(raw: &[String]) -> AppResult<Vec<(String, String)>> {
    raw.iter().map(|v| split_once(v, "--tag")).collect()
}

fn parse_track_tags(raw: &[String]) -> AppResult<HashMap<String, Vec<(String, String)>>> {
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for value in raw {
        let (id, pair) = split_once(value, "--track-tag")?;
        let (key, val) = split_once(&pair, "--track-tag")?;
        out.entry(id).or_default().push((key, val));
    }
    Ok(out)
}

async fn run(cli: Cli) -> AppResult<PathBuf> {
    let progress = Arc::new(Progress::new(cli.verbose));
    let host: HostRef = Arc::new(CliHost::new(&cli, progress.clone()));
    sonic_core::logger::init(host.clone());
    sonic_core::logger::info(format!("SonicRipper CLI {}", host.app_version()));

    let extra_tags = parse_tags(&cli.tags)?;
    let track_tags = parse_track_tags(&cli.track_tags)?;

    if let Some(path) = &cli.tag_file {
        sonic_core::download::retag_file(host, path.clone(), extra_tags).await?;
        return Ok(path.clone());
    }

    apply_settings(&cli, &host).await?;

    let url = cli
        .url
        .clone()
        .ok_or_else(|| AppError::Other("--url is required".to_string()))?;
    let analysis = analyze_with_hints_and_exclusions(
        host.clone(), url, &cli.performer_hints, &cli.exclude_tracks,
    )
    .await?;
    let save_in_folder = cli.mode == "folder";
    let embed_id3_tags = !cli.no_id3;

    match analysis {
        AnalyzeResult::Track { track } => {
            progress.begin_downloads(std::slice::from_ref(&track));
            let opts = DownloadTrackArgs {
                format: cli.format.clone(),
                bitrate: cli.bitrate.clone(),
                sample_rate: cli.sample_rate.clone(),
                video_quality: cli.video_quality.clone(),
                ytdlp_clients: None,
                naming_pattern: cli.naming.clone(),
                embed_id3_tags,
                album_folder: cli.playlist_name.clone(),
                folder_naming_pattern: Some(cli.folder_pattern.clone()),
                is_album: cli.is_album,
                album_name: cli.playlist_name.clone(),
                extra_tags,
                track_tags,
            };
            let path = download_track(host, track, opts).await;
            progress.finish();
            Ok(PathBuf::from(path?))
        }
        AnalyzeResult::Playlist {
            playlist_name,
            is_album,
            tracks,
        } => {
            progress.begin_downloads(&tracks);
            let opts = DownloadBatchArgs {
                format: cli.format.clone(),
                bitrate: cli.bitrate.clone(),
                playlist_name: cli.playlist_name.clone().unwrap_or(playlist_name),
                sample_rate: cli.sample_rate.clone(),
                video_quality: cli.video_quality.clone(),
                ytdlp_clients: None,
                save_in_folder: Some(save_in_folder),
                skip_missing_tracks: cli.skip_missing,
                naming_pattern: cli.naming.clone(),
                embed_id3_tags,
                folder_naming_pattern: cli.folder_pattern.clone(),
                is_album: cli.is_album || is_album,
                concurrency: Some(cli.concurrency),
                extra_tags,
                track_tags,
            };
            let path = download_batch(host, tracks, opts).await;
            progress.finish();
            Ok(PathBuf::from(path?))
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(err) => config_error(err),
    }
}
