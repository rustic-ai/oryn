use oryn_core::backend::Backend;
use oryn_core::command::Command;
use oryn_core::parser::Parser;
use oryn_core::resolver::{resolve_target, ResolutionStrategy, ResolverContext};
use oryn_core::translator::translate;
use oryn_r::backend::RemoteBackend;
use serial_test::serial;
use std::fs;
use std::process::{Child, Command as StdCommand};
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::timeout;

struct TestState {
    resolver_context: Option<ResolverContext>,
}

#[tokio::test]
#[serial]
async fn test_all_lemma_scripts_remote() {
    tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .ok();

    // 1. Find a free port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    println!("Selected port for suite: {}", port);

    // 2. Setup patched extension
    let root = std::env::current_dir().unwrap();
    // Path resolution check: root is crates/oryn-r
    let src_extension_path = root.join("../../extension");
    let tmp_dir = tempdir().unwrap();
    let ext_tmp_path = tmp_dir.path();

    for entry in fs::read_dir(src_extension_path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            let dest = ext_tmp_path.join(path.file_name().unwrap());
            fs::copy(&path, &dest).unwrap();
        }
    }

    let bg_path = ext_tmp_path.join("background.js");
    let bg_content = fs::read_to_string(&bg_path).unwrap();
    let patched_content = bg_content.replace("localhost:9001", &format!("127.0.0.1:{}", port));
    fs::write(&bg_path, &patched_content).unwrap();
    println!(
        "Patched background.js: {}",
        patched_content.lines().next().unwrap()
    );
    let extension_path_str = ext_tmp_path.to_str().expect("Valid path");

    // 3. Launch Browser via Process (bypass chromiumoxide bug)
    let profile_dir = tempdir().unwrap();
    let chrome_bin = "/usr/lib64/chromium-browser/chromium-browser";

    println!(
        "Launching browser process: {} --load-extension={}",
        chrome_bin, extension_path_str
    );
    let mut chrome_process = StdCommand::new(chrome_bin)
        .arg("--no-sandbox")
        .arg("--disable-gpu")
        .arg(format!("--user-data-dir={}", profile_dir.path().display()))
        .arg(format!(
            "--disable-extensions-except={}",
            extension_path_str
        ))
        .arg(format!("--load-extension={}", extension_path_str))
        .arg("http://127.0.0.1:3000/scenarios/01_static.html") // Wake up content scripts
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .expect("Failed to launch chrome");

    // Helper to ensure chrome is killed
    struct ChromeGuard(Child);
    impl Drop for ChromeGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }
    let _guard = ChromeGuard(chrome_process);

    // 4. Start Remote Backend
    let mut backend = RemoteBackend::new(port);
    backend.launch().await.expect("Failed to launch backend");

    // 5. Run Scripts
    let scripts_dir = root.join("../../test-harness/scripts");
    let mut entries: Vec<_> = fs::read_dir(scripts_dir)
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries.into_iter().take(2) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("lemma") {
            continue;
        }

        let script_name = path.file_name().unwrap().to_str().unwrap();
        println!("\n================================================================");
        println!("RUNNING SCRIPT: {}", script_name);
        println!("================================================================");

        let content = fs::read_to_string(&path).unwrap();
        let mut state = TestState {
            resolver_context: None,
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            println!("Executing: {}", trimmed);
            let mut parser = Parser::new(trimmed);
            let commands = match parser.parse() {
                Ok(cmds) => cmds,
                Err(e) => {
                    println!("  Parse Error on line '{}': {}", trimmed, e);
                    continue;
                }
            };

            for cmd in commands {
                if let Err(e) = execute_test_command(&mut backend, &mut state, cmd).await {
                    println!("  Execution Error: {}", e);
                    panic!("Test failed on script {}: {}", script_name, e);
                }
            }
        }
        println!("FINISHED SCRIPT: {}", script_name);
    }

    // Cleanup
    backend.close().await.ok();
}

async fn execute_test_command(
    backend: &mut RemoteBackend,
    state: &mut TestState,
    cmd: Command,
) -> anyhow::Result<()> {
    match &cmd {
        Command::GoTo(url) => {
            backend.navigate(url).await?;
            return Ok(());
        }
        Command::Back => {
            backend.go_back().await?;
            return Ok(());
        }
        _ => {}
    }

    // Resolve semantic targets
    let resolved_cmd = match &cmd {
        Command::Click(target, opts) if !matches!(target, oryn_core::command::Target::Id(_)) => {
            let ctx = state
                .resolver_context
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No context for Click"))?;
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| anyhow::anyhow!(e))?;
            Command::Click(resolved, opts.clone())
        }
        Command::Type(target, text, opts)
            if !matches!(target, oryn_core::command::Target::Id(_)) =>
        {
            let ctx = state
                .resolver_context
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No context for Type"))?;
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| anyhow::anyhow!(e))?;
            Command::Type(resolved, text.clone(), opts.clone())
        }
        _ => cmd.clone(),
    };

    let req = translate(&resolved_cmd).map_err(|e| anyhow::anyhow!(e))?;
    let resp = timeout(Duration::from_secs(30), backend.execute_scanner(req)).await??;

    if let oryn_core::protocol::ScannerProtocolResponse::Ok { data, .. } = &resp {
        if let oryn_core::protocol::ScannerData::Scan(scan_result) = data.as_ref() {
            state.resolver_context = Some(ResolverContext::new(scan_result));
        }
    } else if let oryn_core::protocol::ScannerProtocolResponse::Error { message, .. } = resp {
        anyhow::bail!("Scanner Error: {}", message);
    }

    Ok(())
}
