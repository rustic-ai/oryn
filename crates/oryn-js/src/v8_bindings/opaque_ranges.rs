//! Control-value range wrappers with native identity and lifetime tracking.
use super::*;
use oryn_dom::{OpaqueRangeError, OpaqueRangeId};

pub(super) fn id(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> Option<OpaqueRangeId> {
    if object.internal_field_count() < 4 {
        return None;
    }
    let marker = object.get_internal_field(scope, 2)?;
    let marker = v8::Local::<v8::Value>::try_from(marker).ok()?;
    if !marker.is_true() {
        return None;
    }
    let value = object.get_internal_field(scope, 3)?;
    exact_u64(v8::Local::<v8::Value>::try_from(value).ok()?).map(OpaqueRangeId)
}

fn payload_bytes() -> i64 {
    std::mem::size_of::<oryn_dom::OpaqueRange>() as i64
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: NodeId,
    start: u32,
    end: u32,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    {
        let dom = state.dom.borrow();
        let supported = match element_local_name(&dom, owner) {
            Some("textarea") => true,
            Some("input") => matches!(
                dom.attribute(owner, "type")?
                    .unwrap_or("text")
                    .to_ascii_lowercase()
                    .as_str(),
                "text" | "search" | "url" | "tel" | "password"
            ),
            _ => false,
        };
        if !supported {
            return Err(V8BindingError::Unsupported {
                interface: "OpaqueRange".into(),
                member: "NotSupportedError".into(),
            });
        }
    }
    let count = state.opaque_range_objects.borrow().len();
    if count >= 64 && count.is_multiple_of(64) {
        collect(state)?;
    }
    let range = state
        .dom
        .borrow_mut()
        .create_opaque_range(owner, start, end)
        .map_err(|error| match error {
            DomError::OpaqueRange(OpaqueRangeError::InvalidOffset) => V8BindingError::Unsupported {
                interface: "OpaqueRange".into(),
                member: "IndexSizeError".into(),
            },
            other => other.into(),
        })?;
    let object = match new_stateful_object(scope, state, "OpaqueRange", range.0) {
        Ok(object) => object,
        Err(error) => {
            state.dom.borrow_mut().drop_opaque_range(range)?;
            return Err(error);
        }
    };
    object.set_internal_field(2, v8::Boolean::new(scope, true).into());
    object.set_internal_field(3, v8::BigInt::new_from_u64(scope, range.0).into());
    state
        .opaque_range_objects
        .borrow_mut()
        .insert(range.0, v8::Weak::new(scope, object));
    state
        .range_memory_delta
        .set(state.range_memory_delta.get() + payload_bytes());
    apply_range_memory_accounting(scope, state);
    Ok(object)
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    range: OpaqueRangeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let range = state.dom.borrow().opaque_range(range)?;
    match member {
        // The pinned experimental API hides the native control/container.
        // Keep ordinary AbstractRange container access unchanged.
        "startContainer" | "endContainer" => Ok(v8::undefined(scope).into()),
        "startOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.start).into()),
        "endOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.end).into()),
        "collapsed" => Ok(v8::Boolean::new(scope, range.start == range.end).into()),
        _ => Err(V8BindingError::Unsupported {
            interface: "OpaqueRange".into(),
            member: member.into(),
        }),
    }
}

pub(super) fn collect(state: &V8BindingState) -> Result<usize, BindingError> {
    let (Ok(mut objects), Ok(mut dom)) = (
        state.opaque_range_objects.try_borrow_mut(),
        state.dom.try_borrow_mut(),
    ) else {
        return Ok(0);
    };
    let dead: Vec<_> = objects
        .iter()
        .filter(|(_, wrapper)| wrapper.is_empty())
        .map(|(id, _)| *id)
        .collect();
    for id in &dead {
        dom.drop_opaque_range(OpaqueRangeId(*id))?;
        objects.remove(id);
        state
            .range_memory_delta
            .set(state.range_memory_delta.get() - payload_bytes());
    }
    Ok(dead.len())
}
