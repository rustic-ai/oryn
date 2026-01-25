use super::TargetRequirement;
use oryn_parser::ast::Command;

/// Metadata about a command's resolution requirements.
pub struct CommandMeta {
    /// What kind of target this command needs
    pub requirement: TargetRequirement,
    /// Whether the target can be inferred if missing
    pub allows_inference: bool,
}

impl CommandMeta {
    fn new(requirement: TargetRequirement, allows_inference: bool) -> Self {
        Self {
            requirement,
            allows_inference,
        }
    }

    pub fn for_command(cmd: &Command) -> Self {
        use TargetRequirement::*;

        match cmd {
            Command::Click(_) => Self::new(Clickable, false),
            Command::Type(_) => Self::new(Typeable, false),
            Command::Submit(_) => Self::new(Submittable, true),
            Command::Check(_) | Command::Uncheck(_) => Self::new(Checkable, false),
            Command::Select(_) => Self::new(Selectable, false),
            Command::Dismiss(_) => Self::new(Dismissable, true),
            Command::AcceptCookies => Self::new(Acceptable, true),
            Command::Clear(_) | Command::Focus(_) | Command::Hover(_) => Self::new(Any, false),
            _ => Self::new(Any, false),
        }
    }
}
