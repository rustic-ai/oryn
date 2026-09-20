//! Private native popup AX rows. These identities exist only in the platform
//! snapshot; actions always route to the real owner plus its exact popup state.
use super::*;
#[derive(Clone, Debug)]
pub enum Target {
    Input {
        owner: SemanticRef,
        expected: std::sync::Arc<oryn_dom::TemporalPicker>,
        action: oryn_js::accessibility::PickerAction,
    },
    Permission {
        owner: SemanticRef,
        expected: std::sync::Arc<oryn_dom::PermissionPrompt>,
        choice: oryn_js::platform_control::PromptChoice,
    },
}
impl Target {
    pub fn owner(&self) -> SemanticRef {
        match self {
            Self::Input { owner, .. } | Self::Permission { owner, .. } => *owner,
        }
    }
    pub fn command(&self) -> oryn_js::accessibility::TextCommand {
        match self {
            Self::Input {
                expected, action, ..
            } => oryn_js::accessibility::TextCommand::Picker {
                expected: (**expected).clone(),
                action: action.clone(),
            },
            Self::Permission {
                expected, choice, ..
            } => oryn_js::accessibility::TextCommand::PermissionPrompt {
                expected: (**expected).clone(),
                choice: *choice,
            },
        }
    }
}
fn virtual_id(owner: SemanticRef, session: u64, index: usize) -> SemanticRef {
    let mut hash = 2166136261u32;
    for byte in owner
        .node
        .to_le_bytes()
        .into_iter()
        .chain(session.to_le_bytes())
        .chain((index as u64).to_le_bytes())
    {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
    }
    SemanticRef {
        node: 0xffff_ffff_0000_0000 | u64::from(hash),
        ..owner
    }
}
fn paint_order(paint: &PaintSnapshot) -> BTreeMap<u64, usize> {
    oryn_common::paint_order::stacking_order(
        &paint
            .nodes
            .iter()
            .map(|node| oryn_common::paint_order::StackingNode {
                top_layer: node.top_layer,
                node_id: node.node_id,
                parent_id: node.parent_id,
                style: &node.style,
            })
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .enumerate()
    .map(|(index, id)| (id, index))
    .collect::<BTreeMap<_, _>>()
}
pub(super) fn append(
    snapshot: &mut Snapshot,
    pickers: &[(SemanticRef, oryn_dom::TemporalPicker)],
    paint: &PaintSnapshot,
) -> Result<(), String> {
    let order = paint_order(paint);
    for (owner, picker) in pickers {
        if picker.items.len() > 256 || picker.headings.len() > 7 {
            return Err("native picker AX metadata exceeds budget".into());
        }
        let Some(owner_node) = snapshot.nodes.get(owner).cloned() else {
            continue;
        };
        let expected = std::sync::Arc::new(picker.clone());
        let mut parent = None;
        let mut active = None;
        for (index, paint_node) in paint
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id == Some(owner.node)
                    && (node.tag == "::picker(temporal)" || node.tag.starts_with("::temporal-"))
            })
            .enumerate()
        {
            if snapshot.nodes.len() >= MAX_NODES {
                return Err("native picker AX node budget exceeded".into());
            }
            let id = virtual_id(*owner, picker.session, index);
            if snapshot.nodes.contains_key(&id) {
                return Err("native picker AX identity collision".into());
            }
            let (action, name, role, enabled, selected) = if paint_node.tag == "::picker(temporal)"
            {
                (None, picker.title.clone(), "dialog", true, false)
            } else if let Some(index) = paint_node
                .tag
                .strip_prefix("::temporal-item-")
                .and_then(|index| index.parse::<usize>().ok())
            {
                let item = picker
                    .items
                    .get(index)
                    .ok_or("native picker paint candidate mismatch")?;
                (
                    Some(oryn_js::accessibility::PickerAction::Choose { index }),
                    item.value.clone(),
                    "button",
                    item.enabled,
                    index == picker.active,
                )
            } else {
                match paint_node.tag.as_str() {
                    "::temporal-previous" => (
                        Some(oryn_js::accessibility::PickerAction::Previous),
                        "Previous".into(),
                        "button",
                        true,
                        false,
                    ),
                    "::temporal-next" => (
                        Some(oryn_js::accessibility::PickerAction::Next),
                        "Next".into(),
                        "button",
                        true,
                        false,
                    ),
                    "::temporal-clear" => (
                        Some(oryn_js::accessibility::PickerAction::Clear),
                        "Clear".into(),
                        "button",
                        true,
                        false,
                    ),
                    "::temporal-accept" => (
                        Some(oryn_js::accessibility::PickerAction::Accept),
                        "Accept".into(),
                        "button",
                        true,
                        false,
                    ),
                    "::temporal-title" => (None, picker.title.clone(), "heading", true, false),
                    _ => {
                        let index = paint_node
                            .tag
                            .strip_prefix("::temporal-heading-")
                            .and_then(|index| index.parse::<usize>().ok())
                            .ok_or("unknown native picker paint row")?;
                        (
                            None,
                            picker
                                .headings
                                .get(index)
                                .cloned()
                                .ok_or("native picker heading mismatch")?,
                            "columnheader",
                            true,
                            false,
                        )
                    }
                }
            };
            let mut rect = paint_node
                .geometry
                .box_model
                .as_ref()
                .map(|model| model.border_box);
            if let Some(camera) = paint.visual_viewport {
                rect = rect.and_then(|rect| {
                    let (x, y) = camera.layout_to_visual(rect.x, rect.y)?;
                    let (right, bottom) = camera.layout_to_visual(rect.right(), rect.bottom())?;
                    Some(CssRect {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    })
                });
            }
            let parent_id = parent.unwrap_or(*owner);
            let states = [
                (!enabled, NodeState::Disabled),
                (selected, NodeState::Selected),
            ]
            .into_iter()
            .filter_map(|(present, state)| present.then_some(state))
            .collect();
            let node = Node {
                boundary_journal: None,
                hit: hit::projection(paint_node, paint.visual_viewport),
                paint_order: vec![order.get(&paint_node.node_id).copied().unwrap_or(0)],
                scope: owner_node.scope.clone(),
                revision: snapshot.revision,
                id,
                parent: Some(parent_id),
                children: Vec::new(),
                text: None,
                role: role.into(),
                name,
                description: None,
                value: None,
                states,
                actions: if action.is_some() {
                    BTreeSet::from([SemanticAction::Click])
                } else {
                    BTreeSet::new()
                },
                rect,
                relations: BTreeMap::new(),
                properties: BTreeMap::new(),
            };
            snapshot.nodes.insert(id, node);
            snapshot
                .nodes
                .get_mut(&parent_id)
                .ok_or("native picker AX parent missing")?
                .children
                .push(id);
            if parent.is_none() {
                parent = Some(id);
            }
            if selected {
                active = Some(id);
            }
            if let Some(action) = action {
                snapshot.picker_actions.insert(
                    id,
                    Target::Input {
                        owner: *owner,
                        expected: expected.clone(),
                        action,
                    },
                );
            }
        }
        if let Some(active) = active {
            snapshot
                .nodes
                .get_mut(owner)
                .unwrap()
                .relations
                .insert(AccessibilityRelation::ActiveDescendant, vec![active]);
        }
    }
    Ok(())
}

pub(super) fn append_permission(
    snapshot: &mut Snapshot,
    prompts: &[(SemanticRef, oryn_dom::PermissionPrompt)],
    paint: &PaintSnapshot,
) -> Result<(), String> {
    let order = paint_order(paint);
    for (owner, prompt) in prompts {
        if prompt.id.len() > 128 || prompt.origin.len() > 4096 {
            return Err("native permission AX metadata budget exceeded".into());
        }
        if owner.page_id != snapshot.page || owner.document_generation != snapshot.generation {
            return Err("foreign native permission popup identity".into());
        }
        let owner_node = snapshot.nodes.get(owner).cloned();
        if owner_node.is_none() && prompt.protocol.is_none() {
            continue;
        }
        let expected = std::sync::Arc::new(prompt.clone());
        let mut session = 0u64;
        for byte in prompt.id.bytes() {
            session = session.wrapping_mul(131).wrapping_add(u64::from(byte));
        }
        let mut parent = None;
        for (index, item) in paint
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id == Some(owner.node)
                    && (node.tag == "::picker(permission)" || node.tag.starts_with("::permission-"))
            })
            .enumerate()
        {
            if snapshot.nodes.len() >= MAX_NODES {
                return Err("native permission AX object budget exceeded".into());
            }
            let id = virtual_id(*owner, session, index);
            if snapshot.nodes.contains_key(&id) {
                return Err("native permission AX identity collision".into());
            }
            let (choice, name, role) = match item.tag.as_str() {
                "::picker(permission)" => (None, "Permission request".into(), "dialog"),
                "::permission-title" => (
                    None,
                    if let Some(scheme) = &prompt.protocol {
                        format!("Register {scheme}: protocol handler")
                    } else if prompt.geolocation {
                        "Location permission".into()
                    } else {
                        "Camera and microphone permission".into()
                    },
                    "heading",
                ),
                "::permission-origin" => (None, prompt.origin.clone(), "statictext"),
                "::permission-allow" => (
                    Some(oryn_js::platform_control::PromptChoice::Allow),
                    "Allow".into(),
                    "button",
                ),
                "::permission-deny" => (
                    Some(oryn_js::platform_control::PromptChoice::Deny),
                    "Deny".into(),
                    "button",
                ),
                "::permission-dismiss" => (
                    Some(oryn_js::platform_control::PromptChoice::Dismiss),
                    "Dismiss".into(),
                    "button",
                ),
                _ => return Err("unknown native permission AX row".into()),
            };
            let mut rect = item
                .geometry
                .box_model
                .as_ref()
                .map(|model| model.border_box);
            if let Some(camera) = paint.visual_viewport {
                rect = rect.and_then(|rect| {
                    let (x, y) = camera.layout_to_visual(rect.x, rect.y)?;
                    let (right, bottom) = camera.layout_to_visual(rect.right(), rect.bottom())?;
                    Some(CssRect {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    })
                });
            }
            let parent_id = parent.or_else(|| owner_node.as_ref().map(|_| *owner));
            let selected = choice.is_some_and(|choice| match choice {
                oryn_js::platform_control::PromptChoice::Allow => prompt.active == 0,
                oryn_js::platform_control::PromptChoice::Deny => prompt.active == 1,
                oryn_js::platform_control::PromptChoice::Dismiss => prompt.active == 2,
            });
            snapshot.nodes.insert(
                id,
                Node {
                    boundary_journal: None,
                    hit: hit::projection(item, paint.visual_viewport),
                    paint_order: vec![order.get(&item.node_id).copied().unwrap_or(0)],
                    scope: owner_node.as_ref().and_then(|node| node.scope.clone()),
                    revision: snapshot.revision,
                    id,
                    parent: parent_id,
                    children: Vec::new(),
                    text: None,
                    role: role.into(),
                    name,
                    description: None,
                    value: None,
                    states: if selected {
                        BTreeSet::from([NodeState::Selected])
                    } else {
                        BTreeSet::new()
                    },
                    actions: if choice.is_some() {
                        BTreeSet::from([SemanticAction::Click])
                    } else {
                        BTreeSet::new()
                    },
                    rect,
                    relations: BTreeMap::new(),
                    properties: BTreeMap::new(),
                },
            );
            if let Some(parent_id) = parent_id {
                snapshot
                    .nodes
                    .get_mut(&parent_id)
                    .ok_or("native permission AX parent missing")?
                    .children
                    .push(id);
            } else {
                snapshot.roots.push(id);
            }
            if parent.is_none() {
                parent = Some(id);
            }
            if let Some(choice) = choice {
                snapshot.picker_actions.insert(
                    id,
                    Target::Permission {
                        owner: *owner,
                        expected: expected.clone(),
                        choice,
                    },
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_identity_includes_owner_and_popup_session() {
        let owner = SemanticRef {
            page_id: PageId(1),
            document_generation: 3,
            node: 99,
        };
        assert_ne!(virtual_id(owner, 7, 0), virtual_id(owner, 8, 0));
        assert_ne!(virtual_id(owner, 7, 0), virtual_id(owner, 7, 1));
        assert_ne!(
            virtual_id(owner, 7, 0),
            virtual_id(SemanticRef { node: 100, ..owner }, 7, 0)
        );
    }
    #[cfg(all(feature = "v8-host", feature = "in-process-probe"))]
    #[tokio::test]
    async fn native_ax_popup_rows_press_real_owner_and_retired_rows_cannot_reenter() {
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        page.load_html(
            "https://ax.test/",
            "<input id=d aria-label=Date type=date value=2024-02-28>",
        )
        .await
        .unwrap();
        let owner = page
            .resolve_selector("#d", SemanticAction::Click)
            .await
            .unwrap();
        page.execute(NativeAction {
            target: owner,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .unwrap();
        let snapshot = super::super::capture_document(&page).await.unwrap();
        let target = snapshot
            .nodes
            .values()
            .find(|node| {
                node.name == "2024-02-29" && snapshot.picker_actions.contains_key(&node.id)
            })
            .unwrap();
        assert!(target.parent.is_some());
        assert!(target.rect.is_some());
        assert!(target.actions.contains(&SemanticAction::Click));
        let request = ActionRequest {
            scope: target.scope.clone(),
            target: target.id,
            revision: target.revision,
            action: Action::Press,
        };
        assert!(snapshot.validate_action(&request).is_err());
        super::super::execute(&page, request.clone()).await.unwrap();
        assert_eq!(
            page.evaluate("d.value==='2024-02-29'").await.unwrap(),
            "true"
        );
        assert!(super::super::execute(&page, request).await.is_err());
        page.close().await.unwrap();
    }
}

#[cfg(all(test, feature = "v8-host", feature = "in-process-probe"))]
mod protocol_prompt_tests {
    use super::*;
    #[tokio::test]
    async fn excluded_html_protocol_prompt_remains_actionable_in_root_and_frame_scope() {
        for frame in [false, true] {
            let browser = crate::Browser::in_process_probe();
            let context = browser.new_context();
            let page = context.new_page();
            page.load_html(
                "https://ax.test/",
                if frame {
                    "<iframe srcdoc='<body>child</body>'></iframe>"
                } else {
                    "<body>root</body>"
                },
            )
            .await
            .unwrap();
            if frame {
                assert_eq!(page.evaluate("(()=>{const r=document.querySelector('iframe').getBoundingClientRect();return String(r.width>0&&r.height>0)})()").await.unwrap(),"true","protocol controls need an actual embedding placement even when child HTML is excluded");
            }
            page.evaluate(if frame{"(()=>{const w=frames[0];w.document.documentElement.style.display='none';w.document.documentElement.inert=true;w.navigator.registerProtocolHandler('web+ax','https://ax.test/?%s');return true})()"}else{"(()=>{document.documentElement.style.display='none';document.documentElement.inert=true;navigator.registerProtocolHandler('web+ax','https://ax.test/?%s');return true})()"}).await.unwrap();
            let snapshot = super::super::capture(&page).await.unwrap();
            let target=snapshot.nodes.values().find(|node|node.name=="Deny"&&matches!(snapshot.picker_actions.get(&node.id),Some(Target::Permission{expected,..})if expected.protocol.as_deref()==Some("web+ax"))).unwrap();
            assert_eq!(
                target
                    .scope
                    .as_ref()
                    .is_some_and(|scope| scope.parent_realm.is_some()),
                frame
            );
            let command = snapshot.picker_actions.get(&target.id).unwrap();
            assert!(
                !snapshot.nodes.contains_key(&command.owner()),
                "hidden HTML must not be exposed as authored content"
            );
            assert!(target.rect.is_some());
            let request = ActionRequest {
                scope: target.scope.clone(),
                target: target.id,
                revision: target.revision,
                action: Action::Press,
            };
            super::super::execute(&page, request.clone()).await.unwrap();
            assert!(super::super::execute(&page, request).await.is_err());
            assert!(super::super::capture(&page).await.unwrap().picker_actions.values().all(|target|!matches!(target,Target::Permission{expected,..}if expected.protocol.is_some())));
            page.close().await.unwrap();
        }
    }
}
