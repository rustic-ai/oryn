//! Document-creation snapshots for Location.ancestorOrigins.
use super::*;
use crate::origin::Origin;
/// Captured before fetching a child document. Response/meta referrer policies
/// and later iframe mutations cannot replace the element's admission policy.
#[derive(Clone, Debug)]
pub struct LocationAncestorCapture {
    parent: Origin,
    ancestors: Vec<Origin>,
    referrer_policy: String,
}
impl LocationAncestorCapture {
    pub fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>()
            .checked_add(self.parent.retained_bytes()?)?
            .checked_add(self.referrer_policy.capacity())?
            .checked_add(
                self.ancestors
                    .capacity()
                    .checked_mul(std::mem::size_of::<Origin>())?,
            )?;
        for origin in &self.ancestors {
            bytes = bytes.checked_add(origin.retained_bytes()?)?;
        }
        Some(bytes)
    }

    pub fn resolve(&self, child: &Origin) -> Vec<Origin> {
        let mut masked = self.referrer_policy.eq_ignore_ascii_case("no-referrer")
            || self.referrer_policy.eq_ignore_ascii_case("same-origin") && self.parent != *child;
        let mut output = vec![if masked {
            Origin::opaque()
        } else {
            self.parent.clone()
        }];
        for ancestor in &self.ancestors {
            if masked && ancestor == &self.parent {
                output.push(Origin::opaque());
                continue;
            }
            output.push(ancestor.clone());
            masked = false;
        }
        output
    }
}
impl V8BindingState {
    pub fn capture_location_ancestors(
        &self,
        parent: Origin,
        referrer_policy: String,
    ) -> LocationAncestorCapture {
        LocationAncestorCapture {
            parent,
            ancestors: self.location_ancestor_origins.borrow().clone(),
            referrer_policy: referrer_policy.trim().into(),
        }
    }
    pub fn has_location_ancestor_origins(&self) -> bool {
        !self.location_ancestor_origins.borrow().is_empty()
    }
    pub fn set_location_ancestor_origins(&self, origins: Vec<Origin>) {
        *self.location_ancestor_origins.borrow_mut() = origins;
    }
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let location = new_platform_object(scope, state, "Location", id)?;
    let key = v8::String::new(scope, "oryn.location.ancestorOrigins")
        .ok_or(V8BindingError::Allocation("Location ancestry cache"))?;
    let key = v8::Private::for_api(scope, Some(key));
    let identity = if state.execution_retired() {
        "inactive".into()
    } else {
        format!(
            "{}:{}:{}",
            state.document_root().slot,
            state.document_root().generation,
            state.document_generation()
        )
    };
    if let Some(record) = location
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        && record
            .get_internal_field(scope, 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .is_some_and(|value| value.to_rust_string_lossy(scope) == identity)
        && let Some(value) = record
            .get_internal_field(scope, 0)
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
    {
        return Ok(value);
    }
    let names = if state.execution_retired() {
        Vec::new()
    } else {
        state
            .location_ancestor_origins
            .borrow()
            .iter()
            .map(Origin::serialize)
            .collect()
    };
    let list = if state.execution_retired() {
        stylesheet_sets::inactive_empty_snapshot(scope, state)?
    } else {
        stylesheet_sets::snapshot(scope, state, &names)?
    };
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(2);
    let record = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("Location ancestry cache"))?;
    record.set_internal_field(0, list.into());
    let identity = v8::String::new(scope, &identity)
        .ok_or(V8BindingError::Allocation("Location ancestry identity"))?;
    record.set_internal_field(1, identity.into());
    if location.set_private(scope, key, record.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Location ancestry publication"));
    }
    Ok(list.into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masking_snapshots_only_contiguous_equal_parent_origins() {
        let a = Origin::from_url("https://a.test/");
        let b = Origin::from_url("https://b.test/");
        let c = Origin::from_url("https://c.test/");
        let capture = LocationAncestorCapture {
            parent: b.clone(),
            ancestors: vec![b.clone(), a.clone(), b.clone()],
            referrer_policy: "no-referrer".into(),
        };
        let result = capture.resolve(&c);
        assert_eq!(
            result.iter().map(Origin::serialize).collect::<Vec<_>>(),
            vec!["null", "null", "https://a.test", "https://b.test"]
        );
        assert_ne!(result[0], result[1]);
        let capture = LocationAncestorCapture {
            referrer_policy: "same-origin".into(),
            ..capture
        };
        assert_eq!(capture.resolve(&b)[0], b);
        assert!(capture.resolve(&c)[0].is_opaque());
        let capture = LocationAncestorCapture {
            parent: c,
            ancestors: result,
            referrer_policy: String::new(),
        };
        assert!(capture.resolve(&a)[1].is_opaque());
    }
}
