use clap::{Parser, Subcommand};
use lscope_core::backend::Backend;
use lscope_h::backend::HeadlessBackend;
use lscope_e::backend::EmbeddedBackend;
use lscope_r::backend::RemoteBackend;
use std::process::exit;

mod repl;

#[derive(Parser)]
#[command(name = "lscope", version, about = "Lemmascope Unified CLI")]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Use headless browser (Chromium) via CDP
    Headless,
    /// Use embedded browser (WebDriver/COG)
    Embedded {
        /// WebDriver URL
        #[arg(long, default_value = "http://localhost:8080")]
        driver_url: String,
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
        Mode::Embedded { driver_url } => Box::new(EmbeddedBackend::new(driver_url)),
        Mode::Remote { port } => Box::new(RemoteBackend::new(port)),
    };

    let mut backend_ptr = backend;
    if let Err(e) = backend_ptr.launch().await {
        eprintln!("Failed to launch backend: {}", e);
        exit(1);
    }

    if let Err(e) = repl::run_repl(backend_ptr).await {
        eprintln!("Error during session: {}", e);
        exit(1);
    }
}
