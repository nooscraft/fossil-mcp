use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL_SECS: u64 = 86400; // 24 hours

#[derive(serde::Serialize, serde::Deserialize)]
struct UpdateCache {
    last_check: u64,
    latest_version: String,
}

fn cache_path() -> Option<PathBuf> {
    fossil_config_dir().map(|d| d.join("update-check.json"))
}

/// Returns ~/.fossil-mcp/ , creating it if needed.
pub(crate) fn fossil_config_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let dir = PathBuf::from(home).join(".fossil-mcp");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache() -> Option<UpdateCache> {
    let path = cache_path()?;
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_cache(cache: &UpdateCache) {
    if let Some(path) = cache_path() {
        if let Ok(data) = serde_json::to_string(cache) {
            if let Ok(mut f) = fs::File::create(path) {
                let _ = f.write_all(data.as_bytes());
            }
        }
    }
}

fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let v = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn print_update_notice(current: &str, latest: &str) {
    let latest_display = latest.strip_prefix('v').unwrap_or(latest);
    eprintln!(
        "\n  Update available: {} \u{2192} {}. Run 'fossil-mcp update' to upgrade.\n",
        current, latest_display
    );
}

/// Fetches the latest released version from the GitHub Releases API using
/// `self_update::backends::github::ReleaseList` (blocking HTTP call).
fn fetch_latest_version() -> Option<String> {
    let release = self_update::backends::github::ReleaseList::configure()
        .repo_owner("yfedoseev")
        .repo_name("fossil-mcp")
        .build()
        .ok()?
        .fetch()
        .ok()?;

    release.first().map(|r| r.version.clone())
}

/// Background update check — intended to be called from std::thread::spawn.
///
/// Checks GitHub for newer versions at most once per 24 hours.
/// Prints a notice to stderr if a newer version is available.
/// Silently does nothing on any error.
pub fn check_for_update_background() {
    let current = env!("CARGO_PKG_VERSION");

    // Check cache first
    if let Some(cache) = read_cache() {
        let age = now_epoch().saturating_sub(cache.last_check);
        if age < CHECK_INTERVAL_SECS {
            // Cache is fresh — use cached result
            if is_newer(&cache.latest_version, current) {
                print_update_notice(current, &cache.latest_version);
            }
            return;
        }
    }

    // Cache is stale or missing — fetch from GitHub
    if let Some(latest) = fetch_latest_version() {
        let cache = UpdateCache {
            last_check: now_epoch(),
            latest_version: latest.clone(),
        };
        write_cache(&cache);

        if is_newer(&latest, current) {
            print_update_notice(current, &latest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_basic() {
        assert_eq!(parse_version("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn test_parse_version_with_v_prefix() {
        assert_eq!(parse_version("v0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version("v10.20.30"), Some((10, 20, 30)));
    }

    #[test]
    fn test_parse_version_invalid() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("0.1"), None);
        assert_eq!(parse_version("0.1.0.0"), None);
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("0.1.0-rc.1"), None);
    }

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn test_is_newer_equal() {
        assert!(!is_newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn test_is_newer_with_v_prefix() {
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("0.1.1", "v0.1.0"));
    }

    #[test]
    fn test_is_newer_invalid_returns_false() {
        assert!(!is_newer("invalid", "0.1.0"));
        assert!(!is_newer("0.1.0", "invalid"));
        assert!(!is_newer("invalid", "also-invalid"));
    }
}
