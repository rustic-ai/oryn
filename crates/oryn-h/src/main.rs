use clap::Parser as ClapParser;
use oryn_engine::backend::Backend;
use oryn_engine::command::Command;
use oryn_engine::executor::CommandExecutor;
use oryn_engine::parser::Parser;
use oryn_h::backend::HeadlessBackend;
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
    backend: &mut HeadlessBackend,
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
    backend: &mut HeadlessBackend,
    executor: &mut CommandExecutor,
) -> Result<(), Box<dyn std::error::Error>> {
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
        execute_line(backend, executor, trimmed).await?;
    }
    Ok(())
}

async fn execute_line(
    backend: &mut HeadlessBackend,
    executor: &mut CommandExecutor,
    line: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check for PDF command which requires special handling for headless backend
    let mut parser = Parser::new(line);
    if let Ok(commands) = parser.parse() {
        for cmd in &commands {
            if let Command::Pdf(path) = cmd {
                if let Some(client) = backend.get_client() {
                    println!("Generating PDF to {}...", path);
                    if let Err(e) =
                        oryn_h::features::generate_pdf(&client.page, std::path::Path::new(path))
                            .await
                    {
                        println!("Error generating PDF: {}", e);
                        return Err(format!("PDF Error: {}", e).into());
                    } else {
                        println!("PDF generated successfully.");
                    }
                } else {
                    println!("Error: Backend not ready");
                    return Err("Backend not ready".into());
                }
                return Ok(());
            }
        }
    }

    // Use the shared executor for all other commands
    match executor.execute_line(backend, line).await {
        Ok(result) => {
            println!("{}", result.output);
            Ok(())
        }
        Err(e) => {
            println!("Error: {}", e);
            Err(format!("{}", e).into())
        }
    }
}
