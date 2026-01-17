use clap::Parser as ClapParser;
use oryn_core::backend::Backend;
use oryn_core::command::Command;
use oryn_core::formatter::format_response;
use oryn_core::parser::Parser;
use oryn_core::translator::translate;
use oryn_h::backend::HeadlessBackend; // Use from lib
use std::io::{self, Write};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Script file to execute
    #[arg(long)]
    file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    println!("Starting Oryn Headless Backend...");

    let mut backend = HeadlessBackend::new();
    backend.launch().await?;

    if let Some(file_path) = args.file {
        run_file(&mut backend, &file_path).await?;
    } else {
        run_repl(&mut backend).await?;
    }

    backend.close().await?;
    Ok(())
}

async fn run_file(
    backend: &mut HeadlessBackend,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        execute_line(backend, trimmed).await?;
    }
    Ok(())
}

async fn run_repl(backend: &mut HeadlessBackend) -> Result<(), Box<dyn std::error::Error>> {
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
        execute_line(backend, trimmed).await?;
    }
    Ok(())
}

async fn execute_line(
    backend: &mut HeadlessBackend,
    line: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = Parser::new(line);
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
                        if let Err(e) =
                            oryn_h::features::generate_pdf(&client.page, std::path::Path::new(path))
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
    Ok(())
}
