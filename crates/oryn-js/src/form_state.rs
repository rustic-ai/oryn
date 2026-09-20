//! Worker-safe custom-control restoration data. Submission values are separate.
use oryn_dom::NodeId;
use serde::{Deserialize, Serialize};
pub const MAX_FORM_SNAPSHOT_CONTROLS: usize = 4096;
pub const MAX_FORM_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

/// Producer-side reservation must precede cloning strings/files into a snapshot.
/// This bounds owned payload and record storage, not the source document's heap.
#[derive(Default)]
pub struct FormSnapshotBudget {
    controls: usize,
    bytes: usize,
}
impl FormSnapshotBudget {
    pub fn reserve_control(&mut self) -> Result<(), &'static str> {
        if self.controls >= MAX_FORM_SNAPSHOT_CONTROLS {
            return Err("form history control limit");
        }
        self.reserve_bytes(std::mem::size_of::<SavedFormControl>())?;
        self.controls += 1;
        Ok(())
    }
    pub fn reserve_bytes(&mut self, bytes: usize) -> Result<(), &'static str> {
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or("form history byte limit")?;
        if total > MAX_FORM_SNAPSHOT_BYTES {
            return Err("form history byte limit");
        }
        self.bytes = total;
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Revalidate already materialized snapshots at a native restoration boundary.
pub fn snapshot_bytes(saved: &[SavedFormControl]) -> Result<usize, &'static str> {
    fn file(budget: &mut FormSnapshotBudget, value: &FormFile) -> Result<(), &'static str> {
        budget.reserve_bytes(value.name.len())?;
        budget.reserve_bytes(value.media_type.len())?;
        budget.reserve_bytes(value.bytes.len())
    }
    let mut budget = FormSnapshotBudget::default();
    for control in saved {
        budget.reserve_control()?;
        budget.reserve_bytes(control.definition.len())?;
        budget.reserve_bytes(control.name.len())?;
        if let Some(form) = &control.form {
            budget.reserve_bytes(form.id.len())?;
            budget.reserve_bytes(form.name.len())?;
            budget.reserve_bytes(form.action.len())?;
        }
        match &control.state {
            FormState::Null => {}
            FormState::String(value) => budget.reserve_bytes(value.len())?,
            FormState::File(value) => file(&mut budget, value)?,
            FormState::Entries(entries) => {
                for (name, value) in entries {
                    budget.reserve_bytes(std::mem::size_of::<(String, FormEntryValue)>())?;
                    budget.reserve_bytes(name.len())?;
                    match value {
                        FormEntryValue::String(value) => budget.reserve_bytes(value.len())?,
                        FormEntryValue::File(value) => file(&mut budget, value)?,
                    }
                }
            }
        }
    }
    Ok(budget.bytes())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormFile {
    pub name: String,
    pub media_type: String,
    pub last_modified: i64,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormEntryValue {
    String(String),
    File(FormFile),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormState {
    Null,
    String(String),
    File(FormFile),
    Entries(Vec<(String, FormEntryValue)>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormOwnerIdentity {
    pub node: NodeId,
    pub id: String,
    pub name: String,
    pub action: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedFormControl {
    pub node: NodeId,
    pub document: NodeId,
    pub definition: String,
    pub name: String,
    pub form: Option<FormOwnerIdentity>,
    /// Occurrence in native tree order among matching definition/name/form.
    pub occurrence: usize,
    pub state: FormState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormRestoreTarget {
    /// A retained same-document state must still identify the exact generation.
    Exact,
    /// Native history supplies this mapping after resolving cross-document
    /// ownership and metadata. The binding never guesses by control name.
    Mapped { node: NodeId, document: NodeId },
}

/// Resolve a native history snapshot against newly constructed controls.
/// The caller must first establish the history entry's document/URL authority.
/// Ambiguous forms or duplicated control descriptors are never guessed.
pub fn restoration_mapping(
    saved: &[SavedFormControl],
    live: &[SavedFormControl],
) -> Vec<(usize, FormRestoreTarget)> {
    type Key<'a> = (&'a str, &'a str, usize, Option<(&'a str, &'a str, &'a str)>);
    fn key(control: &SavedFormControl) -> Key<'_> {
        (
            &control.definition,
            &control.name,
            control.occurrence,
            control
                .form
                .as_ref()
                .map(|form| (form.id.as_str(), form.name.as_str(), form.action.as_str())),
        )
    }
    fn ambiguous_forms(
        controls: &[SavedFormControl],
    ) -> std::collections::BTreeSet<(&str, &str, &str)> {
        let mut owners = std::collections::BTreeMap::new();
        for control in controls {
            if let Some(form) = &control.form {
                owners
                    .entry((form.id.as_str(), form.name.as_str(), form.action.as_str()))
                    .or_insert_with(std::collections::BTreeSet::new)
                    .insert((control.document, form.node));
            }
        }
        owners
            .into_iter()
            .filter_map(|(key, nodes)| (nodes.len() > 1).then_some(key))
            .collect()
    }
    let saved_ambiguous = ambiguous_forms(saved);
    let live_ambiguous = ambiguous_forms(live);
    let mut saved_counts = std::collections::BTreeMap::new();
    for control in saved {
        *saved_counts.entry(key(control)).or_insert(0usize) += 1;
    }
    let mut destinations = std::collections::BTreeMap::new();
    for control in live {
        destinations
            .entry(key(control))
            .and_modify(|candidate| *candidate = None)
            .or_insert(Some(control));
    }
    saved
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            if saved_counts.get(&key(entry)) != Some(&1) {
                return None;
            }
            if let Some(form) = &entry.form {
                let form_key = (form.id.as_str(), form.name.as_str(), form.action.as_str());
                if saved_ambiguous.contains(&form_key) || live_ambiguous.contains(&form_key) {
                    return None;
                }
            }
            let candidate = destinations.get(&key(entry)).copied().flatten()?;
            Some((
                index,
                FormRestoreTarget::Mapped {
                    node: candidate.node,
                    document: candidate.document,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    #[test]
    fn reservations_are_bounded_and_failed_reservations_do_not_consume_budget() {
        let mut budget = FormSnapshotBudget::default();
        budget.reserve_bytes(MAX_FORM_SNAPSHOT_BYTES - 1).unwrap();
        assert!(budget.reserve_bytes(2).is_err());
        assert!(budget.reserve_bytes(usize::MAX).is_err());
        assert_eq!(budget.bytes(), MAX_FORM_SNAPSHOT_BYTES - 1);
        budget.reserve_bytes(1).unwrap();
        assert!(budget.reserve_control().is_err());
        let mut controls = FormSnapshotBudget::default();
        for _ in 0..MAX_FORM_SNAPSHOT_CONTROLS {
            controls.reserve_control().unwrap();
        }
        assert!(controls.reserve_control().is_err());
    }
    #[test]
    fn snapshot_accounting_includes_metadata_file_payloads_and_entry_storage() {
        let mut saved = control(1);
        saved.state = FormState::Null;
        let empty = snapshot_bytes(&[saved.clone()]).unwrap();
        saved.state = FormState::Entries(vec![(
            "upload".into(),
            FormEntryValue::File(FormFile {
                name: "a.txt".into(),
                media_type: "text/plain".into(),
                last_modified: 0,
                bytes: vec![1, 2, 3],
            }),
        )]);
        assert_eq!(
            snapshot_bytes(&[saved]).unwrap() - empty,
            std::mem::size_of::<(String, FormEntryValue)>()
                + "upload".len()
                + "a.txt".len()
                + "text/plain".len()
                + 3
        );
    }
    fn control(slot: u32) -> SavedFormControl {
        SavedFormControl {
            node: NodeId {
                slot,
                generation: 1,
            },
            document: NodeId {
                slot: 0,
                generation: 1,
            },
            definition: "x-input".into(),
            name: "entry".into(),
            form: None,
            occurrence: 0,
            state: FormState::String("saved".into()),
        }
    }
    #[test]
    fn mapping_rejects_ambiguous_form_owners_even_with_distinct_control_names() {
        let mut first = control(1);
        let mut second = control(2);
        second.name = "different".into();
        let form = |slot| FormOwnerIdentity {
            node: NodeId {
                slot,
                generation: 1,
            },
            id: String::new(),
            name: String::new(),
            action: "/save".into(),
        };
        first.form = Some(form(10));
        second.form = Some(form(11));
        let saved = vec![first.clone(), second.clone()];
        let mut live = saved.clone();
        live[0].node.slot = 20;
        live[1].node.slot = 21;
        assert!(restoration_mapping(&saved, &live).is_empty());
        // Ambiguity on either side is sufficient, even if the other side has
        // only one currently constructed control.
        assert!(restoration_mapping(&saved, &live[..1]).is_empty());
        assert!(restoration_mapping(&saved[..1], &live).is_empty());
        // Multiple controls in one identified form remain eligible.
        second.form = first.form.clone();
        let same_form = vec![first, second];
        assert_eq!(restoration_mapping(&same_form, &same_form).len(), 2);
        // Distinct native forms with distinct metadata are not ambiguous.
        let mut identified = saved;
        identified[0].form.as_mut().unwrap().id = "first".into();
        identified[1].form.as_mut().unwrap().id = "second".into();
        assert_eq!(restoration_mapping(&identified, &identified).len(), 2);
    }

    #[test]
    fn mapping_requires_unique_full_descriptor_not_recycled_node_or_name() {
        let saved = control(1);
        let mut live = control(8);
        live.document.generation = 2;
        live.node.generation = 2;
        assert_eq!(
            restoration_mapping(std::slice::from_ref(&saved), &[live.clone()]),
            vec![(
                0,
                FormRestoreTarget::Mapped {
                    node: live.node,
                    document: live.document
                }
            )]
        );
        assert!(
            restoration_mapping(std::slice::from_ref(&saved), &[live.clone(), live.clone()])
                .is_empty()
        );
        assert!(restoration_mapping(&[saved.clone(), saved.clone()], &[live.clone()]).is_empty());
        live.definition = "x-other".into();
        assert!(restoration_mapping(std::slice::from_ref(&saved), &[live.clone()]).is_empty());
        live.definition = saved.definition.clone();
        live.occurrence = 1;
        assert!(restoration_mapping(&[saved], &[live]).is_empty());
    }
}

/// Native frame-history matching uses the complete embedding path, never just
/// a control or iframe name. URLs are checked separately before private delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameHistoryKey {
    pub tag: String,
    pub id: String,
    pub name: String,
    pub src: String,
    pub srcdoc: String,
    pub occurrence: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFrameFormState {
    pub path: Vec<FrameHistoryKey>,
    pub url: String,
    pub controls: Vec<SavedFormControl>,
    #[serde(default)]
    pub navigation: Option<SavedFrameNavigation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFrameNavigation {
    #[serde(default)]
    pub saved_names: Vec<(String, String)>,
    #[serde(default)]
    pub document_controls: Vec<SavedFrameControls>,
    #[serde(default)]
    pub joint_route: Option<String>,
    pub history: crate::navigation_history::NavigationHistoryUpdate,
    pub origins: Vec<(String, String)>,
    pub sources: Vec<SavedFrameSource>,
    pub requests: Vec<SavedFrameRequest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFrameControls {
    pub document_id: String,
    pub controls: Vec<SavedFormControl>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFrameSource {
    pub document_id: String,
    pub source: String,
    pub base_url: Option<String>,
    pub controls: Vec<SavedFormControl>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFrameRequest {
    #[serde(default)]
    pub request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
    pub document_id: String,
    pub post: Option<crate::navigation::PostResource>,
    pub authority: Option<crate::navigation::FormNavigationAuthority>,
}
impl SavedFrameNavigation {
    pub fn retained_bytes(&self) -> Result<usize, &'static str> {
        if self.history.entries.is_empty()
            || self.history.entries.len() > 4096
            || self.history.current >= self.history.entries.len()
            || self.sources.len() > 1024
            || self.requests.len() > 4096
            || self.origins.len() > 4096
            || self.saved_names.len() > 4096
            || self.document_controls.len() > 4096
        {
            return Err("frame history descriptor capacity");
        }
        let documents = self
            .history
            .entries
            .iter()
            .map(|entry| entry.document_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut ids = std::collections::BTreeSet::new();
        let mut keys = std::collections::BTreeSet::new();
        if self.history.entries.iter().any(|entry| {
            !entry.scroll.valid() || !ids.insert(&entry.id) || !keys.insert(&entry.key)
        }) {
            return Err("duplicate frame history entry identity");
        }
        for descriptors in [
            self.saved_names
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            self.origins
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            self.sources
                .iter()
                .map(|source| source.document_id.as_str())
                .collect(),
            self.requests
                .iter()
                .map(|request| request.document_id.as_str())
                .collect(),
            self.document_controls
                .iter()
                .map(|entry| entry.document_id.as_str())
                .collect(),
        ] {
            let mut seen = std::collections::BTreeSet::new();
            if descriptors
                .into_iter()
                .any(|id| !documents.contains(id) || !seen.insert(id))
            {
                return Err("invalid frame history document custody");
            }
        }
        if self.document_controls.iter().any(|entry| {
            self.sources
                .iter()
                .any(|source| source.document_id == entry.document_id)
        }) {
            return Err("ambiguous frame control custody");
        }
        let mut controls = 0usize;
        for source in self
            .sources
            .iter()
            .map(|source| &source.controls)
            .chain(self.document_controls.iter().map(|entry| &entry.controls))
        {
            controls = controls
                .checked_add(source.len())
                .ok_or("frame history control overflow")?;
            if controls > MAX_FORM_SNAPSHOT_CONTROLS {
                return Err("frame history control limit");
            }
        }
        let mut budget = FormSnapshotBudget::default();
        for (id, name) in &self.saved_names {
            if name.len() > 65_536 {
                return Err("frame history name length");
            }
            budget.reserve_bytes(id.len())?;
            budget.reserve_bytes(name.len())?;
        }
        if let Some(route) = &self.joint_route {
            if route.is_empty() {
                return Err("empty joint frame route");
            }
            budget.reserve_bytes(route.len())?;
        }
        for entry in &self.history.entries {
            for value in [&entry.key, &entry.id, &entry.url, &entry.document_id] {
                budget.reserve_bytes(value.len())?;
            }
            for value in [&entry.state, &entry.history_state].into_iter().flatten() {
                budget.reserve_bytes(value.retained_bytes())?;
            }
        }
        for (id, origin) in &self.origins {
            budget.reserve_bytes(id.len() + origin.len())?;
        }
        for source in &self.sources {
            budget.reserve_bytes(
                source.document_id.len()
                    + source.source.len()
                    + source.base_url.as_ref().map_or(0, String::len),
            )?;
            budget.reserve_bytes(snapshot_bytes(&source.controls)?)?;
        }
        for entry in &self.document_controls {
            budget.reserve_bytes(entry.document_id.len())?;
            budget.reserve_bytes(snapshot_bytes(&entry.controls)?)?;
        }
        for request in &self.requests {
            budget.reserve_bytes(request.document_id.len())?;
            if let Some(state) = &request.request_state {
                budget.reserve_bytes(state.retained_bytes()?)?;
            }
            if let Some(post) = &request.post {
                budget.reserve_bytes(post.bytes.len() + post.content_type.len())?;
            }
            if let Some(authority) = &request.authority {
                budget.reserve_bytes(authority.0.len())?;
            }
        }
        Ok(budget.bytes())
    }
}
pub fn frame_snapshot_bytes(saved: &[SavedFrameFormState]) -> Result<usize, &'static str> {
    if saved.len() > 256 {
        return Err("frame form history count limit");
    }
    let mut bytes = 0usize;
    let mut controls = 0usize;
    for frame in saved {
        if frame.path.is_empty() || frame.path.len() > 32 {
            return Err("frame form history depth limit");
        }
        controls = controls
            .checked_add(frame.controls.len())
            .ok_or("frame form control overflow")?;
        if controls > MAX_FORM_SNAPSHOT_CONTROLS {
            return Err("frame form control limit");
        }
        let mut count = snapshot_bytes(&frame.controls)?
            .checked_add(frame.url.len())
            .ok_or("frame form history bytes")?;
        if let Some(navigation) = &frame.navigation {
            count = count
                .checked_add(navigation.retained_bytes()?)
                .ok_or("frame history bytes")?;
            for source in navigation
                .sources
                .iter()
                .map(|source| &source.controls)
                .chain(
                    navigation
                        .document_controls
                        .iter()
                        .map(|entry| &entry.controls),
                )
            {
                controls = controls
                    .checked_add(source.len())
                    .ok_or("frame form control overflow")?;
            }
            if controls > MAX_FORM_SNAPSHOT_CONTROLS {
                return Err("frame form control limit");
            }
        }
        for part in &frame.path {
            for text in [&part.tag, &part.id, &part.name, &part.src, &part.srcdoc] {
                count = count
                    .checked_add(text.len())
                    .ok_or("frame form history bytes")?;
            }
        }
        bytes = bytes
            .checked_add(count)
            .filter(|bytes| *bytes <= MAX_FORM_SNAPSHOT_BYTES)
            .ok_or("frame form history byte limit")?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod frame_history_tests {
    use super::*;
    #[test]
    fn frame_history_requires_bounded_nonempty_embedding_path() {
        let mut frame = SavedFrameFormState {
            navigation: None,
            path: Vec::new(),
            url: "https://child.test/".into(),
            controls: Vec::new(),
        };
        assert!(frame_snapshot_bytes(&[frame.clone()]).is_err());
        frame.path.push(FrameHistoryKey {
            tag: "iframe".into(),
            id: "frame".into(),
            name: String::new(),
            src: "/child".into(),
            srcdoc: String::new(),
            occurrence: 0,
        });
        assert!(frame_snapshot_bytes(&[frame.clone()]).is_ok());
        frame.path = vec![frame.path[0].clone(); 33];
        assert!(frame_snapshot_bytes(&[frame]).is_err());
    }
}

#[cfg(test)]
mod navigation_custody_tests {
    use super::*;
    fn navigation() -> SavedFrameNavigation {
        SavedFrameNavigation {
            saved_names: Vec::new(),
            document_controls: Vec::new(),
            joint_route: None,
            history: crate::navigation_history::NavigationHistoryUpdate {
                entries: vec![crate::navigation_history::NavigationHistorySnapshot {
                    scroll: Default::default(),
                    key: "key".into(),
                    id: "entry".into(),
                    document_id: "doc".into(),
                    url: "https://example.test/form".into(),
                    state: None,
                    history_state: None,
                }],
                current: 0,
                navigation_type: "traverse".into(),
                truncates_forward: false,
            },
            origins: vec![("doc".into(), "https://example.test".into())],
            sources: vec![],
            requests: vec![SavedFrameRequest {
                request_state: None,
                document_id: "doc".into(),
                post: Some(crate::navigation::PostResource {
                    bytes: vec![0, 255, 1],
                    content_type: "application/octet-stream".into(),
                }),
                authority: Some(crate::navigation::FormNavigationAuthority(vec![7, 8])),
            }],
        }
    }
    #[test]
    fn frame_history_serialization_preserves_binary_body_and_sealed_source() {
        let saved = navigation();
        saved.retained_bytes().unwrap();
        let bytes = serde_json::to_vec(&saved).unwrap();
        let restored: SavedFrameNavigation = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            restored.requests[0].post.as_ref().unwrap().bytes,
            vec![0, 255, 1]
        );
        assert_eq!(
            restored.requests[0].authority.as_ref().unwrap().0,
            vec![7, 8]
        );
        restored.retained_bytes().unwrap();
    }
    #[test]
    fn frame_history_request_provenance_round_trip_and_budget() {
        let mut saved = navigation();
        let baseline = saved.retained_bytes().unwrap();
        let state = oryn_common::navigation_request::NavigationRequestState {
            initiator_url: Some("https://source.test/path".into()),
            opaque_initiator: false,
            referrer: Some("https://source.test/".into()),
            referrer_policy: "strict-origin-when-cross-origin".into(),
        };
        saved.requests[0].request_state = Some(state.clone());
        assert_eq!(
            saved.retained_bytes().unwrap(),
            baseline + state.retained_bytes().unwrap()
        );
        let restored: SavedFrameNavigation =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(restored.requests[0].request_state, Some(state));
        saved.requests[0]
            .request_state
            .as_mut()
            .unwrap()
            .referrer_policy = "invalid".into();
        assert!(saved.retained_bytes().is_err());
        let mut legacy = serde_json::to_value(navigation()).unwrap();
        legacy["requests"][0]
            .as_object_mut()
            .unwrap()
            .remove("request_state");
        let legacy: SavedFrameNavigation = serde_json::from_value(legacy).unwrap();
        assert!(legacy.requests[0].request_state.is_none());
        legacy.retained_bytes().unwrap();
    }
    #[test]
    fn network_control_descriptors_validate_document_identity_and_legacy_default() {
        let mut saved = navigation();
        saved.document_controls.push(SavedFrameControls {
            document_id: "doc".into(),
            controls: vec![],
        });
        saved.retained_bytes().unwrap();
        let mut json = serde_json::to_value(&saved).unwrap();
        json.as_object_mut().unwrap().remove("document_controls");
        let legacy: SavedFrameNavigation = serde_json::from_value(json).unwrap();
        assert!(legacy.document_controls.is_empty());
        saved.document_controls[0].document_id = "foreign".into();
        assert!(saved.retained_bytes().is_err());
        saved.document_controls[0].document_id = "doc".into();
        saved.sources.push(SavedFrameSource {
            document_id: "doc".into(),
            source: "".into(),
            base_url: None,
            controls: vec![],
        });
        assert!(saved.retained_bytes().is_err());
    }
    #[test]
    fn frame_history_rejects_orphaned_and_duplicate_request_custody() {
        let mut saved = navigation();
        saved.requests[0].document_id = "other".into();
        assert!(saved.retained_bytes().is_err());
        let mut saved = navigation();
        saved.requests.push(saved.requests[0].clone());
        assert!(saved.retained_bytes().is_err());
        let mut saved = navigation();
        saved.history.entries.push(saved.history.entries[0].clone());
        assert!(saved.retained_bytes().is_err());
    }
    #[test]
    fn saved_frame_names_validate_document_identity_and_legacy_unknown() {
        let mut saved = navigation();
        saved.saved_names.push(("doc".into(), "name".into()));
        saved.retained_bytes().unwrap();
        let mut value = serde_json::to_value(&saved).unwrap();
        value.as_object_mut().unwrap().remove("saved_names");
        let legacy: SavedFrameNavigation = serde_json::from_value(value).unwrap();
        assert!(legacy.saved_names.is_empty());
        saved.saved_names.push(("doc".into(), "duplicate".into()));
        assert!(saved.retained_bytes().is_err());
        saved.saved_names = vec![("foreign".into(), "name".into())];
        assert!(saved.retained_bytes().is_err());
    }
}
