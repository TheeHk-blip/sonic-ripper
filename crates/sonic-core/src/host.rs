//! Host abstraction: the small seam between the shared engine and whatever
//! is driving it (Tauri desktop app or CLI).

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc;

use crate::cancel::DownloadRegistry;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub struct ProcessOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum ProcessEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error(String),
    Terminated(Option<i32>),
}

/// A running child process that can be killed when a download is cancelled.
pub trait ProcessChild: Send + Sync {
    fn kill(&self) -> io::Result<()>;
}

pub struct SpawnedProcess {
    pub events: mpsc::Receiver<ProcessEvent>,
    pub child: Arc<dyn ProcessChild>,
}

/// Implemented once per frontend. Everything in `sonic-core` talks to the
/// outside world through this trait.
pub trait Host: Send + Sync + 'static {
    /// Directory for `settings.json` (and other persistent app state).
    fn data_dir(&self) -> PathBuf;

    /// App version, used in startup logs.
    fn app_version(&self) -> String;

    /// Deliver an event to the frontend. The CLI implementation can print it.
    fn emit(&self, event: &str, payload: serde_json::Value);

    /// Cancellation registry shared by all download commands.
    fn registry(&self) -> Arc<DownloadRegistry>;

    /// Run a bundled tool to completion and collect its output.
    fn run_output(&self, name: &str, args: Vec<String>) -> BoxFuture<'_, io::Result<ProcessOutput>>;

    /// Start a bundled tool and stream its stdout/stderr/termination events.
    fn spawn_stream(&self, name: &str, args: Vec<String>) -> io::Result<SpawnedProcess>;

    /// Absolute path of a bundled tool, when it can be located. Used to point
    /// yt-dlp at our ffmpeg/deno sidecars instead of relying on `PATH`.
    fn resolve_binary(&self, name: &str) -> Option<PathBuf>;
}

pub type HostRef = Arc<dyn Host>;

/// Serialize-and-emit helper used by the engine's event sites.
pub fn emit<T: Serialize>(host: &HostRef, event: &str, payload: T) {
    if let Ok(value) = serde_json::to_value(payload) {
        host.emit(event, value);
    }
}
