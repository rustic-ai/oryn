// The embedded Window agent has one isolate and several native root drivers.
// Transfer the unique host for a bounded turn; never create a context-service
// page or widen another page actor's active-root inventory.
use super::*;
#[derive(Default)]
pub(crate) struct Roots {
    drivers: BTreeMap<oryn_js::realm::RealmId, Box<PageState>>,
    next: Option<std::time::Instant>,
    unclaimed: std::collections::VecDeque<oryn_js::realm::RealmId>,
}
#[cfg(test)]
std::thread_local! {pub(crate) static FAIL_ADOPTION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};pub(crate) static FAIL_EXTRACTION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
impl Roots {
    #[cfg(test)]
    pub(crate) fn drivers_len(&self) -> usize {
        self.drivers.len()
    }
    pub(crate) fn next_delay(&self) -> Option<Duration> {
        self.next
            .map(|next| next.saturating_duration_since(Instant::now()))
            .or_else(|| (!self.unclaimed.is_empty()).then_some(Duration::ZERO))
    }
    pub(crate) fn runnable(&self) -> bool {
        self.next_delay().is_some_and(|delay| delay.is_zero())
    }
    pub(crate) fn has_roots(&self) -> bool {
        !self.drivers.is_empty() || !self.unclaimed.is_empty()
    }
    pub(crate) fn recover_host(
        &mut self,
        identity: &std::rc::Rc<()>,
    ) -> Result<crate::host::RawV8Host, crate::host::HostError> {
        let count = self
            .drivers
            .values()
            .map(|state| state.matching_agent_host_count(identity))
            .sum::<usize>();
        if count != 1 {
            return Err(crate::host::HostError::Execution(format!(
                "standalone recovery requires one native host, found {count}"
            )));
        }
        let state = self
            .drivers
            .values_mut()
            .find(|state| state.matching_agent_host_count(identity) == 1)
            .unwrap();
        state
            .extract_agent_host(Some(identity))
            .map_err(|error| crate::host::HostError::Execution(error.to_string()))?
            .ok_or_else(|| {
                crate::host::HostError::Execution("standalone recovery owner changed".into())
            })
    }
    pub(crate) fn poll(
        &mut self,
        host: &mut Option<crate::host::RawV8Host>,
        resources: &BTreeMap<String, String>,
    ) -> Result<(), crate::host::HostError> {
        let owned = host.as_mut().ok_or_else(|| {
            crate::host::HostError::Execution("standalone agent host absent".into())
        })?;
        self.unclaimed.extend(owned.take_created_standalone_roots());
        self.next = self
            .has_roots()
            .then(|| Instant::now() + Duration::from_millis(25));
        while let Some(realm) = self.unclaimed.front().copied() {
            if owned.standalone_root_discarded(realm) {
                self.unclaimed.pop_front();
                continue;
            }
            #[cfg(test)]
            if FAIL_ADOPTION.with(|fault| fault.replace(false)) {
                return Err(crate::host::HostError::Execution(
                    "injected standalone adoption retry".into(),
                ));
            }
            let snapshot = owned.standalone_root_snapshot(realm)?;
            let (url, generation, origin, policy, referrer, broker) = (
                snapshot.url.to_string(),
                snapshot.generation,
                snapshot.origin.clone(),
                snapshot.policy.clone(),
                snapshot.referrer,
                snapshot.broker.clone(),
            );
            let isolation = snapshot.isolation;
            let mut executable =
                crate::executable::RustExecutableDocument::adopt_standalone_initial_root(
                    owned, snapshot,
                )?;
            owned.set_script_isolation_policy(isolation)?;
            executable.inherit_standalone_sources(resources.clone());
            let history = owned.navigation_history_snapshot()?;
            let config = resources
                .get("oryn:standalone-root-config")
                .map(|serialized| {
                    serde_json::from_str::<BrowserConfig>(serialized).map_err(|error| {
                        crate::host::HostError::Execution(format!(
                            "invalid standalone root configuration: {error}"
                        ))
                    })
                })
                .transpose()?
                .unwrap_or_default();
            let mut state = PageState::new(
                PageId(u64::from(realm.0)),
                None,
                config,
                ContextOptions::default(),
                broker,
                #[cfg(unix)]
                None,
            );
            state.standalone_root = Some(realm);
            state.adopt_agent_document(
                executable, url, generation, origin, history, policy, referrer, isolation,
            );
            self.drivers.insert(realm, Box::new(state));
            self.unclaimed.pop_front();
        }
        let identity = host.as_ref().unwrap().agent_host_identity();
        let mut first = None;
        let mut remove = Vec::new();
        for (initial, state) in &mut self.drivers {
            let Some(mut owned) = host.take() else {
                return Err(crate::host::HostError::Execution(
                    "standalone shared host missing before turn".into(),
                ));
            };
            let current = owned
                .standalone_navigation_root(*initial)
                .unwrap_or(*initial);
            if owned.standalone_root_discarded(current) {
                state.clear_cached_documents();
                match state.drain_retired_agent_documents(&mut owned) {
                    Ok(()) if !state.has_retired_agent_documents() => {
                        owned.forget_standalone_lineage(*initial);
                        remove.push(*initial);
                    }
                    Ok(()) => {}
                    Err(error) => {
                        first.get_or_insert_with(|| {
                            crate::host::HostError::Execution(error.to_string())
                        });
                    }
                }
                *host = Some(owned);
                continue;
            }
            let retired = state.drain_retired_agent_documents(&mut owned);
            if let Err(error) = retired {
                first.get_or_insert_with(|| crate::host::HostError::Execution(error.to_string()));
            }
            if let Some(executable) = state.executable.as_mut() {
                if let Err((error, returned)) = executable.install_agent_host(owned) {
                    *host = Some(returned);
                    first.get_or_insert(error);
                    continue;
                }
            } else if state.pending_agent_host.is_none() {
                state.pending_agent_host = Some(owned)
            } else {
                *host = Some(owned);
                return Err(crate::host::HostError::Execution(
                    "standalone pending host already occupied".into(),
                ));
            }
            let result = state.poll_page_tasks();
            // A navigation can transfer custody to the cache, construction recovery or
            // destination driver. Search all native owners before returning to opener.
            #[cfg(test)]
            if FAIL_EXTRACTION.with(|fault| fault.replace(false)) {
                return Err(crate::host::HostError::Execution(
                    "injected standalone extraction retry".into(),
                ));
            }
            *host = state
                .extract_agent_host(Some(&identity))
                .map_err(|error| crate::host::HostError::Execution(error.to_string()))?;
            if host.is_none() {
                return Err(crate::host::HostError::Execution(
                    "standalone navigation lost agent host".into(),
                ));
            }
            if let Err(error) = result {
                first.get_or_insert_with(|| crate::host::HostError::Execution(error.to_string()));
            }
        }
        for realm in remove {
            self.drivers.remove(&realm);
        }
        self.next = (!self.drivers.is_empty()).then(|| Instant::now() + Duration::from_millis(10));
        first.map_or(Ok(()), Err)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opener_navigation_retains_auxiliary_driver_and_unique_agent() {
        let mut page = PageState::new(
            PageId(1),
            None,
            Default::default(),
            Default::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.load_document(
            "https://example.test/start".into(),
            "<!doctype html><title>start</title>",
            None,
            None,
            HistoryUpdate::Replace,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        page.executable
            .as_mut()
            .unwrap()
            .execute("globalThis.popup=open();popup.marker=73;")
            .unwrap();
        let identity = page.executable.as_ref().unwrap().agent_host_identity();
        page.load_document(
            "https://example.test/next".into(),
            "<!doctype html><title>next</title>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        let executable = page.executable.as_mut().unwrap();
        assert!(std::rc::Rc::ptr_eq(
            &identity,
            &executable.agent_host_identity()
        ));
        assert!(executable.has_standalone_auxiliary_roots());
        executable.drain_tasks().unwrap();
        assert_eq!(executable.evaluate("document.title").unwrap(), "next");
    }
}
