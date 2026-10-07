use once_cell::sync::Lazy;
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
    #[serde(default)]
    pub ytdlp_clients: Option<String>,
}

static SETTINGS_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

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
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            AppError::Other(format!(
                "settings.json is unreadable ({e}). Move or delete {} to reset it.",
                path.display()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(AppError::from(e)),
    }
}

async fn save_settings(app: &AppHandle, settings: &AppSettings) -> AppResult<()> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::Other(format!("Failed to serialize settings: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw).await?;
    fs::rename(&tmp, &path).await?;
    Ok(())
}

async fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut AppSettings),
) -> AppResult<AppSettings> {
    let _guard = SETTINGS_LOCK.lock().await;
    let mut settings = load_settings(app).await?;
    change(&mut settings);
    save_settings(app, &settings).await?;
    Ok(settings)
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

const MAX_YTDLP_CLIENT_ENTRIES: usize = 6;
const MAX_YTDLP_CLIENTS_LEN: usize = 200;

pub fn normalize_ytdlp_clients(raw: &str) -> Result<Option<String>, String> {
    if raw.len() > MAX_YTDLP_CLIENTS_LEN {
        return Err(format!(
            "Client chain is too long (max {MAX_YTDLP_CLIENTS_LEN} characters)."
        ));
    }
    let mut entries: Vec<String> = Vec::new();
    for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let canonical = if entry.eq_ignore_ascii_case("default") {
            "default".to_string()
        } else if entry
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ',' | '-' | '+'))
        {
            entry.to_string()
        } else {
            return Err(format!(
                "Invalid client \"{entry}\": use only letters, digits and _ , - +"
            ));
        };
        if !entries.contains(&canonical) {
            entries.push(canonical);
        }
    }
    if entries.len() > MAX_YTDLP_CLIENT_ENTRIES {
        return Err(format!(
            "Too many attempts (max {MAX_YTDLP_CLIENT_ENTRIES}); each one adds delay when a download fails."
        ));
    }
    Ok(if entries.is_empty() {
        None
    } else {
        Some(entries.join(";"))
    })
}

pub async fn resolve_ytdlp_clients(
    app: &AppHandle,
    override_value: Option<&str>,
) -> Option<String> {
    if let Some(v) = override_value.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v.to_string());
    }
    load_settings(app).await.ok().and_then(|s| s.ytdlp_clients)
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
    update_settings(&app, |s| s.download_folder = Some(folder)).await
}

#[tauri::command]
pub async fn set_download_concurrency(
    app: AppHandle,
    concurrency: Option<usize>,
) -> AppResult<AppSettings> {
    update_settings(&app, |s| {
        s.download_concurrency = concurrency.map(clamp_concurrency)
    })
    .await
}

#[tauri::command]
pub async fn set_ytdlp_clients(app: AppHandle, clients: Option<String>) -> AppResult<AppSettings> {
    let normalized = match clients.as_deref() {
        Some(raw) => normalize_ytdlp_clients(raw).map_err(AppError::Other)?,
        None => None,
    };
    update_settings(&app, |s| s.ytdlp_clients = normalized).await
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
    let settings = update_settings(&app, |s| s.youtube_cookies = cookies).await?;
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
    let settings = update_settings(&app, |s| s.cookies_from_browser = browser).await?;
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
            crate::logger::warn(format!("[Settings] failed to load persisted cookies: {e}"));
        }
    }
}
