use clap::{Parser, Subcommand};
#[cfg(feature = "embedded")]
use oryn_e::backend::EmbeddedBackend;
#[cfg(feature = "compat-core")]
use oryn_engine::backend::Backend;
#[cfg(feature = "compat-core")]
use oryn_engine::cli::{self, FileErrorMode, FileOptions, OutputHandlers, ReplOptions};
#[cfg(feature = "compat-core")]
use oryn_engine::executor::CommandExecutor;
#[cfg(feature = "chromium")]
use oryn_h::backend::HeadlessBackend;
use oryn_native::{Browser, ContextOptions, oil::NativeContextOilSession};
#[cfg(feature = "user-browser")]
use oryn_r::backend::RemoteBackend;

#[derive(Parser)]
#[command(name = "oryn", version, about = "Oryn Unified CLI")]
struct Args {
    #[command(subcommand)]
    mode: Mode,

    /// Scripts to execute (non-interactive mode)
    #[arg(long)]
    file: Option<String>,
}

#[derive(Subcommand)]
enum Mode {
    /// Use headless browser (Chromium) via CDP
    #[cfg(feature = "chromium")]
    Headless {
        /// Launch browser in visible mode (not headless)
        #[arg(long)]
        visible: bool,
    },
    /// Use embedded browser (WebDriver/COG). Auto-launches COG if no URL provided.
    #[cfg(feature = "embedded")]
    Embedded {
        /// External WebDriver URL (optional - COG auto-launches if not provided)
        #[arg(long)]
        driver_url: Option<String>,
    },
    /// Use remote browser extension via WebSocket
    #[cfg(feature = "user-browser")]
    Remote {
        /// WebSocket port
        #[arg(long, default_value_t = 9001)]
        port: u16,
    },
    /// Use the Oryn2 native runtime with a URL or local HTML document
    Native(Box<NativeArgs>),
}

#[derive(clap::Args)]
struct NativeArgs {
    /// Print machine-readable native runtime build information and exit
    #[arg(long)]
    runtime_info: bool,
    /// Report the explicitly named in-process probe rather than launching
    /// the production worker boundary
    #[arg(long, requires = "runtime_info")]
    in_process_probe: bool,
    /// Local HTML document to load before executing OIL
    #[arg(long, conflicts_with = "url")]
    html: Option<String>,
    /// URL to navigate to before executing OIL
    #[arg(long, conflicts_with = "html")]
    url: Option<String>,
    /// Permit loopback/private test-harness navigation in this context
    #[arg(long)]
    allow_loopback: bool,
    /// Evaluate JavaScript after loading and print its string result
    #[arg(long)]
    evaluate: Option<String>,
    /// Emit REPL prompts even when stdin is a pipe (Python SDK transport)
    #[arg(long, hide = true)]
    sdk_repl: bool,
    /// Record a policy-scrubbed task trace to this JSON file
    #[arg(long)]
    trace_out: Option<String>,
    /// Save a restorable logical context checkpoint to this JSON file
    #[arg(long)]
    checkpoint_out: Option<String>,
    /// Restore a sealed context checkpoint before executing OIL
    #[arg(long, conflicts_with_all = ["html", "url"])]
    restore_checkpoint: Option<String>,
    /// Replay a recorded task trace from the restored checkpoint and exit
    #[arg(
        long,
        requires = "restore_checkpoint",
        conflicts_with = "branch_strategy"
    )]
    replay_trace: Option<String>,
    /// Capture a sealed checkpoint+trace artifact when OIL execution fails
    #[arg(long)]
    failure_artifact_out: Option<String>,
    /// Replay a sealed failed-task regression artifact and exit
    #[arg(long, conflicts_with_all = ["html", "url", "restore_checkpoint", "replay_trace"])]
    replay_failure: Option<String>,
    /// Restore the checkpoint as a named replay branch
    #[arg(long, requires = "restore_checkpoint")]
    branch_strategy: Option<String>,
    /// Capture network response bodies for deterministic replay
    #[arg(long)]
    record_network: bool,
    /// Fixed Date.now value for deterministic execution
    #[arg(long, requires = "random_seed")]
    deterministic_time_ms: Option<i64>,
    /// Seed for Math.random and Web Crypto during deterministic execution
    #[arg(long, requires = "deterministic_time_ms")]
    random_seed: Option<u64>,
    /// Write a deterministic CPU-rendered PNG screenshot (requires `render` feature)
    #[arg(long)]
    screenshot_out: Option<String>,
    /// Write a single-page CPU-rendered PDF (requires `render` feature)
    #[arg(long)]
    pdf_out: Option<String>,
    /// Raster scale; defaults to the page device-pixel ratio
    #[arg(long)]
    render_scale: Option<f64>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to stderr to avoid polluting stdout (used for IPC)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    #[allow(irrefutable_let_patterns)]
    if let Mode::Native(native) = &args.mode {
        let NativeArgs {
            runtime_info,
            in_process_probe,
            html,
            url,
            allow_loopback,
            evaluate,
            sdk_repl,
            trace_out,
            checkpoint_out,
            restore_checkpoint,
            replay_trace,
            failure_artifact_out,
            replay_failure,
            branch_strategy,
            record_network,
            deterministic_time_ms,
            random_seed,
            screenshot_out,
            pdf_out,
            render_scale,
        } = native.as_ref();
        if *runtime_info {
            let runtime_info = if *in_process_probe {
                if !cfg!(feature = "in-process-probe") {
                    return Err(
                        "--in-process-probe requires the explicit in-process-probe build feature"
                            .into(),
                    );
                }
                oryn_native::runtime_build_info()
            } else {
                let options = if *allow_loopback {
                    ContextOptions::loopback_test()
                } else {
                    ContextOptions::default()
                };
                oryn_native::production_runtime_build_info_with_options(options)?
            };
            println!("{}", serde_json::to_string(&runtime_info)?);
            return Ok(());
        }
        return run_native(
            html.as_deref(),
            url.as_deref(),
            *allow_loopback,
            evaluate.as_deref(),
            args.file.as_deref(),
            *sdk_repl,
            NativeSessionOptions {
                trace_out: trace_out.clone(),
                checkpoint_out: checkpoint_out.clone(),
                restore_checkpoint: restore_checkpoint.clone(),
                replay_trace: replay_trace.clone(),
                failure_artifact_out: failure_artifact_out.clone(),
                replay_failure: replay_failure.clone(),
                branch_strategy: branch_strategy.clone(),
                record_network: *record_network,
                determinism: deterministic_time_ms.zip(*random_seed).map(
                    |(unix_time_ms, random_seed)| oryn_native::DeterminismConfig {
                        unix_time_ms,
                        random_seed,
                    },
                ),
                screenshot_out: screenshot_out.clone(),
                pdf_out: pdf_out.clone(),
                render_scale: *render_scale,
            },
        )
        .await;
    }

    #[cfg(feature = "compat-core")]
    let mut backend: Box<dyn Backend> = match args.mode {
        #[cfg(feature = "chromium")]
        Mode::Headless { visible } => Box::new(HeadlessBackend::new_with_visibility(visible)),
        #[cfg(feature = "embedded")]
        Mode::Embedded { driver_url } => match driver_url {
            Some(url) => Box::new(EmbeddedBackend::with_url(url)),
            None => Box::new(EmbeddedBackend::new()),
        },
        #[cfg(feature = "user-browser")]
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
        Mode::Native(_) => unreachable!("native mode returned before backend dispatch"),
    };

    #[cfg(feature = "compat-core")]
    {
        if let Err(e) = backend.launch().await {
            eprintln!("Failed to launch backend: {}", e);
            return Err(e.into());
        }

        let mut executor = CommandExecutor::new();
        let output = OutputHandlers {
            out: |msg| println!("{}", msg),
            err: |msg| println!("{}", msg),
        };
        let repl_options = ReplOptions {
            banner_lines: &[
                "Backend launched. Enter commands (e.g., 'goto google.com', 'scan').",
                "Semantic targets supported: click \"Sign In\", type email \"user@test.com\"",
                "Type 'exit' or 'quit' to close.",
            ],
            prompt: "> ",
            exit_commands: &["exit", "quit"],
            handle_ctrl_c: false,
            ctrl_c_message: None,
        };

        if let Some(file_path) = args.file {
            if let Err(e) = cli::run_file(
                &mut *backend,
                &mut executor,
                output,
                &file_path,
                FileOptions {
                    stop_on_error: true,
                    error_mode: FileErrorMode::WithLine,
                },
            )
            .await
            {
                eprintln!("Error executing file {}: {}", file_path, e);
                return Err(e);
            }
        } else if let Err(e) =
            cli::run_repl(&mut *backend, &mut executor, output, repl_options).await
        {
            eprintln!("Error during session: {}", e);
            return Err(e);
        }

        backend.close().await?;
        Ok(())
    }

    #[cfg(not(feature = "compat-core"))]
    unreachable!("native mode returned before compatibility dispatch")
}

async fn run_native(
    html_path: Option<&str>,
    url: Option<&str>,
    allow_loopback: bool,
    evaluate: Option<&str>,
    oil_path: Option<&str>,
    sdk_repl: bool,
    session_options: NativeSessionOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{IsTerminal, Write};

    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut options = if allow_loopback {
        ContextOptions::loopback_test()
    } else {
        ContextOptions::default()
    };
    options.determinism = session_options.determinism.clone();
    let browser = Browser::new();
    if let Some(artifact_path) = &session_options.replay_failure {
        let artifact: oryn_native::FailureArtifact =
            serde_json::from_slice(&std::fs::read(artifact_path)?)?;
        let (context, replay) = browser.replay_failure_artifact(options, &artifact).await?;
        println!("{}", serde_json::to_string(&replay)?);
        for descriptor in context.pages() {
            if !matches!(descriptor.status, oryn_native::PageStatus::Closed) {
                let _ = context.close_page(descriptor.page.page_id).await;
            }
        }
        return Ok(());
    }
    if let (Some(checkpoint_path), Some(trace_path)) = (
        &session_options.restore_checkpoint,
        &session_options.replay_trace,
    ) {
        let checkpoint: oryn_native::ContextCheckpoint =
            serde_json::from_slice(&std::fs::read(checkpoint_path)?)?;
        let trace: oryn_native::TaskTraceArchive =
            serde_json::from_slice(&std::fs::read(trace_path)?)?;
        let (context, replay) = browser
            .replay_task_trace(options, &checkpoint, &trace)
            .await?;
        println!("{}", serde_json::to_string(&replay)?);
        for descriptor in context.pages() {
            if !matches!(descriptor.status, oryn_native::PageStatus::Closed) {
                let _ = context.close_page(descriptor.page.page_id).await;
            }
        }
        return Ok(());
    }
    let (context, page) = if let Some(checkpoint_path) = &session_options.restore_checkpoint {
        let checkpoint: oryn_native::ContextCheckpoint =
            serde_json::from_slice(&std::fs::read(checkpoint_path)?)?;
        let (context, restored_pages) = if let Some(strategy) = &session_options.branch_strategy {
            let (context, branch) = browser
                .branch_from_checkpoint(options, &checkpoint, strategy.clone())
                .await?;
            (context, branch.restored_pages)
        } else {
            let (context, restore) = browser.restore_checkpoint(options, &checkpoint).await?;
            (context, restore.pages)
        };
        let page_id = context
            .active_page()
            .map(|page| page.page_id())
            .or_else(|| restored_pages.first().map(|page| page.destination_page_id))
            .ok_or("checkpoint restored no pages")?;
        let page = context.activate_page(page_id)?;
        (context, page)
    } else {
        let context = if session_options.record_network {
            browser.new_recording_context(options, true)
        } else {
            browser.new_context_with_options(options)
        };
        let page = context.new_page();
        if let Some(html_path) = html_path {
            let html = std::fs::read_to_string(html_path)?;
            page.load_html(format!("fixture://{html_path}"), html)
                .await?;
        } else if let Some(url) = url {
            page.goto(url).await?;
        } else {
            page.load_html("about:blank", "<!doctype html>").await?;
        }
        (context, page)
    };
    if let Some(source) = evaluate {
        println!("{}", page.evaluate(source).await?);
    }
    let mut session = NativeContextOilSession::new(context.clone());
    let failure_checkpoint = if session_options.failure_artifact_out.is_some() {
        Some(
            context
                .checkpoint(
                    "before-cli-oil",
                    std::collections::BTreeMap::new(),
                    Vec::new(),
                )
                .await?,
        )
    } else {
        None
    };

    let render_requested =
        session_options.screenshot_out.is_some() || session_options.pdf_out.is_some();
    if let Some(oil_path) = oil_path {
        let input = std::fs::read_to_string(oil_path)?;
        match session.execute(&input).await {
            Ok(outputs) => print_native_outputs(outputs)?,
            Err(error) => {
                if let (Some(path), Some(checkpoint)) = (
                    &session_options.failure_artifact_out,
                    failure_checkpoint.as_ref(),
                ) {
                    write_failure_artifact(&context, checkpoint, &error.to_string(), path).await?;
                }
                return Err(error.into());
            }
        }
    } else if !render_requested {
        let interactive = std::io::stdin().is_terminal() || sdk_repl;
        if interactive {
            eprintln!("Native document loaded. Enter OIL; type 'exit' or 'quit' to close.");
        }
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            if interactive {
                print!("> ");
                std::io::stdout().flush()?;
            }
            let Some(line) = lines.next_line().await? else {
                break;
            };
            if matches!(line.trim(), "exit" | "quit") {
                break;
            }
            match session.execute(&line).await {
                Ok(outputs) => print_native_outputs(outputs)?,
                Err(error) => {
                    eprintln!("{error}");
                    if let (Some(path), Some(checkpoint)) = (
                        &session_options.failure_artifact_out,
                        failure_checkpoint.as_ref(),
                    ) {
                        write_failure_artifact(&context, checkpoint, &error.to_string(), path)
                            .await?;
                    }
                }
            }
        }
    }

    if render_requested {
        render_artifacts(&page, &session_options).await?;
    }

    if let Some(path) = &session_options.trace_out {
        let trace = context.trace_archive(Vec::new()).await?;
        std::fs::write(path, serde_json::to_vec_pretty(&trace)?)?;
    }
    if let Some(path) = &session_options.checkpoint_out {
        let checkpoint = context
            .checkpoint("cli", std::collections::BTreeMap::new(), Vec::new())
            .await?;
        std::fs::write(path, serde_json::to_vec_pretty(&checkpoint)?)?;
    }

    for descriptor in context.pages() {
        if !matches!(descriptor.status, oryn_native::PageStatus::Closed) {
            let _ = context.close_page(descriptor.page.page_id).await;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct NativeSessionOptions {
    trace_out: Option<String>,
    checkpoint_out: Option<String>,
    restore_checkpoint: Option<String>,
    replay_trace: Option<String>,
    failure_artifact_out: Option<String>,
    replay_failure: Option<String>,
    branch_strategy: Option<String>,
    record_network: bool,
    determinism: Option<oryn_native::DeterminismConfig>,
    screenshot_out: Option<String>,
    pdf_out: Option<String>,
    render_scale: Option<f64>,
}

#[cfg(feature = "render")]
async fn render_artifacts(
    page: &oryn_native::PageHandle,
    options: &NativeSessionOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = page.paint_snapshot().await?;
    let render_options = oryn_render::RenderOptions {
        scale: options
            .render_scale
            .unwrap_or(snapshot.viewport.device_pixel_ratio),
        ..oryn_render::RenderOptions::default()
    };
    if let Some(path) = &options.screenshot_out {
        let artifact = oryn_render::render_png(&snapshot, render_options)?;
        std::fs::write(path, artifact.bytes)?;
        for diagnostic in artifact.diagnostics {
            eprintln!("render diagnostic: {diagnostic}");
        }
    }
    if let Some(path) = &options.pdf_out {
        let artifact = oryn_render::render_pdf(&snapshot, render_options)?;
        std::fs::write(path, artifact.bytes)?;
        for diagnostic in artifact.diagnostics {
            eprintln!("render diagnostic: {diagnostic}");
        }
    }
    Ok(())
}

#[cfg(not(feature = "render"))]
async fn render_artifacts(
    _page: &oryn_native::PageHandle,
    options: &NativeSessionOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = options.render_scale;
    Err("screenshot/PDF output requires an Oryn build with the optional `render` feature".into())
}

async fn write_failure_artifact(
    context: &oryn_native::BrowserContext,
    checkpoint: &oryn_native::ContextCheckpoint,
    error: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let trace = context.trace_archive(Vec::new()).await?;
    let failure_id = format!(
        "{}-{}",
        &checkpoint.sha256[..checkpoint.sha256.len().min(16)],
        trace.intents.len()
    );
    let artifact = oryn_native::FailureArtifact::seal(oryn_native::FailureArtifactPayload {
        schema_version: oryn_native::TRACE_ARCHIVE_VERSION,
        failure_id,
        checkpoint: checkpoint.clone(),
        trace,
        error: error.into(),
    })?;
    std::fs::write(path, serde_json::to_vec_pretty(&artifact)?)?;
    Ok(())
}

fn print_native_outputs(
    outputs: Vec<oryn_native::oil::NativeOilOutput>,
) -> Result<(), serde_json::Error> {
    for output in outputs {
        println!("{}", serde_json::to_string(&output)?);
    }
    Ok(())
}
