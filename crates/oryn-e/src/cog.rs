use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

/// Standard port for WPE WebDriver
pub const DEFAULT_COG_PORT: u16 = 8080;

/// Common paths where WPEWebDriver might be installed
const WEBDRIVER_PATHS: &[&str] = &["/usr/bin/WPEWebDriver", "/usr/local/bin/WPEWebDriver"];

/// Common paths where COG might be installed
const COG_PATHS: &[&str] = &[
    "/usr/bin/cog",
    "/usr/local/bin/cog",
    "/snap/bin/wpe-webkit-mir-kiosk.cog",
    "/snap/bin/cog",
];

/// Common paths where weston might be installed
const WESTON_PATHS: &[&str] = &["/usr/bin/weston", "/usr/local/bin/weston"];

/// Returns the default WebDriver URL for a local WPE instance
pub fn default_cog_url() -> String {
    format!("http://localhost:{}", DEFAULT_COG_PORT)
}

/// Detect if we're in a headless environment (no display server)
pub fn is_headless_environment() -> bool {
    std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err()
}

/// Create a temporary directory with MiniBrowser symlink pointing to COG
/// WPEWebDriver expects "MiniBrowser" but we want to use COG
fn create_minibrowser_symlink(cog_path: &str) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    // Use PID + counter + timestamp for uniqueness (handles parallel tests)
    let unique_id = format!(
        "{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    let temp_dir = std::env::temp_dir().join(format!("oryn-wpe-{}", unique_id));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;

    let symlink_path = temp_dir.join("MiniBrowser");

    #[cfg(unix)]
    std::os::unix::fs::symlink(cog_path, &symlink_path)
        .map_err(|e| format!("Failed to create MiniBrowser symlink: {}", e))?;

    #[cfg(not(unix))]
    return Err("MiniBrowser symlink creation only supported on Unix".to_string());

    Ok(temp_dir)
}

/// Find WPEWebDriver binary on the system
pub fn find_webdriver_binary() -> Option<String> {
    // First check PATH
    if let Ok(output) = Command::new("which").arg("WPEWebDriver").output()
        && output.status.success()
        && let Ok(path) = String::from_utf8(output.stdout)
    {
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    // Check common paths
    for path in WEBDRIVER_PATHS {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

/// Find weston binary on the system
pub fn find_weston_binary() -> Option<String> {
    // First check PATH
    if let Ok(output) = Command::new("which").arg("weston").output()
        && output.status.success()
        && let Ok(path) = String::from_utf8(output.stdout)
    {
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    // Check common paths
    for path in WESTON_PATHS {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

/// Find COG binary on the system
pub fn find_cog_binary() -> Option<String> {
    // First check PATH
    if let Ok(output) = Command::new("which").arg("cog").output()
        && output.status.success()
        && let Ok(path) = String::from_utf8(output.stdout)
    {
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    // Check common paths
    for path in COG_PATHS {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

/// Handle to a running WPEWebDriver process (and optional weston for headless)
pub struct CogProcess {
    child: Child,
    port: u16,
    temp_dir: Option<PathBuf>,
    weston_process: Option<Child>,
}

impl CogProcess {
    /// Get the WebDriver URL for this instance
    pub fn webdriver_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
}

impl Drop for CogProcess {
    fn drop(&mut self) {
        info!("Shutting down WPEWebDriver process...");
        let _ = self.child.kill();
        let _ = self.child.wait();

        // Clean up weston if we started it
        if let Some(ref mut weston) = self.weston_process {
            info!("Shutting down weston-headless...");
            let _ = weston.kill();
            let _ = weston.wait();
        }

        // Clean up temp directory with MiniBrowser symlink
        if let Some(ref temp_dir) = self.temp_dir {
            let _ = std::fs::remove_dir_all(temp_dir);
        }
    }
}

/// Check if WPE backend library symlink exists (needed by COG)
fn check_wpe_backend_library() -> Result<(), String> {
    // COG requires libWPEBackend-fdo-1.0.so.
    // Fedora/CentOS often miss the .so symlink. Debian/Ubuntu usually have it.
    let lib_paths = [
        "/usr/lib64/libWPEBackend-fdo-1.0.so",
        "/usr/lib/libWPEBackend-fdo-1.0.so",
        "/usr/lib/x86_64-linux-gnu/libWPEBackend-fdo-1.0.so",
        "/usr/lib/aarch64-linux-gnu/libWPEBackend-fdo-1.0.so",
    ];

    for path in lib_paths {
        if std::path::Path::new(path).exists() {
            return Ok(());
        }
    }

    // Check versioned paths
    let versioned_paths = [
        "/usr/lib64/libWPEBackend-fdo-1.0.so.1",
        "/usr/lib/libWPEBackend-fdo-1.0.so.1",
        "/usr/lib/x86_64-linux-gnu/libWPEBackend-fdo-1.0.so.1",
        "/usr/lib/aarch64-linux-gnu/libWPEBackend-fdo-1.0.so.1",
    ];

    for path in versioned_paths {
        if std::path::Path::new(path).exists() {
            return Ok(()); // In standard environments, the dynamic linker will find it
        }
    }

    Err("WPE backend (libWPEBackend-fdo-1.0.so) not found in standard paths. Please install 'libwpebackend-fdo-1.0-1' or equivalent.".to_string())
}

/// Launch WPEWebDriver which manages COG browser instances
/// If force_headless is true, always use headless mode regardless of display availability
pub async fn launch_cog(port: u16, force_headless: bool) -> Result<CogProcess, String> {
    // WPEWebDriver is the proper WebDriver server for WPE WebKit
    // It handles spawning COG browser instances when sessions are created
    let webdriver_path = find_webdriver_binary().ok_or_else(|| {
        "WPEWebDriver not found. Install with: sudo dnf install wpewebkit (includes WPEWebDriver)"
            .to_string()
    })?;

    // Verify COG is also available (needed by WPEWebDriver)
    let cog_path = find_cog_binary()
        .ok_or_else(|| "COG not found. Install with: sudo dnf install cog".to_string())?;

    // Check WPE backend library (common issue on Fedora)
    check_wpe_backend_library()?;

    // WPEWebDriver expects "MiniBrowser" but we want to use COG
    // Create a symlink so WPEWebDriver finds our COG as MiniBrowser
    let temp_dir = create_minibrowser_symlink(&cog_path)?;
    info!(
        "Created MiniBrowser symlink: {} -> {}",
        temp_dir.join("MiniBrowser").display(),
        cog_path
    );

    // Prepend temp dir to PATH so WPEWebDriver finds our MiniBrowser symlink
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", temp_dir.display(), current_path);

    // Detect headless environment or use forced headless mode
    let headless = force_headless || is_headless_environment();
    if headless {
        info!(
            "Headless mode enabled (forced: {}, no display: {})",
            force_headless,
            is_headless_environment()
        );
    }

    // Weston headless is an alternative to COG's native headless mode.
    // Tested working with weston 14.0.0 + WPE WebKit on Alpine 3.21.
    // See docker/Dockerfile.oryn-e.weston for weston-based headless setup.
    // For simplicity, we use COG's native headless (COG_PLATFORM_NAME=headless) by default
    // as it doesn't require launching a separate compositor process.
    let weston_process: Option<Child> = None;

    info!("Launching WPEWebDriver from: {}", webdriver_path);

    // WPEWebDriver --port=PORT starts the WebDriver server
    // It will spawn COG (via MiniBrowser symlink) when sessions are created
    let mut cmd = Command::new(&webdriver_path);
    cmd.args([&format!("--port={}", port)])
        .env("PATH", &new_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Configure headless mode - use COG's native headless platform
    if headless {
        cmd.env("COG_PLATFORM_NAME", "headless");
        info!("Using COG native headless platform");
        // Ensure XDG_RUNTIME_DIR is set for COG headless
        if std::env::var("XDG_RUNTIME_DIR").is_err() {
            cmd.env("XDG_RUNTIME_DIR", "/tmp");
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch WPEWebDriver: {}", e))?;

    info!("WPEWebDriver launched with PID: {}", child.id());

    // Wait for WebDriver to be ready
    let url = format!("http://localhost:{}/status", port);
    let client = reqwest::Client::new();

    for attempt in 1..=30 {
        sleep(Duration::from_millis(200)).await;

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!("WPEWebDriver ready after {} attempts", attempt);
                return Ok(CogProcess {
                    child,
                    port,
                    temp_dir: Some(temp_dir),
                    weston_process,
                });
            }
            Ok(_) => {
                warn!(
                    "WPEWebDriver responded but not ready yet (attempt {})",
                    attempt
                );
            }
            Err(_) => {
                if attempt % 5 == 0 {
                    info!("Waiting for WPEWebDriver... (attempt {})", attempt);
                }
            }
        }
    }

    // Clean up on failure
    let _ = std::fs::remove_dir_all(&temp_dir);
    if let Some(mut weston) = weston_process {
        let _ = weston.kill();
        let _ = weston.wait();
    }
    Err("WPEWebDriver did not become ready within timeout".to_string())
}

/// Returns standard capabilities required for WPE/COG
/// WPEWebDriver will use our MiniBrowser wrapper (which invokes COG)
/// We use empty capabilities to let WPEWebDriver use its defaults
pub fn wpe_capabilities() -> serde_json::Map<String, serde_json::Value> {
    // Empty capabilities - WPEWebDriver will default to MiniBrowser
    // Our PATH modification ensures it finds our wrapper script
    serde_json::Map::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_url() {
        assert_eq!(default_cog_url(), "http://localhost:8080");
    }

    #[test]
    fn test_wpe_capabilities() {
        let caps = wpe_capabilities();
        // Empty capabilities - WPEWebDriver defaults to MiniBrowser
        assert!(caps.is_empty());
    }

    #[test]
    fn test_find_binaries() {
        // These tests just verify the functions don't panic
        // Actual availability depends on system
        let _ = find_webdriver_binary();
        let _ = find_cog_binary();
    }
}
