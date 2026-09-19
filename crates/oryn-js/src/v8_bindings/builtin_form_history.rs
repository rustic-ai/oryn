//! Native persisted state for built-in controls, sharing FACE identity mapping.
use super::*;
use crate::form_state::{
    FormEntryValue, FormFile, FormOwnerIdentity, FormSnapshotBudget, FormState, SavedFormControl,
};
fn limit() -> V8BindingError {
    V8BindingError::Allocation("built-in form history budget")
}
fn reserve_string_entry(
    budget: &mut FormSnapshotBudget,
    key: &str,
    value: &str,
) -> Result<(), V8BindingError> {
    budget
        .reserve_bytes(std::mem::size_of::<(String, FormEntryValue)>())
        .map_err(|_| limit())?;
    budget.reserve_bytes(key.len()).map_err(|_| limit())?;
    budget.reserve_bytes(value.len()).map_err(|_| limit())
}
pub(super) fn definition(
    dom: &oryn_dom::DomRuntime,
    node: NodeId,
) -> Result<Option<String>, V8BindingError> {
    let NodeKind::Element { name } = &dom.arena().get(node)?.kind else {
        return Ok(None);
    };
    if name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
        || !matches!(name.local.as_ref(), "input" | "textarea" | "select")
    {
        return Ok(None);
    }
    let kind = if name.local.as_ref() == "input" {
        dom.effective_input_type(node)?
    } else {
        String::new()
    };
    if matches!(
        kind.as_str(),
        "button" | "submit" | "reset" | "image" | "hidden"
    ) {
        return Ok(None);
    }
    if !dom.control_autofill_enabled(node)? {
        return Ok(None);
    }
    Ok(Some(format!("#builtin/{}/{kind}", name.local)))
}
pub(super) fn capture(
    state: &Rc<V8BindingState>,
    include: bool,
    budget: &mut FormSnapshotBudget,
) -> Result<Vec<SavedFormControl>, V8BindingError> {
    let dom = state.dom.borrow();
    let mut saved = Vec::new();
    let mut occurrences = BTreeMap::new();
    for node in dom.arena().descendants_inclusive(state.document_root())? {
        if dom.arena().node_document(node)? != Some(state.document_root()) {
            continue;
        }
        let Some(definition) = definition(&dom, node)? else {
            continue;
        };
        budget.reserve_control().map_err(|_| limit())?;
        budget
            .reserve_bytes(definition.len())
            .map_err(|_| limit())?;
        let name = dom.attribute_in_null_namespace(node, "name")?.unwrap_or("");
        budget.reserve_bytes(name.len()).map_err(|_| limit())?;
        let form = dom
            .form_owner(node)?
            .map(|form| {
                for attribute in ["id", "name", "action"] {
                    budget
                        .reserve_bytes(
                            dom.attribute_in_null_namespace(form, attribute)?
                                .unwrap_or("")
                                .len(),
                        )
                        .map_err(|_| limit())?;
                }
                Ok::<_, V8BindingError>(FormOwnerIdentity {
                    node: form,
                    id: dom
                        .attribute_in_null_namespace(form, "id")?
                        .unwrap_or("")
                        .into(),
                    name: dom
                        .attribute_in_null_namespace(form, "name")?
                        .unwrap_or("")
                        .into(),
                    action: dom
                        .attribute_in_null_namespace(form, "action")?
                        .unwrap_or("")
                        .into(),
                })
            })
            .transpose()?;
        let key = (
            definition.clone(),
            name.to_owned(),
            form.as_ref().map(|form| form.node),
        );
        let ordinal = occurrences.entry(key).or_insert(0usize);
        let occurrence = *ordinal;
        *ordinal += 1;
        let mut values = Vec::new();
        if include {
            if dom.is_file_input(node) {
                for file in dom.arena().get(node)?.selected_files.iter() {
                    budget
                        .reserve_bytes(
                            file.name.len()
                                + file.media_type.len()
                                + file.bytes.len()
                                + 4
                                + std::mem::size_of::<(String, FormEntryValue)>(),
                        )
                        .map_err(|_| limit())?;
                    values.push((
                        "file".into(),
                        FormEntryValue::File(FormFile {
                            name: file.name.clone(),
                            media_type: file.media_type.clone(),
                            last_modified: file.last_modified,
                            bytes: file.bytes.to_vec(),
                        }),
                    ));
                }
            } else if dom.is_element(node, "http://www.w3.org/1999/xhtml", "select") {
                for (index, option) in dom.select_options(node)?.into_iter().enumerate() {
                    for (key, enabled) in [
                        ("selected", dom.option_selected(option)?),
                        ("dirty-option", dom.arena().get(option)?.dirty_selectedness),
                    ] {
                        if enabled {
                            let value = index.to_string();
                            reserve_string_entry(budget, key, &value)?;
                            values.push((key.into(), FormEntryValue::String(value)));
                        }
                    }
                }
            } else if definition.ends_with("/checkbox") || definition.ends_with("/radio") {
                let value = dom.checked(node)?.to_string();
                reserve_string_entry(budget, "checked", &value)?;
                values.push(("checked".into(), FormEntryValue::String(value)));
            } else {
                let value = dom.control_value(node)?;
                budget
                    .reserve_bytes(
                        value.len() + 5 + std::mem::size_of::<(String, FormEntryValue)>(),
                    )
                    .map_err(|_| limit())?;
                values.push(("value".into(), FormEntryValue::String(value)));
            }
        }
        if include {
            let record = dom.arena().get(node)?;
            for (key, value) in [
                ("user-validity", record.user_validity),
                ("user-edited", record.user_edited_value),
                ("dirty-value", record.dirty_value),
                ("dirty-checked", record.dirty_checkedness),
            ] {
                let value = if value { "true" } else { "false" };
                reserve_string_entry(budget, key, value)?;
                values.push((key.into(), FormEntryValue::String(value.into())));
            }
        }
        if include && supports_selection(&definition) {
            let length = oryn_dom::dom_string::len(&dom.control_value(node)?) as u32;
            let (start, end, direction) = state
                .text_control_selections
                .borrow()
                .get(&node)
                .cloned()
                .unwrap_or((length, length, "none".into()));
            for (key, value) in [
                ("selection-start", start.to_string()),
                ("selection-end", end.to_string()),
                ("selection-direction", direction),
            ] {
                reserve_string_entry(budget, key, &value)?;
                values.push((key.into(), FormEntryValue::String(value)));
            }
        }
        saved.push(SavedFormControl {
            node,
            document: state.document_root(),
            definition,
            name: name.into(),
            form,
            occurrence,
            state: if include {
                FormState::Entries(values)
            } else {
                FormState::Null
            },
        });
    }
    Ok(saved)
}
pub(super) fn restore(
    state: &Rc<V8BindingState>,
    saved: &SavedFormControl,
    target: crate::form_state::FormRestoreTarget,
) -> Result<(), V8BindingError> {
    let (node, document) = match target {
        crate::form_state::FormRestoreTarget::Exact => (saved.node, saved.document),
        crate::form_state::FormRestoreTarget::Mapped { node, document } => (node, document),
    };
    if state.execution_retired() || document != state.document_root() {
        return Ok(());
    }
    let mut dom = state.dom.borrow_mut();
    if dom.arena().node_document(node)? != Some(document)
        || !dom.is_document_connected(node)?
        || definition(&dom, node)?.as_deref() != Some(saved.definition.as_str())
    {
        return Ok(());
    }
    let FormState::Entries(values) = &saved.state else {
        return Ok(());
    };
    if dom.is_file_input(node) {
        let files = values
            .iter()
            .filter_map(|(key, value)| {
                if key == "file" {
                    if let FormEntryValue::File(file) = value {
                        Some(oryn_dom::SelectedFile {
                            name: file.name.clone(),
                            media_type: file.media_type.clone(),
                            bytes: file.bytes.clone().into(),
                            last_modified: file.last_modified,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        dom.set_selected_files(node, std::sync::Arc::new(files))?;
    } else if dom.is_element(node, "http://www.w3.org/1999/xhtml", "select") {
        let indices = |name: &str| {
            values
                .iter()
                .filter_map(|(key, value)| {
                    if key == name {
                        if let FormEntryValue::String(index) = value {
                            index.parse::<usize>().ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect::<BTreeSet<_>>()
        };
        let selected = indices("selected");
        let dirty = indices("dirty-option");
        dom.set_select_selected_index(node, -1)?;
        let options = dom.select_options(node)?;
        for (index, option) in options.iter().enumerate() {
            if selected.contains(&index) {
                dom.set_option_selected(*option, true)?;
            }
        }
        // Selection setters dirty their targets. Restore the original option
        // flags after all setters so subsequent selected-attribute changes
        // retain their saved behavior, including unselected dirty options.
        for (index, option) in options.into_iter().enumerate() {
            dom.arena_mut().get_mut(option)?.dirty_selectedness = dirty.contains(&index);
        }
        dom.update_selected_content(node)?;
    } else {
        for (key, value) in values {
            let FormEntryValue::String(value) = value else {
                continue;
            };
            match key.as_str() {
                "checked" => dom.set_checked(node, value == "true")?,
                "value" => {
                    dom.set_value(node, value.clone(), saved.definition.ends_with("/password"))?
                }
                _ => {}
            }
        }
    }
    for (key, value) in values {
        let FormEntryValue::String(value) = value else {
            continue;
        };
        let flag = value == "true";
        match key.as_str() {
            "user-validity" => dom.set_user_validity(node, flag)?,
            "user-edited" => dom.arena_mut().get_mut(node)?.user_edited_value = flag,
            "dirty-value" => dom.arena_mut().get_mut(node)?.dirty_value = flag,
            "dirty-checked" => dom.arena_mut().get_mut(node)?.dirty_checkedness = flag,
            _ => {}
        }
    }
    if supports_selection(&saved.definition) {
        let get = |name: &str| {
            values.iter().find_map(|(key, value)| {
                if key == name {
                    if let FormEntryValue::String(value) = value {
                        Some(value.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        };
        if let (Some(start), Some(end)) = (
            get("selection-start").and_then(|value| value.parse::<u32>().ok()),
            get("selection-end").and_then(|value| value.parse::<u32>().ok()),
        ) {
            let length = oryn_dom::dom_string::len(&dom.control_value(node)?) as u32;
            let end = end.min(length);
            let start = start.min(end);
            let direction = get("selection-direction")
                .filter(|value| matches!(*value, "forward" | "backward" | "none"))
                .unwrap_or("none");
            state
                .text_control_selections
                .borrow_mut()
                .insert(node, (start, end, direction.into()));
        }
    }
    Ok(())
}

fn supports_selection(definition: &str) -> bool {
    definition == "#builtin/textarea/"
        || matches!(
            definition.strip_prefix("#builtin/input/"),
            Some("text" | "search" | "url" | "tel" | "password")
        )
}
