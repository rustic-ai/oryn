//! Native assistive-technology consumer. Snapshot reads never enter a worker or V8.
//! OS callbacks enqueue exact-generation actions to the native page owner.
use crate::{NativeAction, PageHandle};
use oryn_common::v3::{
    NodeState, PageId, ProjectionProfile, Revision, SemanticAction, SemanticRef,
};
use oryn_common::v4::{CssRect, PaintSnapshot};
use oryn_common::v5::{AccessibilityRelation, SemanticObservation};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
pub mod macos;

mod frames;
mod hit;
mod live;
mod picker;
mod role_mapping;
mod table;
mod text_geometry;
mod text_markers;
pub use frames::capture;
pub use live::LiveRegion;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NativeSnapshot {
    #[serde(default)]
    pub file_pickers: Vec<(SemanticRef, oryn_dom::FilePicker)>,
    #[serde(default)]
    pub permission_prompts: Vec<(SemanticRef, oryn_dom::PermissionPrompt)>,
    #[serde(default)]
    pub pickers: Vec<(SemanticRef, oryn_dom::TemporalPicker)>,
    pub boundary_journal: oryn_dom::boundary_journal::Snapshot,
    pub semantic: SemanticObservation,
    pub paint: PaintSnapshot,
    pub text: Vec<(SemanticRef, oryn_js::accessibility::TextSnapshot)>,
}
pub const MAX_NODES: usize = 65_536;
#[derive(Clone, Debug)]
pub struct Node {
    pub boundary_journal: Option<std::sync::Arc<oryn_dom::boundary_journal::Snapshot>>,
    pub hit: Option<hit::Hit>,
    pub paint_order: Vec<usize>,
    pub scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
    pub revision: Revision,
    pub id: SemanticRef,
    pub parent: Option<SemanticRef>,
    pub children: Vec<SemanticRef>,
    pub text: Option<oryn_js::accessibility::TextSnapshot>,
    pub role: String,
    pub name: String,
    pub description: Option<String>,
    pub value: Option<String>,
    pub states: BTreeSet<NodeState>,
    pub actions: BTreeSet<SemanticAction>,
    pub rect: Option<CssRect>,
    pub relations: BTreeMap<AccessibilityRelation, Vec<SemanticRef>>,
    pub properties: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub file_pickers: BTreeMap<SemanticRef, oryn_dom::FilePicker>,
    pub picker_actions: BTreeMap<SemanticRef, picker::Target>,
    pub tables: BTreeMap<SemanticRef, table::Table>,
    pub root_scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
    pub page: PageId,
    pub generation: u64,
    pub revision: Revision,
    pub title: String,
    pub viewport: oryn_common::v4::ViewportState,
    pub image: Option<Vec<u8>>,
    pub live_regions: BTreeMap<SemanticRef, LiveRegion>,
    pub placements: BTreeMap<u64, frames::Placement>,
    pub nodes: BTreeMap<SemanticRef, Node>,
    pub roots: Vec<SemanticRef>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Adjust {
        increase: bool,
    },
    FileSelection {
        expected: oryn_dom::FilePicker,
        paths: Option<Vec<std::path::PathBuf>>,
    },
    Input(crate::webdriver_actions::ActionsRequest),
    CancelInput,
    Press,
    Focus,
    SetValue(String),
    SelectText {
        start: u32,
        end: u32,
        expected: oryn_js::accessibility::TextSnapshot,
    },
}
#[derive(Clone, Debug)]
pub struct ActionRequest {
    pub scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
    pub target: SemanticRef,
    pub revision: Revision,
    pub action: Action,
}
impl Snapshot {
    fn text_storage_bytes(&self) -> Result<usize, String> {
        self.nodes
            .values()
            .filter_map(|node| node.text.as_ref())
            .try_fold(0usize, |bytes, text| {
                bytes
                    .checked_add(
                        text.retained_bytes()
                            .ok_or("accessibility text storage overflow")?,
                    )
                    .ok_or_else(|| "accessibility text storage overflow".to_string())
            })
    }

    pub fn compose(
        observation: SemanticObservation,
        paint: &PaintSnapshot,
    ) -> Result<Self, String> {
        let page = &observation.observation.page;
        if page.page_id != paint.page.page_id
            || page.document_generation != paint.page.document_generation
            || observation.observation.revision != paint.revision
        {
            return Err("accessibility and geometry revisions differ".into());
        }
        if observation.accessibility.len() > MAX_NODES {
            return Err("accessibility node budget exceeded".into());
        }
        let views: BTreeMap<_, _> = observation
            .observation
            .nodes
            .iter()
            .map(|n| (n.semantic_ref, n))
            .collect();
        let geometry: BTreeMap<_, _> = paint.nodes.iter().map(|n| (n.node_id, n)).collect();
        let stacking = paint
            .nodes
            .iter()
            .map(|node| oryn_common::paint_order::StackingNode {
                top_layer: node.top_layer,
                node_id: node.node_id,
                parent_id: node.parent_id,
                style: &node.style,
            })
            .collect::<Vec<_>>();
        let paint_order = oryn_common::paint_order::stacking_order(&stacking)
            .into_iter()
            .enumerate()
            .map(|(rank, id)| (id, rank))
            .collect::<BTreeMap<_, _>>();
        let mut nodes = BTreeMap::new();
        for ax in &observation.accessibility {
            if ax.excluded {
                continue;
            }
            let Some(view) = views.get(&ax.semantic_ref) else {
                continue;
            };
            if ax.semantic_ref.page_id != page.page_id
                || ax.semantic_ref.document_generation != page.document_generation
            {
                return Err("foreign accessibility node identity".into());
            }
            let rect = geometry.get(&ax.semantic_ref.node).and_then(|node| {
                if node.geometry.availability != oryn_common::v4::GeometryAvailability::Available {
                    return None;
                }
                let mut rect = node
                    .geometry
                    .client_rects
                    .first()
                    .copied()
                    .or_else(|| node.geometry.box_model.as_ref().map(|b| b.border_box))?;
                for fragment in node.geometry.client_rects.iter().skip(1) {
                    let right = rect.right().max(fragment.right());
                    let bottom = rect.bottom().max(fragment.bottom());
                    rect.x = rect.x.min(fragment.x);
                    rect.y = rect.y.min(fragment.y);
                    rect.width = right - rect.x;
                    rect.height = bottom - rect.y;
                }
                // Geometry already includes CSS transforms and scrolling. Apply only the
                // visual camera transform shared with the screenshot, never projection.matrix twice.
                if let Some(camera) = paint.visual_viewport {
                    let (left, top) = camera.layout_to_visual(rect.x, rect.y)?;
                    let (right, bottom) = camera.layout_to_visual(rect.right(), rect.bottom())?;
                    rect = CssRect {
                        x: left,
                        y: top,
                        width: right - left,
                        height: bottom - top,
                    };
                }
                if [rect.x, rect.y, rect.width, rect.height]
                    .iter()
                    .all(|v| v.is_finite())
                    && rect.width >= 0.
                    && rect.height >= 0.
                {
                    Some(rect)
                } else {
                    None
                }
            });
            // The semantic producer redacts secret values; do not recover them from DOM/paint.
            nodes.insert(
                ax.semantic_ref,
                Node {
                    boundary_journal: None,
                    hit: geometry
                        .get(&ax.semantic_ref.node)
                        .and_then(|node| hit::projection(node, paint.visual_viewport)),
                    paint_order: vec![paint_order.get(&ax.semantic_ref.node).copied().unwrap_or(0)],
                    scope: None,
                    revision: observation.observation.revision,
                    id: ax.semantic_ref,
                    parent: ax.owned_by.or(view.parent),
                    children: Vec::new(),
                    text: None,
                    role: ax.role.clone(),
                    name: ax.name.clone(),
                    description: ax.description.clone(),
                    value: view.value.clone(),
                    states: view.states.clone(),
                    actions: view.actions.clone(),
                    rect,
                    relations: ax.relationships.clone(),
                    properties: ax.properties.clone(),
                },
            );
        }
        let ids: Vec<_> = nodes.keys().copied().collect();
        let mut roots = Vec::new();
        for id in ids {
            let mut parent = nodes[&id].parent;
            let mut visited = BTreeSet::from([id]);
            while let Some(candidate) = parent {
                if !visited.insert(candidate) {
                    return Err("accessibility parent cycle".into());
                }
                if nodes.contains_key(&candidate) {
                    break;
                }
                parent = views.get(&candidate).and_then(|view| view.parent);
            }
            nodes.get_mut(&id).unwrap().parent = parent;
            if let Some(parent) = parent {
                nodes.get_mut(&parent).unwrap().children.push(id);
            } else {
                roots.push(id);
            }
        }
        // Validate ownership edges collectively, including cycles through included nodes.
        for id in nodes.keys() {
            let mut visited = BTreeSet::new();
            let mut cursor = Some(*id);
            while let Some(next) = cursor {
                if !visited.insert(next) {
                    return Err("accessibility ownership cycle".into());
                }
                cursor = nodes.get(&next).and_then(|n| n.parent);
            }
        }
        let order = |id: &SemanticRef| views.get(id).map_or(u32::MAX, |n| n.document_order);
        roots.sort_by_key(order);
        for node in nodes.values_mut() {
            node.children.sort_by_key(order);
        }
        let live_regions = live::project(
            &nodes,
            observation
                .structures
                .iter()
                .filter(|structure| {
                    structure.kind == oryn_common::v5::SemanticStructureKind::LiveRegion
                })
                .map(|structure| structure.root),
        )?;
        let tables = table::project(&nodes)?;
        Ok(Self {
            file_pickers: BTreeMap::new(),
            picker_actions: BTreeMap::new(),
            tables,
            root_scope: None,
            page: page.page_id,
            generation: page.document_generation,
            revision: observation.observation.revision,
            title: page.title.clone(),
            viewport: paint.viewport,
            image: None,
            live_regions,
            placements: frames::placements(paint),
            nodes,
            roots,
        })
    }
    pub fn validate_action(&self, request: &ActionRequest) -> Result<Vec<NativeAction>, String> {
        if request.revision != self.revision {
            return Err("stale accessibility action revision".into());
        }
        if self.picker_actions.contains_key(&request.target) {
            return Err("native picker requires its typed action".into());
        }
        let node = self
            .nodes
            .get(&request.target)
            .ok_or("stale accessibility element")?;
        if node.states.contains(&NodeState::Disabled) {
            return Err("disabled accessibility element".into());
        }
        let actions = match &request.action {
            Action::FileSelection { .. } => {
                return Err("file selection requires native picker ownership".into());
            }
            Action::Input(_) | Action::CancelInput => {
                return Err("OS input requires native root authority".into());
            }
            Action::Adjust { .. } => {
                return Err("adjustment requires native numeric command".into());
            }
            Action::SelectText { .. } => {
                return Err("selection requires typed accessibility text command".into());
            }
            Action::Press => vec![(SemanticAction::Click, None)],
            Action::Focus => vec![(SemanticAction::Focus, None)],
            Action::SetValue(value) => {
                if node.states.contains(&NodeState::Readonly) {
                    return Err("read-only accessibility element".into());
                }
                if node.actions.contains(&SemanticAction::Check) {
                    let checked = match value.as_str() {
                        "1" | "true" => true,
                        "0" | "false" => false,
                        _ => return Err("checkable AX value must be zero or one".into()),
                    };
                    vec![(
                        if checked {
                            SemanticAction::Check
                        } else {
                            SemanticAction::Uncheck
                        },
                        None,
                    )]
                } else if node.actions.contains(&SemanticAction::Select) {
                    vec![(SemanticAction::Select, Some(value.clone()))]
                } else {
                    vec![(SemanticAction::Type, Some(value.clone()))]
                }
            }
        };
        if actions
            .iter()
            .any(|(action, _)| !node.actions.contains(action))
        {
            return Err("unsupported accessibility action".into());
        }
        Ok(actions
            .into_iter()
            .map(|(action, value)| NativeAction {
                target: request.target,
                action,
                value,
            })
            .collect())
    }
}
/// Re-read around geometry capture: never publish mismatched document generations.
async fn capture_document(page: &PageHandle) -> Result<Snapshot, String> {
    for _ in 0..3 {
        let native = match page.accessibility_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if matches!(error, crate::RuntimeError::Semantic(_)) {
                    continue;
                }
                return Err(error.to_string());
            }
        };
        let mut snapshot = Snapshot::compose(native.semantic, &native.paint)?;
        picker::append(&mut snapshot, &native.pickers, &native.paint)?;
        picker::append_permission(&mut snapshot, &native.permission_prompts, &native.paint)?;
        for (owner, request) in native.file_pickers {
            if snapshot.nodes.contains_key(&owner) {
                snapshot.file_pickers.insert(owner, request);
            }
        }
        let boundary_journal = std::sync::Arc::new(native.boundary_journal);
        for node in snapshot.nodes.values_mut() {
            node.boundary_journal = Some(boundary_journal.clone());
        }
        for (target, mut text) in native.text {
            if let Some(node) = snapshot.nodes.get_mut(&target) {
                if let Some(camera) = native.paint.visual_viewport {
                    for caret in text.carets.iter_mut().chain(
                        text.protected_caret
                            .iter_mut()
                            .map(|record| &mut record.caret),
                    ) {
                        if let Some((x, y)) = camera.layout_to_visual(caret.x, caret.y) {
                            caret.x = x;
                            caret.y = y;
                        }
                        caret.rect = caret.rect.and_then(|rect| {
                            let (x, y) = camera.layout_to_visual(rect.x, rect.y)?;
                            let (right, bottom) =
                                camera.layout_to_visual(rect.right(), rect.bottom())?;
                            Some(CssRect {
                                x,
                                y,
                                width: right - x,
                                height: bottom - y,
                            })
                        });
                    }
                    for item in &mut text.boxes {
                        let rect = item.rect;
                        if let (Some((x, y)), Some((right, bottom))) = (
                            camera.layout_to_visual(rect.x, rect.y),
                            camera.layout_to_visual(rect.right(), rect.bottom()),
                        ) {
                            item.rect = CssRect {
                                x,
                                y,
                                width: right - x,
                                height: bottom - y,
                            };
                        }
                    }
                }
                node.text = Some(text);
            }
        }
        #[cfg(feature = "accessibility-host")]
        if page.frame_scope().is_none() {
            let raster = oryn_render_core::render_png(
                &native.paint,
                oryn_render_core::RenderOptions {
                    max_pixels: 16 * 1024 * 1024,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            if raster.bytes.len() > 64 * 1024 * 1024 {
                return Err("accessibility raster budget exceeded".into());
            }
            snapshot.image = Some(raster.bytes);
        }
        return Ok(snapshot);
    }
    Err("document changed during accessibility capture".into())
}

pub async fn execute(
    page: &PageHandle,
    request: ActionRequest,
) -> Result<Vec<crate::webdriver_actions::OsKeyReceipt>, String> {
    if matches!(&request.action, Action::Input(_) | Action::CancelInput) {
        let scope = request
            .scope
            .clone()
            .ok_or("native OS input scope unavailable")?;
        if scope.parent_realm.is_some() || request.target.page_id.0 != page.id() {
            return Err("OS input requires owning top-level page".into());
        }
        let context = crate::host::webdriver_script::ScriptContext {
            selected: Some(scope),
            ..Default::default()
        };
        let command = match &request.action {
            Action::Input(input) => crate::webdriver_actions::ActionCommand::Perform(input.clone()),
            _ => crate::webdriver_actions::ActionCommand::Cancel,
        };
        return match page
            .execute_native_actions_with_receipts(command, context.clone())
            .await
        {
            Ok(receipts) => Ok(receipts),
            Err(error) => {
                let _ = page
                    .execute_webdriver_actions_in_context(
                        crate::webdriver_actions::ActionCommand::Cancel,
                        context,
                    )
                    .await;
                Err(format!("{}: {}", error.code, error.message))
            }
        };
    }
    let scoped = page.with_frame_scope(request.scope.clone());
    let page = &scoped;
    let live = capture_document(page).await?;
    if let Action::FileSelection { expected, paths } = &request.action {
        if live.file_pickers.get(&request.target) != Some(expected) {
            return Err("stale native OS file chooser".into());
        }
        page.complete_native_file_picker(request.target, expected.clone(), paths.clone())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Vec::new());
    }
    if let Action::Adjust { increase } = &request.action {
        let node = live
            .nodes
            .get(&request.target)
            .ok_or("stale AX numeric control")?;
        if request.revision != live.revision
            || !matches!(node.role.as_str(), "slider" | "spinbutton")
            || !node.actions.contains(&SemanticAction::Type)
            || node.states.contains(&NodeState::Disabled)
            || node.states.contains(&NodeState::Readonly)
        {
            return Err("unsupported or stale AX adjustment".into());
        }
        page.accessibility_text(
            request.target,
            oryn_js::accessibility::TextCommand::Adjust {
                increase: *increase,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        return Ok(Vec::new());
    }
    if let Action::SelectText {
        start,
        end,
        expected,
    } = &request.action
    {
        if request.revision != live.revision || !live.nodes.contains_key(&request.target) {
            return Err("stale accessibility selection target".into());
        }
        page.accessibility_text(
            request.target,
            oryn_js::accessibility::TextCommand::Select {
                start: *start,
                end: *end,
                expected_value: expected.value.clone(),
                expected_selection_revision: expected.selection_revision,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }
    if let Some(target) = live.picker_actions.get(&request.target) {
        if request.revision != live.revision
            || !matches!(&request.action, Action::Press)
            || live
                .nodes
                .get(&request.target)
                .is_none_or(|node| node.states.contains(&NodeState::Disabled))
        {
            return Err("stale or unsupported native picker action".into());
        }
        page.accessibility_text(target.owner(), target.command())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Vec::new());
    }
    let actions = live.validate_action(&request)?;
    page.execute_batch(actions)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> Snapshot {
        let id = SemanticRef {
            page_id: PageId(7),
            document_generation: 3,
            node: 99,
        };
        let node = Node {
            boundary_journal: None,
            hit: None,
            paint_order: vec![0],
            scope: None,
            revision: Revision(12),
            id,
            parent: None,
            children: vec![],
            text: None,
            role: "button".into(),
            name: "Save".into(),
            description: None,
            value: None,
            states: BTreeSet::new(),
            actions: BTreeSet::from([SemanticAction::Click, SemanticAction::Focus]),
            rect: None,
            relations: BTreeMap::new(),
            properties: BTreeMap::new(),
        };
        Snapshot {
            file_pickers: BTreeMap::new(),
            picker_actions: BTreeMap::new(),
            tables: BTreeMap::new(),
            root_scope: None,
            page: id.page_id,
            generation: id.document_generation,
            revision: Revision(12),
            title: "Test".into(),
            viewport: Default::default(),
            image: None,
            live_regions: BTreeMap::new(),
            placements: BTreeMap::new(),
            nodes: BTreeMap::from([(id, node)]),
            roots: vec![id],
        }
    }
    #[test]
    fn stale_and_foreign_references_never_become_native_actions() {
        let snapshot = fixture();
        let id = snapshot.roots[0];
        let request = ActionRequest {
            scope: None,
            target: id,
            revision: snapshot.revision,
            action: Action::Press,
        };
        assert_eq!(snapshot.validate_action(&request).unwrap()[0].target, id);
        for target in [
            SemanticRef {
                page_id: PageId(8),
                ..id
            },
            SemanticRef {
                document_generation: 4,
                ..id
            },
            SemanticRef { node: 100, ..id },
        ] {
            assert!(
                snapshot
                    .validate_action(&ActionRequest {
                        target,
                        ..request.clone()
                    })
                    .is_err()
            );
        }
        assert!(
            snapshot
                .validate_action(&ActionRequest {
                    revision: Revision(11),
                    ..request
                })
                .is_err()
        );
    }
    #[test]
    fn disabled_and_unsupported_defaults_are_not_advertised_as_writable() {
        let mut snapshot = fixture();
        let id = snapshot.roots[0];
        let request = ActionRequest {
            scope: None,
            target: id,
            revision: snapshot.revision,
            action: Action::Press,
        };
        snapshot
            .nodes
            .get_mut(&id)
            .unwrap()
            .states
            .insert(NodeState::Disabled);
        assert!(snapshot.validate_action(&request).is_err());
        snapshot.nodes.get_mut(&id).unwrap().states.clear();
        assert!(
            snapshot
                .validate_action(&ActionRequest {
                    action: Action::SetValue("replacement".into()),
                    ..request
                })
                .is_err()
        );
    }
    #[test]
    fn ax_check_and_selection_values_route_to_native_semantic_actions() {
        let mut snapshot = fixture();
        let id = snapshot.roots[0];
        let node = snapshot.nodes.get_mut(&id).unwrap();
        node.actions.insert(SemanticAction::Check);
        node.actions.insert(SemanticAction::Uncheck);
        let request = ActionRequest {
            scope: None,
            target: id,
            revision: snapshot.revision,
            action: Action::SetValue("1".into()),
        };
        assert_eq!(
            snapshot.validate_action(&request).unwrap()[0].action,
            SemanticAction::Check
        );
        assert!(
            snapshot
                .validate_action(&ActionRequest {
                    action: Action::SetValue("mixed".into()),
                    ..request.clone()
                })
                .is_err()
        );
        let node = snapshot.nodes.get_mut(&id).unwrap();
        node.actions.remove(&SemanticAction::Check);
        node.actions.insert(SemanticAction::Select);
        assert_eq!(
            snapshot.validate_action(&request).unwrap()[0].action,
            SemanticAction::Select
        );
    }
    #[test]
    fn ax_value_uses_one_atomic_native_replacement() {
        let mut snapshot = fixture();
        let id = snapshot.roots[0];
        snapshot
            .nodes
            .get_mut(&id)
            .unwrap()
            .actions
            .insert(SemanticAction::Type);
        let actions = snapshot
            .validate_action(&ActionRequest {
                scope: None,
                target: id,
                revision: snapshot.revision,
                action: Action::SetValue("new value".into()),
            })
            .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, SemanticAction::Type);
        assert_eq!(actions[0].value.as_deref(), Some("new value"));
        snapshot
            .nodes
            .get_mut(&id)
            .unwrap()
            .states
            .insert(NodeState::Readonly);
        assert!(
            snapshot
                .validate_action(&ActionRequest {
                    scope: None,
                    target: id,
                    revision: snapshot.revision,
                    action: Action::SetValue("x".into())
                })
                .is_err()
        );
    }
    #[cfg(feature = "in-process-probe")]
    #[tokio::test]
    async fn semantic_geometry_generation_mismatch_is_rejected() {
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        page.load_html("https://example.test/ax", "<button>Save</button>")
            .await
            .unwrap();
        let ax = page
            .observe_semantics(ProjectionProfile::Full)
            .await
            .unwrap();
        let mut paint = page.paint_snapshot().await.unwrap();
        paint.page.document_generation += 1;
        assert!(Snapshot::compose(ax, &paint).is_err());
        page.close().await.unwrap();
    }
}

#[doc(hidden)]
pub async fn execute_owned(page:&PageHandle,mut owned:crate::owned_action_request::OwnedActionRequest)->Result<Vec<crate::webdriver_actions::OwnedOsKeyReceipt>,String>{
    let request=&mut owned.request;
    if matches!(&request.action,Action::Input(_)|Action::CancelInput){
        let scope=request.scope.clone().ok_or("native OS input scope unavailable")?;
        if scope.parent_realm.is_some()||request.target.page_id.0!=page.id(){return Err("OS input requires owning top-level page".into())}
        let context=crate::host::webdriver_script::ScriptContext{selected:Some(scope.clone()),..Default::default()};
        let action=std::mem::replace(&mut request.action,Action::CancelInput);
        let command=match action{Action::Input(input)=>crate::webdriver_actions::ActionCommand::Perform(input),_=>crate::webdriver_actions::ActionCommand::Cancel};
        let mut response=page.scoped_owned_actions(scope,command,context).await.map_err(|error|error.to_string())?;
        match &response.response {
            crate::worker::WorkerResponse::WebdriverActions{result,..}=>result.as_ref().map_err(|error|error.message.clone())?,
            _=>return Err("native owned action reply mismatch".into()),
        }
        return Ok(response.receipts.take().unwrap_or_default());
    }
    let target=request.target;let revision=request.revision;
    let request=std::mem::replace(&mut owned.request,ActionRequest{scope:None,target,revision,action:Action::CancelInput});
    execute(page,request).await.map(|receipts|receipts.into_iter().map(crate::webdriver_actions::OwnedOsKeyReceipt::from).collect())
}
