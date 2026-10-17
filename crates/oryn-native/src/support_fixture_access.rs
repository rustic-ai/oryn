// Included in runtime.rs: immutable parent fixture scope, no author issuance.
impl BrowserContext {
    pub(crate) fn pinned_support_descriptor(&self)
        -> Option<crate::support_descriptor::PinnedSupportDescriptor> {
        self.inner.fixture_network.as_ref().and_then(|fixture| fixture.support)
    }
}
