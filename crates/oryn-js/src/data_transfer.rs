//! Drag/clipboard data-store algorithms. No filesystem paths or ambient grants.
use oryn_dom::SelectedFile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    ReadWrite,
    ReadOnly,
    Protected,
    Detached,
}
#[derive(Clone, Debug)]
pub enum Data {
    String(Vec<u16>),
    File(SelectedFile),
}
#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub kind: Data,
    pub media_type: String,
}
#[derive(Clone, Debug)]
pub struct Store {
    pub mode: Mode,
    pub drop_effect: String,
    pub effect_allowed: String,
    items: Vec<Item>,
    next_id: u64,
    pub revision: u64,
    pub drag_image: Option<(oryn_dom::NodeId, i32, i32)>,
    pub drag_image_snapshot: Option<std::sync::Arc<crate::drag_image::Snapshot>>,
}
impl Default for Store {
    fn default() -> Self {
        Self::new(Mode::ReadWrite)
    }
}
impl Store {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            drop_effect: "none".into(),
            effect_allowed: "uninitialized".into(),
            items: Vec::new(),
            next_id: 1,
            revision: 0,
            drag_image: None,
            drag_image_snapshot: None,
        }
    }
    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            if mode == Mode::Detached {
                if !self.items.is_empty() {
                    self.revision = self.revision.wrapping_add(1);
                }
                self.items.clear();
                self.drag_image = None;
                self.drag_image_snapshot = None;
            }
        }
    }
    pub fn items(&self) -> &[Item] {
        if self.mode == Mode::Detached {
            &[]
        } else {
            &self.items
        }
    }
    pub fn item(&self, id: u64) -> Option<&Item> {
        self.items().iter().find(|item| item.id == id)
    }
    pub fn readable(&self) -> bool {
        matches!(self.mode, Mode::ReadWrite | Mode::ReadOnly)
    }
    pub fn types(&self) -> Vec<String> {
        let mut types = self
            .items()
            .iter()
            .filter(|item| matches!(item.kind, Data::String(_)))
            .map(|item| item.media_type.clone())
            .collect::<Vec<_>>();
        if self
            .items()
            .iter()
            .any(|item| matches!(item.kind, Data::File(_)))
        {
            types.push("Files".into());
        }
        types
    }
    pub fn files(&self) -> Vec<SelectedFile> {
        if !self.readable() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter_map(|item| {
                if let Data::File(file) = &item.kind {
                    Some(file.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    fn insert(&mut self, kind: Data, media_type: String) -> Option<u64> {
        if self.mode != Mode::ReadWrite {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        self.items.push(Item {
            id,
            kind,
            media_type,
        });
        self.revision = self.revision.wrapping_add(1);
        Some(id)
    }
    pub fn add_string(
        &mut self,
        value: Vec<u16>,
        media_type: String,
    ) -> Result<Option<u64>, &'static str> {
        if self.mode != Mode::ReadWrite {
            return Ok(None);
        }
        let media_type = lower_ascii(&media_type);
        if self
            .items
            .iter()
            .any(|item| matches!(item.kind, Data::String(_)) && item.media_type == media_type)
        {
            return Err("NotSupportedError");
        }
        Ok(self.insert(Data::String(value), media_type))
    }
    pub fn add_file(&mut self, file: SelectedFile) -> Option<u64> {
        let media_type = lower_ascii(&file.media_type);
        self.insert(Data::File(file), media_type)
    }
    pub fn set_data(&mut self, format: &str, value: Vec<u16>) {
        if self.mode != Mode::ReadWrite {
            return;
        }
        let format = normalize_format(format);
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|item| matches!(item.kind, Data::String(_)) && item.media_type == format)
        {
            item.kind = Data::String(value);
            self.revision = self.revision.wrapping_add(1);
        } else {
            self.insert(Data::String(value), format);
        }
    }
    pub fn get_data(&self, format: &str) -> Vec<u16> {
        if !self.readable() {
            return Vec::new();
        }
        let normalized = normalize_format(format);
        let Some(Item {
            kind: Data::String(value),
            ..
        }) = self
            .items
            .iter()
            .find(|item| item.media_type == normalized && matches!(item.kind, Data::String(_)))
        else {
            return Vec::new();
        };
        if !format.eq_ignore_ascii_case("url") {
            return value.clone();
        }
        // URI-list first non-comment/nonempty line, with ASCII whitespace trim.
        for line in value.split(|unit| *unit == 10 || *unit == 13) {
            let start = line
                .iter()
                .position(|unit| !matches!(*unit, 9 | 32))
                .unwrap_or(line.len());
            let end = line
                .iter()
                .rposition(|unit| !matches!(*unit, 9 | 32))
                .map(|index| index + 1)
                .unwrap_or(start);
            let line = &line[start..end];
            if !line.is_empty() && line[0] != 35 {
                return line.to_vec();
            }
        }
        Vec::new()
    }
    pub fn clear_data(&mut self, format: Option<&str>) {
        if self.mode != Mode::ReadWrite {
            return;
        }
        let normalized = format.map(normalize_format);
        let previous_len = self.items.len();
        self.items.retain(|item| {
            !matches!(item.kind, Data::String(_))
                || normalized
                    .as_ref()
                    .is_some_and(|format| item.media_type != *format)
        });
        if previous_len != self.items.len() {
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn remove(&mut self, index: usize) -> Result<(), &'static str> {
        if self.mode != Mode::ReadWrite {
            return Err("InvalidStateError");
        }
        if index < self.items.len() {
            self.items.remove(index);
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(())
    }
    pub fn clear(&mut self) {
        if self.mode == Mode::ReadWrite && !self.items.is_empty() {
            self.items.clear();
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn set_drop_effect(&mut self, value: &str) {
        if matches!(value, "none" | "copy" | "link" | "move") {
            self.drop_effect = value.into();
        }
    }
    pub fn set_effect_allowed(&mut self, value: &str) {
        if self.mode == Mode::ReadWrite
            && matches!(
                value,
                "none"
                    | "copy"
                    | "copyLink"
                    | "copyMove"
                    | "link"
                    | "linkMove"
                    | "move"
                    | "all"
                    | "uninitialized"
            )
        {
            self.effect_allowed = value.into();
        }
    }
}
fn lower_ascii(value: &str) -> String {
    let mut units = oryn_dom::dom_string::units(value);
    for unit in &mut units {
        if (65..=90).contains(unit) {
            *unit += 32;
        }
    }
    oryn_dom::dom_string::from_units(&units)
}
pub fn normalize_format(value: &str) -> String {
    match lower_ascii(value).as_str() {
        "text" => "text/plain".into(),
        "url" => "text/uri-list".into(),
        value => value.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }
    fn file() -> SelectedFile {
        SelectedFile {
            name: "a.txt".into(),
            media_type: "text/plain".into(),
            bytes: std::sync::Arc::from(&b"actual"[..]),
            last_modified: 7,
        }
    }
    #[test]
    fn replacement_preserves_order_and_item_identity() {
        let mut store = Store::default();
        store.set_data("TEXT", text("a"));
        let id = store.items()[0].id;
        store.set_data("text/html", text("b"));
        store.set_data("text/plain", text("c"));
        assert_eq!(store.types(), vec!["text/plain", "text/html"]);
        assert_eq!(store.items()[0].id, id);
        assert_eq!(store.get_data("text"), text("c"));
    }
    #[test]
    fn protected_store_exposes_types_not_payload_or_files() {
        let mut store = Store::default();
        store.set_data("text", text("secret"));
        store.add_file(file());
        store.set_mode(Mode::Protected);
        assert_eq!(store.types(), vec!["text/plain", "Files"]);
        assert!(store.get_data("text").is_empty());
        assert!(store.files().is_empty());
        assert_eq!(store.add_string(text("x"), "new".into()), Ok(None));
        assert_eq!(store.remove(0), Err("InvalidStateError"));
        store.clear();
        assert_eq!(store.items().len(), 2);
    }
    #[test]
    fn clear_data_does_not_remove_files() {
        let mut store = Store::default();
        store.add_file(file());
        store.set_data("text", text("a"));
        store.clear_data(None);
        assert_eq!(store.types(), vec!["Files"]);
        assert_eq!(store.files()[0].bytes.as_ref(), b"actual");
    }
    #[test]
    fn removed_item_does_not_rebind_to_reused_index() {
        let mut store = Store::default();
        let id = store
            .add_string(text("a"), "text/a".into())
            .unwrap()
            .unwrap();
        store.remove(0).unwrap();
        store.add_string(text("b"), "text/b".into()).unwrap();
        assert!(store.item(id).is_none());
    }
    #[test]
    fn uri_alias_and_utf16_are_preserved() {
        let mut store = Store::default();
        store.set_data(
            "url",
            text("#comment\r\n https://example.test/ \r\nhttps://other.test/"),
        );
        assert_eq!(store.get_data("URL"), text("https://example.test/"));
        store.set_data("text", vec![0xd800, 97]);
        assert_eq!(store.get_data("text"), vec![0xd800, 97]);
    }
    #[test]
    fn duplicate_string_throws_but_file_types_can_repeat() {
        let mut store = Store::default();
        store.add_string(text("a"), "TEXT/PLAIN".into()).unwrap();
        assert_eq!(
            store.add_string(text("b"), "text/plain".into()),
            Err("NotSupportedError")
        );
        store.add_file(file());
        store.add_file(file());
        assert_eq!(store.files().len(), 2);
        store.set_mode(Mode::Detached);
        assert!(store.items().is_empty());
        assert!(store.types().is_empty());
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventInit {
    pub client_x: i32,
    pub client_y: i32,
    pub screen_x: i32,
    pub screen_y: i32,
    pub button: i16,
    pub buttons: u16,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub meta_key: bool,
    pub shift_key: bool,
    pub related_target: Option<oryn_dom::NodeId>,
}

/// Native drag operation, independent of the script-visible string properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEffect {
    None,
    Copy,
    Link,
    Move,
}
impl DragEffect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Copy => "copy",
            Self::Link => "link",
            Self::Move => "move",
        }
    }
    pub fn from_drop_effect(value: &str) -> Self {
        match value {
            "copy" => Self::Copy,
            "link" => Self::Link,
            "move" => Self::Move,
            _ => Self::None,
        }
    }
    pub fn permitted_by(self, allowed: &str) -> bool {
        match self {
            Self::None => true,
            Self::Copy => matches!(
                allowed,
                "copy" | "copyLink" | "copyMove" | "all" | "uninitialized"
            ),
            Self::Link => matches!(
                allowed,
                "link" | "copyLink" | "linkMove" | "all" | "uninitialized"
            ),
            Self::Move => matches!(
                allowed,
                "move" | "copyMove" | "linkMove" | "all" | "uninitialized"
            ),
        }
    }
}

/// Operation negotiation shared by host drag dispatch and default editing.
/// Hit testing and platform modifier preferences belong to the input host.
/// Merely setting dropEffect does not accept a drop: dragover cancellation (or
/// a native editable/file target default) is required for an accepted operation.
#[derive(Clone, Debug)]
pub struct DragNegotiation {
    allowed: String,
    target: Option<oryn_dom::NodeId>,
    operation: DragEffect,
    finished: bool,
}
impl DragNegotiation {
    pub fn new(store: &Store) -> Self {
        Self {
            allowed: store.effect_allowed.clone(),
            target: None,
            operation: DragEffect::None,
            finished: false,
        }
    }
    pub fn operation(&self) -> DragEffect {
        self.operation
    }
    pub fn target(&self) -> Option<oryn_dom::NodeId> {
        self.target
    }
    /// Initialize each dragenter/dragover event independently. A handler's old
    /// dropEffect must not leak into a subsequent target or modifier state.
    pub fn prepare(
        &mut self,
        target: Option<oryn_dom::NodeId>,
        preferred: DragEffect,
        store: &mut Store,
    ) {
        if self.finished {
            return;
        }
        self.target = target;
        self.operation = DragEffect::None;
        let effect = if target.is_none() {
            DragEffect::None
        } else if preferred != DragEffect::None && preferred.permitted_by(&self.allowed) {
            preferred
        } else {
            [DragEffect::Copy, DragEffect::Link, DragEffect::Move]
                .into_iter()
                .find(|effect| effect.permitted_by(&self.allowed))
                .unwrap_or(DragEffect::None)
        };
        store.drop_effect = effect.as_str().into();
    }
    pub fn accept_over(
        &mut self,
        canceled: bool,
        native_default: Option<DragEffect>,
        store: &Store,
    ) -> DragEffect {
        if self.finished || self.target.is_none() {
            return DragEffect::None;
        }
        let effect = if canceled {
            DragEffect::from_drop_effect(&store.drop_effect)
        } else {
            native_default.unwrap_or(DragEffect::None)
        };
        self.operation = if effect.permitted_by(&self.allowed) {
            effect
        } else {
            DragEffect::None
        };
        self.operation
    }
    /// Call after drop event/default processing. Only the accepted target and
    /// operation can complete; target removal, navigation, Escape and rejected
    /// default editing must finish with none (and must not delete source data).
    pub fn finish(&mut self, drop_succeeded: bool, store: &mut Store) -> DragEffect {
        if !self.finished {
            if !drop_succeeded {
                self.operation = DragEffect::None;
            }
            self.finished = true;
            self.target = None;
            store.drop_effect = self.operation.as_str().into();
            store.set_mode(Mode::Protected);
        }
        self.operation
    }
    pub fn finish_drop(
        &mut self,
        canceled: bool,
        default_succeeded: bool,
        store: &mut Store,
    ) -> DragEffect {
        if !self.finished && canceled {
            let requested = DragEffect::from_drop_effect(&store.drop_effect);
            self.operation = if requested.permitted_by(&self.allowed) {
                requested
            } else {
                DragEffect::None
            };
        }
        self.finish(canceled || default_succeeded, store)
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    #[test]
    fn canceled_drop_final_effect_remains_constrained_by_the_source() {
        for (requested, expected) in [
            ("copy", DragEffect::Copy),
            ("none", DragEffect::None),
            ("move", DragEffect::None),
        ] {
            let mut store = Store {
                effect_allowed: "copy".into(),
                ..Store::default()
            };
            let mut drag = DragNegotiation::new(&store);
            drag.prepare(
                Some(oryn_dom::NodeId {
                    slot: 1,
                    generation: 1,
                }),
                DragEffect::Copy,
                &mut store,
            );
            assert_eq!(drag.accept_over(true, None, &store), DragEffect::Copy);
            store.drop_effect = requested.into();
            assert_eq!(drag.finish_drop(true, false, &mut store), expected);
            assert_eq!(store.drop_effect, expected.as_str());
        }
    }
    #[test]
    fn target_change_requires_fresh_acceptance_and_respects_source_effects() {
        let first = oryn_dom::NodeId {
            slot: 1,
            generation: 1,
        };
        let replacement = oryn_dom::NodeId {
            slot: 1,
            generation: 2,
        };
        let mut store = Store {
            effect_allowed: "copyMove".into(),
            ..Store::default()
        };
        let mut drag = DragNegotiation::new(&store);
        drag.prepare(Some(first), DragEffect::Move, &mut store);
        assert_eq!(store.drop_effect, "move");
        assert_eq!(
            drag.accept_over(false, None, &store),
            DragEffect::None,
            "property alone cannot accept"
        );
        assert_eq!(drag.accept_over(true, None, &store), DragEffect::Move);
        drag.prepare(Some(replacement), DragEffect::Copy, &mut store);
        assert_eq!(drag.target(), Some(replacement));
        assert_eq!(
            drag.operation(),
            DragEffect::None,
            "new target has no inherited acceptance"
        );
        store.drop_effect = "link".into();
        assert_eq!(drag.accept_over(true, None, &store), DragEffect::None);
        assert_eq!(
            drag.accept_over(false, Some(DragEffect::Copy), &store),
            DragEffect::Copy
        );
        assert_eq!(drag.finish(true, &mut store), DragEffect::Copy);
        assert_eq!(store.drop_effect, "copy");
        assert_eq!(drag.target(), None);
        assert_eq!(
            drag.finish(false, &mut store),
            DragEffect::Copy,
            "completion is single-shot"
        );
    }

    #[test]
    fn allowed_effect_matrix_excludes_unrequested_operations() {
        for (allowed, expected) in [
            ("none", [false, false, false]),
            ("copy", [true, false, false]),
            ("link", [false, true, false]),
            ("move", [false, false, true]),
            ("copyLink", [true, true, false]),
            ("copyMove", [true, false, true]),
            ("linkMove", [false, true, true]),
            ("all", [true, true, true]),
            ("uninitialized", [true, true, true]),
            ("unknown", [false, false, false]),
        ] {
            for (effect, expected) in [DragEffect::Copy, DragEffect::Link, DragEffect::Move]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    effect.permitted_by(allowed),
                    expected,
                    "{allowed}: {effect:?}"
                );
            }
        }
    }
    #[test]
    fn pinned_drop_and_dragend_matrix_covers_all_110_variants() {
        // drop/029 and drop/030 share 11 effectAllowed × 5 dropEffect
        // variants; invalid setters retain the initialized native value.
        for endpoint in ["drop", "dragend"] {
            for allowed in [
                "uninitialized",
                "undefined",
                "none",
                "all",
                "copy",
                "move",
                "link",
                "copyMove",
                "copyLink",
                "linkMove",
                "dummy",
            ] {
                for requested in ["none", "copy", "move", "link", "dummy"] {
                    let mut store = Store::default();
                    store.set_effect_allowed(allowed);
                    let normalized = if matches!(allowed, "undefined" | "dummy") {
                        "uninitialized"
                    } else {
                        allowed
                    };
                    assert_eq!(store.effect_allowed, normalized);
                    let allowed_effects: &[&str] = match normalized {
                        "none" => &[],
                        "copy" => &["copy"],
                        "move" => &["move"],
                        "link" => &["link"],
                        "copyMove" => &["copy", "move"],
                        "copyLink" => &["copy", "link"],
                        "linkMove" => &["link", "move"],
                        _ => &["copy", "move", "link"],
                    };
                    let mut drag = DragNegotiation::new(&store);
                    drag.prepare(
                        Some(oryn_dom::NodeId {
                            slot: 1,
                            generation: 1,
                        }),
                        DragEffect::Copy,
                        &mut store,
                    );
                    store.set_drop_effect(requested);
                    let operation = drag.accept_over(true, None, &store);
                    let permitted = if requested == "dummy" {
                        !allowed_effects.is_empty()
                    } else {
                        allowed_effects.contains(&requested)
                    };
                    assert_eq!(
                        operation != DragEffect::None,
                        permitted,
                        "{endpoint}/{allowed}/{requested}"
                    );
                    if permitted {
                        assert!(allowed_effects.contains(&operation.as_str()));
                        if requested != "dummy" {
                            assert_eq!(operation.as_str(), requested);
                        }
                    }
                    let final_effect = drag.finish(permitted, &mut store);
                    assert_eq!(final_effect, operation);
                    assert_eq!(store.drop_effect, operation.as_str());
                    assert_eq!(store.effect_allowed, normalized);
                }
            }
        }
    }
    #[test]
    fn missing_target_and_canceled_completion_never_report_move() {
        let mut store = Store {
            effect_allowed: "move".into(),
            ..Store::default()
        };
        let mut drag = DragNegotiation::new(&store);
        drag.prepare(None, DragEffect::Move, &mut store);
        assert_eq!(store.drop_effect, "none");
        store.drop_effect = "move".into();
        assert_eq!(drag.accept_over(true, None, &store), DragEffect::None);
        assert_eq!(drag.finish(false, &mut store), DragEffect::None);
        assert_eq!(store.drop_effect, "none");
        assert_eq!(store.mode, Mode::Protected);
        drag.prepare(None, DragEffect::Copy, &mut store);
        assert_eq!(drag.finish(true, &mut store), DragEffect::None);
    }
}
