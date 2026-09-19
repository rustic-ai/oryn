//! Ordered, native document top layer. Author attributes cannot create entries.
use super::{DomError, DomRuntime, Mutation, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopLayerKind {
    ModalDialog,
    Popover,
    SelectPicker,
    TemporalPicker,
    PermissionPrompt,
    Fullscreen,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopLayerEntry {
    pub picker_epoch: u64,
    pub picker_scroll: (f64, f64),
    pub picker_reveal: bool,
    pub active_option: Option<NodeId>,
    pub node: NodeId,
    pub kind: TopLayerKind,
    pub previous_focus: Option<NodeId>,
    pub source: Option<NodeId>,
}

impl DomRuntime {
    pub fn reserve_top_layer(&mut self, additional: usize) -> Result<(), DomError> {
        self.top_layer
            .try_reserve(additional)
            .map_err(|_| DomError::TopLayerCapacity)
    }
    pub fn select_picker_active(&self, select: NodeId) -> Result<Option<NodeId>, DomError> {
        let Some(entry) = self
            .top_layer()
            .find(|entry| entry.node == select && entry.kind == TopLayerKind::SelectPicker)
        else {
            return Ok(None);
        };
        let options = self.select_options(select)?;
        Ok(entry.active_option.filter(|option| {
            options.contains(option) && !self.option_disabled(*option).unwrap_or(true)
        }))
    }
    pub fn set_select_picker_active(
        &mut self,
        select: NodeId,
        option: Option<NodeId>,
    ) -> Result<bool, DomError> {
        if let Some(option) = option
            && (!self.select_options(select)?.contains(&option) || self.option_disabled(option)?)
        {
            return Ok(false);
        }
        let Some(entry) = self
            .top_layer
            .iter_mut()
            .find(|entry| entry.node == select && entry.kind == TopLayerKind::SelectPicker)
        else {
            return Ok(false);
        };
        if entry.active_option == option {
            return Ok(false);
        }
        let old = entry.active_option;
        entry.active_option = option;
        entry.picker_reveal = true;
        for node in [Some(select), old, option].into_iter().flatten() {
            self.record_mutation(Mutation::FormState { node });
        }
        Ok(true)
    }
    /// Private picker scrolling never changes the select button's DOM scrollTop.
    pub fn set_select_picker_scroll(
        &mut self,
        select: NodeId,
        offset: (f64, f64),
    ) -> Result<bool, DomError> {
        self.arena.get(select)?;
        if !offset.0.is_finite() || !offset.1.is_finite() {
            return Ok(false);
        }
        let Some(entry) = self
            .top_layer
            .iter_mut()
            .find(|entry| entry.node == select && entry.kind == TopLayerKind::SelectPicker)
        else {
            return Ok(false);
        };
        if entry.picker_scroll == offset && !entry.picker_reveal {
            return Ok(false);
        }
        entry.picker_scroll = offset;
        entry.picker_reveal = false;
        self.record_mutation(Mutation::FormState { node: select });
        Ok(true)
    }
    pub fn set_top_layer_source(&mut self, node: NodeId, source: Option<NodeId>) {
        let old = if let Some(entry) = self.top_layer.iter_mut().find(|entry| entry.node == node) {
            if entry.source == source {
                return;
            }
            let old = entry.source;
            entry.source = source;
            old
        } else {
            return;
        };
        for target in [Some(node), old, source].into_iter().flatten() {
            self.record_mutation(Mutation::FormState { node: target });
        }
    }
    pub fn popover_pointer_down(&mut self, target: Option<NodeId>) {
        self.popover_pointer_down = Some(target);
    }
    pub fn take_popover_pointer_down(&mut self) -> Option<Option<NodeId>> {
        self.popover_pointer_down.take()
    }
    pub fn begin_popover_transition(&mut self, node: NodeId) -> bool {
        self.popover_transitions.insert(node)
    }
    pub fn end_popover_transition(&mut self, node: NodeId) {
        self.popover_transitions.remove(&node);
    }
    pub fn top_layer(&self) -> impl Iterator<Item = TopLayerEntry> + '_ {
        self.top_layer
            .iter()
            .copied()
            .filter(|entry| self.connected.contains(&entry.node))
    }
    pub fn is_modal_dialog(&self, node: NodeId) -> bool {
        self.top_layer()
            .any(|entry| entry.node == node && entry.kind == TopLayerKind::ModalDialog)
    }
    /// Fullscreen is native top-layer state, never an authored attribute.
    pub fn is_fullscreen(&self, node: NodeId) -> bool {
        self.top_layer()
            .any(|entry| entry.node == node && entry.kind == TopLayerKind::Fullscreen)
    }
    pub fn fullscreen_element(&self, document: NodeId) -> Option<NodeId> {
        self.top_layer()
            .filter(|entry| {
                entry.kind == TopLayerKind::Fullscreen
                    && self.node_document(entry.node).ok().flatten() == Some(document)
            })
            .last()
            .map(|entry| entry.node)
    }
    pub fn is_popover_open(&self, node: NodeId) -> bool {
        self.top_layer()
            .any(|entry| entry.node == node && entry.kind == TopLayerKind::Popover)
    }
    pub fn add_to_top_layer(&mut self, node: NodeId, kind: TopLayerKind) -> Result<bool, DomError> {
        self.arena.get(node)?;
        if !self.is_connected(node)? {
            return Ok(false);
        }
        if self
            .top_layer
            .iter()
            .any(|entry| entry.node == node && entry.kind == kind)
        {
            return Ok(false);
        }
        self.top_layer.retain(|entry| entry.node != node);
        let picker_epoch = self.mutation_cursor().0;
        self.top_layer.push(TopLayerEntry {
            picker_epoch,
            picker_scroll: (0., 0.),
            picker_reveal: true,
            active_option: None,
            node,
            kind,
            previous_focus: self.focused,
            source: None,
        });
        // Native control state invalidates layout/style/semantic revisions but
        // creates no author-visible attribute MutationObserver record.
        self.record_mutation(Mutation::FormState { node });
        Ok(true)
    }
    pub fn remove_from_top_layer(&mut self, node: NodeId) -> bool {
        let before = self.top_layer.len();
        self.top_layer.retain(|entry| entry.node != node);
        let changed = before != self.top_layer.len();
        if changed && let Ok(record) = self.arena.get_mut(node) {
            record.temporal_picker = None;
            record.permission_prompt = None;
        }
        if changed {
            self.record_mutation(Mutation::FormState { node });
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_native_state_cannot_be_created_by_attributes_and_retires_on_removal() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        dom.append_child(dom.document(), root).unwrap();
        let first = dom.create_element("dialog");
        let second = dom.create_element("dialog");
        dom.append_child(root, first).unwrap();
        dom.append_child(root, second).unwrap();
        dom.set_attribute(first, "data-oryn-modal", "").unwrap();
        assert!(!dom.is_modal_dialog(first));
        dom.add_to_top_layer(second, TopLayerKind::ModalDialog)
            .unwrap();
        dom.add_to_top_layer(first, TopLayerKind::ModalDialog)
            .unwrap();
        assert_eq!(
            dom.top_layer().map(|entry| entry.node).collect::<Vec<_>>(),
            vec![second, first]
        );
        assert!(
            !dom.add_to_top_layer(second, TopLayerKind::ModalDialog)
                .unwrap()
        );
        dom.remove(root).unwrap();
        assert_eq!(dom.top_layer().count(), 0);
        dom.append_child(dom.document(), root).unwrap();
        assert!(!dom.is_modal_dialog(first));
    }

    #[test]
    fn modal_inertness_uses_topmost_dialog_and_escapes_ancestor_inert() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        dom.append_child(dom.document(), root).unwrap();
        let first = dom.create_element("dialog");
        let second = dom.create_element("dialog");
        let child = dom.create_element("button");
        dom.append_child(root, first).unwrap();
        dom.append_child(root, second).unwrap();
        dom.append_child(second, child).unwrap();
        dom.set_attribute(root, "inert", "").unwrap();
        dom.add_to_top_layer(first, TopLayerKind::ModalDialog)
            .unwrap();
        dom.add_to_top_layer(second, TopLayerKind::ModalDialog)
            .unwrap();
        assert!(dom.is_inert(first).unwrap());
        assert!(!dom.is_inert(second).unwrap());
        assert!(!dom.is_inert(child).unwrap());
        dom.set_attribute(second, "inert", "").unwrap();
        assert!(dom.is_inert(child).unwrap());
        dom.remove_from_top_layer(second);
        assert!(!dom.is_inert(first).unwrap());
    }
}

#[cfg(test)]
mod picker_scroll_tests {
    use super::*;
    #[test]
    fn private_scroll_retires_with_popup_and_keyboard_rearms_reveal() {
        let mut dom = crate::html::parse(
            "<select id=s><option id=a>A</option><option id=b>B</option></select>",
        )
        .into_runtime()
        .unwrap();
        let select = dom.find_by_id("s").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        dom.add_to_top_layer(select, TopLayerKind::SelectPicker)
            .unwrap();
        dom.set_select_picker_active(select, Some(a)).unwrap();
        assert!(dom.set_select_picker_scroll(select, (0., 42.5)).unwrap());
        let entry = dom.top_layer().find(|entry| entry.node == select).unwrap();
        assert_eq!(entry.picker_scroll, (0., 42.5));
        assert!(!entry.picker_reveal);
        assert!(
            !dom.set_select_picker_scroll(select, (0., f64::NAN))
                .unwrap()
        );
        dom.set_select_picker_active(select, Some(b)).unwrap();
        assert!(
            dom.top_layer()
                .find(|entry| entry.node == select)
                .unwrap()
                .picker_reveal
        );
        let old_epoch = dom
            .top_layer()
            .find(|entry| entry.node == select)
            .unwrap()
            .picker_epoch;
        dom.remove_from_top_layer(select);
        dom.add_to_top_layer(select, TopLayerKind::SelectPicker)
            .unwrap();
        let entry = dom.top_layer().find(|entry| entry.node == select).unwrap();
        assert_eq!(entry.picker_scroll, (0., 0.));
        assert!(entry.picker_reveal);
        assert_ne!(entry.picker_epoch, old_epoch);
        assert_eq!(dom.parent(b).unwrap(), Some(select));
    }
}
