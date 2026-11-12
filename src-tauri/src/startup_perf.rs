//! Local, opt-in USB performance harness. No network control endpoint or credentials.
use std::sync::atomic::{AtomicBool, Ordering};
use once_cell::sync::OnceCell;
use std::path::PathBuf;

static LOG: OnceCell<PathBuf> = OnceCell::new();
static BASELINE: AtomicBool = AtomicBool::new(false);
static CONFIG: OnceCell<serde_json::Value> = OnceCell::new();

pub fn init(dir: &std::path::Path) {
    let _ = LOG.set(dir.join("debug.log"));
    // This file can only be placed in our sandbox by the device owner/developer.
    if let Ok(bytes) = std::fs::read(dir.join("media-probe.json")) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            BASELINE.store(value["baseline"].as_bool() == Some(true), Ordering::Relaxed);
            let _ = CONFIG.set(value);
        }
    }
}

pub fn optimized() -> bool { !BASELINE.load(Ordering::Relaxed) }

pub fn log(message: &str) {
    use std::io::Write;
    if let Some(path) = LOG.get() {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[perf] {message}");
        }
    }
}

#[tauri::command]
pub fn media_probe_config() -> serde_json::Value {
    CONFIG.get().cloned().unwrap_or(serde_json::Value::Null)
}
