//! Declarative CSS module placeholders retain identity, not list positions.
use super::*;

pub enum DeclarativeStylesheetResult<'s> {
    Invalid,
    Pending {
        url: String,
    },
    Ready {
        url: String,
        sheet: v8::Local<'s, v8::Value>,
    },
    Failed {
        url: String,
    },
}
pub struct NativeDeclarativeStylesheetHook(
    pub  for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        &Rc<V8BindingState>,
        &str,
    ) -> Result<DeclarativeStylesheetResult<'s>, V8BindingError>,
);
#[derive(Default)]
struct Registry {
    document: Option<NodeId>,
    epoch: u64,
    initialized: BTreeMap<NodeId, usize>,
    pending: Vec<Subscription>,
}
struct Subscription {
    root: NodeId,
    placeholder: NodeId,
    url: String,
    publication_pending: bool,
}
fn registry(scope: &mut v8::PinScope, state: &V8BindingState) -> Rc<RefCell<Registry>> {
    let context = scope.get_current_context();
    let registry = if let Some(registry) = context.get_slot::<RefCell<Registry>>() {
        registry
    } else {
        let registry = Rc::new(RefCell::new(Registry::default()));
        context.set_slot(registry.clone());
        registry
    };
    {
        let mut records = registry.borrow_mut();
        if records.document != Some(state.document_root())
            || records.epoch != state.document_authority_epoch()
        {
            *records = Registry {
                document: Some(state.document_root()),
                epoch: state.document_authority_epoch(),
                ..Default::default()
            };
        }
    }
    registry
}
fn specifiers(value: &str) -> Vec<String> {
    let units = oryn_dom::dom_string::units(value);
    units
        .split(|unit| matches!(*unit, 9 | 10 | 12 | 13 | 32))
        .filter(|part| !part.is_empty())
        .map(oryn_dom::dom_string::from_units)
        .collect()
}
pub(super) fn prepare_root(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let context = state
        .global_object
        .to_local(scope)
        .and_then(|global| global.get_creation_context(scope))
        .ok_or(V8BindingError::Allocation(
            "declarative stylesheet owner realm",
        ))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    prepare_root_in_context(scope, state, root)
}
fn prepare_root_in_context(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    let value = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(root)?;
        if !matches!(record.kind, NodeKind::ShadowRoot { .. })
            || dom.node_document(root)? != Some(state.document_root())
        {
            return Ok(());
        }
        record.shadow_adopted_stylesheet_specifiers.clone()
    };
    let Some(value) = value else { return Ok(()) };
    let Some(hook) = scope
        .get_current_context()
        .get_slot::<NativeDeclarativeStylesheetHook>()
    else {
        return Ok(());
    };
    let registry = registry(scope, state);
    registry
        .borrow_mut()
        .initialized
        .retain(|node, _| state.dom.borrow().arena().get(*node).is_ok());
    if !registry.borrow().initialized.contains_key(&root)
        && registry.borrow().initialized.len() >= 4096
    {
        return Err(V8BindingError::Allocation("declarative stylesheet roots"));
    }
    let tokens = specifiers(&value);
    if tokens.len() > 4096 {
        return Err(V8BindingError::Allocation(
            "declarative stylesheet specifiers",
        ));
    }
    let mut index = registry
        .borrow()
        .initialized
        .get(&root)
        .copied()
        .unwrap_or(0);
    while index < tokens.len() {
        {
            let mut records = registry.borrow_mut();
            records
                .pending
                .retain(|entry| state.dom.borrow().arena().get(entry.root).is_ok());
            if records.pending.len() >= 4096 {
                return Err(V8BindingError::Allocation(
                    "declarative stylesheet subscriptions",
                ));
            }
            records
                .pending
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("declarative stylesheet subscriptions"))?;
        }
        let result = (hook.0)(scope, state, &tokens[index])?;
        let (sheet, url) = match result {
            DeclarativeStylesheetResult::Invalid => {
                index += 1;
                registry.borrow_mut().initialized.insert(root, index);
                continue;
            }
            DeclarativeStylesheetResult::Ready { sheet, .. } => (sheet, None),
            DeclarativeStylesheetResult::Pending { url } => {
                if registry
                    .borrow()
                    .pending
                    .iter()
                    .map(|entry| entry.url.len())
                    .sum::<usize>()
                    .saturating_add(url.len())
                    > 4 * 1024 * 1024
                {
                    return Err(V8BindingError::Allocation(
                        "declarative stylesheet subscription bytes",
                    ));
                }
                let sheet = create_module_stylesheet(scope, "", &url)?;
                (sheet, Some(url))
            }
            DeclarativeStylesheetResult::Failed { url } => {
                (create_module_stylesheet(scope, "", &url)?, None)
            }
        };
        let object = sheet
            .try_cast::<v8::Object>()
            .map_err(|_| V8BindingError::JavaScriptException)?;
        let sheet_node = resolve_receiver(scope, state, object, "CSSStyleSheet")?;
        let mut sheets = state
            .dom
            .borrow()
            .arena()
            .get(root)?
            .adopted_stylesheets
            .clone();
        sheets.push(sheet_node);
        let result = adopted_stylesheets::publish_native_list(scope, state, root, sheets.clone());
        let committed = state.dom.borrow().arena().get(root)?.adopted_stylesheets == sheets;
        if committed {
            index += 1;
            let mut records = registry.borrow_mut();
            records.initialized.insert(root, index);
            if let Some(url) = url {
                records.pending.push(Subscription {
                    root,
                    placeholder: sheet_node,
                    url,
                    publication_pending: false,
                });
            }
        }
        result?;
    }
    Ok(())
}
pub fn prepare_declarative_stylesheets(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            return Err(DomError::Cycle.into());
        }
        let children = state.dom.borrow().arena().get(node)?.children.clone();
        prepare_root(scope, state, node)?;
        pending.extend(children.into_iter().rev());
    }
    Ok(())
}
pub fn complete_declarative_stylesheet(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    url: &str,
    sheet: Option<v8::Local<v8::Value>>,
) -> Result<(), V8BindingError> {
    let context = state
        .global_object
        .to_local(scope)
        .and_then(|global| global.get_creation_context(scope))
        .ok_or(V8BindingError::Allocation(
            "declarative stylesheet owner realm",
        ))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    complete_in_context(scope, state, url, sheet)
}
fn complete_in_context(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    url: &str,
    sheet: Option<v8::Local<v8::Value>>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    let registry = registry(scope, state);
    let replacement = sheet
        .map(|sheet| {
            let object = sheet
                .try_cast::<v8::Object>()
                .map_err(|_| V8BindingError::JavaScriptException)?;
            resolve_receiver(scope, state, object, "CSSStyleSheet")
        })
        .transpose()?;
    let pending = std::mem::take(&mut registry.borrow_mut().pending);
    let mut first_error = None;
    for mut entry in pending {
        if entry.url != url {
            registry.borrow_mut().pending.push(entry);
            continue;
        }
        let current = {
            let dom = state.dom.borrow();
            dom.arena()
                .get(entry.root)
                .ok()
                .filter(|_| {
                    dom.node_document(entry.root).ok().flatten() == Some(state.document_root())
                })
                .map(|record| record.adopted_stylesheets.clone())
        };
        let Some(mut sheets) = current else { continue };
        let Some(replacement) = replacement else {
            continue;
        }; // Failed fetch keeps the original placeholder.
        if !sheets.contains(&entry.placeholder) && !entry.publication_pending {
            continue;
        }
        for sheet in &mut sheets {
            if *sheet == entry.placeholder {
                *sheet = replacement;
            }
        }
        if let Err(error) =
            adopted_stylesheets::publish_native_list(scope, state, entry.root, sheets)
        {
            // Retry publication only; placeholder identity prevents duplicate
            // replacement even when the native list committed before failure.
            entry.publication_pending = true;
            registry.borrow_mut().pending.push(entry);
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending<'s>(
        _: &mut v8::PinScope<'s, '_>,
        _: &Rc<V8BindingState>,
        specifier: &str,
    ) -> Result<DeclarativeStylesheetResult<'s>, V8BindingError> {
        if specifier == "invalid" {
            Ok(DeclarativeStylesheetResult::Invalid)
        } else {
            Ok(DeclarativeStylesheetResult::Pending {
                url: format!("https://module.test/{specifier}"),
            })
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn completion_tracks_placeholder_identity_through_all_author_mutations() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        context.set_slot(Rc::new(NativeDeclarativeStylesheetHook(pending)));
        eval(
            scope,
            r#"
            globalThis.roots=[];globalThis.placeholders=[];
            for(const mode of ['plain','swap','clear','remove','mutate','disconnect']){
                const host=document.createElement('div');document.body.append(host);host.setHTMLUnsafe('<template shadowrootmode="open" shadowrootserializable shadowrootadoptedstylesheets="red green red invalid"><span>text</span></template>');
                const root=host.shadowRoot,list=root.adoptedStyleSheets;if(list.length!==3||list[0]===list[2])throw Error('distinct per-occurrence placeholders');
                const first=list[0],green=list[1],second=list[2];placeholders.push([first,green,second]);roots.push(root);
                if(mode==='swap')root.adoptedStyleSheets=[green,first,second];
                if(mode==='clear')root.adoptedStyleSheets=[];
                if(mode==='remove')root.adoptedStyleSheets=[green];
                if(mode==='mutate')first.replaceSync('span{color:purple}');
                if(mode==='disconnect')host.remove();
            }
        "#,
        );
        let red =
            create_module_stylesheet(scope, "span { color: red; }", "https://module.test/red")
                .unwrap();
        let green =
            create_module_stylesheet(scope, "span { color: green; }", "https://module.test/green")
                .unwrap();
        let key = v8::String::new(scope, "redSheet").unwrap();
        context.global(scope).set(scope, key.into(), red);
        let key = v8::String::new(scope, "greenSheet").unwrap();
        context.global(scope).set(scope, key.into(), green);
        complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
            .unwrap();
        complete_declarative_stylesheet(scope, &state, "https://module.test/green", Some(green))
            .unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const index of [0,4,5])check(roots[index].adoptedStyleSheets[0]===redSheet&&roots[index].adoptedStyleSheets[1]===greenSheet&&roots[index].adoptedStyleSheets[2]===redSheet,'shared replacement including detached');
            check(roots[1].adoptedStyleSheets[0]===greenSheet&&roots[1].adoptedStyleSheets[1]===redSheet,'swap preserved');
            check(roots[2].adoptedStyleSheets.length===0,'clear not restored');check(roots[3].adoptedStyleSheets.length===1&&roots[3].adoptedStyleSheets[0]===greenSheet,'removed slots not restored');
            check(placeholders[0][0].cssRules.length===0&&placeholders[4][0].cssRules[0].cssText.includes('purple'),'placeholder objects unchanged');
            check(roots[0].host.getHTML({serializableShadowRoots:true}).includes('shadowrootadoptedstylesheets="red green red invalid"'),'raw provenance remains');return true;
        })()"#).is_true());
        eval(
            scope,
            "globalThis.failed=document.createElement('div');document.body.append(failed);failed.setHTMLUnsafe('<template shadowrootmode=open shadowrootadoptedstylesheets=missing></template>');globalThis.before=failed.shadowRoot.adoptedStyleSheets[0]",
        );
        complete_declarative_stylesheet(scope, &state, "https://module.test/missing", None)
            .unwrap();
        assert!(
            eval(
                scope,
                "failed.shadowRoot.adoptedStyleSheets[0]===before&&before.cssRules.length===0"
            )
            .is_true()
        );
        assert!(registry(scope, &state).borrow().pending.is_empty());
    }
    #[test]
    fn committed_completion_failure_does_not_duplicate_replacement_on_retry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        context.set_slot(Rc::new(NativeDeclarativeStylesheetHook(pending)));
        let placeholder=eval(scope,"globalThis.host=document.createElement('div');document.body.append(host);host.setHTMLUnsafe('<template shadowrootmode=open shadowrootadoptedstylesheets=red></template>');globalThis.original=host.shadowRoot.adoptedStyleSheets[0];original").try_cast::<v8::Object>().unwrap();
        let placeholder = resolve_receiver(scope, &state, placeholder, "CSSStyleSheet").unwrap();
        let red =
            create_module_stylesheet(scope, "span{color:red}", "https://module.test/red").unwrap();
        adopted_stylesheets::FAIL_NATIVE_REFRESH.with(|fault| fault.set(true));
        assert!(
            complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
                .is_err()
        );
        assert!(
            state.dom.borrow().arena().get(placeholder).is_ok(),
            "original wrapper remains legitimately rooted"
        );
        state
            .dom
            .borrow_mut()
            .retry_pending_stylesheet_reclaims()
            .unwrap();
        let committed = state.dom.borrow().mutation_cursor();
        complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
            .unwrap();
        assert_eq!(
            state.dom.borrow().mutation_cursor(),
            committed,
            "refresh retry does not repeat native list publication"
        );
        let current = eval(
            scope,
            "host.shadowRoot.adoptedStyleSheets.length===1&&host.shadowRoot.adoptedStyleSheets[0]!==original&&host.shadowRoot.adoptedStyleSheets[0].cssRules.length===1",
        );
        assert!(current.is_true());
        assert!(registry(scope, &state).borrow().pending.is_empty());
    }
    #[test]
    fn completion_refresh_retry_preserves_author_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        context.set_slot(Rc::new(NativeDeclarativeStylesheetHook(pending)));
        eval(
            scope,
            "globalThis.host=document.createElement('div');document.body.append(host);host.setHTMLUnsafe('<template shadowrootmode=open shadowrootadoptedstylesheets=red></template>')",
        );
        let red =
            create_module_stylesheet(scope, "span{color:red}", "https://module.test/red").unwrap();
        adopted_stylesheets::FAIL_NATIVE_REFRESH.with(|fault| fault.set(true));
        assert!(
            complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
                .is_err()
        );
        eval(
            scope,
            "globalThis.authored=new CSSStyleSheet();authored.replaceSync('span{color:blue}');host.shadowRoot.adoptedStyleSheets=[authored]",
        );
        complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
            .unwrap();
        assert!(eval(scope,"host.shadowRoot.adoptedStyleSheets.length===1&&host.shadowRoot.adoptedStyleSheets[0]===authored").is_true());
        assert!(registry(scope, &state).borrow().pending.is_empty());
    }
    #[test]
    fn inert_documents_and_retired_epoch_cannot_take_active_module_results() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        context.set_slot(Rc::new(NativeDeclarativeStylesheetHook(pending)));
        assert!(eval(scope,"Document.parseHTMLUnsafe('<div><template shadowrootmode=open shadowrootadoptedstylesheets=red></template></div>').querySelector('div').shadowRoot.adoptedStyleSheets.length===0").is_true());
        eval(
            scope,
            "globalThis.host=document.createElement('div');document.body.append(host);host.setHTMLUnsafe('<template shadowrootmode=open shadowrootadoptedstylesheets=red></template>');globalThis.before=host.shadowRoot.adoptedStyleSheets[0]",
        );
        let red =
            create_module_stylesheet(scope, "span{color:red}", "https://module.test/red").unwrap();
        state.retire_execution();
        complete_declarative_stylesheet(scope, &state, "https://module.test/red", Some(red))
            .unwrap();
        assert!(eval(scope, "host.shadowRoot.adoptedStyleSheets[0]===before").is_true());
    }
}
