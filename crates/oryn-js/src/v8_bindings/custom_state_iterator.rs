//! Live custom-state iterators. The private source wrapper retains the element;
//! monotonic insertion identities preserve cursors without retaining deleted entries.
use super::*;

#[derive(Default)]
pub(super) struct Order {
    next: u64,
    entries: BTreeMap<u64, String>,
}

impl Order {
    pub(super) fn insert(&mut self, value: String) -> Result<(), V8BindingError> {
        // The private JS cursor carries an exact Number, including the terminal
        // cursor one greater than the final insertion identity.
        if self.next >= (1u64 << 53) - 1 {
            return Err(V8BindingError::Allocation(
                "custom state insertion identity exhausted",
            ));
        }
        self.entries.insert(self.next, value);
        self.next += 1;
        Ok(())
    }
    pub(super) fn next_from(&self, cursor: u64) -> Option<(u64, String)> {
        self.entries
            .range(cursor..)
            .next()
            .map(|(id, value)| (id + 1, value.clone()))
    }
    pub(super) fn remove(&mut self, value: &str) {
        self.entries.retain(|_, entry| entry != value);
    }
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

fn key<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.customStateIterator")
        .ok_or(V8BindingError::Allocation("custom state iterator key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

fn prototype_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.customStateIteratorPrototype")
        .ok_or(V8BindingError::Allocation("iterator prototype key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let private = prototype_key(scope)?;
    if global
        .get_private(scope, private)
        .is_some_and(|value| value.is_object())
    {
        return Ok(());
    }
    // V8 exposes the realm's intrinsic directly. Calling ArrayProtoValues here
    // unnecessarily enters JavaScript during native realm installation (and
    // used to misreport any pending exception/termination as allocation loss).
    let template = v8::ObjectTemplate::new(scope);
    let name = v8::String::new(scope, "iteratorPrototype")
        .ok_or(V8BindingError::Allocation("intrinsic name"))?;
    template.set_intrinsic_data_property(
        name.into(),
        v8::Intrinsic::IteratorPrototype,
        v8::PropertyAttribute::NONE,
    );
    let holder = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("intrinsic holder"))?;
    let base = holder
        .get(scope, name.into())
        .filter(|value| value.is_object())
        .ok_or(V8BindingError::Allocation("IteratorPrototype"))?;
    let prototype = v8::Object::new(scope);
    prototype.set_prototype(scope, base);
    let next = v8::Function::builder(next)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("custom state next"))?;
    let name = v8::String::new(scope, "next").ok_or(V8BindingError::Allocation("next name"))?;
    next.set_name(name);
    prototype.define_own_property(
        scope,
        name.into(),
        next.into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "CustomStateSet Iterator")
        .ok_or(V8BindingError::Allocation("iterator tag"))?;
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
    );
    global.set_private(scope, private, prototype.into());
    Ok(())
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    pairs: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let iterator = v8::Object::new(scope);
    let cursor = v8::Integer::new(scope, 0);
    let pairs = v8::Boolean::new(scope, pairs);
    let data = v8::Array::new_with_elements(scope, &[source.into(), cursor.into(), pairs.into()]);
    let private = key(scope)?;
    iterator.set_private(scope, private, data.into());
    let context = scope.get_current_context();
    let global = context.global(scope);
    let private = prototype_key(scope)?;
    let prototype = global
        .get_private(scope, private)
        .filter(|value| value.is_object())
        .ok_or(V8BindingError::Allocation("iterator prototype"))?;
    iterator.set_prototype(scope, prototype);
    Ok(iterator.into())
}

fn next(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    match advance(scope, args.this()) {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn advance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let private = key(scope)?;
    let data = iterator
        .get_private(scope, private)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "CustomStateSet Iterator".into(),
            member: "TypeError".into(),
        })?;
    let source = data
        .get_index(scope, 0)
        .ok_or(V8BindingError::Allocation("iterator source"))?;
    let mut value = None;
    if let Ok(source) = v8::Local::<v8::Object>::try_from(source) {
        let state = binding_state_for_receiver(scope, source)
            .ok_or(V8BindingError::Allocation("iterator owner"))?;
        let node = resolve_receiver(scope, &state, source, "CustomStateSet")?;
        let cursor = data
            .get_index(scope, 1)
            .and_then(|value| value.number_value(scope))
            .unwrap_or(0.0) as u64;
        let entry = state
            .custom_state_order
            .borrow()
            .get(&node)
            .and_then(|order| order.next_from(cursor));
        let next_cursor = if let Some((cursor, entry)) = entry {
            value = Some(entry);
            v8::Number::new(scope, cursor as f64)
        } else {
            v8::Number::new(scope, cursor as f64)
        };
        data.set_index(scope, 1, next_cursor.into());
        if value.is_none() {
            // Once done, appending to the source cannot restart this iterator.
            let empty = v8::null(scope);
            data.set_index(scope, 0, empty.into());
        }
    }
    let result = v8::Object::new(scope);
    let done = v8::Boolean::new(scope, value.is_none());
    let output = if let Some(value) = value {
        let value = v8_string(scope, &value)?;
        if data
            .get_index(scope, 2)
            .is_some_and(|value| value.is_true())
        {
            v8::Array::new_with_elements(scope, &[value, value]).into()
        } else {
            value
        }
    } else {
        v8::undefined(scope).into()
    };
    let value_key =
        v8::String::new(scope, "value").ok_or(V8BindingError::Allocation("iterator value key"))?;
    let done_key =
        v8::String::new(scope, "done").ok_or(V8BindingError::Allocation("iterator done key"))?;
    result.create_data_property(scope, value_key.into(), output);
    result.create_data_property(scope, done_key.into(), done.into());
    Ok(result.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_entries_are_released_without_invalidating_live_cursors() {
        let mut order = Order::default();
        order.insert("first".into()).unwrap();
        order.insert("second".into()).unwrap();
        let (cursor, first) = order.next_from(0).unwrap();
        assert_eq!(first, "first");
        order.remove("first");
        order.remove("second");
        assert!(order.entries.is_empty());
        for index in 0..10_000 {
            let value = index.to_string();
            order.insert(value.clone()).unwrap();
            order.remove(&value);
        }
        assert!(order.entries.is_empty());
        order.insert("after-deletion".into()).unwrap();
        let (cursor, value) = order.next_from(cursor).unwrap();
        assert_eq!(value, "after-deletion");
        order.clear();
        order.insert("after-clear".into()).unwrap();
        assert_eq!(order.next_from(cursor).unwrap().1, "after-clear");
        assert_eq!(order.entries.len(), 1);
    }

    #[test]
    fn insertion_identity_exhaustion_does_not_alias_or_mutate_entries() {
        let mut order = Order {
            next: (1u64 << 53) - 2,
            entries: BTreeMap::new(),
        };
        order.insert("last".into()).unwrap();
        let (cursor, value) = order.next_from(0).unwrap();
        assert_eq!(cursor, (1u64 << 53) - 1);
        assert_eq!(value, "last");
        assert!(order.insert("overflow".into()).is_err());
        assert_eq!(order.entries.len(), 1);
        assert!(order.next_from(cursor).is_none());
        order.clear();
        assert!(order.insert("reset-alias".into()).is_err());
        assert!(order.entries.is_empty());
    }
}

#[cfg(test)]
mod installation_tests {
    use super::*;

    #[test]
    fn native_iterator_installation_never_enters_javascript_or_uses_authored_globals() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(scope,
            "globalThis.expectedIteratorPrototype=Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));Array.prototype.values=()=>{throw Error('authored values')};globalThis.Array=null;").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        {
            let guarded = std::pin::pin!(v8::DisallowJavascriptExecutionScope::new(
                scope,
                v8::OnFailure::ThrowOnFailure
            ));
            let guarded = &mut guarded.init();
            install(guarded, context).unwrap();
            install(guarded, context).unwrap();
        }
        let private = prototype_key(scope).unwrap();
        let prototype = context.global(scope).get_private(scope, private).unwrap();
        let prototype = v8::Local::<v8::Object>::try_from(prototype).unwrap();
        let name = v8::String::new(scope, "expectedIteratorPrototype").unwrap();
        let expected = context.global(scope).get(scope, name.into()).unwrap();
        assert!(
            prototype
                .get_prototype(scope)
                .unwrap()
                .strict_equals(expected)
        );
    }

    #[test]
    fn explicit_target_context_owns_its_intrinsic_even_with_another_realm_entered() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, first);
        install(scope, first).unwrap();
        install(scope, second).unwrap();
        let private = prototype_key(scope).unwrap();
        let first_prototype = first.global(scope).get_private(scope, private).unwrap();
        let second_prototype = second.global(scope).get_private(scope, private).unwrap();
        let first_prototype = v8::Local::<v8::Object>::try_from(first_prototype).unwrap();
        let second_prototype = v8::Local::<v8::Object>::try_from(second_prototype).unwrap();
        assert!(
            !first_prototype
                .get_prototype(scope)
                .unwrap()
                .strict_equals(second_prototype.get_prototype(scope).unwrap())
        );
    }
}
