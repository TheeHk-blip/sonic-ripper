use serde::Serialize;
use std::sync::Arc;

use crate::error::AppError;
use crate::host::{emit, HostRef};
use crate::models::{AnalyzeResult, ScrapedResult};
use crate::{logger, settings, spotify, youtube};

#[derive(Clone, Serialize)]
pub struct AnalyzeProgress {
    pub completed: u32,
    pub total: u32,
}

pub async fn analyze(app: HostRef, url: String) -> Result<AnalyzeResult, AppError> {
    analyze_with_hints(app, url, &[]).await
}

/// Same as `analyze`, with performer credits that must survive YTM filtering.
/// The batch runner passes the requested artist (and album artist) here so a
/// writer-heavy YTM `artists` list cannot erase the real performer.
pub async fn analyze_with_hints(
    app: HostRef,
    url: String,
    hints: &[String],
) -> Result<AnalyzeResult, AppError> {
    analyze_with_hints_and_exclusions(app, url, hints, &[]).await
}

/// Same as `analyze_with_hints`, minus exact video ids that are already owned
/// by an earlier release in the YTM index.
pub async fn analyze_with_hints_and_exclusions(
    app: HostRef,
    url: String,
    hints: &[String],
    excluded_ids: &[String],
) -> Result<AnalyzeResult, AppError> {
    logger::info(format!("[Analyze]: {url}"));
    settings::sync_persisted_youtube_cookies(&app).await;

    if !spotify::looks_like_spotify_link(&url) {
        if youtube::looks_like_youtube_playlist(&url) {
            let (playlist_name, is_album, tracks) =
                youtube::resolve_youtube_playlist(&app, &url, hints, excluded_ids).await;
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
            let mut tracks = youtube::resolve_youtube_url(&app, &url, hints).await;
            if tracks.is_empty() {
                return Err(AppError::TrackNotFound {
                    title: url.clone(),
                    artist: String::new(),
                });
            }
            if tracks.len() == 1 {
                return Ok(AnalyzeResult::Track {
                    track: tracks.remove(0),
                });
            }
            return Ok(AnalyzeResult::Playlist {
                playlist_name: url,
                is_album: false,
                tracks,
            });
        }

        if youtube::looks_like_url(&url) {
            logger::warn(format!("unsupported link: {url}"));
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
        ScrapedResult::Track(t) => {
            let track = youtube::enrich_track(&app, &t, gen_id(), 1, 1).await;
            AnalyzeResult::Track { track }
        }
        ScrapedResult::Playlist {
            playlist_name,
            is_album,
            tracks,
        } => {
            let total = tracks.len() as u32;
            const ANALYZE_CONCURRENCY: usize = 5;
            let semaphore = Arc::new(tokio::sync::Semaphore::new(ANALYZE_CONCURRENCY));
            let completed = Arc::new(std::sync::atomic::AtomicU32::new(0));

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
                    emit(
                        &app,
                        "analyze-progress",
                        AnalyzeProgress {
                            completed: done,
                            total,
                        },
                    );
                    (i, track)
                }));
            }

            let mut enriched: Vec<Option<crate::models::Track>> =
                (0..handles.len()).map(|_| None).collect();
            for handle in handles {
                let (i, track) = handle
                    .await
                    .map_err(|e| AppError::Other(format!("Task join error: {e}")))?;
                enriched[i] = Some(track);
            }
            let enriched: Vec<crate::models::Track> = enriched
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
