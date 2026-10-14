/// Trusted immutable stylesheet bytes. Implementations own their payload's
/// allocation custody; consumers clone the owner, never copy bytes implicitly.
pub trait OwnedStylesheetText: Send + Sync + AsRef<str> {}
