//! ProcessingInstruction pseudo-attribute bindings use native CharacterData.
use super::*;
#[path = "pi_admission.rs"]
mod admission;
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let (initial_owner, initial) =
        node_identity::argument(scope, object.into())?.ok_or_else(|| {
            V8BindingError::Message("TypeError: incompatible ProcessingInstruction receiver".into())
        })?;
    if !matches!(
        initial_owner.dom.borrow().arena().get(initial)?.kind,
        NodeKind::ProcessingInstruction { .. }
    ) {
        return Err(V8BindingError::Message(
            "TypeError: incompatible ProcessingInstruction receiver".into(),
        ));
    }
    let termination = scope.thread_safe_handle();
    let name = if matches!(member, "hasAttributes" | "getAttributeNames") {
        None
    } else {
        Some(required_string(scope, args, 0)?)
    };
    let value = if member == "setAttribute" {
        Some(required_string(scope, args, 1)?)
    } else {
        None
    };
    let force = if member == "toggleAttribute" && args.length() > 1 {
        Some(args.get(1).boolean_value(scope))
    } else {
        None
    };
    let (state, node) =
        node_identity::argument(scope, object.into())?.ok_or(DomError::StaleNode(initial))?;
    if !matches!(
        state.dom.borrow().arena().get(node)?.kind,
        NodeKind::ProcessingInstruction { .. }
    ) {
        return Err(V8BindingError::Message(
            "TypeError: incompatible ProcessingInstruction receiver".into(),
        ));
    }
    if !Rc::ptr_eq(&initial_owner, &state)
        || initial != node
        || state.execution_tasks_stopped()
        || state.execution_suspended()
        || scope.is_execution_terminating()
    {
        return Err(V8BindingError::JavaScriptException);
    }
    if !crate::v8_termination::checkpoint(scope) {
        return Err(V8BindingError::JavaScriptException);
    }
    let mut allocation = admission::Admission::new(scope, &state)?;
    let mut check = || {
        if state.execution_tasks_stopped()
            || state.execution_suspended()
            || termination.is_execution_terminating()
        {
            Err(DomError::ProjectionCancelled)
        } else {
            Ok(())
        }
    };
    let attributes = state
        .dom
        .borrow()
        .arena()
        .processing_instruction_attributes_admitted(node, &mut allocation, &mut check)?;
    if !crate::v8_termination::checkpoint(scope) {
        return Err(V8BindingError::JavaScriptException);
    }
    let existing = name.as_ref().and_then(|name| {
        attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    });
    match member {
        "hasAttributes" => Ok(Some(v8::Boolean::new(scope, !attributes.is_empty()).into())),
        "getAttributeNames" => {
            let length = i32::try_from(attributes.len())
                .map_err(|_| V8BindingError::Allocation("PI attribute names length"))?;
            let array = v8::Array::new(scope, length);
            for (index, (name, _)) in attributes.iter().enumerate() {
                let value = v8_string(scope, name)?;
                if array.set_index(
                    scope,
                    u32::try_from(index)
                        .map_err(|_| V8BindingError::Allocation("PI attribute names index"))?,
                    value,
                ) != Some(true)
                {
                    return Err(V8BindingError::JavaScriptException);
                }
            }
            Ok(Some(array.into()))
        }
        "getAttribute" => Ok(Some(match existing {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        })),
        "hasAttribute" => Ok(Some(v8::Boolean::new(scope, existing.is_some()).into())),
        "setAttribute" => {
            let name = name.unwrap();
            validate_attribute_local_name(&name)?;
            let prepared = state
                .dom
                .borrow()
                .prepare_processing_instruction_attribute(
                    node,
                    &name,
                    value.as_deref(),
                    &mut allocation,
                    &mut check,
                )?;
            if !crate::v8_termination::checkpoint(scope) {
                return Err(V8BindingError::JavaScriptException);
            }
            if let Some(data) = prepared {
                check()?;
                state.dom.borrow_mut().set_text(node, data)?;
            }
            Ok(None)
        }
        "removeAttribute" => {
            let prepared = state
                .dom
                .borrow()
                .prepare_processing_instruction_attribute(
                    node,
                    &name.unwrap(),
                    None,
                    &mut allocation,
                    &mut check,
                )?;
            if !crate::v8_termination::checkpoint(scope) {
                return Err(V8BindingError::JavaScriptException);
            }
            if let Some(data) = prepared {
                check()?;
                state.dom.borrow_mut().set_text(node, data)?;
            }
            Ok(None)
        }
        "toggleAttribute" => {
            let name = name.unwrap();
            validate_attribute_local_name(&name)?;
            let present = existing.is_some();
            let result = force.unwrap_or(!present);
            if result != present {
                let prepared = state
                    .dom
                    .borrow()
                    .prepare_processing_instruction_attribute(
                        node,
                        &name,
                        result.then_some(""),
                        &mut allocation,
                        &mut check,
                    )?;
                if !crate::v8_termination::checkpoint(scope) {
                    return Err(V8BindingError::JavaScriptException);
                }
                if let Some(data) = prepared {
                    check()?;
                    state.dom.borrow_mut().set_text(node, data)?;
                }
            }
            Ok(Some(v8::Boolean::new(scope, result).into()))
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "ProcessingInstruction".into(),
            member: member.into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pi_attributes_and_range_conversion_preserve_native_fixture_contracts() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(189), 1).unwrap();
        let source=v8::String::new(scope,r####"(() => {
  const check = (value, label) => { if (!value) throw Error(label); };
  const creators = [
    (data) => document.createProcessingInstruction('t', data),
    (data) => { const box = document.createElement('div'); box.innerHTML = `<?t ${data}?>`; return box.firstChild; },
    (data) => new DOMParser().parseFromString('<root/>', 'application/xml').createProcessingInstruction('t', data),
    (data) => new DOMParser().parseFromString(`<?t ${data}?>`, 'application/xml').firstChild,
  ];
  for (const create of creators) {
    const pi = create('a="b" x="yy"');
    check(pi.nodeType === 7 && pi.getAttributeNames().join('|') === 'a|x', 'native parsed order');
    const observer = new MutationObserver(() => {});
    observer.observe(pi, {attributes: true, characterData: true, characterDataOldValue: true});
    pi.setAttribute('x', 'a"&<>');
    const mutation = observer.takeRecords();
    check(mutation.length === 1 && mutation[0].type === 'characterData' && mutation[0].target === pi, 'CharacterData publication');
    check(pi.getAttribute('x') === 'a"&<>', 'reference decoding');
    const element = pi.ownerDocument.createElement('el'); element.setAttribute('x', 'a"&<>');
    check(pi.data.endsWith(element.outerHTML.match(/el\s+([^>]+?)\s*\/?>/)[1]), 'serialization parity');
    pi.removeAttribute('x'); pi.setAttribute('$foo','1'); pi.setAttribute('_bar','2');
    check(pi.getAttributeNames().join('|') === 'a|$foo|_bar', 'extended attribute names');
    pi.data='a=b x="yy"'; check(!pi.hasAttributes(), 'all-or-nothing quoted grammar');
    pi.toggleAttribute('Case', true); check(pi.getAttribute('Case') === '' && pi.getAttribute('case') === null, 'case identity');
    for (const value of ['\uD800','\u{F1000}','\u{F0000}','\u{F2000}']) {
      pi.setAttribute('special',value);
      element.setAttribute('special',value);
      check(pi.getAttribute('special')===value,'lossless special value');
      check(pi.data.includes(element.outerHTML.match(/el\s+([^>]+?)\s*\/?>/)[1].split('special=')[1]),'special serialized parity');
    }
    const original=pi.data, marker={};
    try { pi.setAttribute('new', {toString(){throw marker}}); throw Error('accepted coercion'); }
    catch(error) { check(error === marker && pi.data === original, 'exception identity and mutation atomicity'); }
    for (const name of ['=', '/', 'x\t', '>']) {
      try { pi.toggleAttribute(name, false); throw Error('accepted invalid name'); }
      catch(error) { check(error.name === 'InvalidCharacterError', 'name validation precedes force'); }
    }
  }
  const range = document.createRange(), other = document.createRange();
  for (const how of [NaN, Infinity, -Infinity, 65536, 4294967296, '-65536', null, false])
    check(range.compareBoundaryPoints(how, other) === 0, 'unsigned-short conversion');
  try { range.compareBoundaryPoints(99, {}); throw Error('accepted source brand'); }
  catch(error) { check(error instanceof TypeError, 'source brand before algorithm'); }
  const foreign = new Document().createRange();
  try { range.compareBoundaryPoints(99, foreign); throw Error('accepted invalid how'); }
  catch(error) { check(error.name === 'NotSupportedError', 'invalid how before foreign root'); }
  return true;
})()
"####).unwrap();
        let result = super::super::test_eval::run(scope, source);
        assert!(result.is_true());
    }
}
