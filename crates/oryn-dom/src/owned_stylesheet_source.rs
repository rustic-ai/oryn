/// Immutable source bytes and their exact native allocation custody clone
/// together, so DOM snapshots do not copy source bytes under a shared lease.
#[derive(Clone)]
enum StylesheetTextOwner {
    Legacy(Arc<str>),
    Controlled(Arc<dyn oryn_common::owned_stylesheet_text::OwnedStylesheetText>),
}
impl AsRef<str> for StylesheetTextOwner {
    fn as_ref(&self) -> &str {
        match self {
            Self::Legacy(text) => text,
            Self::Controlled(text) => text.as_ref().as_ref(),
        }
    }
}
#[derive(Clone)]
pub struct OwnedNativeStylesheetSource {
    source: StylesheetTextOwner,
    _allocation: Arc<dyn Send + Sync>,
}
impl OwnedNativeStylesheetSource {
    pub fn admitted(source: Arc<str>, allocation: Arc<dyn Send + Sync>) -> Self {
        Self {
            source: StylesheetTextOwner::Legacy(source),
            _allocation: allocation,
        }
    }
    pub fn admitted_controlled(
        source: Arc<dyn oryn_common::owned_stylesheet_text::OwnedStylesheetText>,
        allocation: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            source: StylesheetTextOwner::Controlled(source),
            _allocation: allocation,
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.source.as_ref().len()
    }
}
impl From<String> for OwnedNativeStylesheetSource {
    fn from(source: String) -> Self {
        Self {
            source: StylesheetTextOwner::Legacy(source.into()),
            _allocation: Arc::new(()),
        }
    }
}
impl From<&str> for OwnedNativeStylesheetSource {
    fn from(source: &str) -> Self {
        source.to_owned().into()
    }
}
impl AsRef<str> for OwnedNativeStylesheetSource {
    fn as_ref(&self) -> &str {
        self.source.as_ref()
    }
}
impl std::ops::Deref for OwnedNativeStylesheetSource {
    type Target = str;
    fn deref(&self) -> &str {
        self.source.as_ref()
    }
}
impl std::fmt::Debug for OwnedNativeStylesheetSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OwnedNativeStylesheetSource")
            .field(&self.source.as_ref())
            .finish()
    }
}
impl PartialEq for OwnedNativeStylesheetSource {
    fn eq(&self, other: &Self) -> bool {
        self.source.as_ref() == other.source.as_ref()
    }
}
impl Eq for OwnedNativeStylesheetSource {}
#[cfg(test)]
mod owned_stylesheet_source_tests {
    use super::*;
    #[test]
    fn clone_shares_actual_source_and_custody() {
        let source = OwnedNativeStylesheetSource::admitted(Arc::from("p{}"), Arc::new(()));
        let cloned = source.clone();
        assert!(
            matches!((&source.source, &cloned.source), (StylesheetTextOwner::Legacy(a), StylesheetTextOwner::Legacy(b)) if Arc::ptr_eq(a, b))
        );
        assert!(Arc::ptr_eq(&source._allocation, &cloned._allocation));
        assert_eq!(source, cloned);
    }
}
