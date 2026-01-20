use oryn_core::backend::Backend;
use oryn_core::command::{Command, IntentFilter, ScrollDirection, Target};
use oryn_core::config::loader::ConfigLoader;
use oryn_core::config::schema::OrynConfig;
use oryn_core::formatter::{format_intent_result, format_response_with_intent};
use oryn_core::intent::builtin;
use oryn_core::intent::registry::IntentRegistry;
use oryn_core::pack::manager::PackManager;
use oryn_core::parser::Parser;
use oryn_core::protocol::{
    ScanRequest, ScannerData, ScannerProtocolResponse, ScannerRequest, ScrollRequest,
};
use oryn_core::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use oryn_core::translator::translate;
use serde_json::Value;
use std::io::{self, Write};

/// REPL state holding the last scan result for target resolution.
struct ReplState {
    resolver_context: Option<ResolverContext>,
    pack_manager: PackManager,
    session_intents: oryn_core::intent::session::SessionIntentManager,
    config: OrynConfig,
    observer: oryn_core::learner::observer::Observer,
    recognizer: oryn_core::learner::recognizer::Recognizer,
    proposer: oryn_core::learner::proposer::Proposer,
    pending_proposals: Vec<oryn_core::intent::definition::IntentDefinition>,
}

impl ReplState {
    async fn new() -> Self {
        let mut registry = IntentRegistry::new();
        builtin::register_all(&mut registry);

        // Load config (ignore error for now and use default, or log it)
        let config = ConfigLoader::load_default().await.unwrap_or_else(|e| {
            eprintln!(
                "Warning: Failed to load config, using defaults. Error: {}",
                e
            );
            OrynConfig::default()
        });

        let pack_paths = config.packs.pack_paths.clone();
        let pack_manager = PackManager::new(registry, pack_paths).await;

        let storage = oryn_core::learner::storage::ObservationStorage::new();
        let observer =
            oryn_core::learner::observer::Observer::new(config.learning.clone(), storage);
        let recognizer = oryn_core::learner::recognizer::Recognizer::new(config.learning.clone());
        let proposer = oryn_core::learner::proposer::Proposer::new();

        Self {
            resolver_context: None,
            pack_manager,
            session_intents: oryn_core::intent::session::SessionIntentManager::new(),
            config,
            observer,
            recognizer,
            proposer,
            pending_proposals: Vec::new(),
        }
    }

    fn update_from_response(&mut self, resp: &ScannerProtocolResponse) {
        if let ScannerProtocolResponse::Ok { data, .. } = resp
            && let ScannerData::Scan(scan_result) = data.as_ref()
        {
            self.resolver_context = Some(ResolverContext::new(scan_result));
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
    let mut state = ReplState::new().await;

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
            Ok(())
        }
        Err(e) => {
            println!("Parse Error: {}", e);
            Err(anyhow::anyhow!("Parse Error: {}", e))
        }
    }
}

async fn execute_command(
    backend: &mut Box<dyn Backend>,
    state: &mut ReplState,
    cmd: Command,
) -> anyhow::Result<()> {
    // Handle backend-direct commands
    match &cmd {
        Command::GoTo(url) => {
            // Auto-load pack if configured
            if state.config.packs.auto_load
                && let Some(pack_name) = state.pack_manager.should_auto_load(url)
            {
                if let Err(e) = state.pack_manager.load_pack_by_name(&pack_name).await {
                    // Simplify: Just log error for now, don't fail navigation
                    eprintln!("Warning: Failed to auto-load pack for {}: {}", pack_name, e);
                } else {
                    println!("Auto-loaded pack: {}", pack_name);
                }
            }

            match backend.navigate(url).await {
                Ok(res) => {
                    println!("Navigated to {}", res.url);
                    return Ok(());
                }
                Err(e) => {
                    println!("Navigation Error: {}", e);
                    return Err(anyhow::anyhow!("Navigation Error: {}", e));
                }
            }
        }
        Command::Back => match backend.go_back().await {
            Ok(res) => {
                println!("Back to {}", res.url);
                return Ok(());
            }
            Err(e) => {
                println!("Navigation Error: {}", e);
                return Err(anyhow::anyhow!("Navigation Error: {}", e));
            }
        },
        Command::Forward => match backend.go_forward().await {
            Ok(res) => {
                println!("Forward to {}", res.url);
                return Ok(());
            }
            Err(e) => {
                println!("Navigation Error: {}", e);
                return Err(anyhow::anyhow!("Navigation Error: {}", e));
            }
        },
        Command::Refresh(_) => match backend.refresh().await {
            Ok(res) => {
                println!("Refreshed {}", res.url);
                return Ok(());
            }
            Err(e) => {
                println!("Refresh Error: {}", e);
                return Err(anyhow::anyhow!("Refresh Error: {}", e));
            }
        },
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
        Command::Packs => {
            let packs = state.pack_manager.list_loaded();
            println!("Loaded Packs: {}", packs.len());
            for p in packs {
                println!("- {}v{} ({})", p.pack, p.version, p.description);
            }
            return Ok(());
        }
        Command::Intents(filter) => {
            match filter {
                IntentFilter::Session => {
                    let intents = state.session_intents.list();
                    println!("Session Intents ({}):", intents.len());
                    for intent in intents {
                        println!(
                            "- {} (uses: {})",
                            intent.definition.name, intent.invocation_count
                        );
                    }
                }
                IntentFilter::All => {
                    let registry = state.pack_manager.registry();
                    let mut intents = registry.list();
                    // Sort for stable output
                    intents.sort_by_key(|a| &a.name);

                    println!("Registered Intents ({}):", intents.len());
                    for intent in intents {
                        println!(
                            "- {} (v{}, tier: {:?})",
                            intent.name, intent.version, intent.tier
                        );
                    }
                }
            }
            return Ok(());
        }
        Command::PackLoad(name) => {
            match state.pack_manager.load_pack_by_name(name).await {
                Ok(_) => println!("Loaded pack: {}", name),
                Err(e) => println!("Error loading pack: {}", e),
            }
            return Ok(());
        }
        Command::PackUnload(name) => {
            match state.pack_manager.unload_pack(name) {
                Ok(_) => println!("Unloaded pack: {}", name),
                Err(e) => println!("Error unloading pack: {}", e),
            }
            return Ok(());
        }
        Command::Define(body) => {
            // Parse definition
            match oryn_core::intent::define_parser::parse_define(body) {
                Ok(def) => {
                    let name = def.name.clone();
                    // Add to session manager
                    if let Err(e) = state.session_intents.define(def.clone()) {
                        println!("Error defining session intent: {}", e);
                        return Ok(());
                    }
                    // Register in main registry for resolution
                    state.pack_manager.registry_mut().register(def);
                    println!("Defined session intent: {}", name);
                }
                Err(e) => println!("Definition Error: {}", e),
            }
            return Ok(());
        }
        Command::Undefine(name) => {
            if let Err(e) = state.session_intents.undefine(name) {
                println!("Error undefining: {}", e);
            }
            if state.pack_manager.registry_mut().unregister(name) {
                println!("Undefined session intent: {}", name);
            } else {
                println!(
                    "Intent '{}' not found in registry (or failed to remove)",
                    name
                );
            }
            return Ok(());
        }
        Command::Export(name, path) => {
            match state
                .session_intents
                .export(name, std::path::Path::new(path))
            {
                Ok(_) => println!("Exported '{}' to {}", name, path),
                Err(e) => println!("Export Error: {}", e),
            }
            return Ok(());
        }
        Command::RunIntent(name, params_map) => {
            let params: std::collections::HashMap<String, Value> = params_map
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.to_string())))
                .collect();

            let verifier = oryn_core::intent::verifier::Verifier::new();

            // dereference Box<dyn Backend> to mutable reference to trait object
            let backend_ref = backend.as_mut();

            let mut executor = oryn_core::intent::executor::IntentExecutor::new(
                backend_ref,
                state.pack_manager.registry(),
                &verifier,
            );

            match executor.execute(name, params).await {
                Ok(result) => {
                    println!("{}", format_intent_result(&result, name));
                }
                Err(e) => {
                    println!("❌ Intent execution failed: {}", e);
                }
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
        Command::ScrollUntil(target, direction, options) => {
            let max_iterations: usize = options
                .get("max")
                .and_then(|v| v.parse().ok())
                .unwrap_or(10);

            let scroll_dir = match direction {
                ScrollDirection::Up => oryn_core::protocol::ScrollDirection::Up,
                ScrollDirection::Down => oryn_core::protocol::ScrollDirection::Down,
                ScrollDirection::Left => oryn_core::protocol::ScrollDirection::Left,
                ScrollDirection::Right => oryn_core::protocol::ScrollDirection::Right,
            };

            for iteration in 1..=max_iterations {
                // 1. Scroll in the specified direction
                let scroll_req = ScannerRequest::Scroll(ScrollRequest {
                    id: None,
                    direction: scroll_dir.clone(),
                    amount: Some("page".to_string()),
                });

                if let Err(e) = backend.execute_scanner(scroll_req).await {
                    println!("Scroll Error: {}", e);
                    return Ok(());
                }

                // 2. Observe/scan the page
                let scan_req = ScannerRequest::Scan(ScanRequest::default());
                match backend.execute_scanner(scan_req).await {
                    Ok(resp) => {
                        // Update resolver context
                        state.update_from_response(&resp);

                        // 3. Check if target is now visible
                        if let Some(ctx) = state.get_context() {
                            match resolve_target(target, ctx, ResolutionStrategy::First) {
                                Ok(Target::Id(id)) => {
                                    println!("Found target [{}] after {} scroll(s)", id, iteration);
                                    return Ok(());
                                }
                                _ => {
                                    // Target not found yet, continue scrolling
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("Scan Error: {}", e);
                        return Ok(());
                    }
                }
            }

            println!(
                "Target not found after {} scrolls (hint: try increasing --max)",
                max_iterations
            );
            return Ok(());
        }

        Command::Learn(action) => {
            if !state.config.learning.enabled {
                println!("Learning is disabled. Enable it in config. (learning.enabled = true)");
                return Ok(());
            }
            use oryn_core::command::LearnAction;
            match action {
                LearnAction::Status => {
                    // 1. Get history for current domain
                    let domain = if let Some(ctx) = state.get_context() {
                        if let Ok(url) = url::Url::parse(ctx.url()) {
                            url.host_str().unwrap_or("unknown").to_string()
                        } else {
                            "unknown".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    };

                    if domain == "unknown" {
                        println!("No active domain context. Run 'observe'.");
                        return Ok(());
                    }

                    let history = state.observer.get_history(&domain);
                    if !history.is_empty() {
                        println!(
                            "History for {}: {} actions recorded.",
                            domain,
                            history.len()
                        );

                        // 2. Find patterns
                        let patterns = state.recognizer.find_patterns(&history);
                        println!("Found {} potential intent pattern(s).", patterns.len());

                        // 3. Propose intents
                        state.pending_proposals.clear();
                        for (i, p) in patterns.iter().enumerate() {
                            if let Some(intent) = state.proposer.propose(p) {
                                println!(
                                    "[{}] {} ({} steps, occurrences: {})",
                                    i,
                                    intent.name,
                                    intent.steps.len(),
                                    p.occurrence_count
                                );
                                for step in &intent.steps {
                                    println!("  - {:?}", step);
                                }
                                state.pending_proposals.push(intent);
                            } else {
                                println!("[{}] (Failed to propose intent from pattern)", i);
                            }
                        }
                    } else {
                        println!("No history for domain: {}", domain);
                    }
                }
                LearnAction::Refine(name) => println!("Refine not implemented yet ({})", name),
                LearnAction::Save(name) => {
                    if state.pending_proposals.is_empty() {
                        println!("No pending proposals. Run 'learn status' first.");
                        return Ok(());
                    }

                    // Simple logic: save the first one or match name?
                    // Name provided by user is the DESIRED name.
                    // The proposals have generated names like "intent_3".
                    // So we take the first/best proposal and rename it to `name`.
                    // Or we let user specify ID? "save <id> <name>"?
                    // Parser supports "save <string>".
                    // Let's assume we save the TOP proposal as `name`.

                    if let Some(mut intent) = state.pending_proposals.first().cloned() {
                        // Clone to modify
                        intent.name = name.clone();

                        // Save
                        let domain = intent
                            .triggers
                            .urls
                            .first()
                            .cloned()
                            .unwrap_or("unknown".to_string());

                        // Save as session intent
                        match state.session_intents.define(intent) {
                            Ok(_) => println!(
                                "Saved intent '{}' for domain '{}' (Session)",
                                name, domain
                            ),
                            Err(e) => println!("Error saving intent: {}", e),
                        }
                    }
                }
                LearnAction::Ignore(name) => println!("Ignore not implemented yet ({})", name),
            }
            return Ok(());
        }
        _ => {}
    }

    // Record observation if learning enabled and context available
    if state.config.learning.enabled
        && let Some(ctx) = state.get_context()
        && let Ok(url) = url::Url::parse(ctx.url())
        && let Some(domain) = url.host_str()
    {
        // Convert command to string for MVP
        state
            .observer
            .record(domain, url.as_str(), &format!("{:?}", cmd));
    }

    // 2. Resolve semantic targets if needed
    let resolved_cmd = if command_needs_resolution(&cmd) {
        match state.get_context() {
            Some(ctx) => match resolve_command(&cmd, ctx) {
                Ok(resolved) => resolved,
                Err(e) => {
                    let msg = format!("Resolution Error: {} (hint: run 'observe' first)", e);
                    println!("{}", msg);
                    return Err(anyhow::anyhow!("{}", msg));
                }
            },
            None => {
                let msg = "No scan context. Run 'observe' first to enable semantic targeting.";
                println!("{}", msg);
                return Err(anyhow::anyhow!("{}", msg));
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
                let out = format_response_with_intent(&resp, Some(state.pack_manager.registry()));
                println!("{}", out);
                Ok(())
            }
            Err(e) => {
                println!("Backend Error: {}", e);
                Err(anyhow::anyhow!("Backend Error: {}", e))
            }
        },
        Err(e) => {
            println!("Translation Error: {}", e);
            Err(anyhow::anyhow!("Translation Error: {}", e))
        }
    }
}

pub async fn run_repl(mut backend: Box<dyn Backend>) -> anyhow::Result<()> {
    println!("Backend launched. Enter commands (e.g., 'goto google.com', 'observe').");
    println!("Semantic targets supported: click \"Sign In\", type email \"user@test.com\"");
    println!("Type 'exit' or 'quit' to close.");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut input = String::new();
    let mut state = ReplState::new().await;

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
