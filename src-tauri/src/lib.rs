mod download;
mod error;
mod models;
mod settings;
mod spotify;
mod youtube;

use error::AppError;
use models::AnalyzeResult;
use tauri::AppHandle;

#[tauri::command]
async fn analyze(app: AppHandle, url: String) -> Result<AnalyzeResult, AppError> {
    if !spotify::looks_like_spotify_link(&url) {
        let tracks = if youtube::looks_like_youtube_link(&url) {
            youtube::resolve_youtube_url(&app, &url).await
        } else {
            youtube::direct_search_tracks(&app, &url).await
        };

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
            const ANALYZE_CONCURRENCY: usize = 10;
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(ANALYZE_CONCURRENCY));

            let mut handles = Vec::with_capacity(tracks.len());
            for (i, item) in tracks.into_iter().enumerate() {
                let sem = semaphore.clone();
                let app = app.clone();
                let id = gen_id();
                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire_owned().await.expect("semaphore closed");
                    let track = youtube::enrich_track(&app, &item, id, (i as u32) + 1, total).await;
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
        .invoke_handler(tauri::generate_handler![
            analyze,
            download::download_track,
            download::download_batch,
            download::save_cover_file,
            settings::get_settings,
            settings::set_download_folder,
            settings::set_naming_pattern
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
