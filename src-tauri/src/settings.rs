use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::fs;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub download_folder: Option<String>,
    #[serde(default)]
    pub naming_pattern: Option<String>,
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
pub async fn set_naming_pattern(app: AppHandle, pattern: String) -> AppResult<AppSettings> {
    let mut settings = load_settings(&app).await?;
    settings.naming_pattern = Some(pattern);
    save_settings(&app, &settings).await?;
    Ok(settings)
}

