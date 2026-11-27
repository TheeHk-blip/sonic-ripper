//! pacman-style progress: one bar per in-flight track, a pinned Total row
//! at the bottom, and log lines printed above the block.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::sync::Mutex;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;
use sonic_core::Track;

const LABEL_WIDTH: usize = 34;

#[derive(Default)]
struct Inner {
    total: Option<ProgressBar>,
    analyze: Option<ProgressBar>,
    bars: HashMap<String, ProgressBar>,
    labels: HashMap<String, String>,
    /// Last known percent (0-100) of each in-flight track.
    percents: HashMap<String, u64>,
    finished: u64,
    track_count: u64,
}

pub struct Progress {
    mp: MultiProgress,
    tty: bool,
    verbose: bool,
    inner: Mutex<Inner>,
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

fn row_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:<34} {msg:<12} [{bar:20}] {percent:>3}%")
        .expect("static template is valid")
        .progress_chars("##-")
}

fn analyze_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:<34} {pos}/{len:<5} [{bar:20}]")
        .expect("static template is valid")
        .progress_chars("##-")
}

impl Progress {
    pub fn new(verbose: bool) -> Self {
        let tty = std::io::stderr().is_terminal();
        let mp = if tty {
            MultiProgress::new()
        } else {
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
        };
        Self {
            mp,
            tty,
            verbose,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Print a line above the bars (or plainly to stderr when piped).
    fn log(&self, line: String) {
        if self.tty {
            let _ = self.mp.println(line);
        } else {
            eprintln!("{line}");
        }
    }

    pub fn handle(&self, event: &str, payload: &Value) {
        match event {
            "app-log" => {
                if let (Some(level), Some(msg)) =
                    (payload["level"].as_str(), payload["msg"].as_str())
                {
                    // With bars on screen, only surface warnings/errors unless -v.
                    if self.tty && !self.verbose && !matches!(level, "warn" | "error") {
                        return;
                    }
                    self.log(format!("[{level}] {msg}"));
                }
            }
            "analyze-progress" => {
                if let (Some(done), Some(total)) =
                    (payload["completed"].as_u64(), payload["total"].as_u64())
                {
                    self.on_analyze(done, total);
                }
            }
            "track-progress" => {
                if let (Some(id), Some(phase), Some(pct)) = (
                    payload["trackId"].as_str(),
                    payload["phase"].as_str(),
                    payload["percent"].as_u64(),
                ) {
                    self.on_track_progress(id, phase, pct);
                }
            }
            "track-done" => {
                if let (Some(id), Some(ok)) = (payload["trackId"].as_str(), payload["ok"].as_bool())
                {
                    self.on_track_done(id, ok);
                }
            }
            _ => {}
        }
    }

    fn on_analyze(&self, done: u64, total: u64) {
        if !self.tty {
            eprintln!("Resolving tracks: {done}/{total}");
            return;
        }
        let mut g = self.inner.lock().unwrap();
        let bar = g
            .analyze
            .get_or_insert_with(|| {
                let b = self.mp.add(ProgressBar::new(total));
                b.set_style(analyze_style());
                b.set_prefix("Resolving tracks");
                b
            })
            .clone();
        bar.set_length(total);
        bar.set_position(done);
        if done >= total {
            bar.finish_and_clear();
            g.analyze = None;
        }
    }

    /// Call once after analysis, before downloading. Registers display names
    /// and creates the pinned Total row.
    pub fn begin_downloads(&self, tracks: &[Track]) {
        let mut g = self.inner.lock().unwrap();
        for t in tracks.iter().filter(|t| t.preview_url.is_some()) {
            g.labels.insert(
                t.id.clone(),
                truncate(&format!("{} - {}", t.artist, t.title), LABEL_WIDTH),
            );
            g.track_count += 1;
        }
        if self.tty && g.track_count > 0 {
            // Total is length n*100 so in-flight percents move it smoothly.
            let total = self.mp.add(ProgressBar::new(g.track_count * 100));
            total.set_style(row_style());
            total.set_prefix("Total");
            total.set_message(format!("0/{} files", g.track_count));
            g.total = Some(total);
        }
    }

    fn refresh_total(g: &Inner) {
        if let Some(t) = &g.total {
            let inflight: u64 = g.percents.values().sum();
            t.set_position(g.finished * 100 + inflight);
            t.set_message(format!("{}/{} files", g.finished, g.track_count));
        }
    }

    fn on_track_progress(&self, id: &str, phase: &str, pct: u64) {
        if !self.tty {
            return; // piped: only completion lines are printed
        }
        let mut g = self.inner.lock().unwrap();
        let bar = match g.bars.get(id) {
            Some(b) => b.clone(),
            None => {
                let b = ProgressBar::new(100);
                let b = match g.total.clone() {
                    Some(total) => self.mp.insert_before(&total, b),
                    None => self.mp.add(b),
                };
                b.set_style(row_style());
                b.set_prefix(g.labels.get(id).cloned().unwrap_or_else(|| id.to_string()));
                g.bars.insert(id.to_string(), b.clone());
                b
            }
        };
        let shown = if phase == "downloading" {
            pct.min(100)
        } else {
            100
        };
        bar.set_position(shown);
        bar.set_message(phase.to_string());
        g.percents.insert(id.to_string(), shown);
        Self::refresh_total(&g);
    }

    fn on_track_done(&self, id: &str, ok: bool) {
        let label = {
            let mut g = self.inner.lock().unwrap();
            if let Some(b) = g.bars.remove(id) {
                b.finish_and_clear();
                self.mp.remove(&b);
            }
            g.percents.remove(id);
            g.finished += 1;
            Self::refresh_total(&g);
            g.labels.get(id).cloned().unwrap_or_else(|| id.to_string())
        };
        self.log(format!("{} {label}", if ok { "✓" } else { "✗" }));
    }

    /// Leave the Total row at its final state; clear anything still running.
    pub fn finish(&self) {
        let mut g = self.inner.lock().unwrap();
        for (_, b) in g.bars.drain() {
            b.finish_and_clear();
        }
        if let Some(t) = g.total.take() {
            t.finish();
        }
    }
}
