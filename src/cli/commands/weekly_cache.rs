use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_TTL_SECS: u64 = 86400; // 24 hours
const WEEKLY_URL: &str = "https://fossil-mcp.com/data/weekly_slop.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct WeeklyCache {
    fetched_at: u64,
    json_data: String,
}

fn cache_path() -> Option<PathBuf> {
    crate::update::fossil_config_dir().map(|d| d.join("weekly-cache.json"))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache() -> Option<WeeklyCache> {
    let path = cache_path()?;
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_cache(cache: &WeeklyCache) {
    if let Some(path) = cache_path() {
        if let Ok(data) = serde_json::to_string(cache) {
            if let Ok(mut f) = fs::File::create(path) {
                let _ = f.write_all(data.as_bytes());
            }
        }
    }
}

fn fetch_weekly_json() -> Option<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .new_agent();
    let mut body = agent.get(WEEKLY_URL).call().ok()?.into_body();
    body.read_to_string().ok()
}

fn is_cache_fresh(cache: &WeeklyCache) -> bool {
    now_epoch().saturating_sub(cache.fetched_at) < CACHE_TTL_SECS
}

/// Background prefetch: if cache is stale (>1 day), fetch and write.
/// Silently does nothing on error.
pub fn prefetch_weekly_data() {
    if let Some(cache) = read_cache() {
        if is_cache_fresh(&cache) {
            return;
        }
    }
    if let Some(json) = fetch_weekly_json() {
        write_cache(&WeeklyCache {
            fetched_at: now_epoch(),
            json_data: json,
        });
    }
}

/// Return cached JSON if fresh, otherwise fetch + cache + return.
pub fn load_weekly_json() -> Option<String> {
    if let Some(cache) = read_cache() {
        if is_cache_fresh(&cache) {
            return Some(cache.json_data);
        }
    }
    let json = fetch_weekly_json()?;
    write_cache(&WeeklyCache {
        fetched_at: now_epoch(),
        json_data: json.clone(),
    });
    Some(json)
}
