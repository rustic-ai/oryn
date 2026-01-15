use lscope_core::backend::Backend;
use lscope_core::command::Command;
use lscope_core::formatter::format_response;
use lscope_core::parser::Parser;
use lscope_core::translator::translate;
use std::io::{self, Write};

pub async fn run_repl(mut backend: Box<dyn Backend>) -> anyhow::Result<()> {
    println!("Backend launched. Enter commands (e.g., 'goto google.com', 'scan').");
    println!("Type 'exit' or 'quit' to close.");

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
                    // Handle special backend-specific commands if we can, or just generic navigation
                    if let Command::GoTo(url) = &cmd {
                        match backend.navigate(url).await {
                            Ok(res) => println!("Navigated to {}", res.url),
                            Err(e) => println!("Navigation Error: {}", e),
                        }
                        continue;
                    }

                     // PDF handling is backend specific and currently not in the trait
                     // We could downcast if we really needed to, but for now we skip specific features
                     // or implementing them in the generic Backend trait is better.
                     // For Phase 8.1, we omit PDF support in unified REPL unless we add it to trait.

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
    println!("Session closed.");
    Ok(())
}
