mod cancel;
mod download;
mod error;
mod http;
mod models;
mod settings;
mod spotify;
mod youtube;

use error::AppError;
use models::AnalyzeResult;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::spotify::{
    get_album_hash, pathfinder_hash, set_get_album_hash, set_pathfinder_hash,
    set_spotify_client_token, spotify_client_token,
};

#[derive(Clone, Serialize)]
pub(crate) struct AnalyzeProgress {
    pub completed: u32,
    pub total: u32,
}

#[tauri::command]
fn set_spotify_client_token_cmd(token: String) {
    set_spotify_client_token(token);
}

#[tauri::command]
fn get_spotify_client_token_cmd() -> Option<String> {
    spotify_client_token()
}

#[tauri::command]
fn set_pathfinder_hash_cmd(hash: String) {
    set_pathfinder_hash(hash);
}

#[tauri::command]
fn get_pathfinder_hash_cmd() -> Option<String> {
    pathfinder_hash()
}

#[tauri::command]
fn set_album_hash_cmd(hash: String) {
    set_get_album_hash(hash);
}

#[tauri::command]
fn get_album_hash_cmd() -> Option<String> {
    get_album_hash()
}

#[tauri::command]
async fn analyze(app: AppHandle, url: String) -> Result<AnalyzeResult, AppError> {
    settings::sync_persisted_youtube_cookies(&app).await;

    if !spotify::looks_like_spotify_link(&url) {
        if youtube::looks_like_youtube_playlist(&url) {
            let (playlist_name, is_album, tracks) =
                youtube::resolve_youtube_playlist(&app, &url).await;
            if tracks.is_empty() {
                return Err(AppError::TrackNotFound {
                    title: url.clone(),
                    artist: String::new(),
                });
            }
            return Ok(AnalyzeResult::Playlist {
                playlist_name,
                is_album,
                tracks,
            });
        }

        if youtube::looks_like_youtube_link(&url) {
            let tracks = youtube::resolve_youtube_url(&app, &url).await;
            if tracks.is_empty() {
                return Err(AppError::TrackNotFound {
                    title: url.clone(),
                    artist: String::new(),
                });
            }
            return Ok(AnalyzeResult::Playlist {
                playlist_name: url,
                is_album: false,
                tracks,
            });
        }

        if youtube::looks_like_url(&url) {
            return Err(AppError::UnsupportedLink(format!(
                "\"{url}\" isn't a supported Spotify or YouTube link."
            )));
        }

        let tracks = youtube::direct_search_tracks(&app, &url).await;

        if tracks.is_empty() {
            return Err(AppError::TrackNotFound {
                title: url.clone(),
                artist: String::new(),
            });
        }
        return Ok(AnalyzeResult::Playlist {
            playlist_name: url,
            is_album: false,
            tracks,
        });
    }

    let scraped = spotify::scrape_spotify(&url)
        .await?
        .ok_or_else(|| AppError::SpotifyParseFailed(format!("No result for URL: {url}")))?;

    Ok(match scraped {
        models::ScrapedResult::Track(t) => {
            let track = youtube::enrich_track(&app, &t, gen_id(), 1, 1).await;
            AnalyzeResult::Track { track }
        }
        models::ScrapedResult::Playlist {
            playlist_name,
            is_album,
            tracks,
        } => {
            let total = tracks.len() as u32;
            const ANALYZE_CONCURRENCY: usize = 5;
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(ANALYZE_CONCURRENCY));
            let completed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

            let mut handles = Vec::with_capacity(tracks.len());
            for (i, item) in tracks.into_iter().enumerate() {
                let sem = semaphore.clone();
                let app = app.clone();
                let completed = completed.clone();
                let id = gen_id();
                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire_owned().await.expect("semaphore closed");
                    let track = youtube::enrich_track(&app, &item, id, (i as u32) + 1, total).await;
                    let done = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    let _ = app.emit(
                        "analyze-progress",
                        AnalyzeProgress {
                            completed: done,
                            total,
                        },
                    );
                    (i, track)
                }));
            }

            let mut enriched: Vec<Option<models::Track>> =
                (0..handles.len()).map(|_| None).collect();
            for handle in handles {
                let (i, track) = handle
                    .await
                    .map_err(|e| AppError::Other(format!("Task join error: {e}")))?;
                enriched[i] = Some(track);
            }
            let enriched: Vec<models::Track> = enriched
                .into_iter()
                .map(|t| t.expect("every spawned index is filled before this point"))
                .collect();

            AnalyzeResult::Playlist {
                playlist_name,
                is_album,
                tracks: enriched,
            }
        }
    })
}

fn gen_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos % 0xFFFFFFFFFFF)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(cancel::DownloadRegistry::default())
        .invoke_handler(tauri::generate_handler![
            analyze,
            download::download_track,
            download::download_batch,
            cancel::cancel_download,
            cancel::cancel_batch,
            settings::get_settings,
            settings::set_download_folder,
            settings::set_youtube_cookies,
            settings::set_cookies_from_browser,
            set_spotify_client_token_cmd,
            get_spotify_client_token_cmd,
            set_pathfinder_hash_cmd,
            get_pathfinder_hash_cmd,
            set_album_hash_cmd,
            get_album_hash_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
