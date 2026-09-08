//! Traced ownership and lazy wrapper materialization for nested CSS rules.
use super::*;

pub(super) fn parent_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.css.rule.parent")
        .ok_or(V8BindingError::Allocation("CSS rule parent key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    Ok(object
        .get_private(scope, key)
        .filter(|value| value.is_object())
        .unwrap_or_else(|| v8::null(scope).into()))
}

pub(super) fn children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let identity =
        id(scope, object).ok_or(V8BindingError::Allocation("CSS grouping rule receiver"))?;
    let (owner, views, base_url, quirks, source) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("CSS grouping rule record"))?;
        (
            record.owner,
            record.view.children.clone(),
            record.base_url.clone(),
            record.quirks,
            record.source.clone(),
        )
    };
    let name = v8::String::new(scope, "oryn.css.rule.children")
        .ok_or(V8BindingError::Allocation("CSS child rules key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if let Some(array) = object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return css_rule_lists::wrap_rule(scope, state, owner, object, array);
    }
    let array = v8::Array::new(scope, 0);
    for (index, view) in views.into_iter().enumerate() {
        let child = create(scope, state, owner, view, base_url.clone(), quirks, index)?;
        let child_id =
            id(scope, child).ok_or(V8BindingError::Allocation("CSS child rule identity"))?;
        if let Some(record) = state.page_rules.borrow_mut().get_mut(&child_id) {
            record.parent_rule = Some(identity);
            record.source = source.clone();
        }
        let name = v8::String::new(scope, "oryn.css.rule.parent")
            .ok_or(V8BindingError::Allocation("CSS child parent key"))?;
        let parent_key = v8::Private::for_api(scope, Some(name));
        child.set_private(scope, parent_key, object.into());
        array.set_index(scope, index as u32, child.into());
    }
    object.set_private(scope, key, array.into());
    css_rule_lists::wrap_rule(scope, state, owner, object, array)
}

/// Nested descendants inherit the current sheet association from their root.
/// Deleting a grouping rule detaches that root without severing its children.
pub(super) fn stylesheet_owner(state: &V8BindingState, mut identity: u64) -> Option<NodeId> {
    let records = state.page_rules.borrow();
    loop {
        let record = records.get(&identity)?;
        match record.parent_rule {
            Some(parent) => identity = parent,
            None => return record.parent,
        }
    }
}

fn child_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    identity: u64,
) -> Option<v8::Local<'s, v8::Array>> {
    let object = state
        .page_rules
        .borrow()
        .get(&identity)?
        ._wrapper
        .to_local(scope)?;
    let name = v8::String::new(scope, "oryn.css.rule.children")?;
    let key = v8::Private::for_api(scope, Some(name));
    object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn child_index(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    parent: u64,
    identity: u64,
) -> Result<Option<usize>, V8BindingError> {
    let Some(array) = child_array(scope, state, parent) else {
        return Ok(None);
    };
    for index in 0..array.length() {
        let value = array
            .get_index(scope, index)
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && id(scope, object) == Some(identity)
        {
            return Ok(Some(index as usize));
        }
    }
    Ok(None)
}

pub(super) fn root_identity(
    state: &V8BindingState,
    mut identity: u64,
) -> Result<u64, V8BindingError> {
    let records = state.page_rules.borrow();
    loop {
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("CSS rule ancestor"))?;
        match record.parent_rule {
            Some(parent) => identity = parent,
            None => return Ok(identity),
        }
    }
}

pub(super) fn refresh_subtree(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    identity: u64,
    view: BindingRuleView,
    source: Rc<str>,
    index: usize,
) -> Result<(), V8BindingError> {
    if let Some(array) = child_array(scope, state, identity) {
        for (index, child_view) in view.children.iter().enumerate() {
            let Some(object) = array
                .get_index(scope, index as u32)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                continue;
            };
            if let Some(child) = id(scope, object) {
                refresh_subtree(
                    scope,
                    state,
                    child,
                    child_view.clone(),
                    source.clone(),
                    index,
                )?;
            }
        }
    }
    if let Some(record) = state.page_rules.borrow_mut().get_mut(&identity) {
        record.view = view;
        record.source = source;
        record.index = index;
    }
    Ok(())
}

pub(super) fn view_at<'a>(
    rules: &'a [BindingRuleView],
    path: &[usize],
) -> Option<&'a BindingRuleView> {
    let (index, rest) = path.split_first()?;
    let view = rules.get(*index)?;
    if rest.is_empty() {
        Some(view)
    } else {
        view_at(&view.children, rest)
    }
}

pub(super) fn edit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    identity: u64,
    edit: BindingRuleListEdit<'_>,
) -> Result<(), V8BindingError> {
    let (source, path) = mutation_context(scope, state, identity)?;
    let (owner, base_url, quirks) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("CSS grouping rule record"))?;
        (record.owner, record.base_url.clone(), record.quirks)
    };
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("CSS grouping rule provider"))?;
    let rules = provider
        .mutate_rule_list(&source, &path, edit, &base_url, quirks, true)
        .ok_or(V8BindingError::Allocation("CSS grouping rule mutation"))?
        .map_err(css_rule_mutation_error)?;
    children(scope, state, object)?;
    let array = child_array(scope, state, identity)
        .ok_or(V8BindingError::Allocation("CSS child rule array"))?;
    let length = array.length();
    match edit {
        BindingRuleListEdit::Insert { index, .. } => {
            let view = view_at(&rules, &path)
                .and_then(|view| view.children.get(index))
                .cloned()
                .ok_or(V8BindingError::Allocation("CSS inserted child view"))?;
            let child = create(scope, state, owner, view, base_url, quirks, index)?;
            let child_id = id(scope, child)
                .ok_or(V8BindingError::Allocation("CSS inserted child identity"))?;
            if let Some(record) = state.page_rules.borrow_mut().get_mut(&child_id) {
                record.parent_rule = Some(identity);
            }
            let name = v8::String::new(scope, "oryn.css.rule.parent")
                .ok_or(V8BindingError::Allocation("CSS inserted parent key"))?;
            let key = v8::Private::for_api(scope, Some(name));
            child.set_private(scope, key, object.into());
            for position in (index as u32..length).rev() {
                let value = array
                    .get_index(scope, position)
                    .ok_or(V8BindingError::JavaScriptException)?;
                array.set_index(scope, position + 1, value);
            }
            array.set_index(scope, index as u32, child.into());
        }
        BindingRuleListEdit::Delete { index } => {
            let child = array
                .get_index(scope, index as u32)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .ok_or(V8BindingError::Allocation("CSS removed child"))?;
            detach(scope, state, child)?;
            for position in index as u32 + 1..length {
                let value = array
                    .get_index(scope, position)
                    .ok_or(V8BindingError::JavaScriptException)?;
                array.set_index(scope, position - 1, value);
            }
            let key = v8::String::new(scope, "length")
                .ok_or(V8BindingError::Allocation("CSS child array length"))?;
            array.set(
                scope,
                key.into(),
                v8::Integer::new_from_unsigned(scope, length - 1).into(),
            );
        }
    }
    publish_rule_transaction(
        scope,
        state,
        identity,
        stylesheet_owner(state, identity),
        path,
        rules,
    )
}
