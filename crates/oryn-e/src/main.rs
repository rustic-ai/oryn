use clap::Parser as ClapParser;
use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_engine::cli::{self, FileErrorMode, FileOptions, OutputHandlers, ReplOptions};
use oryn_engine::executor::CommandExecutor;
use tracing::{error, info};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of an external WebDriver server. If not provided, COG will be launched automatically.
    #[arg(short, long)]
    webdriver_url: Option<String>,

    /// Script file to execute
    #[arg(short, long)]
    file: Option<String>,

    /// Verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize logging
    let filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting Oryn Embedded Backend...");

    let mut backend = if let Some(url) = args.webdriver_url {
        info!("Using external WebDriver at {}", url);
        EmbeddedBackend::with_url(url)
    } else {
        info!("Auto-launching COG browser...");
        EmbeddedBackend::new()
    };

    match backend.launch().await {
        Ok(_) => info!("Backend ready."),
        Err(e) => {
            error!("Failed to launch: {}", e);
            std::process::exit(1);
        }
    }

    let mut executor = CommandExecutor::new();
    let output = OutputHandlers {
        out: |msg| println!("{}", msg),
        err: |msg| error!("{}", msg),
    };
    let repl_options = ReplOptions {
        banner_lines: &[
            "Backend ready. Enter commands (e.g., 'goto google.com', 'scan'). Type 'exit' to quit or Ctrl+C to shutdown.",
        ],
        prompt: "> ",
        exit_commands: &["exit", "quit"],
        handle_ctrl_c: true,
        ctrl_c_message: Some("\nShutdown signal received."),
    };

    if let Some(file_path) = args.file {
        cli::run_file(
            &mut backend,
            &mut executor,
            output,
            &file_path,
            FileOptions {
                stop_on_error: true,
                error_mode: FileErrorMode::Plain,
            },
        )
        .await?;
    } else {
        cli::run_repl(&mut backend, &mut executor, output, repl_options).await?;
    }

    backend.close().await?;
    Ok(())
}
