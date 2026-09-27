//! Exact page-lifetime ownership, separate from a shared isolate and navigation.
//! A slot exists before document construction; binding never replaces an owner.

pub(crate) fn prompt_state(control: &oryn_js::prompts::PromptController) -> &'static str {
    match control.inspect() {
        Ok(_) => "live",
        Err(oryn_js::prompts::PromptError::Retired) => "retired",
        Err(_) => "inspect_error",
    }
}

#[cfg(feature = "v8-host")]
mod owned {
    use oryn_common::v3::PageId;
    use oryn_js::prompts::PromptController;
    use std::sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, Ordering},
    };

    pub(crate) struct PageTerminalControl {
        page: PageId,
        controller: OnceLock<Arc<PromptController>>,
        reported: AtomicU8,
    }

    impl PageTerminalControl {
        pub(crate) fn new(page: PageId) -> Arc<Self> {
            Arc::new(Self {
                page,
                controller: OnceLock::new(),
                reported: AtomicU8::new(0),
            })
        }
        pub(crate) fn owns_page(&self, page: PageId) -> bool {
            self.page == page
        }

        pub(crate) fn for_page(self: &Arc<Self>, page: PageId) -> Arc<Self> {
            if self.page == page {
                self.clone()
            } else {
                Self::new(page)
            }
        }

        pub(crate) fn controller(&self) -> Option<&Arc<PromptController>> {
            self.controller.get()
        }
        pub(crate) fn bind(&self, controller: Arc<PromptController>) -> bool {
            let expected = controller.clone();
            let _ = self.controller.set(controller);
            let matches = self
                .controller
                .get()
                .is_some_and(|actual| Arc::ptr_eq(actual, &expected));
            self.report(
                if matches { 1 } else { 2 },
                if matches { "bound" } else { "binding_mismatch" },
            );
            matches
        }
        pub(crate) fn lookup_failed(&self) {
            self.report(4, "lookup_error");
        }
        pub(crate) fn retired(&self) -> bool {
            let state = self
                .controller()
                .map(|control| super::prompt_state(control))
                .unwrap_or("unbound");
            match state {
                "retired" => self.report(8, state),
                "inspect_error" => self.report(16, state),
                "unbound" => self.report(32, state),
                _ => {}
            }
            // Unknown/poisoned control is not evidence of successful retirement.
            state == "retired"
        }
        fn report(&self, bit: u8, state: &'static str) {
            if !crate::native_timing::enabled()
                || self.reported.fetch_or(bit, Ordering::Relaxed) & bit != 0
            {
                return;
            }
            crate::native_timing::point(
                "page_terminal_control",
                serde_json::json!({
                    "owner_page_id": self.page.0, "state": state, "completion_inferred": false
                }),
            );
        }
    }

    pub(crate) fn retired(control: Option<&PageTerminalControl>) -> bool {
        control.is_some_and(PageTerminalControl::retired)
    }
}
#[cfg(feature = "v8-host")]
pub(crate) use owned::{PageTerminalControl, retired};
