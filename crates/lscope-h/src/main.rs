use clap::Parser as ClapParser;
use lscope_core::backend::Backend;
use lscope_core::command::Command;
use lscope_core::formatter::format_response;
use lscope_core::parser::Parser;
use lscope_core::translator::translate;
use lscope_h::backend::HeadlessBackend; // Use from lib
use std::io::{self, Write};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Add flags if needed, e.g. --debug
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let _args = Args::parse();
    println!("Starting Lemmascope Headless Backend...");

    let mut backend = HeadlessBackend::new();
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

                    if let Command::Pdf(path) = &cmd {
                        if let Some(client) = backend.get_client() {
                            println!("Generating PDF to {}...", path);
                            if let Err(e) = lscope_h::features::generate_pdf(
                                &client.page,
                                std::path::Path::new(path),
                            )
                            .await
                            {
                                println!("Error generating PDF: {}", e);
                            } else {
                                println!("PDF generated successfully.");
                            }
                        } else {
                            println!("Error: Backend not ready");
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
