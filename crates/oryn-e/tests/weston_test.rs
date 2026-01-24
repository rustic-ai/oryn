//! Weston headless integration tests
//!
//! These tests launch COG inside a weston headless compositor.
//! Tests are skipped if weston is not available on the system.
//! Run with: cargo test -p oryn-e weston -- --ignored
//!
//! In Docker (oryn-e:weston image), these tests run automatically.

use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_engine::protocol::ScannerRequest;
use serial_test::serial;
use std::os::unix::fs::FileTypeExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

/// Check if weston is available on this system
fn weston_available() -> bool {
    Command::new("which")
        .arg("weston")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if we're already running inside a Wayland compositor
fn has_wayland_display() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Start weston headless compositor and return the process handle
fn start_weston() -> Option<(Child, String)> {
    let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());

    // Ensure runtime dir exists
    let _ = std::fs::create_dir_all(&xdg_runtime);

    let child = Command::new("weston")
        .args([
            "--backend=headless",
            "--shell=desktop",
            "--width=1920",
            "--height=1080",
            "--no-config",
        ])
        .env("XDG_RUNTIME_DIR", &xdg_runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Wait for socket to appear
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));

        // Find wayland socket
        if let Ok(entries) = std::fs::read_dir(&xdg_runtime) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("wayland-")
                    && !name.ends_with(".lock")
                    && (entry.file_type().map(|t| t.is_socket()).unwrap_or(false)
                        || entry.path().exists())
                {
                    return Some((child, name));
                }
            }
        }
    }

    None
}

/// Guard to ensure weston is killed when test completes
struct WestonGuard(Option<Child>);

impl Drop for WestonGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Setup weston environment for tests
/// Returns None if weston should not be started (already have display or not available)
fn setup_weston_env() -> Option<(WestonGuard, String)> {
    // If we already have a Wayland display (e.g., running in Docker with weston),
    // just use it
    if has_wayland_display() {
        let display = std::env::var("WAYLAND_DISPLAY").unwrap();
        println!("Using existing WAYLAND_DISPLAY={}", display);
        return Some((WestonGuard(None), display));
    }

    // Otherwise, try to start weston
    if !weston_available() {
        println!("Weston not available, skipping test");
        return None;
    }

    let (child, display) = start_weston()?;
    println!("Started weston with WAYLAND_DISPLAY={}", display);

    // Set environment for COG to find weston
    // SAFETY: These tests run serially and no other threads access these env vars
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &display);
        // Unset COG_PLATFORM_NAME to use Wayland backend instead of native headless
        std::env::remove_var("COG_PLATFORM_NAME");
    }

    Some((WestonGuard(Some(child)), display))
}

#[tokio::test]
#[serial]
#[ignore] // Run with: cargo test -p oryn-e weston -- --ignored
async fn test_weston_lifecycle() {
    init_tracing();

    let Some((_guard, display)) = setup_weston_env() else {
        println!("Skipping test: weston not available");
        return;
    };

    println!("Testing with WAYLAND_DISPLAY={}", display);

    // Use port 8090 to avoid conflicts
    let mut backend = EmbeddedBackend::new_headless_on_port(8090);

    // Launch (WPEWebDriver + COG connected to weston)
    backend.launch().await.expect("Failed to launch backend");
    println!("Backend launched successfully with weston.");

    // Navigate
    let res = backend.navigate("https://example.com").await;
    assert!(res.is_ok(), "Navigation failed: {:?}", res.err());
    let nav_res = res.unwrap();
    println!("Navigated to: {}", nav_res.url);
    assert!(nav_res.url.contains("example.com"));

    // Close
    let close_res = backend.close().await;
    assert!(close_res.is_ok(), "Close failed: {:?}", close_res.err());

    println!("Weston headless test passed!");
}

#[tokio::test]
#[serial]
#[ignore] // Run with: cargo test -p oryn-e weston -- --ignored
async fn test_weston_scanner() {
    init_tracing();

    let Some((_guard, display)) = setup_weston_env() else {
        println!("Skipping test: weston not available");
        return;
    };

    println!("Testing scanner with WAYLAND_DISPLAY={}", display);

    let mut backend = EmbeddedBackend::new_headless_on_port(8091);
    backend.launch().await.expect("Failed to launch backend");

    // Navigate
    backend
        .navigate("https://example.com")
        .await
        .expect("Navigation failed");

    // Test Scanner Execution
    let scan_req = ScannerRequest::Scan(oryn_engine::protocol::ScanRequest {
        max_elements: Some(10),
        include_hidden: false,
        monitor_changes: false,
        view_all: false,
        near: None,
        viewport_only: false,
    });

    let scan_res = backend.execute_scanner(scan_req).await;
    assert!(
        scan_res.is_ok(),
        "Scanner execution failed: {:?}",
        scan_res.err()
    );

    match scan_res.unwrap() {
        oryn_engine::protocol::ScannerProtocolResponse::Ok { data, .. } => {
            println!("Scanner response received: {:?}", data);
        }
        oryn_engine::protocol::ScannerProtocolResponse::Error { code, message, .. } => {
            panic!("Scanner returned error: {} - {}", code, message);
        }
    }

    let _ = backend.close().await;
    println!("Weston scanner test passed!");
}

#[tokio::test]
#[serial]
#[ignore] // Run with: cargo test -p oryn-e weston -- --ignored
async fn test_weston_navigation() {
    init_tracing();

    let Some((_guard, display)) = setup_weston_env() else {
        println!("Skipping test: weston not available");
        return;
    };

    println!("Testing navigation with WAYLAND_DISPLAY={}", display);

    let mut backend = EmbeddedBackend::new_headless_on_port(8092);
    backend.launch().await.expect("Failed to launch backend");

    // Navigate to first page
    backend
        .navigate("https://example.com")
        .await
        .expect("Nav failed");

    // Test refresh
    let refresh_res = backend.refresh().await;
    assert!(
        refresh_res.is_ok(),
        "Refresh failed: {:?}",
        refresh_res.err()
    );
    println!("Refresh successful: {}", refresh_res.unwrap().url);

    // Test go_back
    let back_res = backend.go_back().await;
    assert!(back_res.is_ok(), "Go back failed: {:?}", back_res.err());
    println!("Go back result: {}", back_res.unwrap().url);

    // Test go_forward
    let fwd_res = backend.go_forward().await;
    assert!(fwd_res.is_ok(), "Go forward failed: {:?}", fwd_res.err());
    println!("Go forward result: {}", fwd_res.unwrap().url);

    // Test cookies
    let cookies = backend.get_cookies().await.expect("Get cookies failed");
    println!("Cookies: {:?}", cookies);

    // Test tabs
    let tabs = backend.get_tabs().await.expect("Get tabs failed");
    assert!(!tabs.is_empty(), "Tabs should not be empty");
    println!("Tabs: {:?}", tabs);

    let _ = backend.close().await;
    println!("Weston navigation test passed!");
}
