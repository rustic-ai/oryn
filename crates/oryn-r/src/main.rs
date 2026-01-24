use clap::Parser as ClapParser;
use oryn_engine::backend::Backend;
use oryn_engine::executor::CommandExecutor;
use oryn_r::backend::RemoteBackend;
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

    let mut executor = CommandExecutor::new();

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

        match executor.execute_line(&mut backend, trimmed).await {
            Ok(result) => {
                println!("{}", result.output);
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }

    backend.close().await?;
    Ok(())
}
