//! Retained editor/editor and control/editor move, sharing a compound undo.
use super::*;

#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    BeforeDelete,
    SelectDestination,
    BeforeInsert,
    Commit,
    InputDelete,
    InputInsert,
    Done,
}

pub(super) struct RichMove {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    source: SelectedSource,
    target: NodeId,
    store: Store,
    caret: BoundaryPoint,
    text: String,
    phase: Phase,
    source_owner: Option<NodeId>,
    target_owner: Option<NodeId>,
    target_control: bool,
    before_target: Option<String>,
    next_target: Option<String>,
    target_cursor: Option<oryn_dom::MutationCursor>,
    data: Option<crate::clipboard::ClipboardData>,
    html: Option<String>,
    destination: Option<oryn_dom::LiveRangeId>,
    event: Option<clipboard_actions::ClipboardNotification>,
    edit: Option<retained_editing::Compound>,
    result: bool,
}
impl RichMove {
    pub(super) fn set_input_owner(&mut self, state: &Rc<V8BindingState>) {
        self.input_owner = native_pointer::InputOwner::new(state);
    }
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        source: &SelectedSource,
        target: NodeId,
        store: &Store,
        caret: BoundaryPoint,
        text: &str,
    ) -> Self {
        let mut store = store.clone();
        store.set_mode(Mode::ReadOnly);
        Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            source: source.clone(),
            target,
            store,
            caret,
            text: text.into(),
            phase: Phase::Prepare,
            source_owner: None,
            target_owner: None,
            target_control: false,
            before_target: None,
            next_target: None,
            target_cursor: None,
            data: None,
            html: None,
            destination: None,
            event: None,
            edit: None,
            result: false,
        }
    }
    fn live(&self) -> bool {
        let state = &self.owner;
        self.input_owner.current()
            && !state.execution_retired()
            && !state.execution_tasks_stopped()
            && (
                self.source.document,
                self.source.generation,
                self.source.epoch,
                self.source.authority,
            ) == (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            )
            && self
                .source_owner
                .into_iter()
                .chain(self.target_owner)
                .all(|node| {
                    let dom = state.dom.borrow();
                    dom.is_document_connected(node).unwrap_or(false)
                        && dom.arena().node_document(node).ok().flatten()
                            == Some(self.source.document)
                })
    }
    fn set_source(&self) -> Result<(), V8BindingError> {
        match &self.source.kind {
            SelectedKind::Control {
                node, start, end, ..
            } => {
                self.owner
                    .text_control_selections
                    .borrow_mut()
                    .insert(*node, (*start, *end, "none".into()));
                Ok(())
            }
            SelectedKind::Range { range, .. } => set_selection_boundaries(
                &self.owner,
                BoundaryPoint {
                    container: range.start_container,
                    offset: range.start_offset,
                },
                BoundaryPoint {
                    container: range.end_container,
                    offset: range.end_offset,
                },
            )
            .map(|_| ()),
        }
    }
    fn unchanged(&self) -> Result<bool, V8BindingError> {
        let state = &self.owner;
        if !self.source.move_target(state, self.target)
            || default_effect(state, self.target, &self.store).is_none()
        {
            return Ok(false);
        }
        if self.target_control != state.dom.borrow().is_text_control(self.target) {
            return Ok(false);
        }
        if let Some(value) = &self.before_target {
            let dom = state.dom.borrow();
            let Some(cursor) = self.target_cursor else {
                return Ok(false);
            };
            let batch = dom.mutations_since(cursor);
            return Ok(dom.control_value(self.target)?==*value&&!batch.truncated&&!batch.mutations.iter().any(|mutation|matches!(mutation,
                oryn_dom::Mutation::FormState{node}|oryn_dom::Mutation::Attribute{node,..}|oryn_dom::Mutation::DocumentOwnership{node,..}|oryn_dom::Mutation::Removed{node,..}if *node==self.target)));
        }
        Ok(self
            .target_owner
            .is_some_and(|host| caret_owned_by(state, self.caret, host)))
    }
    fn release_destination(&mut self) {
        if let Some(range) = self.destination.take() {
            let _ = self.owner.dom.borrow_mut().drop_live_range(range);
        }
    }
    pub(super) fn cancel(&mut self) {
        self.event.take();
        if let Some(edit) = &mut self.edit {
            edit.cancel();
        }
        self.release_destination();
        self.phase = Phase::Done;
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(Some(self.result));
            }
            if !self.live() {
                self.cancel();
                continue;
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                Phase::Prepare => {
                    if !self.source.move_target(&state, self.target)
                        || default_effect(&state, self.target, &self.store).is_none()
                    {
                        self.cancel();
                        continue;
                    }
                    self.source_owner = match &self.source.kind {
                        SelectedKind::Control { node, .. } => Some(*node),
                        SelectedKind::Range { host, .. } => *host,
                    };
                    if self.source_owner.is_none() {
                        self.cancel();
                        continue;
                    }
                    self.target_control = state.dom.borrow().is_text_control(self.target);
                    self.target_owner = if self.target_control {
                        Some(self.target)
                    } else {
                        editing_host_for_node(
                            &state.dom.borrow(),
                            Some(self.target),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )?
                    };
                    if self.target_owner.is_none() {
                        self.cancel();
                        continue;
                    }
                    if let SelectedKind::Range { range, .. } = &self.source.kind
                        && !self.target_control
                    {
                        let dom = state.dom.borrow();
                        let inside = compare_boundary_points(
                            &dom,
                            (self.caret.container, self.caret.offset),
                            (range.start_container, range.start_offset),
                        )? >= 0
                            && compare_boundary_points(
                                &dom,
                                (self.caret.container, self.caret.offset),
                                (range.end_container, range.end_offset),
                            )? <= 0;
                        drop(dom);
                        if inside {
                            self.cancel();
                            continue;
                        }
                    }
                    if self.target_control {
                        let value = state.dom.borrow().control_value(self.target)?;
                        let mut units = oryn_dom::dom_string::units(&value);
                        if self.caret.offset as usize > units.len() {
                            self.cancel();
                            continue;
                        }
                        units.splice(
                            self.caret.offset as usize..self.caret.offset as usize,
                            oryn_dom::dom_string::units(&self.text),
                        );
                        if state
                            .dom
                            .borrow()
                            .control_length_limit(self.target, "maxlength")?
                            .is_some_and(|max| units.len() > max)
                        {
                            self.cancel();
                            continue;
                        }
                        self.before_target = Some(value);
                        self.next_target = Some(oryn_dom::dom_string::from_units(&units));
                        self.target_cursor = Some(state.dom.borrow().mutation_cursor());
                    }
                    self.data = Some(clipboard_actions::to_data(&self.store)?);
                    self.html = if !self.target_control
                        && self.store.types().iter().any(|kind| kind == "text/html")
                        && !host_is_plaintext_only(&state.dom.borrow(), self.target_owner.unwrap())
                    {
                        Some(clipboard_actions::sanitized_html_for_state(
                            &state,
                            &oryn_dom::dom_string::from_units(&self.store.get_data("text/html")),
                        )?)
                    } else {
                        None
                    };
                    self.set_source()?;
                    self.phase = Phase::BeforeDelete;
                }
                Phase::BeforeDelete
                | Phase::BeforeInsert
                | Phase::InputDelete
                | Phase::InputInsert => {
                    let before = matches!(self.phase, Phase::BeforeDelete | Phase::BeforeInsert);
                    let deletion = matches!(self.phase, Phase::BeforeDelete | Phase::InputDelete);
                    if self.event.is_none() {
                        if before && !self.unchanged()? {
                            self.cancel();
                            continue;
                        }
                        self.event = Some(clipboard_actions::ClipboardNotification::editing(
                            scope,
                            &state,
                            if deletion {
                                self.source_owner.unwrap()
                            } else {
                                self.target_owner.unwrap()
                            },
                            if before { "beforeinput" } else { "input" },
                            if deletion {
                                "deleteByDrag"
                            } else {
                                "insertFromDrop"
                            },
                            (!deletion && self.target_control).then_some(self.text.as_str()),
                            if !deletion && !self.target_control {
                                self.data.as_ref()
                            } else {
                                None
                            },
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.event.take();
                    self.phase = match self.phase {
                        Phase::BeforeDelete => Phase::SelectDestination,
                        Phase::BeforeInsert => Phase::Commit,
                        Phase::InputDelete => Phase::InputInsert,
                        _ => Phase::Done,
                    };
                    if before && !accepted {
                        self.cancel();
                        continue;
                    }
                }
                Phase::SelectDestination => {
                    if !self.unchanged()? {
                        self.cancel();
                        continue;
                    }
                    if self.target_control {
                        state.text_control_selections.borrow_mut().insert(
                            self.target,
                            (self.caret.offset, self.caret.offset, "none".into()),
                        );
                    } else {
                        set_selection_boundaries(&state, self.caret, self.caret)?;
                    }
                    self.phase = Phase::BeforeInsert;
                }
                Phase::Commit => {
                    if self.edit.is_none() {
                        if !self.unchanged()? {
                            self.cancel();
                            continue;
                        }
                        self.set_source()?;
                        if !self.target_control && self.destination.is_none() {
                            self.destination = Some(
                                state
                                    .dom
                                    .borrow_mut()
                                    .create_live_range(self.caret, self.caret)?,
                            );
                        }
                        self.edit = Some(retained_editing::Compound::new(
                            &state,
                            &[self.source_owner.unwrap(), self.target_owner.unwrap()],
                        )?);
                    }
                    let source = self.source.kind.clone();
                    let target = self.target;
                    let destination = self.destination;
                    let before = self.before_target.clone();
                    let next = self.next_target.clone();
                    let text = self.text.clone();
                    let html = self.html.clone();
                    let caret = self.caret;
                    let Some(changed) =
                        self.edit
                            .as_mut()
                            .unwrap()
                            .poll_future(scope, &state, move |context| {
                                Box::pin(async move {
                                    let state: &V8BindingState = &context.state;
                                    match source {
                                        SelectedKind::Control {
                                            node, start, end, ..
                                        } => {
                                            context.control(node, start, end, "").await?;
                                            state
                                                .text_control_selections
                                                .borrow_mut()
                                                .insert(node, (start, start, "none".into()));
                                        }
                                        SelectedKind::Range { .. } => {
                                            editing_algorithms::replace_selection_with_text(
                                                &context, "",
                                            )
                                            .await?
                                        }
                                    }
                                    if let (Some(before), Some(next)) = (before, next) {
                                        if !state.dom.borrow().is_connected(target)? {
                                            return Err(V8BindingError::Allocation(
                                                "drop removed its destination",
                                            ));
                                        }
                                        context
                                            .control(
                                                target,
                                                0,
                                                oryn_dom::dom_string::len(&before) as u32,
                                                next.clone(),
                                            )
                                            .await?;
                                        if state.dom.borrow().control_value(target)? != next {
                                            return Err(V8BindingError::Allocation(
                                                "drop control normalization changed payload",
                                            ));
                                        }
                                        let position =
                                            caret
                                                .offset
                                                .saturating_add(
                                                    oryn_dom::dom_string::len(&text) as u32
                                                );
                                        state
                                            .text_control_selections
                                            .borrow_mut()
                                            .insert(target, (position, position, "none".into()));
                                    } else {
                                        let destination =
                                            state
                                                .dom
                                                .borrow()
                                                .live_range(destination.ok_or(
                                                    V8BindingError::Allocation("drop caret"),
                                                )?)?
                                                .start;
                                        set_selection_boundaries(state, destination, destination)?;
                                        match html {
                                            Some(html) => {
                                                editing_algorithms::replace_selection_with_html(
                                                    &context, &html, true,
                                                )
                                                .await?
                                            }
                                            None => {
                                                editing_algorithms::insert_editing_text(
                                                    &context, &text,
                                                )
                                                .await?
                                            }
                                        }
                                    }
                                    Ok(())
                                })
                            })?
                    else {
                        return Ok(None);
                    };
                    self.result = changed;
                    self.release_destination();
                    self.phase = if self.result {
                        Phase::InputDelete
                    } else {
                        Phase::Done
                    };
                }
                Phase::Done => return Ok(Some(self.result)),
            }
        }
    }
}
impl Drop for RichMove {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn interrupted_rich_and_mixed_moves_keep_both_values_and_shared_undo() {
        super::super::super::tests::initialize_v8();
        for (source_control, target_control) in [(true, false), (false, true), (false, false)] {
            let source_html = if source_control {
                "<input id=a value=abcd>"
            } else {
                "<div id=a contenteditable>abcd</div>"
            };
            let target_html = if target_control {
                "<input id=b value=xy>"
            } else {
                "<div id=b contenteditable>xy</div>"
            };
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(&format!("{source_html}{target_html}"))
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(231), 1).unwrap();
            let a = state.dom.borrow().find_by_id("a").unwrap();
            let b = state.dom.borrow().find_by_id("b").unwrap();
            let (kind, watched) = if source_control {
                (
                    SelectedKind::Control {
                        node: a,
                        start: 1,
                        end: 3,
                        value: "abcd".into(),
                    },
                    BTreeSet::from([a]),
                )
            } else {
                let text = state.dom.borrow().children(a).unwrap()[0];
                (
                    SelectedKind::Range {
                        range: RangeRecord {
                            start_container: text,
                            start_offset: 1,
                            end_container: text,
                            end_offset: 3,
                        },
                        host: Some(a),
                    },
                    BTreeSet::from([a, text]),
                )
            };
            let source = SelectedSource {
                kind,
                document: state.document_root(),
                generation: state.document_generation(),
                epoch: state.document_stream_epoch(),
                authority: state.document_authority_epoch(),
                cursor: state.dom.borrow().mutation_cursor(),
                watched,
            };
            let caret = BoundaryPoint {
                container: if target_control {
                    b
                } else {
                    state.dom.borrow().children(b).unwrap()[0]
                },
                offset: 1,
            };
            let mut store = Store::default();
            store.set_data("text/plain", "bc".encode_utf16().collect());
            eval(
                scope,
                "globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');globalThis.trace=[];for(const n of [a,b])for(const k of ['beforeinput','input'])n.addEventListener(k,e=>trace.push(n.id+':'+k+':'+e.inputType));",
            );
            let mut job = RichMove::new(&state, &source, b, &store, caret, "bc");
            for _ in 0..4 {
                native_platform::publication::fail_after_next_callback();
                assert!(job.poll(scope).is_err());
                state.set_execution_suspended(true);
                assert_eq!(job.poll(scope).unwrap(), None);
                state.set_execution_suspended(false);
            }
            assert_eq!(job.poll(scope).unwrap(), Some(true));
            assert_eq!(job.poll(scope).unwrap(), Some(true));
            assert!(job.destination.is_none());
            eval(
                scope,
                "if((a.value??a.textContent)!=='ad'||(b.value??b.textContent)!=='xbcy'||trace.join(',')!=='a:beforeinput:deleteByDrag,b:beforeinput:insertFromDrop,a:input:deleteByDrag,b:input:insertFromDrop')throw Error('rich move replayed');",
            );
            if source_control {
                assert!(native_keyboard::undo_control(&state, a, false).unwrap());
            } else if target_control {
                assert!(native_keyboard::undo_control(&state, b, false).unwrap());
            } else {
                assert!(undo_edit(&state, false).unwrap());
            }
            eval(
                scope,
                "if((a.value??a.textContent)!=='abcd'||(b.value??b.textContent)!=='xy')throw Error('mixed undo split');",
            );
            assert!(!undo_edit(&state, false).unwrap());
        }
    }
    #[test]
    fn same_bytes_destination_replacement_does_not_authorize_source_deletion() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=a contenteditable>abcd</div><input id=b value=xy>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(232), 1).unwrap();
        let a = state.dom.borrow().find_by_id("a").unwrap();
        let b = state.dom.borrow().find_by_id("b").unwrap();
        let text = state.dom.borrow().children(a).unwrap()[0];
        let source = SelectedSource {
            kind: SelectedKind::Range {
                range: RangeRecord {
                    start_container: text,
                    start_offset: 1,
                    end_container: text,
                    end_offset: 3,
                },
                host: Some(a),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([a, text]),
        };
        let mut store = Store::default();
        store.set_data("text/plain", "bc".encode_utf16().collect());
        eval(
            scope,
            "globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');b.addEventListener('beforeinput',()=>{b.value='replacement';b.value='xy'});",
        );
        let mut job = RichMove::new(
            &state,
            &source,
            b,
            &store,
            BoundaryPoint {
                container: b,
                offset: 1,
            },
            "bc",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert_eq!(job.poll(scope).unwrap(), Some(false));
        eval(
            scope,
            "if(a.textContent!=='abcd'||b.value!=='xy')throw Error('replacement target accepted stale move');",
        );
    }
}
