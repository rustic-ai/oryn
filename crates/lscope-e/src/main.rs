use clap::Parser as ClapParser;
use lscope_core::backend::Backend;
use lscope_core::command::Command;
use lscope_core::formatter::format_response;
use lscope_core::parser::Parser;
use lscope_core::translator::translate;
use lscope_e::backend::EmbeddedBackend;
use lscope_e::cog;
use std::io::{self, Write};
use tracing::{error, info};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of the WebDriver server (e.g., http://localhost:8080)
    #[arg(short, long, default_value_t = cog::default_cog_url())]
    webdriver_url: String,

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
    info!("Target WebDriver: {}", args.webdriver_url);

    let mut backend = EmbeddedBackend::new(args.webdriver_url);

    match backend.launch().await {
        Ok(_) => info!("Connected to WebDriver successfully."),
        Err(e) => {
            error!("Failed to connect to WebDriver: {}", e);
            error!("Ensure COG or another WebDriver is running.");
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
