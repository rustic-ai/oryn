use oryn_common::resolver::ResolutionStrategy;

/// What kind of element a command requires.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetRequirement {
    /// Any interactive element (default)
    Any,

    /// Must be typeable: input, textarea, contenteditable
    Typeable,

    /// Must be clickable: button, link, interactive element
    Clickable,

    /// Must be checkable: checkbox, radio
    Checkable,

    /// Must be submittable: form or submit button
    Submittable,

    /// Must be a container of specific type
    Container(ContainerType),

    /// Must be selectable: select element
    Selectable,

    /// Must be a dismiss/close button
    Dismissable,

    /// Must be an accept/confirm button
    Acceptable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerType {
    Form,
    Modal,
    Dialog,
    Any,
}

impl TargetRequirement {
    /// Convert to legacy ResolutionStrategy for scoring
    pub fn to_strategy(&self) -> ResolutionStrategy {
        match self {
            Self::Typeable => ResolutionStrategy::PreferInput,
            Self::Clickable | Self::Dismissable | Self::Acceptable => {
                ResolutionStrategy::PreferClickable
            }
            Self::Checkable => ResolutionStrategy::PreferCheckable,
            _ => ResolutionStrategy::Best,
        }
    }
}
