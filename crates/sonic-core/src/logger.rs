use once_cell::sync::OnceCell;
use serde::Serialize;

use crate::host::HostRef;

static HOST: OnceCell<HostRef> = OnceCell::new();

pub fn init(host: HostRef) {
    let _ = HOST.set(host);
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
    if let Some(host) = HOST.get() {
        crate::host::emit(
            host,
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
