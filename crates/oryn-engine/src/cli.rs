use crate::backend::Backend;
use crate::executor::CommandExecutor;
use std::error::Error;
use std::io::{self, Write};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Clone, Copy)]
pub struct OutputHandlers {
    pub out: fn(&str),
    pub err: fn(&str),
}

pub enum FileErrorMode {
    Plain,
    WithLine,
}

pub struct FileOptions {
    pub stop_on_error: bool,
    pub error_mode: FileErrorMode,
}

pub struct ReplOptions<'a> {
    pub banner_lines: &'a [&'a str],
    pub prompt: &'a str,
    pub exit_commands: &'a [&'a str],
    pub handle_ctrl_c: bool,
    pub ctrl_c_message: Option<&'a str>,
}

async fn execute_line<B: Backend + ?Sized>(
    backend: &mut B,
    executor: &mut CommandExecutor,
    line: &str,
) -> Result<String, String> {
    match executor.execute_line(backend, line).await {
        Ok(result) => Ok(result.output),
        Err(e) => Err(format!("{}", e)),
    }
}

pub async fn run_file<B: Backend + ?Sized>(
    backend: &mut B,
    executor: &mut CommandExecutor,
    output: OutputHandlers,
    path: &str,
    options: FileOptions,
) -> Result<(), Box<dyn Error>> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match execute_line(backend, executor, trimmed).await {
            Ok(result) => (output.out)(&result),
            Err(err) => {
                match options.error_mode {
                    FileErrorMode::Plain => (output.err)(&format!("Error: {}", err)),
                    FileErrorMode::WithLine => {
                        (output.err)(&format!("Error executing line '{}': {}", trimmed, err))
                    }
                }
                if options.stop_on_error {
                    return Err(io::Error::other(err).into());
                }
            }
        }
    }
    Ok(())
}

/// Possible outcomes from reading a single REPL line.
enum ReadLineResult {
    /// A non-empty input line to process.
    Input(String),
    /// Empty line or no input yet -- skip and re-prompt.
    Skip,
    /// EOF or exit command -- terminate the loop.
    Exit,
    /// I/O error while reading.
    Error(io::Error),
}

async fn read_line(
    reader: &mut tokio::io::Lines<BufReader<tokio::io::Stdin>>,
    exit_commands: &[&str],
    handle_ctrl_c: bool,
    ctrl_c_message: Option<&str>,
    output: OutputHandlers,
) -> ReadLineResult {
    if handle_ctrl_c {
        tokio::select! {
            line = reader.next_line() => {
                classify_line(line, exit_commands)
            }
            _ = tokio::signal::ctrl_c() => {
                if let Some(message) = ctrl_c_message {
                    (output.out)(message);
                }
                ReadLineResult::Exit
            }
        }
    } else {
        classify_line(reader.next_line().await, exit_commands)
    }
}

fn classify_line(
    result: Result<Option<String>, io::Error>,
    exit_commands: &[&str],
) -> ReadLineResult {
    match result {
        Ok(Some(input)) => {
            let trimmed = input.trim().to_string();
            if trimmed.is_empty() {
                ReadLineResult::Skip
            } else if exit_commands.contains(&trimmed.as_str()) {
                ReadLineResult::Exit
            } else {
                ReadLineResult::Input(trimmed)
            }
        }
        Ok(None) => ReadLineResult::Exit,
        Err(e) => ReadLineResult::Error(e),
    }
}

pub async fn run_repl<B: Backend + ?Sized>(
    backend: &mut B,
    executor: &mut CommandExecutor,
    output: OutputHandlers,
    options: ReplOptions<'_>,
) -> Result<(), Box<dyn Error>> {
    for line in options.banner_lines {
        (output.out)(line);
    }

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = io::stdout();

    loop {
        print!("{}", options.prompt);
        stdout.flush()?;

        match read_line(
            &mut reader,
            options.exit_commands,
            options.handle_ctrl_c,
            options.ctrl_c_message,
            output,
        )
        .await
        {
            ReadLineResult::Input(line) => match execute_line(backend, executor, &line).await {
                Ok(result) => (output.out)(&result),
                Err(err) => (output.err)(&format!("Error: {}", err)),
            },
            ReadLineResult::Skip => continue,
            ReadLineResult::Exit => break,
            ReadLineResult::Error(e) => return Err(e.into()),
        }
    }
    Ok(())
}
