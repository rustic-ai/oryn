// Retained native cascade input, deliberately separate from serializable
// LayoutInput. Native handles retain graph allocation ownership.
#[derive(Clone)]
pub struct RetainedCascadeSheets {
    lock: style::shared_lock::SharedRwLock,
    by_index: std::sync::Arc<BTreeMap<usize, cssom::native_import_graph::NativeSheet>>,
    revision: u64,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl std::fmt::Debug for RetainedCascadeSheets {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("RetainedCascadeSheets")
            .field("count", &self.by_index.len())
            .field("revision", &self.revision)
            .finish()
    }
}
impl RetainedCascadeSheets {
    pub fn new(
        lock: style::shared_lock::SharedRwLock,
        by_index: BTreeMap<usize, cssom::native_import_graph::NativeSheet>,
        revision: u64,
        charge: std::sync::Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            lock,
            by_index: std::sync::Arc::new(by_index),
            revision,
            _charge: charge,
        }
    }
}
impl StyleEngine {
    /// Publish the shared native cascade while returning the prior ownership.
    /// The caller releases it after its native transaction borrows are gone.
    pub fn swap_retained_stylesheets(&mut self, sheets: Option<RetainedCascadeSheets>) -> Option<RetainedCascadeSheets> {
        self.static_layout = None;
        self.instance_sample = None;
        std::mem::replace(&mut self.retained_sheets, sheets)
    }

    pub fn replace_retained_stylesheets(&mut self, sheets: Option<RetainedCascadeSheets>) {
        // Resource publication and CSSOM edits invalidate computed style even
        // when the DOM source strings are unchanged. The provider owns one lock
        // for its entire lifetime, including native animation continuity.
        self.static_layout = None;
        self.instance_sample = None;
        self.retained_sheets = sheets;
    }
    pub fn retained_stylesheet_revision(&self) -> Option<u64> {
        self.retained_sheets.as_ref().map(|sheets| sheets.revision)
    }
}
