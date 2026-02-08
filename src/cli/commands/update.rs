use crate::cli::{use_colors, C};

/// Returns the asset target string matching our release artifact naming.
fn asset_target() -> &'static str {
    if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "macos-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-aarch64"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x86_64"
    } else {
        "unknown"
    }
}

/// Check for updates only — query GitHub and print result.
fn check_only(c: &C) -> Result<String, crate::core::Error> {
    let current = env!("CARGO_PKG_VERSION");
    eprintln!("  {}  Checking for updates...", c.dim("\u{25cb}"));

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("yfedoseev")
        .repo_name("fossil-mcp")
        .build()
        .map_err(|e| crate::core::Error::config(format!("Failed to configure update check: {e}")))?
        .fetch()
        .map_err(|e| crate::core::Error::config(format!("Failed to fetch releases: {e}")))?;

    let latest = releases
        .first()
        .map(|r| r.version.as_str())
        .unwrap_or(current);

    let latest_clean = latest.strip_prefix('v').unwrap_or(latest);

    if latest_clean != current {
        eprintln!(
            "  {}  Update available: {} \u{2192} {}",
            c.yellow("\u{25cf}"),
            c.dim(current),
            c.green(latest_clean),
        );
        eprintln!("     Run {} to upgrade.", c.cyan("fossil-mcp update"),);
    } else {
        eprintln!(
            "  {}  Already up to date ({})",
            c.green("\u{2713}"),
            c.green(current),
        );
    }

    Ok(String::new())
}

/// Download and install the latest version.
fn do_update(c: &C) -> Result<String, crate::core::Error> {
    let current = env!("CARGO_PKG_VERSION");
    let target = asset_target();

    if target == "unknown" {
        return Err(crate::core::Error::config(
            "Unsupported platform for self-update. Please download manually from https://github.com/yfedoseev/fossil-mcp/releases",
        ));
    }

    eprintln!("  {}  Checking for updates...", c.dim("\u{25cb}"));

    let status = self_update::backends::github::Update::configure()
        .repo_owner("yfedoseev")
        .repo_name("fossil-mcp")
        .bin_name("fossil-mcp")
        .current_version(current)
        .target(target)
        .show_output(false)
        .show_download_progress(use_colors())
        .no_confirm(true)
        .build()
        .map_err(|e| crate::core::Error::config(format!("Failed to configure updater: {e}")))?
        .update()
        .map_err(|e| crate::core::Error::config(format!("Update failed: {e}")))?;

    if status.updated() {
        eprintln!(
            "  {}  Updated: {} \u{2192} {}",
            c.green("\u{2713}"),
            c.dim(current),
            c.green(status.version()),
        );
    } else {
        eprintln!(
            "  {}  Already up to date ({})",
            c.green("\u{2713}"),
            c.green(current),
        );
    }

    Ok(String::new())
}

/// Entry point for the `update` subcommand.
pub fn run(check: bool) -> Result<String, crate::core::Error> {
    let c = C::new();
    if check {
        check_only(&c)
    } else {
        do_update(&c)
    }
}
