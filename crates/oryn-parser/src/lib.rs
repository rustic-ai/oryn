pub mod ast;
pub mod normalizer;
pub mod parser;
pub mod resolver;
pub mod translator;

pub use ast::*;
pub use normalizer::normalize;
pub use parser::{parse, OilParser, Rule};

use crate::resolver::{ResolverContext, resolve_target, ResolverError, ResolutionStrategy};
use crate::translator::{translate, TranslationError};
use oryn_common::protocol::Action;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("Parse error: {0}")]
    Parse(#[from] parser::ParseError),
    // Normalization is infallible currently (returns String)
    #[error("Resolution error: {0}")]
    Resolution(#[from] ResolverError),
    #[error("Translation error: {0}")]
    Translate(#[from] TranslationError),
}

/// Helper to configure resolution strategy based on command type (optional).
fn get_strategy_for_cmd(_cmd: &Command) -> ResolutionStrategy {
    // defaults to First
    ResolutionStrategy::First
}

pub fn process(input: &str, ctx: &ResolverContext) -> Result<Vec<Action>, ProcessError> {
    // 1. Normalize (String level)
    let normalized = normalize(input);
    
    // 2. Parse
    let script = parse(&normalized)?;
    
    let mut actions = Vec::new();
    for line in script.lines {
        if let Some(mut cmd) = line.command {
            // 3. Resolve
            let strategy = get_strategy_for_cmd(&cmd);
            resolve_command_targets(&mut cmd, ctx, strategy)?;
            
            // 4. Translate
            let action = translate(&cmd)?;
            actions.push(action);
        }
    }
    Ok(actions)
}

fn resolve_command_targets(cmd: &mut Command, ctx: &ResolverContext, strategy: ResolutionStrategy) -> Result<(), ResolverError> {
    match cmd {
        Command::Text(c) => if let Some(t) = &mut c.target { *t = resolve_target(t, ctx, strategy)?; },
        Command::Screenshot(c) => if let Some(t) = &mut c.target { *t = resolve_target(t, ctx, strategy)?; },
        Command::Box(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        
        Command::Click(c) => c.target = resolve_target(&c.target, ctx, ResolutionStrategy::PreferClickable)?, // Special strategy?
        Command::Type(c) => c.target = resolve_target(&c.target, ctx, ResolutionStrategy::PreferInput)?,
        Command::Clear(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        Command::Select(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        Command::Check(c) => c.target = resolve_target(&c.target, ctx, ResolutionStrategy::PreferCheckable)?,
        Command::Uncheck(c) => c.target = resolve_target(&c.target, ctx, ResolutionStrategy::PreferCheckable)?,
        Command::Hover(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        Command::Focus(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        Command::Scroll(c) => if let Some(t) = &mut c.target { *t = resolve_target(t, ctx, strategy)?; },
        Command::Submit(c) => if let Some(t) = &mut c.target { *t = resolve_target(t, ctx, strategy)?; },
        
        Command::Wait(c) => match &mut c.condition {
            WaitCondition::Visible(t) => *t = resolve_target(t, ctx, strategy)?,
            WaitCondition::Hidden(t) => *t = resolve_target(t, ctx, strategy)?,
            _ => {}
        },
        
        Command::ScrollUntil(c) => c.target = resolve_target(&c.target, ctx, strategy)?,
        
        // FrameSwitchCmd - FrameTarget
        Command::Frame(c) => {
            if let crate::ast::FrameTarget::Target(t) = &mut c.target {
                *t = resolve_target(t, ctx, strategy)?;
            }
        },
        
        // Commands without targets or string targets that don't need resolution
        _ => {}
    }
    Ok(())
}
