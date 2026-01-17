use oryn_core::backend::Backend;
use oryn_core::command::{Command, Target};
use oryn_core::formatter::format_response;
use oryn_core::parser::Parser;
use oryn_core::protocol::{ScannerData, ScannerProtocolResponse};
use oryn_core::resolver::{resolve_target, ResolutionStrategy, ResolverContext};
use oryn_core::translator::translate;
use std::io::{self, Write};

/// REPL state holding the last scan result for target resolution.
struct ReplState {
    resolver_context: Option<ResolverContext>,
}

impl ReplState {
    fn new() -> Self {
        Self {
            resolver_context: None,
        }
    }

    fn update_from_response(&mut self, resp: &ScannerProtocolResponse) {
        if let ScannerProtocolResponse::Ok { data, .. } = resp {
            if let ScannerData::Scan(scan_result) = data.as_ref() {
                self.resolver_context = Some(ResolverContext::new(scan_result));
            }
        }
    }

    fn get_context(&self) -> Option<&ResolverContext> {
        self.resolver_context.as_ref()
    }
}

/// Resolve targets in a command if needed.
fn resolve_command(cmd: &Command, ctx: &ResolverContext) -> Result<Command, String> {
    match cmd {
        Command::Click(target, opts) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Click(resolved, opts.clone()))
        }
        Command::Type(target, text, opts) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Type(resolved, text.clone(), opts.clone()))
        }
        Command::Clear(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Clear(resolved))
        }
        Command::Check(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Check(resolved))
        }
        Command::Uncheck(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Uncheck(resolved))
        }
        Command::Hover(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Hover(resolved))
        }
        Command::Focus(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Focus(resolved))
        }
        Command::Select(target, value) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Select(resolved, value.clone()))
        }
        Command::Submit(target) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Submit(resolved))
        }
        Command::Scroll(Some(target), opts) => {
            let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                .map_err(|e| e.to_string())?;
            Ok(Command::Scroll(Some(resolved), opts.clone()))
        }
        // Commands without targets pass through
        _ => Ok(cmd.clone()),
    }
}

/// Check if a target needs resolution (is not already an ID).
fn needs_resolution(target: &Target) -> bool {
    !matches!(target, Target::Id(_))
}

/// Check if command has a target that needs resolution.
fn command_needs_resolution(cmd: &Command) -> bool {
    match cmd {
        Command::Click(t, _)
        | Command::Type(t, _, _)
        | Command::Clear(t)
        | Command::Check(t)
        | Command::Uncheck(t)
        | Command::Hover(t)
        | Command::Focus(t)
        | Command::Select(t, _)
        | Command::Submit(t) => needs_resolution(t),
        Command::Scroll(Some(t), _) => needs_resolution(t),
        _ => false,
    }
}

pub async fn run_file(mut backend: Box<dyn Backend>, path: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut state = ReplState::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        execute_line(&mut backend, &mut state, trimmed).await?;
    }

    backend.close().await?;
    Ok(())
}

async fn execute_line(
    backend: &mut Box<dyn Backend>,
    state: &mut ReplState,
    line: &str,
) -> anyhow::Result<()> {
    // 1. Parse Intent Command
    let mut parser = Parser::new(line);
    match parser.parse() {
        Ok(commands) => {
            for cmd in commands {
                execute_command(backend, state, cmd).await?;
            }
        }
        Err(e) => println!("Parse Error: {}", e),
    }
    Ok(())
}

async fn execute_command(
    backend: &mut Box<dyn Backend>,
    state: &mut ReplState,
    cmd: Command,
) -> anyhow::Result<()> {
    // Handle backend-direct commands
    match &cmd {
        Command::GoTo(url) => {
            match backend.navigate(url).await {
                Ok(res) => println!("Navigated to {}", res.url),
                Err(e) => println!("Navigation Error: {}", e),
            }
            return Ok(());
        }
        Command::Back => {
            match backend.go_back().await {
                Ok(res) => println!("Back to {}", res.url),
                Err(e) => println!("Navigation Error: {}", e),
            }
            return Ok(());
        }
        Command::Forward => {
            match backend.go_forward().await {
                Ok(res) => println!("Forward to {}", res.url),
                Err(e) => println!("Navigation Error: {}", e),
            }
            return Ok(());
        }
        Command::Refresh(_) => {
            match backend.refresh().await {
                Ok(res) => println!("Refreshed {}", res.url),
                Err(e) => println!("Refresh Error: {}", e),
            }
            return Ok(());
        }
        Command::Screenshot(_) => {
            match backend.screenshot().await {
                Ok(bytes) => {
                    println!("Screenshot captured ({} bytes)", bytes.len())
                }
                Err(e) => println!("Screenshot Error: {}", e),
            }
            return Ok(());
        }
        Command::Pdf(_) => {
            match backend.pdf().await {
                Ok(bytes) => {
                    println!("PDF captured ({} bytes)", bytes.len())
                }
                Err(e) => println!("PDF Error: {}", e),
            }
            return Ok(());
        }
        Command::Cookies(_) => {
            match backend.get_cookies().await {
                Ok(cookies) => {
                    println!("Cookies ({}):", cookies.len());
                    for c in cookies {
                        println!("  {} = {} (domain: {:?})", c.name, c.value, c.domain);
                    }
                }
                Err(e) => println!("Cookies Error: {}", e),
            }
            return Ok(());
        }
        Command::Tabs(_) => {
            match backend.get_tabs().await {
                Ok(tabs) => {
                    println!("Tabs ({}):", tabs.len());
                    for t in tabs {
                        println!("  - [{}] {} ({})", t.id, t.title, t.url);
                    }
                }
                Err(e) => println!("Tabs Error: {}", e),
            }
            return Ok(());
        }
        Command::Press(key, opts) => {
            let modifiers: Vec<String> = opts
                .iter()
                .filter_map(|(k, v)| if v == "true" { Some(k.clone()) } else { None })
                .collect();
            match backend.press_key(key, &modifiers).await {
                Ok(_) => println!("Pressed {}", key),
                Err(e) => println!("Key Error: {}", e),
            }
            return Ok(());
        }
        _ => {}
    }

    // 2. Resolve semantic targets if needed
    let resolved_cmd = if command_needs_resolution(&cmd) {
        match state.get_context() {
            Some(ctx) => match resolve_command(&cmd, ctx) {
                Ok(resolved) => resolved,
                Err(e) => {
                    println!("Resolution Error: {} (hint: run 'observe' first)", e);
                    return Ok(());
                }
            },
            None => {
                println!("No scan context. Run 'observe' first to enable semantic targeting.");
                return Ok(());
            }
        }
    } else {
        cmd.clone()
    };

    // 3. Translate and execute
    match translate(&resolved_cmd) {
        Ok(req) => match backend.execute_scanner(req).await {
            Ok(resp) => {
                // Update resolver context if this was a scan
                state.update_from_response(&resp);
                let out = format_response(&resp);
                println!("{}", out);
            }
            Err(e) => println!("Backend Error: {}", e),
        },
        Err(e) => println!("Translation Error: {}", e),
    }
    Ok(())
}

pub async fn run_repl(mut backend: Box<dyn Backend>) -> anyhow::Result<()> {
    println!("Backend launched. Enter commands (e.g., 'goto google.com', 'observe').");
    println!("Semantic targets supported: click \"Sign In\", type email \"user@test.com\"");
    println!("Type 'exit' or 'quit' to close.");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut input = String::new();
    let mut state = ReplState::new();

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

        execute_line(&mut backend, &mut state, trimmed).await?;
    }

    backend.close().await?;
    println!("Session closed.");
    Ok(())
}
