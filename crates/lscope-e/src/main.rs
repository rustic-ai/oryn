use clap::Parser as ClapParser;
use lscope_core::backend::Backend;
use lscope_core::command::Command;
use lscope_core::formatter::format_response;
use lscope_core::parser::Parser;
use lscope_core::translator::translate;
use lscope_e::backend::EmbeddedBackend;
use std::io::{self, Write};
use tracing::{error, info};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of an external WebDriver server. If not provided, COG will be launched automatically.
    #[arg(short, long)]
    webdriver_url: Option<String>,

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

    info!("Starting Lemmascope Embedded Backend...");

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

    println!(
        "Backend ready. Enter commands (e.g., 'goto google.com', 'scan').Type 'exit' to quit."
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

        // Parse
        let mut parser = Parser::new(trimmed);
        match parser.parse() {
            Ok(commands) => {
                for cmd in commands {
                    // Handle Navigation Directly? Or use backend.navigate only via translator?
                    // Implementation Plan says "Command -> ScannerCommand translation".
                    // But GoTo is not a ScannerCommand. It's a high level command.
                    // Let's handle GoTo explicitly like lscope-h.

                    if let Command::GoTo(url) = &cmd {
                        match backend.navigate(url).await {
                            Ok(res) => println!("Navigated to {}", res.url),
                            Err(e) => error!("Navigation failed: {}", e),
                        }
                        continue;
                    }

                    // Translate to ScannerCommand
                    match translate(&cmd) {
                        Ok(req) => match backend.execute_scanner(req).await {
                            Ok(resp) => {
                                let out = format_response(&resp);
                                println!("{}", out);
                            }
                            Err(e) => error!("Backend Error: {}", e),
                        },
                        Err(e) => error!("Translation Error: {}", e),
                    }
                }
            }
            Err(e) => error!("Parse Error: {}", e),
        }
    }

    backend.close().await?;
    Ok(())
}
