use clap::{Parser, Subcommand};
use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_engine::cli::{self, FileErrorMode, FileOptions, OutputHandlers, ReplOptions};
use oryn_engine::executor::CommandExecutor;
use oryn_h::backend::HeadlessBackend;
use oryn_native::{Browser, ContextOptions, oil::NativeOilSession};
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
    Headless {
        /// Launch browser in visible mode (not headless)
        #[arg(long)]
        visible: bool,
    },
    /// Use embedded browser (WebDriver/COG). Auto-launches COG if no URL provided.
    Embedded {
        /// External WebDriver URL (optional - COG auto-launches if not provided)
        #[arg(long)]
        driver_url: Option<String>,
    },
    /// Use remote browser extension via WebSocket
    Remote {
        /// WebSocket port
        #[arg(long, default_value_t = 9001)]
        port: u16,
    },
    /// Use the Oryn2 native runtime with a URL or local HTML document
    Native {
        /// Print machine-readable native runtime build information and exit
        #[arg(long)]
        runtime_info: bool,
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
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to stderr to avoid polluting stdout (used for IPC)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    if let Mode::Native {
        runtime_info,
        html,
        url,
        allow_loopback,
        evaluate,
    } = &args.mode
    {
        if *runtime_info {
            println!(
                "{}",
                serde_json::to_string(&oryn_native::runtime_build_info())?
            );
            return Ok(());
        }
        return run_native(
            html.as_deref(),
            url.as_deref(),
            *allow_loopback,
            evaluate.as_deref(),
            args.file.as_deref(),
        )
        .await;
    }

    let mut backend: Box<dyn Backend> = match args.mode {
        Mode::Headless { visible } => Box::new(HeadlessBackend::new_with_visibility(visible)),
        Mode::Embedded { driver_url } => match driver_url {
            Some(url) => Box::new(EmbeddedBackend::with_url(url)),
            None => Box::new(EmbeddedBackend::new()),
        },
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
        Mode::Native { .. } => unreachable!("native mode returned before backend dispatch"),
    };

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
    } else if let Err(e) = cli::run_repl(&mut *backend, &mut executor, output, repl_options).await {
        eprintln!("Error during session: {}", e);
        return Err(e);
    }

    backend.close().await?;
    Ok(())
}

async fn run_native(
    html_path: Option<&str>,
    url: Option<&str>,
    allow_loopback: bool,
    evaluate: Option<&str>,
    oil_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    use tokio::io::{AsyncBufReadExt, BufReader};

    let options = if allow_loopback {
        ContextOptions::loopback_test()
    } else {
        ContextOptions::default()
    };
    let page = Browser::new().new_context_with_options(options).new_page();
    if let Some(html_path) = html_path {
        let html = std::fs::read_to_string(html_path)?;
        page.load_html(format!("fixture://{html_path}"), html)
            .await?;
    } else if let Some(url) = url {
        page.goto(url).await?;
    } else {
        page.load_html("about:blank", "<!doctype html>").await?;
    }
    if let Some(source) = evaluate {
        println!("{}", page.evaluate(source).await?);
    }
    let mut session = NativeOilSession::new(page.clone());

    if let Some(oil_path) = oil_path {
        let input = std::fs::read_to_string(oil_path)?;
        print_native_outputs(session.execute(&input).await?)?;
    } else {
        eprintln!("Native document loaded. Enter OIL; type 'exit' or 'quit' to close.");
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            print!("> ");
            std::io::stdout().flush()?;
            let Some(line) = lines.next_line().await? else {
                break;
            };
            if matches!(line.trim(), "exit" | "quit") {
                break;
            }
            match session.execute(&line).await {
                Ok(outputs) => print_native_outputs(outputs)?,
                Err(error) => eprintln!("{error}"),
            }
        }
    }

    page.close().await?;
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
