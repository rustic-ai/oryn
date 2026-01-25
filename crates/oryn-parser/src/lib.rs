pub mod ast;
pub mod normalizer;
pub mod parser;
pub mod translator;

pub use ast::*;
pub use normalizer::normalize;
pub use oryn_common::resolver;
pub use parser::{parse, OilParser, Rule};

use crate::translator::{translate, TranslationError};
use oryn_common::protocol::Action;
use oryn_common::resolver::{resolve_target, ResolutionStrategy, ResolverContext, ResolverError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("Parse error: {0}")]
    Parse(#[from] parser::ParseError),
    #[error("Resolution error: {0}")]
    Resolution(#[from] ResolverError),
    #[error("Translation error: {0}")]
    Translate(#[from] TranslationError),
}

/// Process OIL input through the full pipeline: normalize, parse, resolve, and translate.
pub fn process(input: &str, ctx: &ResolverContext) -> Result<Vec<Action>, ProcessError> {
    let normalized = normalize(input);
    let script = parse(&normalized)?;

    let mut actions = Vec::new();
    for line in script.lines {
        if let Some(mut cmd) = line.command {
            resolve_command_targets(&mut cmd, ctx)?;
            let action = translate(&cmd)?;
            actions.push(action);
        }
    }
    Ok(actions)
}

fn resolve_with_strategy(
    target: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    let resolver_target = target.to_resolver_target();
    let resolved = resolve_target(&resolver_target, ctx, strategy)?;
    Ok(Target::from_resolver_target(&resolved))
}

fn resolve(target: &Target, ctx: &ResolverContext) -> Result<Target, ResolverError> {
    resolve_with_strategy(target, ctx, ResolutionStrategy::First)
}

fn resolve_optional(
    target: &mut Option<Target>,
    ctx: &ResolverContext,
) -> Result<(), ResolverError> {
    if let Some(t) = target {
        *t = resolve(t, ctx)?;
    }
    Ok(())
}

fn resolve_command_targets(cmd: &mut Command, ctx: &ResolverContext) -> Result<(), ResolverError> {
    use ResolutionStrategy::*;

    match cmd {
        Command::Text(c) => resolve_optional(&mut c.target, ctx)?,
        Command::Screenshot(c) => resolve_optional(&mut c.target, ctx)?,
        Command::Box(c) => c.target = resolve(&c.target, ctx)?,
        Command::Click(c) => c.target = resolve_with_strategy(&c.target, ctx, PreferClickable)?,
        Command::Type(c) => c.target = resolve_with_strategy(&c.target, ctx, PreferInput)?,
        Command::Clear(c) => c.target = resolve(&c.target, ctx)?,
        Command::Select(c) => c.target = resolve(&c.target, ctx)?,
        Command::Check(c) => c.target = resolve_with_strategy(&c.target, ctx, PreferCheckable)?,
        Command::Uncheck(c) => c.target = resolve_with_strategy(&c.target, ctx, PreferCheckable)?,
        Command::Hover(c) => c.target = resolve(&c.target, ctx)?,
        Command::Focus(c) => c.target = resolve(&c.target, ctx)?,
        Command::Scroll(c) => resolve_optional(&mut c.target, ctx)?,
        Command::Submit(c) => resolve_optional(&mut c.target, ctx)?,
        Command::ScrollUntil(c) => c.target = resolve(&c.target, ctx)?,

        Command::Wait(c) => {
            if let WaitCondition::Visible(t) | WaitCondition::Hidden(t) = &mut c.condition {
                let needs_resolution = t.relation.is_some()
                    || matches!(t.atomic, TargetAtomic::Id(_) | TargetAtomic::Role(_));
                if needs_resolution {
                    *t = resolve(t, ctx)?;
                }
            }
        }

        Command::Frame(c) => {
            if let FrameTarget::Target(t) = &mut c.target {
                *t = resolve(t, ctx)?;
            }
        }

        _ => {}
    }
    Ok(())
}
