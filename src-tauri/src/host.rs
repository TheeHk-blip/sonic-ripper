//! Tauri implementation of the `sonic-core` host seam.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sonic_core::cancel::DownloadRegistry;
use sonic_core::host::{
    BoxFuture, Host, ProcessChild, ProcessEvent, ProcessOutput, SpawnedProcess,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandChild as TauriCommandChild;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

pub struct TauriHost {
    app: AppHandle,
    registry: Arc<DownloadRegistry>,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            registry: Arc::new(DownloadRegistry::default()),
        }
    }
}

struct TauriProcessChild(Mutex<Option<TauriCommandChild>>);

impl ProcessChild for TauriProcessChild {
    fn kill(&self) -> io::Result<()> {
        let child = self.0.lock().unwrap().take();
        match child {
            Some(child) => child.kill().map_err(|e| io::Error::other(e.to_string())),
            None => Ok(()),
        }
    }
}

impl Host for TauriHost {
    fn data_dir(&self) -> PathBuf {
        self.app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        if event == "app-log" {
            if let (Some(level), Some(msg)) = (payload["level"].as_str(), payload["msg"].as_str()) {
                println!("[{level}] {msg}");
            }
        }
        let _ = self.app.emit(event, payload);
    }

    fn registry(&self) -> Arc<DownloadRegistry> {
        self.registry.clone()
    }

    fn run_output(&self, name: &str, args: Vec<String>) -> BoxFuture<'_, io::Result<ProcessOutput>> {
        let name = name.to_string();
        Box::pin(async move {
            let output = self
                .app
                .shell()
                .sidecar(&name)
                .map_err(|e| io::Error::other(e.to_string()))?
                .args(args)
                .output()
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;
            Ok(ProcessOutput {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })
    }

    fn spawn_stream(&self, name: &str, args: Vec<String>) -> io::Result<SpawnedProcess> {
        let (mut rx, child) = self
            .app
            .shell()
            .sidecar(name)
            .map_err(|e| io::Error::other(e.to_string()))?
            .args(args)
            .spawn()
            .map_err(|e| io::Error::other(e.to_string()))?;

        let (tx, events) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let mapped = match event {
                    CommandEvent::Stdout(bytes) => ProcessEvent::Stdout(bytes),
                    CommandEvent::Stderr(bytes) => ProcessEvent::Stderr(bytes),
                    CommandEvent::Error(e) => ProcessEvent::Error(e),
                    CommandEvent::Terminated(payload) => ProcessEvent::Terminated(payload.code),
                    _ => continue,
                };
                if tx.send(mapped).await.is_err() {
                    break;
                }
            }
        });

        Ok(SpawnedProcess {
            events,
            child: Arc::new(TauriProcessChild(Mutex::new(Some(child)))),
        })
    }

    fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let file = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        let path = exe_dir.join(file);
        path.exists().then_some(path)
    }
}
