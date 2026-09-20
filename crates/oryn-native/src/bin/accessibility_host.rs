//! Native macOS assistive-technology host. Uses the production page worker.
#[cfg(target_os = "macos")]
fn main() {
    use oryn_native::{
        Browser,
        accessibility_platform::{
            self,
            macos::{Host, Update},
        },
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("usage: oryn-accessibility-host <url>");
        std::process::exit(2);
    };
    let (actions, requests) = mpsc::sync_channel(64);
    let (updates, snapshots) = mpsc::sync_channel(128);
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let host = Host::new(actions).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1)
    });
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("native host async runtime");
        runtime.block_on(async move {
            let browser = Browser::new();
            let context = browser.new_context();
            let page = context.new_page();
            if let Err(error) = page.goto(url).await {
                let _ = updates.send(Err(error.to_string()));
                return;
            }
            while !stop.load(Ordering::Acquire) {
                while let Ok(request) = requests.try_recv() {
                    let native_input = matches!(
                        &request.action,
                        accessibility_platform::Action::Input(_)
                            | accessibility_platform::Action::CancelInput
                    );
                    match accessibility_platform::execute(&page, request).await {
                        Ok(receipts) => {
                            if !receipts.is_empty()
                                && updates.send(Ok(Update::Receipts(receipts))).is_err()
                            {
                                stop.store(true, Ordering::Release);
                                break;
                            }
                        }
                        Err(error) => {
                            if native_input {
                                let _ = updates.try_send(Err(error));
                                stop.store(true, Ordering::Release);
                                break;
                            }
                            eprintln!("accessibility action rejected: {error}");
                        }
                    }
                }
                match accessibility_platform::capture(&page).await {
                    Ok(snapshot) => {
                        if matches!(
                            updates.try_send(Ok(Update::Snapshot(Box::new(snapshot)))),
                            Err(mpsc::TrySendError::Disconnected(_))
                        ) {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("accessibility snapshot withheld: {error}");
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let _ = page.close().await;
        });
    });
    let result = host.run(snapshots);
    stopped.store(true, Ordering::Release);
    let _ = worker.join();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("oryn-accessibility-host currently requires macOS");
    std::process::exit(2);
}
