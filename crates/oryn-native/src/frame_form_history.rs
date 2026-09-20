//! Child-document FACE history uses native embedding descriptors and exact URLs.
use super::*;
use oryn_js::form_state::{FrameHistoryKey, SavedFrameFormState};
pub(super) fn frame_history_path_key(path: &[FrameHistoryKey]) -> Result<String, HostError> {
    // Mutable src/srcdoc are response data, not restoration owner identity.
    // Excluding them also avoids duplicating archived source in every claim.
    let identity = path
        .iter()
        .map(|part| (&part.tag, &part.id, &part.name, part.occurrence))
        .collect::<Vec<_>>();
    serde_json::to_string(&identity).map_err(|error| HostError::Execution(error.to_string()))
}
impl RustExecutableDocument {
    pub(crate) fn form_history_root_identity(&self) -> Option<(RealmId, u64, u64)> {
        self.automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_none() && realm.live)
            .map(|realm| (realm.realm, realm.document_generation, realm.stream_epoch))
    }
    pub(super) fn frame_form_path(
        &self,
        mut realm: RealmId,
    ) -> Result<Vec<FrameHistoryKey>, HostError> {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        while realm != self.top_realm {
            if !seen.insert(realm) || path.len() >= 32 {
                return Err(HostError::Execution(
                    "cyclic frame form history path".into(),
                ));
            }
            let frame = self
                .frame_realms
                .get(&realm)
                .ok_or_else(|| HostError::Execution("retired frame form history".into()))?;
            let parent = self
                .automation_realms()
                .into_iter()
                .find(|entry| entry.realm == frame.parent_realm && entry.live)
                .ok_or_else(|| HostError::Execution("retired parent form history".into()))?;
            let dom = self
                .agent
                .host
                .automation_dom(parent.realm)
                .ok_or_else(|| HostError::Execution("frame form DOM missing".into()))?;
            let dom = dom.borrow();
            fn key(dom: &DomRuntime, node: NodeId) -> Result<FrameHistoryKey, HostError> {
                let NodeKind::Element { name } = &dom
                    .arena()
                    .get(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .kind
                else {
                    return Err(HostError::Execution("frame owner is not an element".into()));
                };
                let mut values = Vec::new();
                for attribute in ["id", "name", "src", "srcdoc"] {
                    let value = dom
                        .attribute_in_null_namespace(node, attribute)
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .unwrap_or("");
                    if value.len() > 64 * 1024 {
                        return Err(HostError::LimitExceeded {
                            resource: "frame history owner metadata".into(),
                        });
                    }
                    values.push(value.to_owned());
                }
                Ok(FrameHistoryKey {
                    tag: name.local.to_string(),
                    id: values.remove(0),
                    name: values.remove(0),
                    src: values.remove(0),
                    srcdoc: values.remove(0),
                    occurrence: 0,
                })
            }
            let mut identity = key(&dom, frame.frame)?;
            let mut found = false;
            for node in dom
                .arena()
                .descendants_inclusive(parent.document_root)
                .map_err(|error| HostError::Execution(error.to_string()))?
            {
                if node == frame.frame {
                    found = true;
                    break;
                }
                if matches!(&dom.arena().get(node).map_err(|error|HostError::Execution(error.to_string()))?.kind,NodeKind::Element{name} if name.local.as_ref()==identity.tag)
                {
                    let candidate = key(&dom, node)?;
                    let mut plain = identity.clone();
                    plain.occurrence = 0;
                    if candidate.tag == plain.tag
                        && candidate.id == plain.id
                        && candidate.name == plain.name
                    {
                        identity.occurrence += 1;
                    }
                }
            }
            if !found {
                return Err(HostError::Execution("detached frame history owner".into()));
            }
            path.push(identity);
            realm = frame.parent_realm;
        }
        path.reverse();
        Ok(path)
    }
    pub(crate) fn capture_child_form_history(
        &mut self,
    ) -> Result<Vec<SavedFrameFormState>, HostError> {
        let previous = self.agent.host.active_realm();
        let result = (|| {
            let realms = self.automation_realms();
            let mut saved = Vec::new();
            for realm in realms
                .into_iter()
                .filter(|realm| realm.parent_realm.is_some() && realm.live)
            {
                let path = self.frame_form_path(realm.realm)?;
                let retained =
                    oryn_js::form_state::frame_snapshot_bytes(&saved).map_err(|resource| {
                        HostError::LimitExceeded {
                            resource: resource.into(),
                        }
                    })?;
                let metadata = path.iter().fold(realm.url.len(), |bytes, part| {
                    bytes.saturating_add(
                        part.tag.len()
                            + part.id.len()
                            + part.name.len()
                            + part.src.len()
                            + part.srcdoc.len(),
                    )
                });
                let available = oryn_js::form_state::MAX_FORM_SNAPSHOT_BYTES
                    .checked_sub(retained.saturating_add(metadata))
                    .ok_or_else(|| HostError::LimitExceeded {
                        resource: "frame history output bytes".into(),
                    })?;
                let controls = self
                    .agent
                    .host
                    .capture_form_history_in_realm_limited(realm.realm, available)?;
                let controls_bytes =
                    oryn_js::form_state::snapshot_bytes(&controls).map_err(|resource| {
                        HostError::LimitExceeded {
                            resource: resource.into(),
                        }
                    })?;
                let navigation = self.capture_frame_navigation_history(
                    realm.realm,
                    available.saturating_sub(controls_bytes),
                )?;
                if controls.is_empty() && navigation.is_none() {
                    continue;
                }
                saved.push(SavedFrameFormState {
                    path,
                    url: realm.url,
                    controls,
                    navigation,
                });
                oryn_js::form_state::frame_snapshot_bytes(&saved).map_err(|resource| {
                    HostError::LimitExceeded {
                        resource: resource.into(),
                    }
                })?;
            }
            Ok(saved)
        })();
        let restored = self.agent.host.activate_realm(previous);
        match result {
            Ok(saved) => {
                restored?;
                Ok(saved)
            }
            Err(error) => {
                let _ = restored;
                Err(error)
            }
        }
    }
    pub(crate) fn restore_child_form_history(
        &mut self,
        saved: &[SavedFrameFormState],
    ) -> Result<(), HostError> {
        oryn_js::form_state::frame_snapshot_bytes(saved).map_err(|resource| {
            HostError::LimitExceeded {
                resource: resource.into(),
            }
        })?;
        let identity = self
            .form_history_root_identity()
            .ok_or_else(|| HostError::Execution("root form history retired".into()))?;
        self.pending_frame_form_history = Some((identity, saved.to_vec()));
        self.deliver_pending_frame_form_history()
    }
    pub(super) fn deliver_pending_frame_form_history(&mut self) -> Result<(), HostError> {
        let previous = self.agent.host.active_realm();
        let result = (|| {
            let Some((identity, entries)) = self.pending_frame_form_history.take() else {
                return Ok(());
            };
            if self.form_history_root_identity() != Some(identity) {
                return Ok(());
            }
            let mut pending = Vec::new();
            for entry in entries {
                if self.form_history_root_identity() != Some(identity) {
                    return Ok(());
                }
                let claim_key = frame_history_path_key(&entry.path)?;
                if let Some((parent, frame, token)) =
                    self.frame_history_restore_claims.get(&claim_key)
                {
                    let current = self.agent.host.automation_frame_live(*parent, *frame)
                        && self.agent.host.automation_dom(*parent).is_some_and(|dom| {
                            dom.borrow()
                                .frame_navigation_identity_is_current(*frame, token)
                        });
                    if !current {
                        self.frame_history_restore_claims.remove(&claim_key);
                        continue;
                    }
                }
                let matches =
                    self.automation_realms()
                        .into_iter()
                        .filter(|realm| {
                            realm.live
                                && !self.agent.host.realm_document_load_stopped(realm.realm)
                                && realm.parent_realm.is_some()
                                && realm.url == entry.url
                                && self.frame_form_path(realm.realm).ok().as_ref().is_some_and(
                                    |path| {
                                        if entry.navigation.is_some() {
                                            path.len() == entry.path.len()
                                                && path.iter().zip(&entry.path).all(|(a, b)| {
                                                    a.tag == b.tag
                                                        && a.id == b.id
                                                        && a.name == b.name
                                                        && a.occurrence == b.occurrence
                                                })
                                        } else {
                                            path == &entry.path
                                        }
                                    },
                                )
                        })
                        .collect::<Vec<_>>();
                if matches.len() != 1 {
                    // A committed redirect is terminal for this saved document's
                    // controls; it must not wait for a later URL coincidence.
                    let terminal = self
                        .frame_history_restore_claims
                        .get(&claim_key)
                        .is_some_and(|(parent, frame, _)| {
                            self.frame_realms.iter().any(|(realm, owner)| {
                                owner.parent_realm == *parent
                                    && owner.frame == *frame
                                    && !self.agent.host.realm_document_load_stopped(*realm)
                                    && !self
                                        .pending_frame_navigations
                                        .contains_key(&(*parent, *frame))
                                    && !self
                                        .frame_resource_preparations
                                        .contains_key(&(*parent, *frame))
                                    && !self.frame_scripts_pending(Some(*realm))
                            })
                        });
                    if terminal {
                        self.frame_history_restore_claims.remove(&claim_key);
                    } else {
                        pending.push(entry);
                    }
                    continue;
                }
                let realm = &matches[0];
                if let Some((parent, frame, _)) = self.frame_history_restore_claims.get(&claim_key)
                    && !self
                        .frame_realms
                        .get(&realm.realm)
                        .is_some_and(|owner| owner.parent_realm == *parent && owner.frame == *frame)
                {
                    self.frame_history_restore_claims.remove(&claim_key);
                    continue;
                }
                // Consume before native restoration may enter FACE author callbacks.
                let claimed = self
                    .frame_history_restore_claims
                    .remove(&claim_key)
                    .is_some();
                self.validate_action_context(realm)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if !claimed {
                    self.restore_frame_navigation_history(realm.realm, entry.navigation.as_ref())?;
                }
                if let Some(preparation) =
                    self.frame_script_preparations
                        .iter_mut()
                        .find(|preparation| {
                            preparation.load.realm == realm.realm && !preparation.lifecycle_queued
                        })
                {
                    preparation.restored_form_state = Some(entry.controls);
                } else {
                    self.agent
                        .host
                        .restore_form_history_in_realm(realm.realm, &entry.controls)?;
                }
            }
            if !pending.is_empty() {
                self.pending_frame_form_history = Some((identity, pending));
            }
            Ok(())
        })();
        let restored = self.agent.host.activate_realm(previous);
        result.and(restored)
    }
}

#[cfg(test)]
fn execute_face_fixture(page: &mut RustExecutableDocument, source: &str) {
    // Keep the original single synchronous execution. Collect diagnostics only
    // after its original error; do not add a poll/checkpoint or retry the script.
    if let Err(error) = page.agent.host.execute(source) {
        let reports = page.agent.host.take_module_error_reports();
        let realms = page
            .automation_realms()
            .into_iter()
            .filter(|realm| realm.live && realm.parent_realm.is_some())
            .map(|realm| {
                (
                    realm.realm,
                    realm.document_root,
                    realm.document_generation,
                    realm.stream_epoch,
                )
            })
            .collect::<Vec<_>>();
        let stages = page
            .agent
            .host
            .evaluate_string("JSON.stringify(globalThis.__faceConstructionStages??[])")
            .unwrap_or_else(|diagnostic_error| {
                format!("stage evaluation failed: {diagnostic_error}")
            });
        let snapshot = page
            .agent
            .host
            .evaluate_string(
                r#"JSON.stringify({
            url:document.URL,readyState:document.readyState,
            documentRegistry:document.customElementRegistry===customElements,
            definitions:['x-history','x-state'].map(name=>[name,typeof customElements.get(name)]),
            candidates:Array.from(document.querySelectorAll('x-history,x-state'),element=>({
                name:element.localName,internals:typeof element.i,
                registry:element.customElementRegistry===customElements,
                ownerDocument:element.ownerDocument===document,defined:element.matches(':defined')
            }))
        })"#,
            )
            .unwrap_or_else(|diagnostic_error| {
                format!("diagnostic evaluation failed: {diagnostic_error}")
            });
        panic!(
            "original FACE fixture execution failed: {error}; native author error reports: {reports:?}; live child realm/document/generation/epoch: {realms:?}; construction stages: {stages}; candidate snapshot: {snapshot}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HTML: &str = r#"<iframe id=child name=child srcdoc='<form id=f><x-history name=value></x-history></form>'></iframe>"#;
    const DEFINE: &str = "globalThis.__faceConstructionStages=[];globalThis.restored=[];customElements.define('x-history',class extends HTMLElement{static formAssociated=true;constructor(){__faceConstructionStages.push('constructor-enter');super();__faceConstructionStages.push('super-return');this.i=this.attachInternals();__faceConstructionStages.push('internals-return')}formStateRestoreCallback(value,mode){restored.push(value+':'+mode)}})";
    fn child(page: &RustExecutableDocument) -> RealmId {
        page.automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm
    }
    #[test]
    fn child_history_waits_for_definition_and_delivers_only_once() {
        let mut source = RustExecutableDocument::load(HTML).unwrap();
        let realm = child(&source);
        source.agent.host.activate_realm(realm).unwrap();
        execute_face_fixture(&mut source, DEFINE);
        execute_face_fixture(
            &mut source,
            "document.querySelector('x-history').i.setFormValue('submit','child private state')",
        );
        let saved = source.capture_child_form_history().unwrap();
        assert_eq!(saved.len(), 1);
        let mut destination = RustExecutableDocument::load(HTML).unwrap();
        destination.restore_child_form_history(&saved).unwrap();
        let realm = child(&destination);
        destination.agent.host.activate_realm(realm).unwrap();
        execute_face_fixture(&mut destination, DEFINE);
        destination
            .agent
            .host
            .run_custom_element_reactions()
            .unwrap();
        destination.agent.host.execute("if(restored.join('|')!=='child private state:restore')throw Error('child restoration')").unwrap();
        destination.checkpoint().unwrap();
        destination.agent.host.activate_realm(realm).unwrap();
        destination
            .agent
            .host
            .execute("if(restored.length!==1)throw Error('repeated restoration')")
            .unwrap();
    }
    #[test]
    fn mismatched_frame_url_keeps_private_state_undelivered() {
        let mut page = RustExecutableDocument::load(HTML).unwrap();
        let realm = child(&page);
        let path = page.frame_form_path(realm).unwrap();
        let saved = SavedFrameFormState {
            navigation: None,
            path,
            url: "https://foreign.example/".into(),
            controls: Vec::new(),
        };
        page.restore_child_form_history(&[saved]).unwrap();
        assert_eq!(page.pending_frame_form_history.as_ref().unwrap().1.len(), 1);
    }
}

#[cfg(test)]
mod builtin_and_navigation_restore_tests {
    use super::*;
    // This Navigation entry disposal test needs a tuple origin. The unbased convenience
    // loader creates an opaque document whose Navigation entries are disabled.
    fn navigation_page(html: &str) -> RustExecutableDocument {
        RustExecutableDocument::load_with_environment(
            html,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://history.test/root").unwrap()),
        )
        .unwrap()
    }
    #[test]
    fn joint_history_parent_and_child_steps_share_length_and_traverse_together() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>child</p>'></iframe>",
        )
        .unwrap();
        page.execute("globalThis.initialLength=history.length;location.hash='#parent';document.getElementById('f').contentWindow.location.hash='#child';globalThis.lengthAfter=history.length;globalThis.pops=0;onpopstate=()=>pops++;document.getElementById('f').contentWindow.onpopstate=()=>parent.pops++").unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(lengthAfter===initialLength+2)")
                .unwrap(),
            "true"
        );
        page.agent.host.execute("history.go(-2)").unwrap();
        let task = page.take_history_traversal().unwrap().unwrap();
        page.run_history_task(task, None).unwrap();
        page.checkpoint().unwrap();
        assert_eq!(page.agent.host.evaluate_string("String(location.hash===''&&document.getElementById('f').contentWindow.location.hash===''&&pops===2)").unwrap(),"true");
    }
    #[test]
    fn consecutive_srcdoc_replaces_but_fragment_then_new_source_pushes_and_restores() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>one</p>'></iframe>",
        )
        .unwrap();
        page.execute("globalThis.initialLength=history.length;document.getElementById('f').srcdoc='<p>two</p>'").unwrap();
        for _ in 0..64 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("String(history.length===initialLength&&document.getElementById('f').contentDocument.querySelector('p').textContent==='two')").unwrap(),"true");
        page.execute("document.getElementById('f').contentWindow.location.hash='saved';document.getElementById('f').srcdoc='<p>three</p>'").unwrap();
        for _ in 0..64 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(history.length===initialLength+2)")
                .unwrap(),
            "true"
        );
        let receipt = page.begin_joint_history_delta(-1).unwrap().unwrap();
        for _ in 0..64 {
            page.drain_tasks().unwrap();
            page.checkpoint().unwrap();
            if receipt.poll().is_some() {
                break;
            }
        }
        assert_eq!(receipt.poll(), Some(Ok(())));
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("String(document.getElementById('f').contentWindow.location.href==='about:srcdoc#saved'&&document.getElementById('f').contentDocument.querySelector('p').textContent==='two')").unwrap(),"true");
    }
    #[test]
    fn frame_history_restores_cross_origin_name_but_keeps_same_origin_latest_name() {
        for cross_origin in [false, true] {
            let next = if cross_origin {
                "https://b.test/second"
            } else {
                "https://a.test/second"
            };
            let resources = [
                (
                    "https://a.test/first".into(),
                    "<!doctype html><p>first</p>".into(),
                ),
                (next.into(), "<!doctype html><p>second</p>".into()),
            ]
            .into_iter()
            .collect();
            let mut page = RustExecutableDocument::load_with_resources(
                "<!doctype html><iframe id=f src='https://a.test/first'></iframe>",
                &resources,
            )
            .unwrap();
            for _ in 0..64 {
                page.drain_tasks().unwrap();
                if !page.frame_scripts_pending(None) {
                    break;
                }
            }
            let realm = page
                .automation_realms()
                .into_iter()
                .find(|realm| realm.parent_realm.is_some() && realm.live)
                .unwrap()
                .realm;
            page.agent.host.activate_realm(realm).unwrap();
            page.agent.host.execute("window.name='original'").unwrap();
            page.agent.host.activate_realm(page.top_realm).unwrap();
            page.execute(&format!(
                "document.getElementById('f').src={}",
                serde_json::to_string(next).unwrap()
            ))
            .unwrap();
            for _ in 0..64 {
                page.drain_tasks().unwrap();
                if !page.frame_scripts_pending(None) {
                    break;
                }
            }
            let realm = page
                .automation_realms()
                .into_iter()
                .find(|realm| realm.parent_realm.is_some() && realm.live)
                .unwrap()
                .realm;
            assert_eq!(
                page.agent.host.native_window_target_name(realm).as_deref(),
                Some("original")
            );
            page.agent.host.activate_realm(realm).unwrap();
            page.agent.host.execute("window.name='latest'").unwrap();
            let receipt = page.begin_joint_history_delta(-1).unwrap().unwrap();
            for _ in 0..64 {
                page.drain_tasks().unwrap();
                page.checkpoint().unwrap();
                if receipt.poll().is_some() {
                    break;
                }
            }
            assert_eq!(receipt.poll(), Some(Ok(())));
            let realm = page
                .automation_realms()
                .into_iter()
                .find(|realm| realm.parent_realm.is_some() && realm.live)
                .unwrap()
                .realm;
            assert_eq!(
                page.agent.host.native_window_target_name(realm).as_deref(),
                Some(if cross_origin { "original" } else { "latest" })
            );
        }
    }
    #[test]
    fn ancestor_frame_restore_recovers_native_name_from_saved_document_descriptor() {
        let html = "<!doctype html><iframe id=f srcdoc='<p>child</p>'></iframe>";
        let mut source = RustExecutableDocument::load(html).unwrap();
        let realm = source
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        source.agent.host.activate_realm(realm).unwrap();
        source
            .agent
            .host
            .execute("window.name='saved-child'")
            .unwrap();
        let saved = source.capture_child_form_history().unwrap();
        assert!(
            saved[0]
                .navigation
                .as_ref()
                .unwrap()
                .saved_names
                .iter()
                .any(|(_, name)| name == "saved-child")
        );
        let mut restored = RustExecutableDocument::load(html).unwrap();
        restored.restore_child_form_history(&saved).unwrap();
        for _ in 0..32 {
            restored.drain_tasks().unwrap();
            if !restored.frame_scripts_pending(None) {
                break;
            }
        }
        let realm = restored
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        assert_eq!(
            restored
                .agent
                .host
                .native_window_target_name(realm)
                .as_deref(),
            Some("saved-child")
        );
    }
    #[test]
    fn native_ui_joint_delta_uses_same_parent_child_plan_and_completion() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><iframe id=f srcdoc='<p>child</p>'></iframe>",
        )
        .unwrap();
        page.execute(
            "location.hash='one';document.getElementById('f').contentWindow.location.hash='two'",
        )
        .unwrap();
        let receipt = page.begin_joint_history_delta(-2).unwrap().unwrap();
        page.checkpoint().unwrap();
        assert_eq!(receipt.poll(), Some(Ok(())));
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string("String(location.hash===''&&document.getElementById('f').contentWindow.location.hash==='')").unwrap(),"true");
    }
    #[test]
    fn parent_push_disposes_child_forward_entries_and_updates_shared_length() {
        let mut page =
            navigation_page("<!doctype html><iframe id=f srcdoc='<p>child</p>'></iframe>");
        page.execute("globalThis.initialLength=history.length;const child=document.getElementById('f').contentWindow;child.location.hash='future';globalThis.disposed=0;child.navigation.currentEntry.addEventListener('dispose',()=>disposed++)").unwrap();
        let receipt = page.begin_joint_history_delta(-1).unwrap().unwrap();
        page.checkpoint().unwrap();
        assert_eq!(receipt.poll(), Some(Ok(())));
        page.execute("history.pushState(null,'','#replacement')")
            .unwrap();
        page.checkpoint().unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(disposed===1&&history.length===initialLength+1)")
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn admitted_form_restore_resumes_without_readmitting_completed_callbacks() {
        let mut page =
            RustExecutableDocument::load("<form><x-state name=field></x-state></form>").unwrap();
        page.execute("globalThis.restored=[];customElements.define('x-state',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('value','saved')}formStateRestoreCallback(value){restored.push(value)}})").unwrap();
        let saved = page.capture_form_history().unwrap();
        let mut admitted = false;
        page.agent
            .host
            .resume_form_history_in_realm(page.top_realm, &saved, &mut admitted)
            .unwrap();
        assert!(admitted);
        page.agent
            .host
            .resume_form_history_in_realm(page.top_realm, &saved, &mut admitted)
            .unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("restored.join(',')")
                .unwrap(),
            "saved"
        );
    }
    #[test]
    fn native_builtin_form_history_restores_without_input_or_change_events() {
        let html = "<form><input name=text value=default><input name=check type=checkbox><textarea name=area>default</textarea><select name=choice><option>A</option><option>B</option></select><input name=skip autocomplete=off value=default></form>";
        let mut source = RustExecutableDocument::load(html).unwrap();
        source.execute("document.querySelector('[name=text]').value='saved';document.querySelector('[name=text]').setSelectionRange(1,3,'backward');document.querySelector('[name=check]').checked=true;document.querySelector('[name=area]').value='lines';document.querySelector('select').selectedIndex=1;document.querySelector('[name=skip]').value='private'").unwrap();
        let saved = source.capture_form_history().unwrap();
        let mut target = RustExecutableDocument::load(html).unwrap();
        target.execute("globalThis.events=[];document.addEventListener('input',()=>events.push('input'));document.addEventListener('change',()=>events.push('change'))").unwrap();
        target.restore_form_history(&saved).unwrap();
        assert_eq!(target.agent.host.evaluate_string("String(document.querySelector('[name=text]').value==='saved'&&document.querySelector('[name=text]').selectionStart===1&&document.querySelector('[name=text]').selectionEnd===3&&document.querySelector('[name=text]').selectionDirection==='backward'&&document.querySelector('[name=check]').checked&&document.querySelector('[name=area]').value==='lines'&&document.querySelector('select').selectedIndex===1&&document.querySelector('[name=skip]').value==='default'&&events.length===0)").unwrap(),"true");
    }
    #[test]
    fn reserved_frame_history_replaces_srcdoc_source_before_first_child_script() {
        let html = r#"<script>globalThis.events=[]</script><iframe id=f srcdoc="<script>parent.events.push('original')</script>"></iframe>"#;
        let mut source = RustExecutableDocument::load(html).unwrap();
        source.execute("document.getElementById('f').srcdoc=\"<script>parent.events.push('saved')<\\/script>\"").unwrap();
        for _ in 0..32 {
            source.drain_tasks().unwrap();
            if !source.frame_scripts_pending(None) {
                break;
            }
        }
        let saved = source.capture_child_form_history().unwrap();
        let resources = BTreeMap::from([(
            "oryn:frame-history".into(),
            serde_json::to_string(&saved).unwrap(),
        )]);
        let mut restored = RustExecutableDocument::load_with_resources(html, &resources).unwrap();
        for _ in 0..32 {
            restored.drain_tasks().unwrap();
            if !restored.frame_scripts_pending(None) {
                break;
            }
        }
        restored
            .agent
            .host
            .activate_realm(restored.top_realm)
            .unwrap();
        assert_eq!(
            restored
                .agent
                .host
                .evaluate_string("events.join('|')")
                .unwrap(),
            "saved"
        );
        let current = restored.capture_child_form_history().unwrap();
        let before = saved[0].navigation.as_ref().unwrap();
        let after = current[0].navigation.as_ref().unwrap();
        assert_eq!(
            before.history.entries[before.history.current].id,
            after.history.entries[after.history.current].id
        );
        assert_eq!(
            before.history.entries[before.history.current].document_id,
            after.history.entries[after.history.current].document_id
        );
    }
}

#[cfg(test)]
mod restoration_claim_tests {
    use super::*;
    #[test]
    fn removed_and_reinserted_frame_cannot_replay_claimed_history() {
        let mut page = RustExecutableDocument::load(
            "<iframe id=f srcdoc='<input name=value value=default>'></iframe>",
        )
        .unwrap();
        let realm = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        page.agent.host.activate_realm(realm).unwrap();
        page.agent
            .host
            .execute("document.querySelector('input').value='private'")
            .unwrap();
        let saved = page.capture_child_form_history().unwrap();
        let owner = page.frame_realms.get(&realm).unwrap().clone();
        let dom = page.agent.host.automation_dom(owner.parent_realm).unwrap();
        let token = dom.borrow_mut().frame_navigation_identity(owner.frame);
        page.frame_history_restore_claims.insert(
            frame_history_path_key(&saved[0].path).unwrap(),
            (owner.parent_realm, owner.frame, token),
        );
        page.pending_frame_form_history = Some((page.form_history_root_identity().unwrap(), saved));
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent
            .host
            .execute(
                "globalThis.f=document.getElementById('f');f.remove();document.body.appendChild(f)",
            )
            .unwrap();
        page.deliver_pending_frame_form_history().unwrap();
        assert!(page.pending_frame_form_history.is_none());
        assert!(page.frame_history_restore_claims.is_empty());
    }
    #[test]
    fn redirected_claim_is_terminal_instead_of_waiting_for_future_url_match() {
        let mut page =
            RustExecutableDocument::load("<iframe id=f srcdoc='<input name=value>'></iframe>")
                .unwrap();
        let mut saved = page.capture_child_form_history().unwrap();
        let realm = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        let owner = page.frame_realms.get(&realm).unwrap().clone();
        let dom = page.agent.host.automation_dom(owner.parent_realm).unwrap();
        let token = dom.borrow_mut().frame_navigation_identity(owner.frame);
        saved[0].url = "https://original.test/redirected".into();
        page.frame_history_restore_claims.insert(
            frame_history_path_key(&saved[0].path).unwrap(),
            (owner.parent_realm, owner.frame, token),
        );
        page.pending_frame_form_history = Some((page.form_history_root_identity().unwrap(), saved));
        page.deliver_pending_frame_form_history().unwrap();
        assert!(page.pending_frame_form_history.is_none());
        assert!(page.frame_history_restore_claims.is_empty());
    }
    #[test]
    fn stopped_prior_document_cannot_consume_same_url_restoration() {
        let mut page =
            RustExecutableDocument::load("<iframe id=f srcdoc='<input name=value>'></iframe>")
                .unwrap();
        let saved = page.capture_child_form_history().unwrap();
        let realm = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        let owner = page.frame_realms.get(&realm).unwrap().clone();
        let dom = page.agent.host.automation_dom(owner.parent_realm).unwrap();
        let token = dom.borrow_mut().frame_navigation_identity(owner.frame);
        page.frame_history_restore_claims.insert(
            frame_history_path_key(&saved[0].path).unwrap(),
            (owner.parent_realm, owner.frame, token),
        );
        page.pending_frame_form_history = Some((page.form_history_root_identity().unwrap(), saved));
        page.agent.host.stop_pending_document_load(realm).unwrap();
        page.deliver_pending_frame_form_history().unwrap();
        assert!(page.pending_frame_form_history.is_some());
        assert_eq!(page.frame_history_restore_claims.len(), 1);
    }
}

#[cfg(test)]
mod nested_reentrant_restore_tests {
    use super::*;
    #[test]
    fn nested_saved_source_rebinds_only_after_native_parent_restoration() {
        let html = r#"<script>globalThis.events=[]</script><iframe id=outer srcdoc="<iframe id=inner srcdoc='<script>top.events.push(1)</script>'></iframe>"></iframe>"#;
        let mut source = RustExecutableDocument::load(html).unwrap();
        let replacement = "<iframe id=inner srcdoc='<script>top.events.push(2)</script>'></iframe>";
        source
            .execute(&format!(
                "document.getElementById('outer').srcdoc={}",
                serde_json::to_string(replacement).unwrap()
            ))
            .unwrap();
        for _ in 0..64 {
            source.drain_tasks().unwrap();
            if !source.frame_scripts_pending(None) {
                break;
            }
        }
        let saved = source.capture_child_form_history().unwrap();
        assert_eq!(saved.len(), 2);
        let resources = BTreeMap::from([(
            "oryn:frame-history".into(),
            serde_json::to_string(&saved).unwrap(),
        )]);
        let mut restored = RustExecutableDocument::load_with_resources(html, &resources).unwrap();
        for _ in 0..64 {
            restored.drain_tasks().unwrap();
            if !restored.frame_scripts_pending(None) {
                break;
            }
        }
        restored
            .agent
            .host
            .activate_realm(restored.top_realm)
            .unwrap();
        assert_eq!(
            restored
                .agent
                .host
                .evaluate_string("events.join(',')")
                .unwrap(),
            "2"
        );
        assert!(restored.pending_frame_navigation_restore.is_empty());
    }
    #[test]
    fn face_restore_removing_its_frame_cannot_deliver_following_old_callbacks() {
        let html =
            "<iframe id=f srcdoc='<x-state name=a></x-state><x-state name=b></x-state>'></iframe>";
        let mut source = RustExecutableDocument::load(html).unwrap();
        let realm = source
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        source.agent.host.activate_realm(realm).unwrap();
        execute_face_fixture(
            &mut source,
            "globalThis.__faceConstructionStages=[];customElements.define('x-state',class extends HTMLElement{static formAssociated=true;constructor(){__faceConstructionStages.push('constructor-enter');super();__faceConstructionStages.push('super-return');this.i=this.attachInternals();__faceConstructionStages.push('internals-return')}});document.querySelectorAll('x-state').forEach(e=>e.i.setFormValue('value','private'))",
        );
        let saved = source.capture_child_form_history().unwrap();
        let mut destination = RustExecutableDocument::load(html).unwrap();
        destination
            .agent
            .host
            .execute("globalThis.restored=[]")
            .unwrap();
        let realm = destination
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap()
            .realm;
        destination.agent.host.activate_realm(realm).unwrap();
        execute_face_fixture(
            &mut destination,
            "globalThis.__faceConstructionStages=[];customElements.define('x-state',class extends HTMLElement{static formAssociated=true;constructor(){__faceConstructionStages.push('constructor-enter');super();__faceConstructionStages.push('super-return');this.i=this.attachInternals();__faceConstructionStages.push('internals-return')}formStateRestoreCallback(value){parent.restored.push(value);frameElement.remove()}})",
        );
        destination.restore_child_form_history(&saved).unwrap();
        destination.checkpoint().unwrap();
        destination
            .agent
            .host
            .activate_realm(destination.top_realm)
            .unwrap();
        assert_eq!(
            destination
                .agent
                .host
                .evaluate_string("restored.join(',')")
                .unwrap(),
            "private"
        );
    }
}

#[cfg(test)]
mod nested_cross_origin_history_tests {
    use super::*;
    fn settle(page: &mut RustExecutableDocument) {
        for _ in 0..128 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) && page.pending_frame_navigation_restore.is_empty()
            {
                return;
            }
        }
        panic!("nested history restoration did not settle");
    }
    fn child(page: &RustExecutableDocument, parent: RealmId) -> RealmId {
        page.automation_realms()
            .into_iter()
            .find(|realm| realm.live && realm.parent_realm == Some(parent))
            .unwrap()
            .realm
    }
    #[test]
    fn cross_origin_nested_network_history_restores_parent_selection_and_each_documents_controls() {
        let html = "<iframe id=outer src='https://outer.test/first'></iframe>";
        let mut resources=BTreeMap::from([
            ("https://outer.test/first".into(),"<input name=outer value=first><iframe id=inner src='https://inner.test/first'></iframe>".into()),
            ("https://outer.test/second".into(),"<input name=outer value=second><iframe id=inner src='https://inner.test/second'></iframe>".into()),
            ("https://inner.test/first".into(),"<input name=inner value=first>".into()),
            ("https://inner.test/second".into(),"<input name=inner value=second>".into()),
        ]);
        let base = Url::parse("https://root.test/").unwrap();
        let mut source = RustExecutableDocument::load_with_environment(
            html,
            &resources,
            None,
            Some(base.clone()),
        )
        .unwrap();
        settle(&mut source);
        let outer = child(&source, source.top_realm);
        source.agent.host.activate_realm(outer).unwrap();
        source
            .agent
            .host
            .execute("location.href='https://outer.test/second'")
            .unwrap();
        source.drain_frame_location_navigations().unwrap();
        settle(&mut source);
        let outer = child(&source, source.top_realm);
        let inner = child(&source, outer);
        source.agent.host.activate_realm(outer).unwrap();
        source.agent.host.execute("document.querySelector('input').value='outer saved';window.name='outer saved name'").unwrap();
        source.agent.host.activate_realm(inner).unwrap();
        source.agent.host.execute("document.querySelector('input').value='inner saved';window.name='inner saved name'").unwrap();
        source.agent.host.activate_realm(source.top_realm).unwrap();
        let saved = source.capture_child_form_history().unwrap();
        assert_eq!(saved.len(), 2);
        resources.insert(
            "oryn:frame-history".into(),
            serde_json::to_string(&saved).unwrap(),
        );
        let mut restored =
            RustExecutableDocument::load_with_environment(html, &resources, None, Some(base))
                .unwrap();
        settle(&mut restored);
        let outer = child(&restored, restored.top_realm);
        let inner = child(&restored, outer);
        for (realm, url, value, name) in [
            (
                outer,
                "https://outer.test/second",
                "outer saved",
                "outer saved name",
            ),
            (
                inner,
                "https://inner.test/second",
                "inner saved",
                "inner saved name",
            ),
        ] {
            restored.agent.host.activate_realm(realm).unwrap();
            assert_eq!(restored.agent.host.location_url().unwrap(), url);
            assert_eq!(
                restored
                    .agent
                    .host
                    .evaluate_string("document.querySelector('input').value")
                    .unwrap(),
                value
            );
            assert_eq!(
                restored
                    .agent
                    .host
                    .native_window_target_name(realm)
                    .as_deref(),
                Some(name)
            );
            assert_eq!(restored.agent.host.evaluate_string("String((()=>{try{void top.document;return false}catch(error){return error.name==='SecurityError'}})())").unwrap(),"true");
        }
        assert!(restored.pending_frame_navigation_restore.is_empty());
    }
}
