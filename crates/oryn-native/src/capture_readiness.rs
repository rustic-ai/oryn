//! Actor-resumable image preparation. Native image completions publish intrinsic
//! dimensions before a fresh capture layout; no layout getter waits for I/O.
use super::*;
pub(super) enum Reply {
    Paint(oneshot::Sender<Result<PaintSnapshot, RuntimeError>>),
    Accessibility(
        oneshot::Sender<Result<crate::accessibility_platform::NativeSnapshot, RuntimeError>>,
    ),
    Print(
        oryn_common::v4::PrintOptions,
        oneshot::Sender<Result<oryn_common::v4::PrintSnapshot, RuntimeError>>,
    ),
}
enum Ready {
    Paint(PaintSnapshotPlan),
    Print(PaintSnapshotPlan, Vec<oryn_layout::print::PrintPageLayout>),
}
pub(super) struct Pending {
    reply: Reply,
    generation: u64,
    identity: Option<crate::executable::CaptureDocumentIdentity>,
    deadline: std::time::Instant,
    started: bool,
    before: Option<crate::host::NativePageTransition>,
    after: Option<crate::host::NativePageTransition>,
    after_complete: bool,
    print_entered: bool,
    failure: Option<RuntimeError>,
    cleanup_deadline: Option<std::time::Instant>,
    ready: Option<Ready>,
    retry_at: Option<std::time::Instant>,
    lifecycle_error: Option<RuntimeError>,
    css_capture: Option<std::rc::Rc<crate::executable::NativeCssCapture>>,
}
impl Pending {
    pub(super) fn new(state: &PageState, reply: Reply) -> Self {
        Self {
            reply,
            generation: state.document_generation,
            identity: identity(state),
            deadline: std::time::Instant::now() + state.config.request_timeout,
            started: false,
            before: None,
            after: None,
            after_complete: false,
            print_entered: false,
            failure: None,
            cleanup_deadline: None,
            ready: None,
            retry_at: None,
            lifecycle_error: None,
            css_capture: None,
        }
    }
    pub(super) fn next_delay(&self, state: &PageState) -> std::time::Duration {
        use std::time::{Duration, Instant};
        if !self.live(state) || (self.failure.is_none() && self.closed()) {
            return Duration::ZERO;
        }
        let deadline = if self.failure.is_some() {
            self.cleanup_deadline.unwrap_or(self.deadline)
        } else {
            self.deadline
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Duration::ZERO;
        }
        if self
            .before
            .as_ref()
            .or(self.after.as_ref())
            .is_some_and(|receipt| receipt.suspended())
        {
            return remaining;
        }
        if let Some(retry) = self.retry_at {
            return remaining.min(retry.saturating_duration_since(Instant::now()));
        }
        if self.started
            && self.ready.is_none()
            && self
                .css_capture
                .as_ref()
                .is_some_and(|owner| owner.pending())
        {
            return remaining;
        }
        if self.before.is_some() || self.after.is_some() {
            remaining.min(Duration::from_millis(10))
        } else {
            Duration::ZERO
        }
    }
    pub(super) fn queued_delay(&self, state: &PageState) -> std::time::Duration {
        if !self.live(state) || self.closed() {
            std::time::Duration::ZERO
        } else {
            self.deadline
                .saturating_duration_since(std::time::Instant::now())
        }
    }
    pub(super) fn queued_error(&self, state: &PageState) -> Option<RuntimeError> {
        if !self.live(state) {
            Some(RuntimeError::PaintUnavailable(
                "queued capture document retired".into(),
            ))
        } else if self.closed() {
            Some(RuntimeError::Closed)
        } else if std::time::Instant::now() >= self.deadline {
            Some(RuntimeError::PaintUnavailable(
                "capture readiness queue deadline exceeded".into(),
            ))
        } else {
            None
        }
    }
    fn live(&self, state: &PageState) -> bool {
        self.generation == state.document_generation && self.identity == identity(state)
    }
    fn closed(&self) -> bool {
        match &self.reply {
            Reply::Paint(reply) => reply.is_closed(),
            Reply::Accessibility(reply) => reply.is_closed(),
            Reply::Print(_, reply) => reply.is_closed(),
        }
    }
    /// Terminal shutdown cancels native receipts and releases capture custody;
    /// it never begins author callbacks from an actor destructor.
    pub(super) fn reject(mut self, _state: &mut PageState, error: RuntimeError) {
        self.before = None;
        self.after = None;
        self.ready = None;
        if let Some(owner) = self.css_capture.take() {
            let _ = owner.release();
        }
        match self.reply {
            Reply::Paint(reply) => {
                let _ = reply.send(Err(error));
            }
            Reply::Accessibility(reply) => {
                let _ = reply.send(Err(error));
            }
            Reply::Print(_, reply) => {
                let _ = reply.send(Err(error));
            }
        }
    }
    fn fail(mut self, state: &mut PageState, error: RuntimeError) -> Option<Self> {
        if !self.live(state)
            || !self.print_entered
            || self.after_complete
            || !matches!(&self.reply, Reply::Print(..))
        {
            self.reject(state, error);
            return None;
        }
        self.before = None;
        self.ready = None;
        self.failure = Some(error);
        self.retry_at = None;
        self.cleanup_deadline = Some(
            std::time::Instant::now()
                + state
                    .config
                    .request_timeout
                    .min(std::time::Duration::from_secs(1)),
        );
        self.poll_cleanup(state)
    }
    fn poll_cleanup(mut self, state: &mut PageState) -> Option<Self> {
        if !self.live(state)
            || self
                .cleanup_deadline
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            let error = self.failure.take().expect("capture cleanup owns failure");
            self.reject(state, error);
            return None;
        }
        if self
            .retry_at
            .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return Some(self);
        }
        let result = (|| {
            let Some(executable) = state.executable.as_mut() else {
                return Ok(true);
            };
            if self.after.is_none() {
                self.after = Some(executable.prepare_print_lifecycle("afterprint")?);
            }
            executable.poll_print_lifecycle(self.after.as_mut().unwrap())
        })();
        match result {
            Ok(true) => {
                self.after = None;
                self.after_complete = true;
                let error = self.failure.take().expect("capture cleanup owns failure");
                self.reject(state, error);
                None
            }
            Ok(false) => Some(self),
            Err(_error) => {
                self.retry_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
                Some(self)
            }
        }
    }
    pub(super) fn poll(
        mut self,
        state: &mut PageState,
        sender: &mpsc::WeakSender<PageCommand>,
    ) -> Option<Self> {
        if self.css_capture.is_none() {
            match crate::executable::NativeCssCapture::new() {
                Ok(owner) => self.css_capture = Some(owner),
                Err(error) => {
                    self.reject(state, map_host_error(error));
                    return None;
                }
            }
        }
        let _capture_scope = state
            .executable
            .as_ref()
            .map(|page| page.activate_css_capture(self.css_capture.as_ref().unwrap().clone()));
        if self.failure.is_some() {
            return self.poll_cleanup(state);
        }
        if !self.live(state) {
            return self.fail(
                state,
                RuntimeError::PaintUnavailable(
                    "capture document retired during image preparation".into(),
                ),
            );
        }
        if self.closed() {
            return self.fail(state, RuntimeError::Closed);
        }
        if std::time::Instant::now() >= self.deadline {
            let error = self.lifecycle_error.take().unwrap_or_else(|| {
                RuntimeError::PaintUnavailable("capture image readiness deadline exceeded".into())
            });
            return self.fail(state, error);
        }
        if self
            .retry_at
            .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return Some(self);
        }
        self.retry_at = None;
        if self.started
            && self.ready.is_none()
            && self
                .css_capture
                .as_ref()
                .is_some_and(|owner| owner.pending())
        {
            return Some(self);
        }
        if !self.started {
            if let Reply::Print(options, _) = &self.reply {
                if let Err(error) = oryn_layout::print::validate_options(options) {
                    return self.fail(state, RuntimeError::PaintUnavailable(error.to_string()));
                }
                if let Some(executable) = state.executable.as_mut() {
                    if self.before.is_none() {
                        match executable.prepare_print_lifecycle("beforeprint") {
                            Ok(receipt) => self.before = Some(receipt),
                            Err(error) => {
                                return self.fail(state, map_host_error(error));
                            }
                        }
                    }
                    let result = executable.poll_print_lifecycle(self.before.as_mut().unwrap());
                    self.print_entered |= self.before.as_ref().unwrap().started();
                    match result {
                        Ok(true) => {
                            self.before = None;
                        }
                        Ok(false) => return Some(self),
                        Err(error) => {
                            if self.lifecycle_error.is_none() {
                                self.lifecycle_error = Some(map_host_error(error));
                            }
                            self.retry_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_millis(25),
                            );
                            return Some(self);
                        }
                    }
                }
            }
            if !self.live(state) {
                return self.fail(
                    state,
                    RuntimeError::PaintUnavailable(
                        "capture document changed during beforeprint".into(),
                    ),
                );
            }
            self.started = true;
        }
        if self.ready.is_none() {
            let result = match &self.reply {
                Reply::Paint(_) | Reply::Accessibility(_) => {
                    state.paint_snapshot_plan().map(Ready::Paint)
                }
                Reply::Print(options, _) => state
                    .capture_print_layout(options)
                    .map(|(plan, pages)| Ready::Print(plan, pages)),
            };
            let ready = match result {
                Ok(ready) => ready,
                Err(error) => {
                    return self.fail(state, error);
                }
            };
            if !self.live(state) {
                return self.fail(
                    state,
                    RuntimeError::PaintUnavailable(
                        "capture document changed during preparation".into(),
                    ),
                );
            }
            // Discard provisional geometry: decoded dimensions may change every
            // line/page. Resume on the actor after completion, then lay out anew.
            if self
                .css_capture
                .as_ref()
                .is_some_and(|owner| owner.pending())
            {
                return Some(self);
            }
            self.ready = Some(ready);
        }
        if matches!(&self.reply, Reply::Print(..)) && !self.after_complete {
            if let Some(executable) = state.executable.as_mut() {
                if self.after.is_none() {
                    match executable.prepare_print_lifecycle("afterprint") {
                        Ok(receipt) => self.after = Some(receipt),
                        Err(error) => {
                            self.started = false;
                            return self.fail(state, map_host_error(error));
                        }
                    }
                }
                match executable.poll_print_lifecycle(self.after.as_mut().unwrap()) {
                    Ok(true) => {
                        self.after = None;
                        self.after_complete = true;
                    }
                    Ok(false) => return Some(self),
                    Err(error) => {
                        if self.lifecycle_error.is_none() {
                            self.lifecycle_error = Some(map_host_error(error));
                        }
                        self.retry_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
                        return Some(self);
                    }
                }
            }
        }
        if !self.live(state) {
            return self.fail(
                state,
                RuntimeError::PaintUnavailable(
                    "capture document changed during lifecycle completion".into(),
                ),
            );
        }
        let ready = self
            .ready
            .take()
            .expect("capture layout retained until lifecycle completion");
        if let Some(owner) = self.css_capture.as_ref() {
            if let Err(error) = owner.release() {
                self.started = false;
                return self.fail(state, map_host_error(error));
            }
        }
        match (self.reply, ready) {
            (Reply::Paint(reply), Ready::Paint(plan)) => {
                let completion = sender.clone();
                let generation = self.generation;
                let stamp = state
                    .executable
                    .as_ref()
                    .and_then(|page| page.root_paint_inventory_stamp());
                std::thread::spawn(move || {
                    let result = complete_paint_snapshot(plan);
                    if let Some(sender) = completion.upgrade() {
                        let _ = sender.blocking_send(PageCommand::PaintSnapshotCompleted {
                            generation,
                            stamp,
                            result,
                            reply,
                        });
                    } else {
                        let _ = reply.send(Err(RuntimeError::Closed));
                    }
                });
            }
            (Reply::Accessibility(reply), Ready::Paint(plan)) => {
                let mut snapshot =
                    match state.accessibility_snapshot_prepared(plan.snapshot.clone()) {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            let _ = reply.send(Err(error));
                            return None;
                        }
                    };
                let completion = sender.clone();
                let generation = self.generation;
                let stamp = state
                    .executable
                    .as_ref()
                    .and_then(|page| page.root_paint_inventory_stamp());
                std::thread::spawn(move || {
                    let result = complete_paint_snapshot(plan).map(|paint| {
                        snapshot.paint = paint;
                        snapshot
                    });
                    if let Some(sender) = completion.upgrade() {
                        let _ = sender.blocking_send(PageCommand::AccessibilitySnapshotCompleted {
                            generation,
                            stamp,
                            result,
                            reply,
                        });
                    } else {
                        let _ = reply.send(Err(RuntimeError::Closed));
                    }
                });
            }
            (Reply::Print(_, reply), Ready::Print(plan, pages)) => {
                let max_fragments = state.config.max_layout_nodes;
                std::thread::spawn(move || {
                    let result = complete_paint_snapshot(plan)
                        .and_then(|source| complete_print_snapshot(source, pages, max_fragments));
                    let _ = reply.send(result);
                });
            }
            _ => unreachable!("native capture kind"),
        }
        None
    }
}
fn identity(state: &PageState) -> Option<crate::executable::CaptureDocumentIdentity> {
    state
        .executable
        .as_ref()
        .and_then(|page| page.capture_document_identity())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> PageState {
        let mut state = PageState::new(
            PageId(252),
            None,
            Default::default(),
            Default::default(),
            None,
            #[cfg(unix)]
            None,
        );
        let executable =
            crate::executable::RustExecutableDocument::load("<body>Print body</body>").unwrap();
        state.document = Some(executable.document.clone());
        state.executable = Some(executable);
        state
    }
    #[test]
    fn serialized_capture_does_not_restore_cancellation_into_replacement_stream() {
        let mut state = page();
        let original = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let replacement = original + std::time::Duration::from_secs(30);
        state
            .executable
            .as_mut()
            .unwrap()
            .replace_native_paint_cancellation(Some(crate::executable::NativePaintCancellation {
                cancellation: None,
                deadline: original,
            }));
        let result = state.prepare_capture_until_ready(None, |state| {
            let page = state.executable.as_mut().unwrap();
            page.execute(
                "document.open();document.write('<body>replacement</body>');document.close()",
            )
            .map_err(map_host_error)?;
            page.replace_native_paint_cancellation(Some(
                crate::executable::NativePaintCancellation {
                    cancellation: None,
                    deadline: replacement,
                },
            ));
            Ok(())
        });
        assert!(matches!(result, Err(RuntimeError::StaleSemanticRef)));
        let retained = state
            .executable
            .as_mut()
            .unwrap()
            .replace_native_paint_cancellation(None)
            .unwrap();
        assert_eq!(retained.deadline, replacement);
    }

    #[test]
    fn rejected_print_cancels_unstarted_tree_without_delivering_callbacks() {
        let mut state = page();
        state.executable.as_mut().unwrap().execute("globalThis.trace=[];onbeforeprint=()=>trace.push('before');onafterprint=()=>trace.push('after')").unwrap();
        let (reply, mut result) = oneshot::channel();
        let mut pending = Pending::new(&state, Reply::Print(Default::default(), reply));
        pending.before = Some(
            state
                .executable
                .as_mut()
                .unwrap()
                .prepare_print_lifecycle("beforeprint")
                .unwrap(),
        );
        pending.reject(&mut state, RuntimeError::Closed);
        assert!(matches!(result.try_recv(), Ok(Err(RuntimeError::Closed))));
        assert_eq!(
            state
                .executable
                .as_mut()
                .unwrap()
                .evaluate("trace.length")
                .unwrap(),
            "0"
        );
    }
    #[test]
    fn print_failure_finishes_afterprint_once_before_returning_original_error() {
        let mut state = page();
        state.executable.as_mut().unwrap().execute("globalThis.trace=[];onbeforeprint=()=>trace.push('before');onafterprint=()=>trace.push('after')").unwrap();
        let (reply, mut result) = oneshot::channel();
        let mut pending = Pending::new(&state, Reply::Print(Default::default(), reply));
        let executable = state.executable.as_mut().unwrap();
        let mut before = executable.prepare_print_lifecycle("beforeprint").unwrap();
        assert!(executable.poll_print_lifecycle(&mut before).unwrap());
        pending.print_entered = before.started();
        pending.started = true;
        assert!(
            pending
                .fail(
                    &mut state,
                    RuntimeError::PaintUnavailable("original capture failure".into())
                )
                .is_none()
        );
        assert!(
            matches!(result.try_recv(),Ok(Err(RuntimeError::PaintUnavailable(message))) if message=="original capture failure")
        );
        assert_eq!(
            state
                .executable
                .as_mut()
                .unwrap()
                .evaluate("trace.join()")
                .unwrap(),
            "before,after"
        );
    }
    #[test]
    fn actor_shutdown_cancels_started_print_without_new_author_callbacks() {
        let mut state = page();
        state.executable.as_mut().unwrap().execute("globalThis.trace=[];onbeforeprint=()=>trace.push('before');onafterprint=()=>trace.push('after')").unwrap();
        let (reply, mut result) = oneshot::channel();
        let mut pending = Pending::new(&state, Reply::Print(Default::default(), reply));
        let executable = state.executable.as_mut().unwrap();
        let mut before = executable.prepare_print_lifecycle("beforeprint").unwrap();
        assert!(executable.poll_print_lifecycle(&mut before).unwrap());
        pending.print_entered = before.started();
        pending.started = true;
        pending.after = Some(executable.prepare_print_lifecycle("afterprint").unwrap());
        pending.reject(&mut state, RuntimeError::Closed);
        assert!(matches!(result.try_recv(), Ok(Err(RuntimeError::Closed))));
        assert_eq!(
            state
                .executable
                .as_mut()
                .unwrap()
                .evaluate("trace.join()")
                .unwrap(),
            "before"
        );
    }

    #[test]
    fn expired_queued_print_is_rejected_without_lifecycle_callbacks() {
        let mut state = page();
        state.executable.as_mut().unwrap().execute("globalThis.trace=[];onbeforeprint=()=>trace.push('before');onafterprint=()=>trace.push('after')").unwrap();
        let (reply, mut result) = oneshot::channel();
        let mut pending = Pending::new(&state, Reply::Print(Default::default(), reply));
        pending.deadline = std::time::Instant::now();
        assert!(pending.queued_delay(&state).is_zero());
        let error = pending
            .queued_error(&state)
            .expect("queued deadline expires independently of front capture");
        pending.reject(&mut state, error);
        assert!(
            matches!(result.try_recv(),Ok(Err(RuntimeError::PaintUnavailable(message))) if message.contains("queue deadline"))
        );
        assert_eq!(
            state
                .executable
                .as_mut()
                .unwrap()
                .evaluate("trace.length")
                .unwrap(),
            "0"
        );
    }

    #[test]
    fn deferred_print_keeps_reply_and_original_lifecycle_until_retry() {
        let mut state = page();
        state.executable.as_mut().unwrap().execute("globalThis.trace=[];onbeforeprint=()=>trace.push('before');onafterprint=()=>trace.push('after')").unwrap();
        let (reply, mut result) = oneshot::channel();
        let mut pending = Pending::new(&state, Reply::Print(Default::default(), reply));
        pending.before = Some(
            state
                .executable
                .as_mut()
                .unwrap()
                .prepare_print_lifecycle("beforeprint")
                .unwrap(),
        );
        pending.retry_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        let (sender, _receiver) = mpsc::channel(4);
        let weak = sender.downgrade();
        assert!(!pending.next_delay(&state).is_zero());
        let pending = pending
            .poll(&mut state, &weak)
            .expect("print remains parked until retry");
        assert!(pending.before.is_some());
        assert!(!pending.started);
        assert!(matches!(
            result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(
            state
                .executable
                .as_mut()
                .unwrap()
                .evaluate("trace.length")
                .unwrap(),
            "0"
        );
        pending.reject(&mut state, RuntimeError::Closed);
        assert!(matches!(result.try_recv(), Ok(Err(RuntimeError::Closed))));
    }
}
