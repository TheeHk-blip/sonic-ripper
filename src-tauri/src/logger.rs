use once_cell::sync::OnceCell;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

static APP_HANDLE: OnceCell<AppHandle> = OnceCell::new();

pub fn init(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

#[derive(Clone, Serialize)]
pub struct LogLine {
    pub level: String,
    pub msg: String,
    pub ts: String,
}

pub fn log(level: &str, msg: impl Into<String>) {
    let msg = msg.into();
    let ts = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    println!("[{level}] {msg}");
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "app-log",
            LogLine {
                level: level.to_string(),
                msg,
                ts,
            },
        );
    }
}

pub fn info(msg: impl Into<String>) {
    log("info", msg);
}

pub fn warn(msg: impl Into<String>) {
    log("warn", msg);
}

pub fn error(msg: impl Into<String>) {
    log("error", msg);
}

pub fn proc(source: &str, line: &str) {
    log("proc", format!("[{source}] {line}"));
}
