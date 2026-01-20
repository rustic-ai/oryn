use clap::Parser as ClapParser;
use oryn_core::backend::Backend;
use oryn_core::executor::CommandExecutor;
use oryn_e::backend::EmbeddedBackend;
use std::io::{self, Write};
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

    if let Some(file_path) = args.file {
        run_file(&mut backend, &mut executor, &file_path).await?;
    } else {
        run_repl(&mut backend, &mut executor).await?;
    }

    backend.close().await?;
    Ok(())
}

async fn run_file(
    backend: &mut EmbeddedBackend,
    executor: &mut CommandExecutor,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        execute_line(backend, executor, trimmed).await?;
    }
    Ok(())
}

async fn run_repl(
    backend: &mut EmbeddedBackend,
    executor: &mut CommandExecutor,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Backend ready. Enter commands (e.g., 'goto google.com', 'scan'). Type 'exit' to quit."
    );
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut input = String::new();

    loop {
        print!("> ");
        stdout.flush()?;
        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "exit" || trimmed == "quit" {
            break;
        }
        execute_line(backend, executor, trimmed).await?;
    }
    Ok(())
}

async fn execute_line(
    backend: &mut EmbeddedBackend,
    executor: &mut CommandExecutor,
    line: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match executor.execute_line(backend, line).await {
        Ok(result) => {
            println!("{}", result.output);
            Ok(())
        }
        Err(e) => {
            error!("Error: {}", e);
            Err(format!("{}", e).into())
        }
    }
}
