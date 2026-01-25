use oryn_engine::backend::Backend;
use oryn_engine::executor::CommandExecutor;
use std::io::{self, Write};

pub async fn run_repl(mut backend: Box<dyn Backend>) -> anyhow::Result<()> {
    println!("Backend launched. Enter commands (e.g., 'goto google.com', 'scan').");
    println!("Semantic targets supported: click \"Sign In\", type email \"user@test.com\"");
    println!("Type 'exit' or 'quit' to close.");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut input = String::new();
    let mut executor = CommandExecutor::new();

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

        match executor.execute_line(&mut *backend, trimmed).await {
            Ok(res) => {
                println!("{}", res.output);
            }
            Err(e) => {
                // Use default formatting from error
                println!("Error: {}", e);
            }
        }
    }

    backend.close().await?;
    println!("Session closed.");
    Ok(())
}

pub async fn run_file(mut backend: Box<dyn Backend>, path: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut executor = CommandExecutor::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match executor.execute_line(&mut *backend, trimmed).await {
            Ok(res) => {
                println!("{}", res.output);
            }
            Err(e) => {
                println!("Error executing line '{}': {}", trimmed, e);
                return Err(anyhow::anyhow!("Execution failed: {}", e));
            }
        }
    }

    backend.close().await?;
    Ok(())
}
