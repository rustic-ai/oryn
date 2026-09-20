//! Departing frame callbacks retain the accepted response before body ownership
//! passes to parsing. No author callback can cause an implicit response refetch.

/// Inputs retained together for admit frame departure.
pub(super) struct FrameDepartureAdmission<'a> {
    pub(super) parent: RealmId,
    pub(super) frame: NodeId,
    pub(super) previous: Option<RealmId>,
    pub(super) url: String,
    pub(super) child_origin: oryn_js::origin::Origin,
    pub(super) context: &'a mut Option<FrameNavigationContext>,
    pub(super) response: &'a mut Option<frame_resource_preparation::FrameResourceCompletion>,
}
use super::*;
use oryn_js::v8_bindings::NativePageSwap;
use std::{sync::Arc, time::Instant};
#[cfg(test)]
thread_local! {static PAUSE_AFTER_SWAP:Cell<bool>=const{Cell::new(false)};static FAIL_READY_PREPARATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
pub(super) fn fail_ready_preparation() -> bool {
    FAIL_READY_PREPARATION.with(|failure| failure.replace(false))
}
#[derive(Clone)]
pub(super) struct Guard {
    pub(super) timing: oryn_js::navigation_timing::NavigationTiming,
    parent: RealmId,
    frame: NodeId,
    previous: RealmId,
    parent_identity: (NodeId, u64, u64, bool),
    previous_identity: (NodeId, u64, u64, bool),
    connection: Option<u64>,
    navigation: Arc<()>,
}
impl Guard {
    fn current(&self, host: &RawV8Host) -> bool {
        host.automation_realm_identity(self.parent) == Some(self.parent_identity)
            && host.automation_realm_identity(self.previous) == Some(self.previous_identity)
            && host.automation_frame_generation(self.parent, self.frame) == self.connection
            && host.automation_frame_live(self.parent, self.frame)
            && !host.realm_document_load_stopped(self.parent)
            && host.automation_dom(self.parent).is_some_and(|dom| {
                dom.borrow()
                    .frame_navigation_identity_is_current(self.frame, &self.navigation)
            })
    }
}
pub(super) struct Departure {
    guard: Guard,
    url: String,
    kind: String,
    disclose: bool,
    context: Option<FrameNavigationContext>,
    response: Option<frame_resource_preparation::FrameResourceCompletion>,
    swap: Option<NativePageSwap>,
    hide: Option<crate::host::NativePageTransition>,
    unload: Option<crate::host::NativePageTransition>,
    unload_guards: Vec<oryn_js::v8_bindings::UnloadGuard>,
    phase: u8,
    retry: Option<Instant>,
    canceled: bool,
}
impl Departure {
    pub(super) fn cancel(&mut self, host: &RawV8Host) {
        self.canceled = true;
        if let Some(swap) = &mut self.swap {
            swap.cancel();
        }
        self.hide.take();
        self.unload.take();
        self.unload_guards.clear();
        if let Some(context) = &self.context {
            if let Some(observer) = &context.execution_observer {
                observer.complete(oryn_js::navigation::NavigationExecutionOutcome::Canceled);
            }
            if let Some(destination) = &context.destination {
                host.fail_joint_history_entry(
                    self.guard.previous,
                    &destination.id,
                    oryn_js::joint_history::JointTraversalFailure::Canceled,
                );
            }
        }
    }
    fn poll(&mut self, host: &mut RawV8Host) -> Result<bool, HostError> {
        if !self.guard.current(host) {
            self.cancel(host);
            return Ok(true);
        }
        host.activate_realm(self.guard.previous)?;
        if self.swap.is_none() {
            if let Some(destination) = self
                .context
                .as_ref()
                .and_then(|context| context.destination.clone())
            {
                host.stage_swap_destination(destination)?;
            }
            self.swap = Some(host.begin_page_swap(&self.url, &self.kind, self.disclose)?);
        }
        if self.phase == 0 {
            if !host.poll_page_swap(self.swap.as_mut().unwrap())? {
                return Ok(false);
            }
            if self.swap.as_ref().unwrap().canceled() {
                self.cancel(host);
                return Ok(true);
            }
            self.phase = 1;
            #[cfg(test)]
            if PAUSE_AFTER_SWAP.with(|pause| pause.replace(false)) {
                return Ok(false);
            }
        }
        if self.phase == 1 {
            if self.hide.is_none() {
                self.unload_guards = host.begin_native_unload_tree(self.guard.previous)?;
                self.hide = Some(host.prepare_native_page_transition(
                    self.guard.previous,
                    "pagehide",
                    false,
                )?);
            }
            if !host.resume_native_page_transition(self.hide.as_mut().unwrap())? {
                return Ok(false);
            }
            self.phase = 2;
        }
        if self.unload.is_none() {
            self.unload =
                Some(host.prepare_native_page_transition(self.guard.previous, "unload", false)?);
        }
        if !host.resume_native_page_transition(self.unload.as_mut().unwrap())? {
            return Ok(false);
        }
        if let (Some((start, end)), Some(origin)) = (
            self.unload
                .as_ref()
                .and_then(crate::host::NativePageTransition::unload_interval),
            host.realm_descriptor(self.guard.previous)
                .map(|descriptor| descriptor.origin.serialize()),
        ) {
            self.guard
                .timing
                .unload(origin, start, end)
                .map_err(HostError::Execution)?;
        }
        host.activate_realm(self.guard.previous)?;
        if !self.guard.current(host) || host.has_location_navigation() {
            self.cancel(host);
            return Ok(true);
        }
        Ok(true)
    }
}
impl Drop for Departure {
    fn drop(&mut self) {
        if let Some(context) = &self.context {
            if let Some(observer) = &context.execution_observer {
                observer.complete(oryn_js::navigation::NavigationExecutionOutcome::Canceled);
            }
            let owner = self.swap.as_ref().and_then(NativePageSwap::owner);
            if let (Some(owner), Some(destination)) = (owner, context.destination.as_ref()) {
                owner.fail_joint_history_entry(
                    &destination.id,
                    oryn_js::joint_history::JointTraversalFailure::Canceled,
                );
            }
        }
    }
}
impl RustExecutableDocument {
    pub(super) fn frame_departure_delay(&self) -> Option<Duration> {
        if !self.ready_frame_jobs.is_empty() {
            return Some(Duration::ZERO);
        }
        self.frame_departures
            .values()
            .map(|pending| {
                pending
                    .retry
                    .map(|at| at.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::ZERO)
            })
            .min()
    }
    pub(super) fn admit_frame_departure(
        &mut self,
        params: FrameDepartureAdmission<'_>,
    ) -> Result<bool, HostError> {
        let FrameDepartureAdmission {
            parent,
            frame,
            previous,
            url,
            child_origin,
            context,
            response,
        } = params;

        let key = (parent, frame);
        if self
            .completed_frame_departures
            .get(&key)
            .is_some_and(|guard| guard.current(&self.agent.host))
        {
            return Ok(true);
        }
        self.completed_frame_departures.remove(&key);
        let Some(previous) = previous else {
            return Ok(true);
        };
        if self.frame_departures.len() >= 128 {
            return Err(HostError::LimitExceeded {
                resource: "pending frame departures".into(),
            });
        }
        let parent_identity = self
            .agent
            .host
            .automation_realm_identity(parent)
            .ok_or_else(|| HostError::Execution("frame departure parent missing".into()))?;
        let previous_identity = self
            .agent
            .host
            .automation_realm_identity(previous)
            .ok_or_else(|| HostError::Execution("frame departure document missing".into()))?;
        let connection = self.agent.host.automation_frame_generation(parent, frame);
        let dom = self
            .agent
            .host
            .automation_dom(parent)
            .ok_or_else(|| HostError::Execution("frame departure DOM missing".into()))?;
        let navigation = dom.borrow_mut().frame_navigation_identity(frame);
        let original = self.agent.host.active_realm();
        self.agent.host.activate_realm(previous)?;
        let history = self.agent.host.navigation_history_snapshot();
        self.agent.host.activate_realm(original)?;
        let history = history?;
        let traverses = context
            .as_ref()
            .and_then(|context| context.destination.as_ref())
            .is_some_and(|destination| {
                context
                    .as_ref()
                    .and_then(|context| context.session.as_ref())
                    .or_else(|| {
                        self.frame_realms
                            .get(&previous)
                            .and_then(|owner| owner.session.as_ref())
                    })
                    .map(|session| session.update.as_ref())
                    .unwrap_or(&history)
                    .entries
                    .iter()
                    .any(|entry| entry.id == destination.id)
            });
        let same_srcdoc = url == "about:srcdoc"
            && history
                .entries
                .get(history.current)
                .is_some_and(|entry| entry.url == url);
        let kind = if context.as_ref().is_some_and(|context| context.reload) {
            "reload"
        } else if traverses {
            "traverse"
        } else if same_srcdoc
            || self
                .agent
                .host
                .realm_is_initial_navigation_document(previous)
            || context.as_ref().is_some_and(|context| context.replace)
        {
            "replace"
        } else {
            "push"
        };
        let disclose = !child_origin.is_opaque()
            && self
                .agent
                .host
                .realm_descriptor(previous)
                .is_some_and(|descriptor| descriptor.origin == child_origin);
        let timing = response
            .as_ref()
            .and_then(|response| response.timing.clone())
            .unwrap_or_else(|| {
                oryn_js::navigation_timing::NavigationTiming::new(
                    Instant::now(),
                    if kind == "reload" {
                        oryn_js::navigation_timing::Kind::Reload
                    } else if kind == "traverse" {
                        oryn_js::navigation_timing::Kind::BackForward
                    } else {
                        oryn_js::navigation_timing::Kind::Navigate
                    },
                )
            });
        self.frame_departures.insert(
            key,
            Departure {
                guard: Guard {
                    timing,
                    parent,
                    frame,
                    previous,
                    parent_identity,
                    previous_identity,
                    connection,
                    navigation,
                },
                url,
                kind: kind.into(),
                disclose,
                context: context.take(),
                response: response.take(),
                swap: None,
                hide: None,
                unload: None,
                unload_guards: Vec::new(),
                phase: 0,
                retry: None,
                canceled: false,
            },
        );
        Ok(false)
    }
    pub(super) fn poll_frame_departures(
        &mut self,
    ) -> Vec<(
        RealmId,
        NodeId,
        Option<frame_resource_preparation::FrameResourceCompletion>,
    )> {
        self.completed_frame_departures.retain(|key, _| {
            self.ready_frame_jobs
                .iter()
                .any(|(parent, frame, _)| (*parent, *frame) == *key)
        });
        let original = self.agent.host.active_realm();
        let keys = self.frame_departures.keys().copied().collect::<Vec<_>>();
        let mut ready = Vec::new();
        for key in keys {
            let mut pending = self.frame_departures.remove(&key).unwrap();
            if pending.retry.is_some_and(|retry| retry > Instant::now()) {
                self.frame_departures.insert(key, pending);
                continue;
            }
            match pending.poll(&mut self.agent.host) {
                Ok(true) => {
                    if !pending.canceled
                        && pending.phase == 2
                        && pending.guard.current(&self.agent.host)
                        && !pending.swap.as_ref().is_some_and(NativePageSwap::canceled)
                    {
                        self.completed_frame_departures
                            .insert(key, pending.guard.clone());
                        if let Some(context) = pending.context.take() {
                            self.pending_frame_navigations.insert(key, context);
                        }
                        ready.push((key.0, key.1, pending.response.take()));
                    }
                }
                Ok(false) => {
                    pending.retry = Some(Instant::now() + Duration::from_millis(16));
                    self.frame_departures.insert(key, pending);
                }
                Err(error) => {
                    self.diagnostics.push(script_diagnostic(error));
                    pending.retry = Some(Instant::now() + Duration::from_millis(25));
                    self.frame_departures.insert(key, pending);
                }
            }
        }
        let _ = self.agent.host.activate_realm(original);
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_departure_retains_swap_then_hide_before_source_commit() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>old</p>'></iframe>",
        )
        .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("globalThis.calls=[];const w=document.getElementById('f').contentWindow;w.onpageswap=()=>calls.push('swap');w.onpagehide=()=>calls.push('hide');document.getElementById('f').srcdoc='<p>new</p>'").unwrap();
        PAUSE_AFTER_SWAP.with(|pause| pause.set(true));
        page.drain_dynamic_frames().unwrap();
        assert_eq!(page.frame_departures.len(), 1);
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("calls.join()+':'+document.getElementById('f').contentDocument.querySelector('p').textContent").unwrap(),"swap:old");
        for pending in page.frame_departures.values_mut() {
            pending.retry = None;
        }
        page.drain_dynamic_frames().unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("calls.join()+':'+document.getElementById('f').contentDocument.querySelector('p').textContent").unwrap(),"swap,hide:new");
        assert!(page.frame_departures.is_empty());
        assert!(page.completed_frame_departures.is_empty());
    }
    #[test]
    fn frame_unload_self_navigation_and_document_write_do_not_abort_accepted_replacement() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>old</p>'></iframe>",
        )
        .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("globalThis.calls=[];const f=document.getElementById('f'),w=f.contentWindow;w.onunload=()=>{calls.push('unload');w.location.hash='#ignored';w.document.open();w.document.write('<p>ignored</p>')};f.srcdoc='<p>accepted</p>'").unwrap();
        for _ in 0..64 {
            for pending in page.frame_departures.values_mut() {
                pending.retry = None;
            }
            page.drain_tasks().unwrap();
            if page.frame_departures.is_empty() && !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("calls.join()+':'+document.getElementById('f').contentDocument.querySelector('p').textContent+':'+document.getElementById('f').contentWindow.location.hash").unwrap(),"unload:accepted:");
    }
    #[test]
    fn parent_frame_departure_delivers_nested_pagehide_before_replacement() {
        let mut page=RustExecutableDocument::load(r#"<!doctype html><iframe id=f srcdoc="<iframe id=n srcdoc='<p>nested</p>'></iframe>"></iframe>"#).unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("globalThis.calls=[];const w=document.getElementById('f').contentWindow;w.onpagehide=()=>calls.push('parent');w.document.getElementById('n').contentWindow.onpagehide=()=>calls.push('nested');document.getElementById('f').srcdoc='<p>new</p>'").unwrap();
        page.drain_dynamic_frames().unwrap();
        for pending in page.frame_departures.values_mut() {
            pending.retry = None;
        }
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "parent,nested"
        );
    }
    #[test]
    fn removing_frame_between_departure_phases_cancels_old_source() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>old</p>'></iframe>",
        )
        .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("globalThis.calls=[];const w=document.getElementById('f').contentWindow;w.onpageswap=()=>calls.push('swap');w.onpagehide=()=>calls.push('hide');document.getElementById('f').srcdoc='<p>new</p>'").unwrap();
        PAUSE_AFTER_SWAP.with(|pause| pause.set(true));
        page.drain_dynamic_frames().unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent
            .host
            .execute("document.getElementById('f').remove()")
            .unwrap();
        for pending in page.frame_departures.values_mut() {
            pending.retry = None;
        }
        page.drain_dynamic_frames().unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("calls.join()").unwrap(),
            "swap"
        );
        assert!(page.frame_departures.is_empty());
        assert!(page.completed_frame_departures.is_empty());
    }
    #[test]
    fn ready_frame_current_and_suffix_survive_preparation_failure() {
        let mut page=RustExecutableDocument::load("<!doctype html><iframe id=a srcdoc='<p>old-a</p>'></iframe><iframe id=b srcdoc='<p>old-b</p>'></iframe>").unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("document.getElementById('a').srcdoc='<p>new-a</p>';document.getElementById('b').srcdoc='<p>new-b</p>'").unwrap();
        FAIL_READY_PREPARATION.with(|failure| failure.set(true));
        assert!(page.drain_dynamic_frames().is_err());
        assert_eq!(page.ready_frame_jobs.len(), 2);
        assert_eq!(page.frame_departure_delay(), Some(Duration::ZERO));
        page.drain_dynamic_frames().unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("['a','b'].map(id=>document.getElementById(id).contentDocument.querySelector('p').textContent).join(',')").unwrap(),"new-a,new-b");
        assert!(page.ready_frame_jobs.is_empty());
    }

    #[test]
    fn nested_pagehide_cannot_open_departing_parent_but_child_only_departure_leaves_parent_unguarded()
     {
        for event in ["pagehide", "visibilitychange"] {
            for whole_parent in [true, false] {
                let mut page=RustExecutableDocument::load(r#"<!doctype html><iframe id=f srcdoc="<p>parent</p><iframe id=n srcdoc='<p>child</p>'></iframe>"></iframe>"#).unwrap();
                for _ in 0..32 {
                    page.drain_tasks().unwrap();
                    if !page.frame_scripts_pending(None) {
                        break;
                    }
                }
                page.agent.host.activate_realm(page.top_realm).unwrap();
                page.agent.host.execute(&format!("globalThis.result='';const f=document.getElementById('f'),w=f.contentWindow,n=w.document.getElementById('n');{}.addEventListener('{event}',()=>{{w.document.open();result=String(w.document.childNodes.length===0)}})",if event=="pagehide"{"n.contentWindow"}else{"n.contentDocument"})).unwrap();
                page.agent
                    .host
                    .execute(if whole_parent {
                        "f.srcdoc='<p>replacement</p>'"
                    } else {
                        "n.srcdoc='<p>replacement</p>'"
                    })
                    .unwrap();
                for _ in 0..64 {
                    for departure in page.frame_departures.values_mut() {
                        departure.retry = None;
                    }
                    page.drain_tasks().unwrap();
                    if page.frame_departures.is_empty() && !page.frame_scripts_pending(None) {
                        break;
                    }
                }
                page.agent.host.activate_realm(page.top_realm).unwrap();
                assert_eq!(
                    page.agent.host.evaluate_string("result").unwrap(),
                    if whole_parent { "false" } else { "true" }
                );
            }
        }
    }
}
