//! Outer dimensions follow the top-level native headless window rectangle.
use super::*;

pub(super) fn outer_size(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(f64, f64), V8BindingError> {
    let mut current = state.clone();
    let mut ancestors = BTreeSet::new();
    loop {
        if current.execution_retired() {
            return Ok((0., 0.));
        }
        if !ancestors.insert(current.instance_id) {
            return Err(V8BindingError::Allocation("outer dimension parent cycle"));
        }
        let relation = current.frame_viewport_parent.borrow().clone();
        let Some((parent, frame, connection)) = relation else {
            // Oryn has no browser chrome: WebDriver's outer rectangle is the
            // top-level viewport in CSS pixels, independent of device scale.
            refresh_layout_in_scope(scope, &current)?;
            return Ok(current.layout.borrow().viewport.unwrap_or((1280., 720.)));
        };
        let Some(parent) = parent.upgrade() else {
            return Ok((0., 0.));
        };
        // A detached/replaced frame has no live enclosing window. Never follow
        // an expired connection into a replacement document's window geometry.
        let live = connection.is_some()
            && !parent.execution_retired()
            && parent.dom.borrow().frame_context_generation(frame) == connection
            && parent.dom.borrow().is_connected(frame)?;
        if !live {
            return Ok((0., 0.));
        }
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluate(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn outer_dimensions_reject_disconnected_or_replaced_frame_parent() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let parent_context = v8::Context::new(scope, Default::default());
        let (parent, frame) = {
            let scope = &mut v8::ContextScope::new(scope, parent_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=other></div><iframe id=f></iframe></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let frame = dom.borrow().find_by_id("f").unwrap();
            dom.borrow_mut().track_frame_context(frame);
            let parent =
                install_generated_bindings(scope, parent_context, dom, RealmId(0), 1).unwrap();
            parent
                .set_native_layout_viewport(800., 600., false)
                .unwrap();
            (parent, frame)
        };
        let child_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, child_context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let child = install_generated_bindings(scope, child_context, dom, RealmId(1), 1).unwrap();
        child.set_native_layout_viewport(240., 120., false).unwrap();
        child.set_frame_viewport_parent(&parent, frame).unwrap();
        assert_eq!(outer_size(scope, &child).unwrap(), (800., 600.));
        let other = parent.dom.borrow().find_by_id("other").unwrap();
        parent.dom.borrow_mut().append_child(other, frame).unwrap();
        assert!(parent.dom.borrow().is_connected(frame).unwrap());
        assert_eq!(outer_size(scope, &child).unwrap(), (0., 0.));
        child.set_frame_viewport_parent(&parent, frame).unwrap();
        assert_eq!(outer_size(scope, &child).unwrap(), (800., 600.));
        parent.dom.borrow_mut().remove(frame).unwrap();
        child.set_frame_viewport_parent(&parent, frame).unwrap();
        assert!(!parent.execution_retired() && !child.execution_retired());
        assert_eq!(outer_size(scope, &child).unwrap(), (0., 0.));
    }

    #[test]
    fn outer_dimensions_are_numeric_live_and_replaceable() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_native_layout_viewport(800., 600., false).unwrap();
        evaluate(
            scope,
            r#"
            function check(ok, message) { if (!ok) throw Error(message); }
            globalThis.dimensions = ['outerWidth', 'outerHeight'].map(name =>
                Object.getOwnPropertyDescriptor(globalThis, name));
            check(dimensions.every(Boolean), 'outer dimensions are own global attributes');
            check(!Object.hasOwn(Window.prototype, 'outerWidth') && !Object.hasOwn(Window.prototype, 'outerHeight'), 'outer dimensions are absent from Window.prototype');
            check(outerWidth === 800 && outerHeight === 600, 'numeric CSS dimensions');
            check(Number.isFinite(outerWidth-innerWidth) && Number.isFinite(outerHeight-innerHeight), 'WPT chrome arithmetic');
            check(outerWidth-innerWidth === 0 && outerHeight-innerHeight === 0, 'headless chrome');
            for (const descriptor of dimensions) {
                check(descriptor.enumerable && descriptor.configurable && typeof descriptor.get === 'function' && typeof descriptor.set === 'function', 'replaceable descriptor');
                let bad = false;
                try { descriptor.get.call({}); } catch (error) { bad = error instanceof TypeError; }
                check(bad, 'getter receiver brand');
            }
            outerWidth = { author: true };
            outerHeight = -7;
            innerWidth = 1;
            innerHeight = 2;
            check(outerWidth.author && outerHeight === -7, 'author replacements');
            const replacement = Object.getOwnPropertyDescriptor(globalThis, 'outerWidth');
            check(replacement.writable && replacement.enumerable && replacement.configurable && !replacement.get, 'own data property');
            true
        "#,
        );
        state.set_native_layout_viewport(640., 480., true).unwrap();
        evaluate(
            scope,
            r#"
            check(outerWidth.author && outerHeight === -7, 'resize preserves replacements');
            check(dimensions[0].get.call(globalThis) === 640 && dimensions[1].get.call(globalThis) === 480, 'native getter tracks resized viewport');
            Object.defineProperty(globalThis, 'outerWidth', dimensions[0]);
            Object.defineProperty(globalThis, 'outerHeight', dimensions[1]);
            check(outerWidth === 640 && outerHeight === 480, 'restored accessors');
            true
        "#,
        );
        state.retire_execution();
        assert_eq!(outer_size(scope, &state).unwrap(), (0., 0.));
    }
}
