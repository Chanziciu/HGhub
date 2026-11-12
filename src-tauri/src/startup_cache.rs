//! Bounded, account-scoped cache for fast reopening. Lives in iOS's purgeable Caches folder.
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{collections::{HashSet, hash_map::DefaultHasher}, hash::{Hash, Hasher}, path::PathBuf, sync::Mutex, time::{SystemTime, UNIX_EPOCH}};

const TTL: u64 = 6 * 3600;
const BUDGET: u64 = 128 * 1024 * 1024;
const MAX_ENTRY: u64 = 16 * 1024 * 1024;

#[derive(Default)]
struct Store { dir: Option<PathBuf>, scope: String, pins: HashSet<String>, claimed: bool, opening_api: Option<String> }
static STORE: Lazy<Mutex<Store>> = Lazy::new(|| Mutex::new(Store::default()));
#[derive(Serialize, Deserialize)]
struct Header { key: String, expires: u64, content_type: Option<String> }
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
fn hash(value: &str) -> String {
    let mut hash = DefaultHasher::new(); value.hash(&mut hash); format!("{:016x}", hash.finish())
}
fn name(key: &str) -> String { format!("{}.cache", hash(key)) }

pub fn init(dir: PathBuf, token: Option<String>) {
    let _ = std::fs::create_dir_all(&dir);
    let pins = std::fs::read(dir.join("pins.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    *STORE.lock().unwrap() = Store { dir: Some(dir), scope: hash(token.as_deref().unwrap_or("anonymous")), pins, claimed: false, opening_api: None };
}
pub fn set_scope(token: Option<String>) { STORE.lock().unwrap().scope = hash(token.as_deref().unwrap_or("anonymous")); }
pub fn key(resource: &str) -> String { format!("{}:{resource}", STORE.lock().unwrap().scope) }
pub fn current_scope(key: &str) -> bool { key.starts_with(&format!("{}:", STORE.lock().unwrap().scope)) }

pub fn get(key: &str) -> Option<(Option<String>, Vec<u8>)> {
    if !crate::startup_perf::optimized() { return None; }
    let path = STORE.lock().unwrap().dir.as_ref()?.join(name(key));
    if std::fs::metadata(&path).ok()?.len() > MAX_ENTRY { return None; }
    let mut bytes = std::fs::read(&path).ok()?;
    let length = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    if length > 32768 { return None; }
    let header: Header = serde_json::from_slice(bytes.get(4..4 + length)?).ok()?;
    // Verify the full key as well as the filename hash: collisions cannot return another resource.
    if header.key != key || header.expires <= now() { return None; }
    bytes.drain(..4 + length);
    Some((header.content_type, bytes))
}

pub fn put(key: &str, content_type: Option<&str>, bytes: &[u8]) {
    if !crate::startup_perf::optimized() || bytes.len() as u64 > MAX_ENTRY - 32768 { return; }
    let store = STORE.lock().unwrap();
    let Some(dir) = &store.dir else { return };
    if !key.starts_with(&format!("{}:", store.scope)) { return; }
    let header = Header { key: key.to_string(), expires: now() + TTL, content_type: content_type.map(str::to_string) };
    let Ok(json) = serde_json::to_vec(&header) else { return };
    let mut data = Vec::with_capacity(4 + json.len() + bytes.len());
    data.extend_from_slice(&(json.len() as u32).to_le_bytes()); data.extend_from_slice(&json); data.extend_from_slice(bytes);
    let file = name(key);
    let temporary = dir.join(format!("{file}.tmp"));
    if std::fs::write(&temporary, data).is_err() { return; }
    if std::fs::rename(&temporary, dir.join(&file)).is_err() { let _ = std::fs::remove_file(temporary); return; }
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|entry| {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.ends_with(".cache") { return None; }
        let meta = entry.metadata().ok()?;
        Some((meta.modified().ok()?, meta.len(), file, entry.path()))
    }).collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort_by_key(|e| e.0);
    for (_, length, file, path) in entries {
        if total <= BUDGET { break; }
        if !store.pins.contains(&file) && std::fs::remove_file(path).is_ok() { total = total.saturating_sub(length); }
    }
}

/// Keep the opening video's manifest/key/first few fragments while older watched data is evicted.
pub fn opening_play(key: &str) {
    let mut store = STORE.lock().unwrap();
    if store.opening_api.is_none() { store.opening_api = Some(name(key)); }
}

/// A cached API response is useful only when its matching opening assets are still present.
pub fn opening_ready(api_key: &str, url: &str) -> bool {
    use std::io::Read;
    if !crate::startup_perf::optimized() { return false; }
    let store = STORE.lock().unwrap();
    let Some(dir) = &store.dir else { return false };
    let manifest = name(&format!("{}:{url}", store.scope));
    if store.opening_api.is_some() { return dir.join(manifest).is_file(); }
    if !store.pins.contains(&name(api_key)) || !store.pins.contains(&manifest) { return false; }
    store.pins.iter().all(|file| {
        let Ok(mut handle) = std::fs::File::open(dir.join(file)) else { return false };
        let mut size = [0u8; 4];
        if handle.read_exact(&mut size).is_err() { return false; }
        let length = u32::from_le_bytes(size) as usize;
        if length > 32768 { return false; }
        let mut bytes = vec![0u8; length];
        handle.read_exact(&mut bytes).is_ok() && serde_json::from_slice::<Header>(&bytes).is_ok_and(|header| header.expires > now())
    })
}

pub fn refresh_opening(api_key: &str, resources: Vec<String>) {
    let mut store = STORE.lock().unwrap();
    if store.opening_api.as_deref() != Some(name(api_key).as_str()) { return; }
    let scope = store.scope.clone();
    store.pins = resources.into_iter().map(|url| name(&format!("{scope}:{url}"))).collect();
    store.pins.insert(name(api_key));
    if let Some(dir) = &store.dir {
        if let Ok(bytes) = serde_json::to_vec(&store.pins) { let _ = std::fs::write(dir.join("pins.json"), bytes); }
    }
}

pub fn pin_opening(resources: impl IntoIterator<Item = String>) {
    if !crate::startup_perf::optimized() { return; }
    let mut store = STORE.lock().unwrap();
    if store.claimed { return; }
    store.claimed = true;
    let scope = store.scope.clone();
    store.pins = resources.into_iter().map(|url| name(&format!("{scope}:{url}"))).collect();
    if let Some(file) = store.opening_api.clone() { store.pins.insert(file); }
    if let Some(dir) = &store.dir {
        if let Ok(bytes) = serde_json::to_vec(&store.pins) { let _ = std::fs::write(dir.join("pins.json"), bytes); }
    }
}

pub fn clear() {
    let mut store = STORE.lock().unwrap();
    if let Some(dir) = &store.dir {
        // Delete only our own flat cache records, never walk another application directory.
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            if file.ends_with(".cache") || file.ends_with(".tmp") || file == "pins.json" { let _ = std::fs::remove_file(entry.path()); }
        }
    }
    store.pins.clear(); store.claimed = false; store.opening_api = None;
}
