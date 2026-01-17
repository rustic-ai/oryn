use clap::Parser as ClapParser;
use oryn_core::backend::Backend;
use oryn_core::command::Command;
use oryn_core::formatter::format_response;
use oryn_core::parser::Parser;
use oryn_core::translator::translate;
use oryn_r::backend::RemoteBackend; // Use from lib
use std::io::{self, Write};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 9001)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    println!("Starting Oryn Remote Backend on port {}...", args.port);
    println!(
        "Please connect the browser extension to ws://localhost:{}",
        args.port
    );

    let mut backend = RemoteBackend::new(args.port);
    backend.launch().await?;

    println!("Backend launched. Enter commands (e.g., 'goto google.com', 'scan').");

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

        // 1. Parse Intent Command
        let mut parser = Parser::new(trimmed);
        match parser.parse() {
            Ok(commands) => {
                for cmd in commands {
                    if let Command::GoTo(url) = &cmd {
                        match backend.navigate(url).await {
                            Ok(res) => println!("Navigated to {}", res.url),
                            Err(e) => println!("Error: {}", e),
                        }
                        continue;
                    }

                    match translate(&cmd) {
                        Ok(req) => match backend.execute_scanner(req).await {
                            Ok(resp) => {
                                let out = format_response(&resp);
                                println!("{}", out);
                            }
                            Err(e) => println!("Backend Error: {}", e),
                        },
                        Err(e) => println!("Translation Error: {}", e),
                    }
                }
            }
            Err(e) => println!("Parse Error: {}", e),
        }
    }

    backend.close().await?;
    Ok(())
}
