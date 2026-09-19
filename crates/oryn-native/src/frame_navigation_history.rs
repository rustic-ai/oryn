//! Frame history publication consumes native accepted identities before scripts.
use super::*;
use oryn_js::navigation_api::{NavigationHistorySnapshot, NavigationHistoryUpdate};
use std::sync::Arc;
pub(super) fn commit(
    previous: Option<NavigationHistoryUpdate>,
    mut destination: NavigationHistorySnapshot,
    kind: &str,
    same_origin: bool,
) -> Result<NavigationHistoryUpdate, HostError> {
    let Some(mut history) = previous.filter(|_| same_origin) else {
        return Ok(NavigationHistoryUpdate {
            entries: vec![destination],
            current: 0,
            navigation_type: kind.into(),
            truncates_forward: false,
        });
    };
    if history.current >= history.entries.len() {
        return Err(HostError::Execution(
            "invalid native frame history cursor".into(),
        ));
    }
    if let Some(index) = history
        .entries
        .iter()
        .position(|entry| entry.id == destination.id)
    {
        history.entries[index] = destination;
        history.current = index;
        history.navigation_type = if kind == "reload" {
            "reload"
        } else {
            "traverse"
        }
        .into();
        history.truncates_forward = false;
    } else if matches!(kind, "replace" | "reload") {
        destination.key = history.entries[history.current].key.clone();
        history.entries[history.current] = destination;
        history.navigation_type = kind.into();
        history.truncates_forward = false;
    } else {
        history.entries.truncate(history.current + 1);
        if history.entries.len() >= 4096 {
            return Err(HostError::LimitExceeded {
                resource: "frame navigation history entries".into(),
            });
        }
        history.entries.push(destination);
        history.current = history.entries.len() - 1;
        history.navigation_type = "push".into();
        history.truncates_forward = true;
    }
    Ok(history)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: format!("key-{id}"),
            id: id.into(),
            url: format!("https://example.test/{id}"),
            document_id: format!("doc-{id}"),
            state: None,
            history_state: None,
        }
    }
    #[test]
    fn accepted_frame_destination_preserves_identity_and_forward_traversal() {
        let first = entry("first");
        let second = entry("second");
        let history = NavigationHistoryUpdate {
            entries: vec![first.clone(), second],
            current: 1,
            navigation_type: "push".into(),
            truncates_forward: false,
        };
        let result = commit(Some(history), first.clone(), "push", true).unwrap();
        assert_eq!(result.current, 0);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.navigation_type, "traverse");
        assert_eq!(result.entries[0].document_id, first.document_id);
    }
    #[test]
    fn cross_origin_frame_commit_does_not_disclose_old_entries() {
        let history = NavigationHistoryUpdate {
            entries: vec![entry("private")],
            current: 0,
            navigation_type: "push".into(),
            truncates_forward: false,
        };
        let result = commit(Some(history), entry("destination"), "push", false).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].id, "destination");
    }
    #[test]
    fn frame_push_after_traversal_truncates_only_forward_entries() {
        let history = NavigationHistoryUpdate {
            entries: vec![entry("first"), entry("forward")],
            current: 0,
            navigation_type: "traverse".into(),
            truncates_forward: false,
        };
        let result = commit(Some(history), entry("new"), "push", true).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[1].id, "new");
        assert!(result.truncates_forward);
    }
}

/// Owned network-document controls, independent of response/source replay.
#[derive(Debug)]
pub(super) struct ArchivedControls {
    pub(super) controls: Vec<oryn_js::form_state::SavedFormControl>,
    _bytes: ScriptSourceReservation,
}
impl ArchivedControls {
    pub(super) fn capture(
        host: &mut RawV8Host,
        realm: RealmId,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Arc<Self>, HostError> {
        let available = MAX_QUEUED_SCRIPT_SOURCE_BYTES
            .saturating_sub(budget.load(std::sync::atomic::Ordering::Acquire));
        let mut reservation =
            ScriptSourceReservation::reserve(budget, available).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "frame controls".into(),
                }
            })?;
        let controls = host.capture_form_history_in_realm_limited(realm, available)?;
        let size = oryn_js::form_state::snapshot_bytes(&controls).map_err(|resource| {
            HostError::LimitExceeded {
                resource: resource.into(),
            }
        })?;
        if size > reservation.bytes {
            return Err(HostError::LimitExceeded {
                resource: "frame controls".into(),
            });
        }
        reservation.used.fetch_sub(
            reservation.bytes - size,
            std::sync::atomic::Ordering::AcqRel,
        );
        reservation.bytes = size;
        Ok(Arc::new(Self {
            controls,
            _bytes: reservation,
        }))
    }
    pub(super) fn restore(
        controls: &[oryn_js::form_state::SavedFormControl],
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Arc<Self>, HostError> {
        let size = oryn_js::form_state::snapshot_bytes(controls).map_err(|resource| {
            HostError::LimitExceeded {
                resource: resource.into(),
            }
        })?;
        let reservation = ScriptSourceReservation::reserve(budget, size).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "frame controls".into(),
            }
        })?;
        Ok(Arc::new(Self {
            controls: controls.to_vec(),
            _bytes: reservation,
        }))
    }
}

/// Session-history source, independent of whether a live realm is in BFCache.
/// Only inline documents use this; network responses continue through policy-checked fetch.
pub(super) struct ArchivedSource {
    pub(super) source: Arc<str>,
    pub(super) base_url: Option<Url>,
    pub(super) forms: RefCell<Vec<oryn_js::form_state::SavedFormControl>>,
    _form_bytes: RefCell<Option<ScriptSourceReservation>>,
    _bytes: ScriptSourceReservation,
}
impl std::fmt::Debug for ArchivedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchivedSource")
            .field("bytes", &self.source.len())
            .finish()
    }
}
impl ArchivedSource {
    pub(super) fn capture(
        source: &str,
        base_url: Option<Url>,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Arc<Self>, HostError> {
        let bytes = ScriptSourceReservation::reserve(
            budget,
            source
                .len()
                .saturating_add(base_url.as_ref().map_or(0, |url| url.as_str().len())),
        )
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "frame history source bytes".into(),
        })?;
        Ok(Arc::new(Self {
            source: Arc::from(source),
            base_url,
            forms: RefCell::new(Vec::new()),
            _form_bytes: RefCell::new(None),
            _bytes: bytes,
        }))
    }
}

impl ArchivedSource {
    fn install_forms(
        &self,
        forms: Vec<oryn_js::form_state::SavedFormControl>,
        reservation: ScriptSourceReservation,
    ) {
        *self.forms.borrow_mut() = forms;
        *self._form_bytes.borrow_mut() = Some(reservation);
    }
    pub(super) fn capture_forms(
        &self,
        host: &mut RawV8Host,
        realm: RealmId,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        // Reserve available page custody before the binding copies file/string
        // payloads. Shrink the lease after capture; failure leaves old state intact.
        let available = MAX_QUEUED_SCRIPT_SOURCE_BYTES
            .saturating_sub(budget.load(std::sync::atomic::Ordering::Acquire));
        let mut reservation =
            ScriptSourceReservation::reserve(budget, available).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "frame archived controls".into(),
                }
            })?;
        let forms = host.capture_form_history_in_realm_limited(realm, available)?;
        let bytes = oryn_js::form_state::snapshot_bytes(&forms).map_err(|resource| {
            HostError::LimitExceeded {
                resource: resource.into(),
            }
        })?;
        if bytes > reservation.bytes {
            return Err(HostError::LimitExceeded {
                resource: "frame archived controls".into(),
            });
        }
        reservation.used.fetch_sub(
            reservation.bytes - bytes,
            std::sync::atomic::Ordering::AcqRel,
        );
        reservation.bytes = bytes;
        self.install_forms(forms, reservation);
        Ok(())
    }
    fn clone_forms(
        &self,
        saved: &[oryn_js::form_state::SavedFormControl],
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        let bytes = oryn_js::form_state::snapshot_bytes(saved).map_err(|resource| {
            HostError::LimitExceeded {
                resource: resource.into(),
            }
        })?;
        let reservation = ScriptSourceReservation::reserve(budget, bytes).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "frame archived controls".into(),
            }
        })?;
        self.install_forms(saved.to_vec(), reservation);
        Ok(())
    }
}

fn history_entries_bytes<'a>(
    entries: impl Iterator<Item = &'a NavigationHistorySnapshot>,
) -> Result<usize, HostError> {
    let mut bytes = 0usize;
    let mut count = 0usize;
    for entry in entries {
        if !entry.scroll.valid() {
            return Err(HostError::Execution(
                "invalid frame history scroll state".into(),
            ));
        }
        count += 1;
        if count > 4097 {
            return Err(HostError::LimitExceeded {
                resource: "frame session entries".into(),
            });
        }
        let size = std::mem::size_of::<NavigationHistorySnapshot>()
            .saturating_add(512)
            .saturating_add(entry.document_id.len().saturating_mul(2))
            .saturating_add(entry.url.len())
            .saturating_add(entry.key.len())
            .saturating_add(entry.id.len())
            .saturating_add(entry.url.len())
            .saturating_add(entry.document_id.len())
            .saturating_add(
                entry
                    .state
                    .as_ref()
                    .map_or(0, |state| state.retained_bytes()),
            )
            .saturating_add(
                entry
                    .history_state
                    .as_ref()
                    .map_or(0, |state| state.retained_bytes()),
            );
        bytes = bytes
            .checked_add(size)
            .filter(|bytes| *bytes <= MAX_QUEUED_SCRIPT_SOURCE_BYTES)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "frame session metadata".into(),
            })?;
    }
    Ok(bytes)
}
pub(super) fn reserve_history_metadata(
    previous: Option<&NavigationHistoryUpdate>,
    destination: &NavigationHistorySnapshot,
    budget: &Arc<std::sync::atomic::AtomicUsize>,
) -> Result<Arc<ScriptSourceReservation>, HostError> {
    let bytes = history_entries_bytes(
        previous
            .into_iter()
            .flat_map(|history| history.entries.iter())
            .chain(std::iter::once(destination)),
    )?;
    ScriptSourceReservation::reserve(budget, bytes)
        .map(Arc::new)
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "frame session publication metadata".into(),
        })
}
#[derive(Clone, Debug)]
pub(super) struct SessionHistory {
    pub(super) names: Option<Arc<SavedNames>>,
    pub(super) _metadata: Option<Arc<ScriptSourceReservation>>,
    pub(super) requests: Arc<BTreeMap<String, Arc<FrameRequest>>>,
    pub(super) update: Arc<NavigationHistoryUpdate>,
    pub(super) origins: Arc<BTreeMap<String, oryn_js::origin::Origin>>,
}
#[derive(Debug)]
pub(super) struct SavedNames {
    pub(super) values: BTreeMap<String, String>,
    _bytes: ScriptSourceReservation,
}
impl SavedNames {
    pub(super) fn from_pairs<'a>(
        values: impl IntoIterator<Item = (&'a str, &'a str)>,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Arc<Self>, HostError> {
        let mut pairs = Vec::new();
        let mut size = 0usize;
        for (id, name) in values {
            if pairs.len() >= 4096 || name.len() > 65_536 {
                return Err(HostError::LimitExceeded {
                    resource: "frame history names".into(),
                });
            }
            size = size
                .checked_add(id.len())
                .and_then(|bytes| bytes.checked_add(name.len()))
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<(String, String)>()))
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "frame history names".into(),
                })?;
            pairs.push((id, name));
        }
        let bytes = ScriptSourceReservation::reserve(budget, size).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "frame history names".into(),
            }
        })?;
        let values = pairs
            .into_iter()
            .map(|(id, name)| (id.to_owned(), name.to_owned()))
            .collect();
        Ok(Arc::new(Self {
            values,
            _bytes: bytes,
        }))
    }
}
impl SessionHistory {
    pub(super) fn capture_name(
        &mut self,
        origin: &oryn_js::origin::Origin,
        name: &str,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        let old = self.names.as_ref();
        let mut values: BTreeMap<&str, &str> = old
            .map(|names| {
                names
                    .values
                    .iter()
                    .filter(|(id, _)| {
                        self.update
                            .entries
                            .iter()
                            .any(|entry| &entry.document_id == *id)
                    })
                    .map(|(id, value)| (id.as_str(), value.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        // Opaque origins compare only by exact native identity, never serialized null.
        for entry in &self.update.entries {
            if self.origins.get(&entry.document_id) == Some(origin) {
                values.insert(&entry.document_id, name);
            }
        }
        self.names = Some(SavedNames::from_pairs(values, budget)?);
        Ok(())
    }
    pub(super) fn saved_name(&self, document: &str) -> Option<&str> {
        self.names
            .as_ref()?
            .values
            .get(document)
            .map(String::as_str)
    }

    fn visible_range(&self) -> (usize, usize) {
        let current = self.update.current;
        let selected = &self.update.entries[current];
        let origin = self.origins.get(&selected.document_id);
        let visible = |index: usize| {
            origin.is_some_and(|origin| {
                !origin.is_opaque()
                    && self.origins.get(&self.update.entries[index].document_id) == Some(origin)
            })
        };
        let mut start = current;
        let mut end = current + 1;
        while start > 0 && visible(start - 1) {
            start -= 1;
        }
        while end < self.update.entries.len() && visible(end) {
            end += 1;
        }
        (start, end)
    }
    pub(super) fn visible(&self) -> NavigationHistoryUpdate {
        let (start, end) = self.visible_range();
        NavigationHistoryUpdate {
            entries: self.update.entries[start..end].to_vec(),
            current: self.update.current - start,
            navigation_type: self.update.navigation_type.clone(),
            truncates_forward: false,
        }
    }
    pub(super) fn sync_native(
        &self,
        host: &mut RawV8Host,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        let (start, end) = self.visible_range();
        let _full_copy = ScriptSourceReservation::reserve(
            budget,
            history_entries_bytes(self.update.entries.iter())?,
        )
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "native frame history copy".into(),
        })?;
        {
            let _copy = ScriptSourceReservation::reserve(
                budget,
                history_entries_bytes(self.update.entries[start..end].iter())?,
            )
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "visible frame history copy".into(),
            })?;
            host.sync_navigation_history(self.visible())?;
        }
        host.sync_session_history((*self.update).clone())
    }
    pub(super) fn retain_metadata(
        &mut self,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        let bytes = history_entries_bytes(self.update.entries.iter())?;
        self._metadata = Some(Arc::new(
            ScriptSourceReservation::reserve(budget, bytes).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "frame session metadata".into(),
                }
            })?,
        ));
        Ok(())
    }
    pub(super) fn merge(
        &mut self,
        visible: NavigationHistoryUpdate,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        let current = &self.update.entries[self.update.current];
        let selected = visible
            .entries
            .get(visible.current)
            .ok_or_else(|| HostError::Execution("invalid frame history selection".into()))?;
        if selected.document_id != current.document_id {
            return Err(HostError::Execution(
                "frame history update document mismatch".into(),
            ));
        }
        let (start, end) = self.visible_range();
        let count = start.saturating_add(visible.entries.len()).saturating_add(
            if visible.truncates_forward {
                0
            } else {
                self.update.entries.len() - end
            },
        );
        if count > 4096 {
            return Err(HostError::LimitExceeded {
                resource: "frame session history".into(),
            });
        }
        let retained = self.update.entries[..start]
            .iter()
            .chain(visible.entries.iter())
            .chain(
                self.update.entries[end..]
                    .iter()
                    .filter(|_| !visible.truncates_forward),
            );
        let bytes = history_entries_bytes(retained)?;
        let reservation = ScriptSourceReservation::reserve(budget, bytes).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "frame session metadata".into(),
            }
        })?;
        let mut entries = self.update.entries[..start].to_vec();
        entries.extend(visible.entries);
        if !visible.truncates_forward {
            entries.extend_from_slice(&self.update.entries[end..]);
        }
        if entries.len() > 4096 {
            return Err(HostError::LimitExceeded {
                resource: "frame session history".into(),
            });
        }
        self.update = Arc::new(NavigationHistoryUpdate {
            entries,
            current: start + visible.current,
            navigation_type: visible.navigation_type,
            truncates_forward: visible.truncates_forward,
        });
        self._metadata = Some(Arc::new(reservation));
        Ok(())
    }
}
#[cfg(test)]
mod session_tests {
    use super::*;
    fn entry(id: &str, url: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: id.into(),
            id: id.into(),
            url: url.into(),
            document_id: id.into(),
            state: None,
            history_state: None,
        }
    }
    #[test]
    fn frame_session_keeps_hidden_cross_origin_history_outside_visible_segment() {
        let a = entry("a", "https://a.test/1");
        let b = entry("b", "https://b.test/2");
        let c = entry("c", "https://a.test/3");
        let origins = [a.clone(), b.clone(), c.clone()]
            .into_iter()
            .map(|entry| {
                (
                    entry.document_id,
                    oryn_js::origin::Origin::from_url(&entry.url),
                )
            })
            .collect();
        let mut session = SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::new()),
            update: Arc::new(NavigationHistoryUpdate {
                entries: vec![a, b, c],
                current: 1,
                navigation_type: "traverse".into(),
                truncates_forward: false,
            }),
            origins: Arc::new(origins),
        };
        let mut visible = session.visible();
        assert_eq!(visible.entries.len(), 1);
        visible.entries[0].history_state = None;
        visible.navigation_type = "replace".into();
        session
            .merge(visible, &Arc::new(std::sync::atomic::AtomicUsize::new(0)))
            .unwrap();
        assert_eq!(session.update.entries.len(), 3);
        assert_eq!(session.update.current, 1);
        assert_eq!(session.update.entries[0].id, "a");
        assert_eq!(session.update.entries[2].id, "c");
    }
    #[test]
    fn saved_names_share_strict_origin_group_and_reserve_before_replacing() {
        let a = entry("a", "https://a.test/1");
        let b = entry("b", "https://a.test/2");
        let c = entry("c", "https://b.test/3");
        let origin = oryn_js::origin::Origin::from_url(&a.url);
        let origins = [a.clone(), b.clone(), c.clone()]
            .into_iter()
            .map(|entry| {
                (
                    entry.document_id,
                    oryn_js::origin::Origin::from_url(&entry.url),
                )
            })
            .collect();
        let mut session = SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::new()),
            update: Arc::new(NavigationHistoryUpdate {
                entries: vec![a, b, c],
                current: 1,
                navigation_type: "push".into(),
                truncates_forward: false,
            }),
            origins: Arc::new(origins),
        };
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session.capture_name(&origin, "latest", &budget).unwrap();
        assert_eq!(session.saved_name("a"), Some("latest"));
        assert_eq!(session.saved_name("b"), Some("latest"));
        assert_eq!(session.saved_name("c"), None);
        let original = session.names.clone().unwrap();
        let retained = budget.load(std::sync::atomic::Ordering::Acquire);
        budget.store(
            MAX_QUEUED_SCRIPT_SOURCE_BYTES,
            std::sync::atomic::Ordering::Release,
        );
        assert!(
            session
                .capture_name(&origin, "replacement", &budget)
                .is_err()
        );
        assert!(Arc::ptr_eq(session.names.as_ref().unwrap(), &original));
        budget.store(retained, std::sync::atomic::Ordering::Release);
        drop(original);
        drop(session);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[test]
    fn opaque_saved_name_capture_never_groups_distinct_null_origins() {
        let a = entry("a", "data:text/html,a");
        let b = entry("b", "data:text/html,b");
        let first = oryn_js::origin::Origin::from_url(&a.url);
        let second = oryn_js::origin::Origin::from_url(&b.url);
        assert_ne!(first, second);
        let mut session = SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::new()),
            update: Arc::new(NavigationHistoryUpdate {
                entries: vec![a, b],
                current: 0,
                navigation_type: "push".into(),
                truncates_forward: false,
            }),
            origins: Arc::new(
                [("a".into(), first.clone()), ("b".into(), second)]
                    .into_iter()
                    .collect(),
            ),
        };
        session
            .capture_name(
                &first,
                "private",
                &Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            )
            .unwrap();
        assert_eq!(session.saved_name("a"), Some("private"));
        assert_eq!(session.saved_name("b"), None);
    }
    #[test]
    fn inherited_srcdoc_uses_native_origin_for_history_visibility() {
        let a = entry("a", "about:srcdoc");
        let b = entry("b", "about:blank#next");
        let origin = oryn_js::origin::Origin::from_url("https://parent.test/");
        let session = SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::new()),
            update: Arc::new(NavigationHistoryUpdate {
                entries: vec![a, b],
                current: 1,
                navigation_type: "push".into(),
                truncates_forward: false,
            }),
            origins: Arc::new(
                [("a".into(), origin.clone()), ("b".into(), origin)]
                    .into_iter()
                    .collect(),
            ),
        };
        assert_eq!(session.visible().entries.len(), 2);
    }
}

#[derive(Debug)]
pub(super) struct FrameRequest {
    pub(super) request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
    pub(super) post: Option<oryn_js::navigation::PostResource>,
    pub(super) authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    _bytes: ScriptSourceReservation,
}
impl FrameRequest {
    pub(super) fn capture(
        request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
        post: Option<oryn_js::navigation::PostResource>,
        authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Arc<Self>, HostError> {
        let size = post
            .as_ref()
            .map_or(0, |post| post.bytes.len() + post.content_type.len())
            + authority.as_ref().map_or(0, |authority| authority.0.len())
            + request_state
                .as_ref()
                .map(|state| state.retained_bytes())
                .transpose()
                .map_err(|error| HostError::Execution(error.into()))?
                .unwrap_or(0);
        let bytes = ScriptSourceReservation::reserve(budget, size).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "frame history request bytes".into(),
            }
        })?;
        Ok(Arc::new(Self {
            request_state,
            post,
            authority,
            _bytes: bytes,
        }))
    }
}

impl RustExecutableDocument {
    pub(super) fn capture_frame_navigation_history(
        &mut self,
        realm: RealmId,
        max_bytes: usize,
    ) -> Result<Option<oryn_js::form_state::SavedFrameNavigation>, HostError> {
        use oryn_js::form_state::{
            SavedFrameControls, SavedFrameNavigation, SavedFrameRequest, SavedFrameSource,
        };
        let Some(owner) = self.frame_realms.get(&realm).cloned() else {
            return Ok(None);
        };
        self.agent.host.activate_realm(realm)?;
        let visible = self
            .agent
            .host
            .navigation_history_snapshot_limited(max_bytes)?;
        let origin = self
            .agent
            .host
            .realm_descriptor(realm)
            .ok_or_else(|| HostError::Execution("frame history origin missing".into()))?
            .origin
            .clone();
        let mut session = match owner.session {
            Some(mut session) => {
                session.merge(visible, &self.dynamic_scripts.source_bytes)?;
                session
            }
            None => {
                let mut session = SessionHistory {
                    names: None,
                    _metadata: None,
                    requests: Arc::new(BTreeMap::new()),
                    origins: Arc::new(
                        visible
                            .entries
                            .iter()
                            .map(|entry| (entry.document_id.clone(), origin.clone()))
                            .collect(),
                    ),
                    update: Arc::new(visible),
                };
                session.retain_metadata(&self.dynamic_scripts.source_bytes)?;
                session
            }
        };
        if let Some(name) = self.agent.host.native_window_target_name(realm) {
            session.capture_name(&origin, &name, &self.dynamic_scripts.source_bytes)?;
        }
        let current = session.update.entries[session.update.current]
            .document_id
            .clone();
        let mut sources = self
            .archived_frame_sources
            .iter()
            .filter(|((parent, frame, _), _)| {
                *parent == owner.parent_realm && *frame == owner.frame
            })
            .map(|((_, _, document), source)| (document.clone(), source.clone()))
            .collect::<BTreeMap<_, _>>();
        if let Some(source) = owner.source {
            source.capture_forms(
                &mut self.agent.host,
                realm,
                &self.dynamic_scripts.source_bytes,
            )?;
            sources.insert(current.clone(), source);
        } else {
            if self.archived_frame_controls.len() >= 1024
                && !self.archived_frame_controls.contains_key(&(
                    owner.parent_realm,
                    owner.frame,
                    current.clone(),
                ))
            {
                return Err(HostError::LimitExceeded {
                    resource: "frame control history entries".into(),
                });
            }
            let controls = ArchivedControls::capture(
                &mut self.agent.host,
                realm,
                &self.dynamic_scripts.source_bytes,
            )?;
            self.archived_frame_controls
                .insert((owner.parent_realm, owner.frame, current.clone()), controls);
        }
        let documents = session
            .update
            .entries
            .iter()
            .map(|entry| entry.document_id.clone())
            .collect::<BTreeSet<_>>();
        sources.retain(|id, _| documents.contains(id));
        let document_controls = self
            .archived_frame_controls
            .iter()
            .filter(|((parent, frame, document), _)| {
                *parent == owner.parent_realm
                    && *frame == owner.frame
                    && documents.contains(document)
            })
            .map(|((_, _, document), controls)| (document.clone(), controls.clone()))
            .collect::<Vec<_>>();
        let mut output_budget = oryn_js::form_state::FormSnapshotBudget::default();
        output_budget
            .reserve_bytes(oryn_js::form_state::MAX_FORM_SNAPSHOT_BYTES.saturating_sub(max_bytes))
            .map_err(|resource| HostError::LimitExceeded {
                resource: resource.into(),
            })?;
        let mut reserve = |bytes| {
            output_budget
                .reserve_bytes(bytes)
                .map_err(|resource| HostError::LimitExceeded {
                    resource: resource.into(),
                })
        };
        for entry in &session.update.entries {
            for value in [&entry.key, &entry.id, &entry.url, &entry.document_id] {
                reserve(value.len())?;
            }
            for state in [&entry.state, &entry.history_state] {
                if let Some(state) = state {
                    reserve(state.retained_bytes())?;
                }
            }
        }
        for (id, origin) in session
            .origins
            .iter()
            .filter(|(id, _)| documents.contains(*id))
        {
            reserve(id.len() + origin.serialize().len())?;
        }
        for (id, source) in &sources {
            reserve(
                id.len()
                    + source.source.len()
                    + source.base_url.as_ref().map_or(0, |url| url.as_str().len()),
            )?;
            reserve(
                oryn_js::form_state::snapshot_bytes(&source.forms.borrow()).map_err(
                    |resource| HostError::LimitExceeded {
                        resource: resource.into(),
                    },
                )?,
            )?;
        }
        for (id, request) in session
            .requests
            .iter()
            .filter(|(id, _)| documents.contains(*id))
        {
            reserve(id.len())?;
            if let Some(state) = &request.request_state {
                reserve(
                    state
                        .retained_bytes()
                        .map_err(|error| HostError::Execution(error.into()))?,
                )?;
            }
            if let Some(post) = &request.post {
                reserve(post.bytes.len() + post.content_type.len())?;
            }
            if let Some(authority) = &request.authority {
                reserve(authority.0.len())?;
            }
        }
        if let Some(names) = &session.names {
            for (id, name) in &names.values {
                if documents.contains(id) {
                    reserve(id.len())?;
                    reserve(name.len())?;
                }
            }
        }
        for (document, controls) in &document_controls {
            reserve(document.len())?;
            reserve(
                oryn_js::form_state::snapshot_bytes(&controls.controls).map_err(|resource| {
                    HostError::LimitExceeded {
                        resource: resource.into(),
                    }
                })?,
            )?;
        }
        let snapshot = SavedFrameNavigation {
            saved_names: session
                .names
                .as_ref()
                .map(|names| {
                    names
                        .values
                        .iter()
                        .filter(|(id, _)| documents.contains(*id))
                        .map(|(id, name)| (id.clone(), name.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            document_controls: document_controls
                .into_iter()
                .map(|(document_id, controls)| SavedFrameControls {
                    document_id,
                    controls: controls.controls.clone(),
                })
                .collect(),
            joint_route: Some(self.agent.host.joint_history_route()?),
            history: (*session.update).clone(),
            origins: session
                .origins
                .iter()
                .filter(|(id, _)| documents.contains(*id))
                .map(|(id, origin)| (id.clone(), origin.serialize()))
                .collect(),
            sources: sources
                .into_iter()
                .map(|(document_id, source)| SavedFrameSource {
                    document_id,
                    source: source.source.to_string(),
                    base_url: source.base_url.as_ref().map(Url::to_string),
                    controls: source.forms.borrow().clone(),
                })
                .collect(),
            requests: session
                .requests
                .iter()
                .filter(|(id, _)| documents.contains(*id))
                .map(|(document_id, request)| SavedFrameRequest {
                    request_state: request.request_state.clone(),
                    document_id: document_id.clone(),
                    post: request.post.clone(),
                    authority: request.authority.clone(),
                })
                .collect(),
        };
        snapshot
            .retained_bytes()
            .map_err(|resource| HostError::LimitExceeded {
                resource: resource.into(),
            })?;
        Ok(Some(snapshot))
    }
    pub(super) fn restore_frame_navigation_history(
        &mut self,
        realm: RealmId,
        saved: Option<&oryn_js::form_state::SavedFrameNavigation>,
    ) -> Result<(), HostError> {
        let Some(saved) = saved else {
            return Ok(());
        };
        saved
            .retained_bytes()
            .map_err(|resource| HostError::LimitExceeded {
                resource: resource.into(),
            })?;
        let Some(owner) = self.frame_realms.get(&realm).cloned() else {
            return Ok(());
        };
        self.agent.host.activate_realm(realm)?;
        let metadata = Arc::new(
            ScriptSourceReservation::reserve(
                &self.dynamic_scripts.source_bytes,
                history_entries_bytes(saved.history.entries.iter())?,
            )
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "restored frame metadata".into(),
            })?,
        );
        if let Some(route) = &saved.joint_route {
            if self
                .agent
                .host
                .joint_history_snapshot()?
                .selected(route)
                .is_some()
            {
                self.agent.host.restore_joint_history_route(route)?;
            }
        }
        let selected = &saved.history.entries[saved.history.current];
        if self.agent.host.location_url()? != selected.url {
            return Err(HostError::Execution(
                "restored frame history URL mismatch".into(),
            ));
        }
        // Serialized origin metadata controls history visibility only. It never
        // grants resource, realm, or cross-origin access authority.
        let mut origins = saved
            .origins
            .iter()
            .map(|(id, origin)| (id.clone(), oryn_js::origin::Origin::from_url(origin)))
            .collect::<BTreeMap<_, _>>();
        let mut requests = BTreeMap::new();
        for request in &saved.requests {
            requests.insert(
                request.document_id.clone(),
                FrameRequest::capture(
                    request.request_state.clone(),
                    request.post.clone(),
                    request.authority.clone(),
                    &self.dynamic_scripts.source_bytes,
                )?,
            );
        }
        let native_document = selected.document_id.clone();
        let history = saved.history.clone();
        let native_origin = self
            .agent
            .host
            .realm_descriptor(realm)
            .ok_or_else(|| HostError::Execution("restored frame realm missing".into()))?
            .origin
            .clone();
        origins.insert(native_document.clone(), native_origin);
        let names = SavedNames::from_pairs(
            saved
                .saved_names
                .iter()
                .map(|(id, name)| (id.as_str(), name.as_str())),
            &self.dynamic_scripts.source_bytes,
        )?;
        let session = SessionHistory {
            names: Some(names),
            _metadata: Some(metadata),
            update: Arc::new(history),
            origins: Arc::new(origins),
            requests: Arc::new(requests),
        };
        if self.archived_frame_controls.len().saturating_add(
            saved
                .document_controls
                .iter()
                .filter(|entry| {
                    !self.archived_frame_controls.contains_key(&(
                        owner.parent_realm,
                        owner.frame,
                        entry.document_id.clone(),
                    ))
                })
                .count(),
        ) > 1024
        {
            return Err(HostError::LimitExceeded {
                resource: "frame control history entries".into(),
            });
        }
        for entry in &saved.document_controls {
            let controls =
                ArchivedControls::restore(&entry.controls, &self.dynamic_scripts.source_bytes)?;
            self.archived_frame_controls.insert(
                (owner.parent_realm, owner.frame, entry.document_id.clone()),
                controls,
            );
        }
        for source in &saved.sources {
            let archived = ArchivedSource::capture(
                &source.source,
                source
                    .base_url
                    .as_ref()
                    .and_then(|url| Url::parse(url).ok()),
                &self.dynamic_scripts.source_bytes,
            )?;
            archived.clone_forms(&source.controls, &self.dynamic_scripts.source_bytes)?;
            self.archived_frame_sources.insert(
                (owner.parent_realm, owner.frame, source.document_id.clone()),
                archived,
            );
        }
        if let Some(name) = session.saved_name(&selected.document_id) {
            self.agent
                .host
                .set_native_window_target_name(realm, name.to_owned())?;
        }
        session.sync_native(&mut self.agent.host, &self.dynamic_scripts.source_bytes)?;
        self.agent
            .host
            .sync_navigation_activation(None, "traverse")?;
        if let Some(owner) = self.frame_realms.get_mut(&realm) {
            owner.source = self
                .archived_frame_sources
                .get(&(
                    owner.parent_realm,
                    owner.frame,
                    selected.document_id.clone(),
                ))
                .cloned();
            if let Some(source) = owner.source.clone() {
                self.archived_frame_sources
                    .insert((owner.parent_realm, owner.frame, native_document), source);
            }
            owner.session = Some(session);
        }
        Ok(())
    }
}
impl RustExecutableDocument {
    pub(super) fn bind_pending_frame_history_owners(&mut self) -> Result<(), HostError> {
        for entry in &self.pending_frame_navigation_restore {
            let key = frame_form_history::frame_history_path_key(&entry.path)?;
            if self.frame_history_initial_claims.contains_key(&key) {
                continue;
            }
            let matches = self
                .frame_realms
                .iter()
                .filter_map(|(realm, owner)| {
                    self.frame_form_path(*realm)
                        .ok()
                        .filter(|path| {
                            path.len() == entry.path.len()
                                && path.iter().zip(&entry.path).all(|(a, b)| {
                                    a.tag == b.tag
                                        && a.id == b.id
                                        && a.name == b.name
                                        && a.occurrence == b.occurrence
                                })
                        })
                        .map(|_| owner.clone())
                })
                .collect::<Vec<_>>();
            if matches.len() == 1 {
                let owner = &matches[0];
                if let Some(dom) = self.agent.host.automation_dom(owner.parent_realm) {
                    let token = dom.borrow_mut().frame_navigation_identity(owner.frame);
                    self.frame_history_initial_claims
                        .insert(key, (owner.parent_realm, owner.frame, token));
                }
            }
        }
        Ok(())
    }
    /// Apply root-history child state before this child's first parser script.
    pub(super) fn restore_pending_frame_before_scripts(
        &mut self,
        realm: RealmId,
    ) -> Result<Option<Vec<oryn_js::form_state::SavedFormControl>>, HostError> {
        let Some(owner) = self.frame_realms.get(&realm).cloned() else {
            return Ok(None);
        };
        let Ok(path) = self.frame_form_path(realm) else {
            return Ok(None);
        };
        let equivalent = |left: &[oryn_js::form_state::FrameHistoryKey],
                          right: &[oryn_js::form_state::FrameHistoryKey]| {
            left.len() == right.len()
                && left.iter().zip(right).all(|(a, b)| {
                    a.tag == b.tag
                        && a.id == b.id
                        && a.name == b.name
                        && a.occurrence == b.occurrence
                })
        };
        let matches = self
            .pending_frame_navigation_restore
            .iter()
            .enumerate()
            .filter(|(_, entry)| equivalent(&path, &entry.path))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Ok(None);
        }
        let mut saved = self.pending_frame_navigation_restore.remove(matches[0]);
        let key = frame_form_history::frame_history_path_key(&saved.path)?;
        if let Some((parent, frame, token)) = self.frame_history_initial_claims.remove(&key) {
            if parent != owner.parent_realm
                || frame != owner.frame
                || !self.agent.host.automation_frame_live(parent, frame)
                || !self.agent.host.automation_dom(parent).is_some_and(|dom| {
                    dom.borrow()
                        .frame_navigation_identity_is_current(frame, &token)
                })
            {
                return Ok(None);
            }
        }
        self.agent.host.activate_realm(realm)?;
        if let Some(navigation) = saved.navigation.as_mut() {
            if let Some(route) = &navigation.joint_route {
                let timeline = self.agent.host.joint_history_snapshot()?;
                if let Some(selected) = timeline.selected(route) {
                    let current = navigation
                        .history
                        .entries
                        .iter()
                        .position(|entry| entry.id == selected.id && entry.key == selected.key)
                        .ok_or_else(|| {
                            HostError::Execution("saved frame joint destination missing".into())
                        })?;
                    if current != navigation.history.current {
                        saved.controls = navigation
                            .sources
                            .iter()
                            .find(|source| source.document_id == selected.document_id)
                            .map(|source| source.controls.clone())
                            .or_else(|| {
                                navigation
                                    .document_controls
                                    .iter()
                                    .find(|entry| entry.document_id == selected.document_id)
                                    .map(|entry| entry.controls.clone())
                            })
                            .unwrap_or_default();
                        navigation.history.current = current;
                    }
                    navigation.history.entries[current] = selected.clone();
                    self.agent.host.restore_joint_history_route(route)?;
                }
            }
        }
        let Some(navigation) = saved.navigation.as_ref() else {
            return Ok(Some(saved.controls));
        };
        let destination = navigation.history.entries[navigation.history.current].clone();
        self.agent.host.activate_realm(realm)?;
        if let Some((_, name)) = navigation
            .saved_names
            .iter()
            .find(|(id, _)| id == &destination.document_id)
        {
            self.agent
                .host
                .set_native_window_target_name(realm, name.clone())?;
        }
        let source_matches = navigation
            .sources
            .iter()
            .find(|source| source.document_id == destination.document_id)
            .is_none_or(|source| {
                owner
                    .source
                    .as_ref()
                    .is_some_and(|current| current.source.as_ref() == source.source)
            });
        let replay_post = navigation.requests.iter().any(|request| {
            request.document_id == destination.document_id && request.post.is_some()
        });
        if self.agent.host.location_url()? == destination.url && source_matches && !replay_post {
            self.restore_frame_navigation_history(realm, Some(navigation))?;
            return Ok(Some(saved.controls));
        }
        let metadata = Arc::new(
            ScriptSourceReservation::reserve(
                &self.dynamic_scripts.source_bytes,
                history_entries_bytes(navigation.history.entries.iter())?,
            )
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "restored frame metadata".into(),
            })?,
        );
        let mut requests = BTreeMap::new();
        for request in &navigation.requests {
            requests.insert(
                request.document_id.clone(),
                FrameRequest::capture(
                    request.request_state.clone(),
                    request.post.clone(),
                    request.authority.clone(),
                    &self.dynamic_scripts.source_bytes,
                )?,
            );
        }
        let origins = navigation
            .origins
            .iter()
            .map(|(id, origin)| (id.clone(), oryn_js::origin::Origin::from_url(origin)))
            .collect();
        let names = SavedNames::from_pairs(
            navigation
                .saved_names
                .iter()
                .map(|(id, name)| (id.as_str(), name.as_str())),
            &self.dynamic_scripts.source_bytes,
        )?;
        let session = SessionHistory {
            names: Some(names),
            _metadata: Some(metadata),
            update: Arc::new(navigation.history.clone()),
            origins: Arc::new(origins),
            requests: Arc::new(requests),
        };
        if self.archived_frame_controls.len().saturating_add(
            navigation
                .document_controls
                .iter()
                .filter(|entry| {
                    !self.archived_frame_controls.contains_key(&(
                        owner.parent_realm,
                        owner.frame,
                        entry.document_id.clone(),
                    ))
                })
                .count(),
        ) > 1024
        {
            return Err(HostError::LimitExceeded {
                resource: "frame control history entries".into(),
            });
        }
        for entry in &navigation.document_controls {
            let controls =
                ArchivedControls::restore(&entry.controls, &self.dynamic_scripts.source_bytes)?;
            self.archived_frame_controls.insert(
                (owner.parent_realm, owner.frame, entry.document_id.clone()),
                controls,
            );
        }
        for source in &navigation.sources {
            let archived = ArchivedSource::capture(
                &source.source,
                source
                    .base_url
                    .as_ref()
                    .and_then(|url| Url::parse(url).ok()),
                &self.dynamic_scripts.source_bytes,
            )?;
            archived.clone_forms(&source.controls, &self.dynamic_scripts.source_bytes)?;
            self.archived_frame_sources.insert(
                (owner.parent_realm, owner.frame, source.document_id.clone()),
                archived,
            );
        }
        let source = self
            .archived_frame_sources
            .get(&(
                owner.parent_realm,
                owner.frame,
                destination.document_id.clone(),
            ))
            .cloned();
        let request = session.requests.get(&destination.document_id);
        let post = request.and_then(|request| request.post.clone());
        let authority = request.and_then(|request| request.authority.clone());
        self.agent.host.activate_realm(owner.parent_realm)?;
        let dom =
            self.agent.host.active_dom().ok_or_else(|| {
                HostError::Execution("restored frame parent DOM unavailable".into())
            })?;
        dom.borrow_mut()
            .remove_attribute(owner.frame, "srcdoc")
            .map_err(|error| HostError::Execution(error.to_string()))?;
        dom.borrow_mut()
            .set_attribute(owner.frame, "src", destination.url.clone())
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.pending_frame_navigations.insert(
            (owner.parent_realm, owner.frame),
            FrameNavigationContext {
                execution_observer: None,
                download: None,
                session: Some(session),
                archived_source: source,
                destination: Some(destination),
                history: Some(navigation.history.clone()),
                replace: false,
                reload: false,
                script_authority: None,
                form_authority: authority,
                initiator_url: None,
                referrer_policy: None,
                post,
            },
        );
        self.agent.host.reschedule_dynamic_frame(owner.frame)?;
        let claim_key = frame_form_history::frame_history_path_key(&saved.path)?;
        let claim = dom.borrow_mut().frame_navigation_identity(owner.frame);
        self.frame_history_restore_claims
            .insert(claim_key, (owner.parent_realm, owner.frame, claim));
        // Retain state until the replacement parser has finished; source custody is
        // already queued under an exact current native parent/frame navigation token.
        let identity = self
            .form_history_root_identity()
            .ok_or_else(|| HostError::Execution("restored frame root retired".into()))?;
        if let Some((owner, entries)) = self
            .pending_frame_form_history
            .as_mut()
            .filter(|(owner, _)| *owner == identity)
        {
            let _ = owner;
            entries.push(saved);
        } else {
            self.pending_frame_form_history = Some((identity, vec![saved]));
        }
        let stale = self
            .frame_realms
            .keys()
            .copied()
            .filter(|candidate| {
                let mut cursor = Some(*candidate);
                let mut seen = BTreeSet::new();
                while let Some(current) = cursor {
                    if current == realm {
                        return true;
                    }
                    if !seen.insert(current) {
                        break;
                    }
                    cursor = self
                        .frame_realms
                        .get(&current)
                        .map(|owner| owner.parent_realm);
                }
                false
            })
            .collect::<BTreeSet<_>>();
        // This native history replacement authorizes descendant descriptors to
        // bind to its newly constructed parent. Author moves/replacements never
        // clear these claims and therefore cannot obtain this rebinding.
        self.frame_history_initial_claims
            .retain(|_, (parent, _, _)| !stale.contains(parent));
        for old in &stale {
            self.agent.host.stop_pending_document_load(*old)?;
        }
        self.frame_script_preparations
            .retain(|pending| !stale.contains(&pending.load.realm));
        Ok(None)
    }
}

#[cfg(test)]
mod request_custody_tests {
    use super::*;
    #[test]
    fn frame_history_post_custody_shared_until_last_owner_and_redirect_semantics() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let post = oryn_js::navigation::PostResource {
            bytes: vec![0, 255, 1],
            content_type: "application/octet-stream".into(),
        };
        let mut redirected = Some(post.clone());
        oryn_js::navigation::PostResource::redirect(&mut redirected, 307);
        let request = FrameRequest::capture(None, redirected, None, &budget).unwrap();
        let retained = request.clone();
        drop(request);
        assert_eq!(retained.post.as_ref().unwrap().bytes, vec![0, 255, 1]);
        assert!(budget.load(std::sync::atomic::Ordering::SeqCst) > 0);
        drop(retained);
        assert_eq!(budget.load(std::sync::atomic::Ordering::SeqCst), 0);
        let mut redirected = Some(post);
        oryn_js::navigation::PostResource::redirect(&mut redirected, 303);
        assert!(redirected.is_none());
    }
    #[test]
    fn frame_history_get_request_provenance_owns_and_releases_its_bytes() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = oryn_common::navigation_request::NavigationRequestState {
            initiator_url: Some("https://source.test/".into()),
            opaque_initiator: false,
            referrer: None,
            referrer_policy: "no-referrer".into(),
        };
        let size = state.retained_bytes().unwrap();
        let request = FrameRequest::capture(Some(state.clone()), None, None, &budget).unwrap();
        assert_eq!(budget.load(std::sync::atomic::Ordering::SeqCst), size);
        assert_eq!(request.request_state, Some(state));
        let second = request.clone();
        drop(request);
        assert_eq!(budget.load(std::sync::atomic::Ordering::SeqCst), size);
        drop(second);
        assert_eq!(budget.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    #[test]
    fn frame_response_native_origin_retains_opaque_grant_identity() {
        let grant = oryn_js::worker_wire::AgentOrigin {
            serialized: "null".into(),
            authority: format!("opaque-frame:{}:1:1", "a".repeat(64)),
            opaque: true,
        };
        let first = native_frame_response_origin(&grant).unwrap();
        let second = native_frame_response_origin(&grant).unwrap();
        assert_eq!(first, second);
        assert!(first.is_opaque());
        let invalid = oryn_js::worker_wire::AgentOrigin {
            serialized: "https://example.test".into(),
            ..grant
        };
        assert!(native_frame_response_origin(&invalid).is_err());
    }
}

#[cfg(test)]
mod archive_allocation_tests {
    use super::*;
    #[test]
    fn archived_controls_reserve_before_clone_and_release_with_source() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source = ArchivedSource::capture("html", None, &budget).unwrap();
        let node = NodeId {
            slot: 1,
            generation: 1,
        };
        let saved = oryn_js::form_state::SavedFormControl {
            node,
            document: node,
            definition: "#builtin/input/text".into(),
            name: "value".into(),
            form: None,
            occurrence: 0,
            state: oryn_js::form_state::FormState::String("retained".into()),
        };
        source
            .clone_forms(std::slice::from_ref(&saved), &budget)
            .unwrap();
        let retained = budget.load(std::sync::atomic::Ordering::Acquire);
        assert!(retained > 4);
        let exhaust =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES - retained)
                .unwrap();
        assert!(
            source
                .clone_forms(std::slice::from_ref(&saved), &budget)
                .is_err()
        );
        assert_eq!(source.forms.borrow()[0].state, saved.state);
        drop(exhaust);
        drop(source);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
}

#[cfg(test)]
mod session_allocation_tests {
    use super::*;
    #[test]
    fn frame_session_clones_share_history_and_failed_merge_preserves_old_snapshot() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let entry = NavigationHistorySnapshot {
            scroll: Default::default(),
            key: "key".into(),
            id: "id".into(),
            url: "https://example.test/".into(),
            document_id: "document".into(),
            state: None,
            history_state: None,
        };
        let mut session = SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::new()),
            update: Arc::new(NavigationHistoryUpdate {
                entries: vec![entry],
                current: 0,
                navigation_type: "push".into(),
                truncates_forward: false,
            }),
            origins: Arc::new(
                [(
                    "document".into(),
                    oryn_js::origin::Origin::from_url("https://example.test/"),
                )]
                .into_iter()
                .collect(),
            ),
        };
        session.retain_metadata(&budget).unwrap();
        let retained = session.clone();
        assert!(Arc::ptr_eq(&session.update, &retained.update));
        let bytes = budget.load(std::sync::atomic::Ordering::Acquire);
        let full =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES - bytes)
                .unwrap();
        let visible = session.visible();
        assert!(session.merge(visible, &budget).is_err());
        assert!(Arc::ptr_eq(&session.update, &retained.update));
        drop(full);
        drop(session);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), bytes);
        drop(retained);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[test]
    fn native_history_snapshot_rejects_capacity_before_owned_snapshot_and_keeps_current() {
        let mut page = RustExecutableDocument::load("<p>snapshot</p>").unwrap();
        let before = page.agent.host.navigation_history_snapshot().unwrap();
        assert!(
            page.agent
                .host
                .navigation_history_snapshot_limited(0)
                .is_err()
        );
        let after = page.agent.host.navigation_history_snapshot().unwrap();
        assert_eq!(
            before.entries[before.current].id,
            after.entries[after.current].id
        );
    }
}

#[cfg(test)]
mod network_control_custody_tests {
    use super::*;
    #[test]
    fn network_controls_have_independent_budgeted_custody_and_native_restore() {
        let mut page = RustExecutableDocument::load("<input name=field value=default>").unwrap();
        page.execute("document.querySelector('input').value='network saved'")
            .unwrap();
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let controls =
            ArchivedControls::capture(&mut page.agent.host, page.top_realm, &budget).unwrap();
        let bytes = budget.load(std::sync::atomic::Ordering::Acquire);
        assert!(bytes > 0);
        page.execute("document.querySelector('input').value='changed'")
            .unwrap();
        page.agent
            .host
            .restore_form_history_in_realm(page.top_realm, &controls.controls)
            .unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("document.querySelector('input').value")
                .unwrap(),
            "network saved"
        );
        let retained = controls.clone();
        drop(controls);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), bytes);
        let full =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES - bytes)
                .unwrap();
        assert!(ArchivedControls::restore(&retained.controls, &budget).is_err());
        drop(full);
        drop(retained);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
}
