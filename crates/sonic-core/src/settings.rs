use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::fs;

use crate::error::{AppError, AppResult};
use crate::host::HostRef;
use crate::youtube;

pub const DEFAULT_BATCH_CONCURRENCY: usize = 6;
pub const MIN_BATCH_CONCURRENCY: usize = 1;
pub const MAX_BATCH_CONCURRENCY: usize = 16;

fn clamp_concurrency(value: usize) -> usize {
    value.clamp(MIN_BATCH_CONCURRENCY, MAX_BATCH_CONCURRENCY)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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
// get-catalog launches one short-lived sonic-cli per album/single. Those runs
// already receive every setting on the command line, so keeping them here
// avoids creating and rewriting a private settings.json for every download.
static MEMORY_SETTINGS: Lazy<Mutex<Option<AppSettings>>> = Lazy::new(|| Mutex::new(None));

pub fn use_in_memory_settings(settings: AppSettings) {
    *MEMORY_SETTINGS.lock().unwrap() = Some(settings);
}

pub fn clear_in_memory_settings() {
    *MEMORY_SETTINGS.lock().unwrap() = None;
}

fn in_memory_mode() -> bool {
    MEMORY_SETTINGS.lock().unwrap().is_some()
}

pub fn settings_path(app: &HostRef) -> AppResult<PathBuf> {
    Ok(app.data_dir().join("settings.json"))
}

pub async fn load_settings(app: &HostRef) -> AppResult<AppSettings> {
    if let Some(settings) = MEMORY_SETTINGS.lock().unwrap().clone() {
        return Ok(settings);
    }
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

async fn save_settings(app: &HostRef, settings: &AppSettings) -> AppResult<()> {
    if in_memory_mode() {
        return Ok(());
    }
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
    app: &HostRef,
    change: impl FnOnce(&mut AppSettings),
) -> AppResult<AppSettings> {
    let _guard = SETTINGS_LOCK.lock().await;
    let mut settings = load_settings(app).await?;
    let before = settings.clone();
    change(&mut settings);
    if settings != before {
        save_settings(app, &settings).await?;
    }
    if in_memory_mode() {
        *MEMORY_SETTINGS.lock().unwrap() = Some(settings.clone());
    }
    Ok(settings)
}

pub async fn require_download_folder(app: &HostRef) -> AppResult<PathBuf> {
    let settings = load_settings(app).await?;
    match settings.download_folder {
        Some(folder) if !folder.trim().is_empty() => Ok(PathBuf::from(folder)),
        _ => Err(AppError::Other(
            "No download folder is set. Choose one in Settings first.".to_string(),
        )),
    }
}

pub async fn resolve_batch_concurrency(app: &HostRef, override_value: Option<usize>) -> usize {
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
    app: &HostRef,
    override_value: Option<&str>,
) -> Option<String> {
    if let Some(v) = override_value.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v.to_string());
    }
    load_settings(app).await.ok().and_then(|s| s.ytdlp_clients)
}

pub async fn get_settings(app: HostRef) -> AppResult<AppSettings> {
    load_settings(&app).await
}

pub async fn set_download_folder(app: HostRef, folder: String) -> AppResult<AppSettings> {
    let path = PathBuf::from(&folder);
    let meta = fs::metadata(&path)
        .await
        .map_err(|_| AppError::Other(format!("Folder does not exist: {folder}")))?;
    if !meta.is_dir() {
        return Err(AppError::Other(format!("Not a folder: {folder}")));
    }
    update_settings(&app, |s| s.download_folder = Some(folder)).await
}

pub async fn set_download_concurrency(
    app: HostRef,
    concurrency: Option<usize>,
) -> AppResult<AppSettings> {
    update_settings(&app, |s| {
        s.download_concurrency = concurrency.map(clamp_concurrency)
    })
    .await
}

pub async fn set_ytdlp_clients(app: HostRef, clients: Option<String>) -> AppResult<AppSettings> {
    let normalized = match clients.as_deref() {
        Some(raw) => normalize_ytdlp_clients(raw).map_err(AppError::Other)?,
        None => None,
    };
    update_settings(&app, |s| s.ytdlp_clients = normalized).await
}

pub async fn set_youtube_cookies(
    app: HostRef,
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

pub async fn set_cookies_from_browser(
    app: HostRef,
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

pub async fn sync_persisted_youtube_cookies(app: &HostRef) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::DownloadRegistry;
    use crate::host::{BoxFuture, Host, ProcessOutput, SpawnedProcess};
    use std::fs as std_fs;
    use std::io;
    use std::sync::{Arc, Mutex};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct TestHost {
        data_dir: PathBuf,
        registry: Arc<DownloadRegistry>,
    }

    impl Host for TestHost {
        fn data_dir(&self) -> PathBuf {
            self.data_dir.clone()
        }

        fn app_version(&self) -> String {
            "test".to_string()
        }

        fn emit(&self, _event: &str, _payload: serde_json::Value) {}

        fn registry(&self) -> Arc<DownloadRegistry> {
            self.registry.clone()
        }

        fn run_output(
            &self,
            _name: &str,
            _args: Vec<String>,
        ) -> BoxFuture<'_, io::Result<ProcessOutput>> {
            Box::pin(async { Err(io::Error::other("unused in settings tests")) })
        }

        fn spawn_stream(&self, _name: &str, _args: Vec<String>) -> io::Result<SpawnedProcess> {
            Err(io::Error::other("unused in settings tests"))
        }

        fn resolve_binary(&self, _name: &str) -> Option<PathBuf> {
            None
        }
    }

    fn test_host(dir: &std::path::Path) -> HostRef {
        Arc::new(TestHost {
            data_dir: dir.to_path_buf(),
            registry: Arc::new(DownloadRegistry::default()),
        })
    }

    struct MemoryGuard;

    impl Drop for MemoryGuard {
        fn drop(&mut self) {
            clear_in_memory_settings();
        }
    }

    #[tokio::test]
    async fn unchanged_writes_are_skipped_and_memory_mode_never_touches_disk() {
        let _test_guard = TEST_LOCK.lock().unwrap();
        clear_in_memory_settings();
        let _memory_guard = MemoryGuard;

        let dir = tempfile::tempdir().unwrap();
        let app = test_host(dir.path());
        let path = settings_path(&app).unwrap();
        let original = r#"{"downloadConcurrency":3}"#;
        std_fs::write(&path, original).unwrap();

        let same = set_download_concurrency(app.clone(), Some(3)).await.unwrap();
        assert_eq!(same.download_concurrency, Some(3));
        assert_eq!(std_fs::read_to_string(&path).unwrap(), original);
        assert!(!path.with_extension("json.tmp").exists());

        let memory_dir = tempfile::tempdir().unwrap();
        let memory_app = test_host(memory_dir.path());
        use_in_memory_settings(AppSettings {
            download_folder: Some("/music".to_string()),
            download_concurrency: Some(2),
            ytdlp_clients: Some("web_music".to_string()),
            ..AppSettings::default()
        });
        assert_eq!(load_settings(&memory_app).await.unwrap().download_concurrency, Some(2));

        let changed = set_download_concurrency(memory_app.clone(), Some(7)).await.unwrap();
        assert_eq!(changed.download_concurrency, Some(7));
        assert_eq!(load_settings(&memory_app).await.unwrap().download_concurrency, Some(7));
        assert!(!settings_path(&memory_app).unwrap().exists());
    }
}
