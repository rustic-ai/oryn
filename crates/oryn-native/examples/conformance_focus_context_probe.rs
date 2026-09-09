//! Pinned HTML fixtures through the native browser context and canonical WPT server.
use oryn_native::runtime::{Browser, ContextOptions, WaitPredicate};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let base = args.next().ok_or("expected canonical WPT server URL")?;
    for fixture in args {
        let uploads: std::collections::BTreeMap<String, Vec<String>> =
            match std::env::var("ORYN_PROBE_UPLOAD_ASSETS") {
                Ok(path) => serde_json::from_slice(&std::fs::read(path)?)?,
                Err(std::env::VarError::NotPresent) => Default::default(),
                Err(error) => return Err(error.into()),
            };
        let assets = uploads.get(&fixture).cloned().unwrap_or_default();
        let mut options = ContextOptions::loopback_test();
        if !assets.is_empty() {
            options.action_policy.rules.insert(
                oryn_native::ConsequentialAction::FileUpload,
                oryn_native::PolicyRule::Allow,
            );
        }
        let context = Browser::in_process_probe().new_context_with_options(options);
        for asset in assets {
            let path = std::path::Path::new(&asset);
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("upload.bin");
            context.import_upload_as(
                &asset,
                filename,
                "application/octet-stream",
                std::fs::read(path)?,
            );
        }
        let page = context.new_page();
        let result: Result<serde_json::Value, Box<dyn std::error::Error>> = async {
            let navigation_error = page.goto(format!("{base}/{fixture}")).await.err().map(|error| error.to_string());
            let wait_error = if navigation_error.is_none() {
                page.wait_for(WaitPredicate::SelectorExists("#oryn-wpt-result".into())).await.err().map(|error| error.to_string())
            } else { None };
            let output = match page.evaluate("__oryn_drain(true);JSON.stringify({publication:document.getElementById('oryn-wpt-result')?.textContent||'',tests:globalThis.__oryn_wpt_results||[]})").await {
                Ok(output) => output,
                Err(error) => return Ok(serde_json::json!({"complete":false,"tests":[],"navigation_error":navigation_error,"evaluation_error":error.to_string()})),
            };
            let observed: serde_json::Value = serde_json::from_str(&output)?;
            if let Some(published) = observed["publication"].as_str()
                .and_then(|value| value.strip_prefix("__ORYN_WPT_DONE__")) {
                let mut result: serde_json::Value = serde_json::from_str(published)?;
                if let Some(error) = navigation_error {
                    result["complete"] = false.into();
                    result["navigation_error"] = error.into();
                }
                Ok(result)
            } else {
                Ok(serde_json::json!({"complete":false,"tests":observed["tests"],"wait_error":wait_error,"navigation_error":navigation_error}))
            }
        }.await;
        let row = match result {
            Ok(result) => {
                serde_json::json!({"execution_mode":"isolated_native_context_fixture","fixture":fixture,"result":result})
            }
            Err(error) => {
                serde_json::json!({"execution_mode":"isolated_native_context_fixture","fixture":fixture,"runtime_error":error.to_string()})
            }
        };
        println!("{row}");
    }
    Ok(())
}
