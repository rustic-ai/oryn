use clap::{Parser, Subcommand};
use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_h::backend::HeadlessBackend;
use oryn_r::backend::RemoteBackend;
// use std::process::exit;

mod repl;

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

    let backend: Box<dyn Backend> = match args.mode {
        Mode::Headless { visible } => Box::new(HeadlessBackend::new_with_visibility(visible)),
        Mode::Embedded { driver_url } => match driver_url {
            Some(url) => Box::new(EmbeddedBackend::with_url(url)),
            None => Box::new(EmbeddedBackend::new()), // Auto-launch COG
        },
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
    };

    let mut backend = backend;
    if let Err(e) = backend.launch().await {
        eprintln!("Failed to launch backend: {}", e);
        return Err(e.into());
    }

    if let Some(file_path) = args.file {
        if let Err(e) = repl::run_file(backend, &file_path).await {
            eprintln!("Error executing file {}: {}", file_path, e);
            return Err(e.into());
        }
    } else if let Err(e) = repl::run_repl(backend).await {
        eprintln!("Error during session: {}", e);
        return Err(e.into());
    }

    Ok(())
}
