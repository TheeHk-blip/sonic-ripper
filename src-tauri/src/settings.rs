use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::fs;

use crate::error::{AppError, AppResult};
use crate::youtube;

pub const DEFAULT_BATCH_CONCURRENCY: usize = 6;
pub const MIN_BATCH_CONCURRENCY: usize = 1;
pub const MAX_BATCH_CONCURRENCY: usize = 16;

fn clamp_concurrency(value: usize) -> usize {
    value.clamp(MIN_BATCH_CONCURRENCY, MAX_BATCH_CONCURRENCY)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub download_folder: Option<String>,
    #[serde(default)]
    pub youtube_cookies: Option<String>,
    #[serde(default)]
    pub cookies_from_browser: Option<String>,
    #[serde(default)]
    pub download_concurrency: Option<usize>,
}

fn settings_path(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(format!("Could not resolve app data dir: {e}")))?;
    Ok(dir.join("settings.json"))
}

pub async fn load_settings(app: &AppHandle) -> AppResult<AppSettings> {
    let path = settings_path(app)?;
    match fs::read_to_string(&path).await {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(AppError::from(e)),
    }
}

pub async fn save_settings(app: &AppHandle, settings: &AppSettings) -> AppResult<()> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::Other(format!("Failed to serialize settings: {e}")))?;
    fs::write(&path, raw).await?;
    Ok(())
}

pub async fn require_download_folder(app: &AppHandle) -> AppResult<PathBuf> {
    let settings = load_settings(app).await?;
    match settings.download_folder {
        Some(folder) if !folder.trim().is_empty() => Ok(PathBuf::from(folder)),
        _ => Err(AppError::Other(
            "No download folder is set. Choose one in Settings first.".to_string(),
        )),
    }
}

pub async fn resolve_batch_concurrency(app: &AppHandle, override_value: Option<usize>) -> usize {
    if let Some(value) = override_value {
        return clamp_concurrency(value);
    }
    match load_settings(app).await {
        Ok(settings) => clamp_concurrency(
            settings
                .download_concurrency
                .unwrap_or(DEFAULT_BATCH_CONCURRENCY),
        ),
        Err(_) => DEFAULT_BATCH_CONCURRENCY,
    }
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> AppResult<AppSettings> {
    load_settings(&app).await
}

#[tauri::command]
pub async fn set_download_folder(app: AppHandle, folder: String) -> AppResult<AppSettings> {
    let path = PathBuf::from(&folder);
    let meta = fs::metadata(&path)
        .await
        .map_err(|_| AppError::Other(format!("Folder does not exist: {folder}")))?;
    if !meta.is_dir() {
        return Err(AppError::Other(format!("Not a folder: {folder}")));
    }

    let mut settings = load_settings(&app).await?;
    settings.download_folder = Some(folder);
    save_settings(&app, &settings).await?;
    Ok(settings)
}

#[tauri::command]
pub async fn set_download_concurrency(
    app: AppHandle,
    concurrency: Option<usize>,
) -> AppResult<AppSettings> {
    let mut settings = load_settings(&app).await?;
    settings.download_concurrency = concurrency.map(clamp_concurrency);
    save_settings(&app, &settings).await?;
    Ok(settings)
}

#[tauri::command]
pub async fn set_youtube_cookies(
    app: AppHandle,
    cookies: Option<String>,
) -> AppResult<AppSettings> {
    let Some(cookies) = cookies else {
        return load_settings(&app).await;
    };
    let cookies = Some(cookies).filter(|s| !s.trim().is_empty());

    let mut settings = load_settings(&app).await?;
    settings.youtube_cookies = cookies;
    save_settings(&app, &settings).await?;
    youtube::set_youtube_cookies(settings.youtube_cookies.clone());
    Ok(settings)
}

#[tauri::command]
pub async fn set_cookies_from_browser(
    app: AppHandle,
    browser: Option<String>,
) -> AppResult<AppSettings> {
    let Some(browser) = browser else {
        return load_settings(&app).await;
    };
    let browser = Some(browser).filter(|s| !s.trim().is_empty());

    let mut settings = load_settings(&app).await?;
    settings.cookies_from_browser = browser;
    save_settings(&app, &settings).await?;
    youtube::set_cookies_from_browser(settings.cookies_from_browser.clone());
    Ok(settings)
}

pub async fn sync_persisted_youtube_cookies(app: &AppHandle) {
    match load_settings(app).await {
        Ok(settings) => {
            let cookie_source = match (&settings.youtube_cookies, &settings.cookies_from_browser) {
                (Some(_), _) => "youtube_cookies (file/string)".to_string(),
                (None, Some(browser)) => format!("cookies_from_browser: {browser}"),
                (None, None) => "none configured".to_string(),
            };
            crate::logger::info(format!("[Settings] cookies for analyze: {cookie_source}"));

            youtube::set_youtube_cookies(settings.youtube_cookies);
            youtube::set_cookies_from_browser(settings.cookies_from_browser);
        }
        Err(e) => {
            eprintln!("[Settings] failed to load persisted cookies for analyze: {e}");
        }
    }
}
