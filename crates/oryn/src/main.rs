use clap::{Parser, Subcommand};
use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_engine::cli::{self, FileErrorMode, FileOptions, OutputHandlers, ReplOptions};
use oryn_engine::executor::CommandExecutor;
use oryn_h::backend::HeadlessBackend;
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to stderr to avoid polluting stdout (used for IPC)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    let mut backend: Box<dyn Backend> = match args.mode {
        Mode::Headless { visible } => Box::new(HeadlessBackend::new_with_visibility(visible)),
        Mode::Embedded { driver_url } => match driver_url {
            Some(url) => Box::new(EmbeddedBackend::with_url(url)),
            None => Box::new(EmbeddedBackend::new()),
        },
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
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
