use std::collections::HashMap;
use std::sync::Mutex;

use tauri_plugin_shell::process::CommandChild;
use tokio_util::sync::CancellationToken;

// Tracks in-flight track downloads so the frontend can cancel a single
// track or a whole batch. Registered as Tauri managed state and shared
// by every command in the app.
#[derive(Default)]
pub struct DownloadRegistry {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, CancellationToken>,
    children: HashMap<String, CommandChild>,
}

impl DownloadRegistry {
    pub fn begin_track(&self, track_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.inner
            .lock()
            .unwrap()
            .tokens
            .insert(track_id.to_string(), token.clone());
        token
    }

    pub fn end_track(&self, track_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.tokens.remove(track_id);
        inner.children.remove(track_id);
    }

    pub fn set_child(&self, track_id: &str, child: CommandChild) {
        self.inner
            .lock()
            .unwrap()
            .children
            .insert(track_id.to_string(), child);
    }

    pub fn clear_child(&self, track_id: &str) {
        self.inner.lock().unwrap().children.remove(track_id);
    }

    pub fn cancel_track(&self, track_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let found = inner.tokens.get(track_id).is_some();
        if let Some(token) = inner.tokens.get(track_id) {
            token.cancel();
        }
        if let Some(child) = inner.children.remove(track_id) {
            let _ = child.kill();
        }
        found
    }
}

#[tauri::command]
pub fn cancel_download(registry: tauri::State<'_, DownloadRegistry>, track_id: String) -> bool {
    registry.cancel_track(&track_id)
}

#[tauri::command]
pub fn cancel_batch(registry: tauri::State<'_, DownloadRegistry>, track_ids: Vec<String>) -> u32 {
    track_ids
        .iter()
        .filter(|id| registry.cancel_track(id))
        .count() as u32
}
