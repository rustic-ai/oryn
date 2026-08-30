use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oryn-webidl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let command = arguments
        .next()
        .ok_or_else(|| "expected `generate` or `check`".to_string())?;
    let input = arguments
        .next()
        .ok_or_else(|| "expected WebIDL input path".to_string())?;
    let output = arguments
        .next()
        .ok_or_else(|| "expected generated Rust output path".to_string())?;
    if arguments.next().is_some() {
        return Err("unexpected extra arguments".into());
    }
    let source = fs::read_to_string(&input).map_err(|error| error.to_string())?;
    let definitions = oryn_webidl::parse_definitions(&source).map_err(|error| error.to_string())?;
    let generated =
        oryn_webidl::emit_binding_registry(&definitions).map_err(|error| error.to_string())?;
    match command.as_str() {
        "generate" => fs::write(output, generated).map_err(|error| error.to_string()),
        "check" => {
            let existing = fs::read_to_string(output).map_err(|error| error.to_string())?;
            if existing == generated {
                Ok(())
            } else {
                Err("generated bindings are stale; run `oryn-webidl generate`".into())
            }
        }
        _ => Err(format!("unknown command {command:?}")),
    }
}
