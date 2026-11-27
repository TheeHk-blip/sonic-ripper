mod host;

use std::sync::Arc;

use sonic_core::{AppError, AppResult, HostRef};
use tauri::Manager;

use crate::host::TauriHost;

fn host(state: &tauri::State<'_, HostRef>) -> HostRef {
    state.inner().clone()
}

#[tauri::command]
fn set_spotify_client_token_cmd(token: String) {
    sonic_core::spotify::set_spotify_client_token(token);
}

#[tauri::command]
fn get_spotify_client_token_cmd() -> Option<String> {
    sonic_core::spotify::spotify_client_token()
}

#[tauri::command]
fn set_pathfinder_hash_cmd(hash: String) {
    sonic_core::spotify::set_pathfinder_hash(hash);
}

#[tauri::command]
fn get_pathfinder_hash_cmd() -> Option<String> {
    sonic_core::spotify::pathfinder_hash()
}

#[tauri::command]
fn set_album_hash_cmd(hash: String) {
    sonic_core::spotify::set_get_album_hash(hash);
}

#[tauri::command]
fn get_album_hash_cmd() -> Option<String> {
    sonic_core::spotify::get_album_hash()
}

#[tauri::command]
fn log_frontend(level: String, msg: String) {
    match level.as_str() {
        "error" => sonic_core::logger::error(msg),
        "warn" => sonic_core::logger::warn(msg),
        _ => sonic_core::logger::info(msg),
    }
}

#[tauri::command]
async fn analyze(state: tauri::State<'_, HostRef>, url: String) -> Result<sonic_core::AnalyzeResult, AppError> {
    sonic_core::analyze(host(&state), url).await
}

#[tauri::command]
async fn download_track(
    state: tauri::State<'_, HostRef>,
    track: sonic_core::Track,
    opts: sonic_core::DownloadTrackArgs,
) -> AppResult<String> {
    sonic_core::download_track(host(&state), track, opts).await
}

#[tauri::command]
async fn download_batch(
    state: tauri::State<'_, HostRef>,
    tracks: Vec<sonic_core::Track>,
    args: sonic_core::DownloadBatchArgs,
) -> AppResult<String> {
    sonic_core::download_batch(host(&state), tracks, args).await
}

#[tauri::command]
fn cancel_download(state: tauri::State<'_, HostRef>, track_id: String) -> bool {
    host(&state).registry().cancel_track(&track_id)
}

#[tauri::command]
fn cancel_batch(state: tauri::State<'_, HostRef>, track_ids: Vec<String>) -> u32 {
    track_ids
        .iter()
        .filter(|id| host(&state).registry().cancel_track(id))
        .count() as u32
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, HostRef>) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::get_settings(host(&state)).await
}

#[tauri::command]
async fn set_download_folder(
    state: tauri::State<'_, HostRef>,
    folder: String,
) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::set_download_folder(host(&state), folder).await
}

#[tauri::command]
async fn set_youtube_cookies(
    state: tauri::State<'_, HostRef>,
    cookies: Option<String>,
) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::set_youtube_cookies(host(&state), cookies).await
}

#[tauri::command]
async fn set_cookies_from_browser(
    state: tauri::State<'_, HostRef>,
    browser: Option<String>,
) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::set_cookies_from_browser(host(&state), browser).await
}

#[tauri::command]
async fn set_download_concurrency(
    state: tauri::State<'_, HostRef>,
    concurrency: Option<usize>,
) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::set_download_concurrency(host(&state), concurrency).await
}

#[tauri::command]
async fn set_ytdlp_clients(
    state: tauri::State<'_, HostRef>,
    clients: Option<String>,
) -> AppResult<sonic_core::settings::AppSettings> {
    sonic_core::settings::set_ytdlp_clients(host(&state), clients).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let runtime: HostRef = Arc::new(TauriHost::new(app.handle().clone()));
            app.manage(runtime.clone());
            sonic_core::logger::init(runtime.clone());
            sonic_core::logger::info(format!(
                "SonicRipper {}",
                app.package_info().version
            ));

            let sync_host = runtime.clone();
            tauri::async_runtime::spawn(async move {
                sonic_core::settings::sync_persisted_youtube_cookies(&sync_host).await;
            });

            std::panic::set_hook(Box::new(|info| {
                sonic_core::logger::error(info.to_string());
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            analyze,
            log_frontend,
            download_track,
            download_batch,
            cancel_download,
            cancel_batch,
            get_settings,
            set_download_folder,
            set_youtube_cookies,
            set_cookies_from_browser,
            set_download_concurrency,
            set_ytdlp_clients,
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
