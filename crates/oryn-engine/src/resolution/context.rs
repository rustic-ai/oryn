use oryn_common::protocol::{DetectedPatterns, Element, Rect, ScanResult};

/// All context available for resolution decisions.
pub struct ResolutionContext<'a> {
    /// Latest scan result
    pub scan: &'a ScanResult,

    /// Currently focused element (if known)
    focused: Option<u32>,

    /// Current scope (if resolving within a container)
    scope: Option<u32>,

    /// Recent command history (for context)
    history: Vec<RecentCommand>,
}

#[derive(Debug, Clone)]
pub struct RecentCommand {
    pub command_type: String,
    pub target_id: Option<u32>,
}

impl<'a> ResolutionContext<'a> {
    pub fn new(scan: &'a ScanResult) -> Self {
        Self {
            scan,
            focused: None,
            scope: None,
            history: vec![],
        }
    }

    pub fn with_focus(mut self, focused: u32) -> Self {
        self.focused = Some(focused);
        self
    }

    /// Create a scoped context for resolution within a container.
    pub fn scoped_to(&self, container_id: u32) -> ResolutionContext<'_> {
        ResolutionContext {
            scan: self.scan,
            focused: self.focused,
            scope: Some(container_id),
            history: self.history.clone(),
        }
    }

    /// Get elements, optionally filtered by scope.
    pub fn elements(&self) -> Box<dyn Iterator<Item = &Element> + '_> {
        let scope = self.scope;
        let scope_rect = scope.and_then(|id| {
            self.scan
                .elements
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.rect.clone())
        });

        Box::new(
            self.scan
                .elements
                .iter()
                .filter(move |e| match &scope_rect {
                    Some(rect) => is_inside(&e.rect, rect),
                    None => true,
                }),
        )
    }

    /// Get detected patterns.
    pub fn patterns(&self) -> Option<&DetectedPatterns> {
        self.scan.patterns.as_ref()
    }

    /// Get the focused element.
    pub fn focused(&self) -> Option<u32> {
        self.focused
    }

    /// Get element by ID.
    pub fn get_element(&self, id: u32) -> Option<&Element> {
        self.scan.elements.iter().find(|e| e.id == id)
    }

    pub fn to_resolver_context(&self) -> oryn_common::resolver::ResolverContext {
        oryn_common::resolver::ResolverContext::new(self.scan)
    }
}

/// Check if one rectangle is completely inside another.
pub fn is_inside(inner: &Rect, outer: &Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}
