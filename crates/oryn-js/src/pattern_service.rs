//! Private-context RegExp service in the existing agent isolate. An isolate
//! slot owns teardown; retained DOM snapshots receive a closed-service error
//! after isolate retirement instead of keeping a dangling V8 pointer alive.
use oryn_dom::{DomError, PatternMatcher};
use std::{cell::Cell, ffi::c_void, fmt, ptr::NonNull, rc::Rc};

const MAX_PATTERN_UNITS: usize = 16 * 1024;
const MAX_VALUE_UNITS: usize = 1024 * 1024;

unsafe extern "C" {
    fn oryn_v8_pattern_service_new(isolate: v8::UnsafeRawIsolatePtr) -> *mut c_void;
    fn oryn_v8_pattern_service_delete(service: *mut c_void);
    fn oryn_v8_pattern_match(
        service: *mut c_void,
        pattern: *const u16,
        pattern_len: usize,
        value: *const u16,
        value_len: usize,
    ) -> i32;
}

struct Matcher {
    native: Cell<Option<NonNull<c_void>>>,
}
impl fmt::Debug for Matcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentPatternMatcher")
            .field("closed", &self.native.get().is_none())
            .finish()
    }
}
impl Matcher {
    fn close(&self) {
        if let Some(native) = self.native.take() {
            // SAFETY: Owner is an isolate slot. Pinned rusty_v8 150.4.0 drops
            // slots in prepare_annex_for_dispose, BEFORE V8 Isolate::Dispose.
            // All references are Rc/thread-affine. The C++ destructor enters
            // that still-live isolate and releases every persistent handle.
            unsafe { oryn_v8_pattern_service_delete(native.as_ptr()) };
        }
    }
}
struct Owner(Rc<Matcher>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub(crate) fn for_isolate(scope: &mut v8::PinScope) -> Result<Rc<dyn PatternMatcher>, DomError> {
    if let Some(owner) = scope.get_slot::<Owner>() {
        return Ok(owner.0.clone());
    }
    // SAFETY: The isolate is entered. The service is registered immediately
    // in its unique owner slot; no service pointer is exposed to authored JS.
    let native = NonNull::new(unsafe { oryn_v8_pattern_service_new(scope.as_raw_isolate_ptr()) })
        .ok_or(DomError::PatternEvaluation("engine allocation"))?;
    let matcher = Rc::new(Matcher {
        native: Cell::new(Some(native)),
    });
    scope.set_slot(Owner(matcher.clone()));
    Ok(matcher)
}

impl PatternMatcher for Matcher {
    fn matches(&self, pattern: &str, value: &str) -> Result<Option<bool>, DomError> {
        let native = self
            .native
            .get()
            .ok_or(DomError::PatternEvaluation("engine retired"))?;
        // Bound the lossless UTF-16 adapter's temporary allocation as well as
        // the buffers sent across FFI (the encoded surrogate form uses <=6
        // bytes per unit plus its fixed prefix).
        if pattern.len() > MAX_PATTERN_UNITS * 6 + 64 || value.len() > MAX_VALUE_UNITS * 6 + 64 {
            return Err(DomError::PatternEvaluation("input size limit"));
        }
        let pattern = oryn_dom::dom_string::units(pattern);
        let value = oryn_dom::dom_string::units(value);
        if pattern.len() > MAX_PATTERN_UNITS || value.len() > MAX_VALUE_UNITS {
            return Err(DomError::PatternEvaluation("input size limit"));
        }
        // SAFETY: Owner has not retired, buffers live through this synchronous
        // call, sizes are bounded, and Rc prevents cross-thread access. The
        // native service invokes no authored callbacks or Rust DOM accesses.
        match unsafe {
            oryn_v8_pattern_match(
                native.as_ptr(),
                pattern.as_ptr(),
                pattern.len(),
                value.as_ptr(),
                value.len(),
            )
        } {
            0 => Ok(Some(false)),
            1 => Ok(Some(true)),
            2 => Ok(None),
            _ => Err(DomError::PatternEvaluation("engine failure or termination")),
        }
    }
}
