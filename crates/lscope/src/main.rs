use clap::{Parser, Subcommand};
use lscope_core::backend::Backend;
use lscope_e::backend::EmbeddedBackend;
use lscope_h::backend::HeadlessBackend;
use lscope_r::backend::RemoteBackend;
use std::process::exit;

mod repl;

#[derive(Parser)]
#[command(name = "lscope", version, about = "Lemmascope Unified CLI")]
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
    Headless,
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
async fn main() {
    // Initialize logging if not already done by env logger
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let backend: Box<dyn Backend> = match args.mode {
        Mode::Headless => Box::new(HeadlessBackend::new()),
        Mode::Embedded { driver_url } => match driver_url {
            Some(url) => Box::new(EmbeddedBackend::with_url(url)),
            None => Box::new(EmbeddedBackend::new()), // Auto-launch COG
        },
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
    };

    let mut backend_ptr = backend;
    if let Err(e) = backend_ptr.launch().await {
        eprintln!("Failed to launch backend: {}", e);
        exit(1);
    }

    if let Some(file_path) = args.file {
        if let Err(e) = repl::run_file(backend_ptr, &file_path).await {
            eprintln!("Error executing file {}: {}", file_path, e);
            exit(1);
        }
    } else if let Err(e) = repl::run_repl(backend_ptr).await {
        eprintln!("Error during session: {}", e);
        exit(1);
    }
}
