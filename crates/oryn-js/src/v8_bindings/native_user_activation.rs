//! Window user-activation objects read native state and never own grants.
use super::*;

fn private<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, name).unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn illegal(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor");
}
fn read(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let receiver = args.this();
    let snapshot = private(scope, "oryn.native.userActivation.snapshot");
    if let Some(bits) = receiver
        .get_private(scope, snapshot)
        .filter(|value| value.is_int32())
        .and_then(|value| value.int32_value(scope))
    {
        let bit = if args.data().int32_value(scope) == Some(1) {
            2
        } else {
            1
        };
        rv.set_bool(bits & bit != 0);
        return;
    }

    let key = private(scope, "oryn.native.userActivation.owner");
    let owner = receiver
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value());
    let context = receiver.get_creation_context(scope);
    let state = context.and_then(|context| context.get_slot::<V8BindingState>());
    let Some(state) = state.filter(|state| owner == Some((state.instance_id, true))) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let sticky = args.data().int32_value(scope) == Some(1);
    rv.set_bool(if sticky {
        state.has_sticky_user_activation()
    } else {
        state.has_transient_user_activation()
    });
}
fn navigator_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let key = private(scope, "oryn.native.navigator.userActivation");
    let Some(value) = args
        .this()
        .get_private(scope, key)
        .filter(|value| value.is_object())
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("activation owner"))?;
    let key = v8::String::new(scope, "navigator").unwrap();
    let navigator = context
        .global(scope)
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("activation Navigator"))?;
    let constructor =
        v8::Function::new(scope, illegal).ok_or(V8BindingError::Allocation("UserActivation"))?;
    let key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("UserActivation prototype"))?;
    for (name, index) in [("isActive", 0), ("hasBeenActive", 1)] {
        let data = v8::Integer::new(scope, index);
        let getter = v8::Function::builder(read)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("activation getter"))?;
        let undefined = v8::undefined(scope);
        let mut descriptor =
            v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        let key = v8::String::new(scope, name).unwrap();
        prototype.define_property(scope, key.into(), &descriptor);
    }
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "UserActivation").unwrap();
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
    context
        .global(scope)
        .create_data_property(scope, name.into(), constructor.into());
    let saved = private(scope, "oryn.native.userActivation.prototype");
    context
        .global(scope)
        .set_private(scope, saved, prototype.into());
    let object = v8::Object::new(scope);
    object.set_prototype(scope, prototype.into());
    let owner = v8::BigInt::new_from_u64(scope, state.instance_id);
    let key = private(scope, "oryn.native.userActivation.owner");
    object.set_private(scope, key, owner.into());
    let key = private(scope, "oryn.native.navigator.userActivation");
    navigator.set_private(scope, key, object.into());
    let getter = v8::Function::new(scope, navigator_get)
        .ok_or(V8BindingError::Allocation("navigator activation getter"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "userActivation").unwrap();
    navigator.define_property(scope, key.into(), &descriptor);
    Ok(())
}

/// A message carries observation only; this object never grants activation.
pub(super) fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: [bool; 2],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let global = scope.get_current_context().global(scope);
    let key = private(scope, "oryn.native.userActivation.prototype");
    let prototype = if let Some(prototype) = global
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        prototype
    } else {
        let prototype = v8::Object::new(scope);
        for (name, index) in [("isActive", 0), ("hasBeenActive", 1)] {
            let data = v8::Integer::new(scope, index);
            let getter = v8::Function::builder(read)
                .data(data.into())
                .build(scope)
                .ok_or(V8BindingError::Allocation("activation snapshot getter"))?;
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let name = v8::String::new(scope, name)
                .ok_or(V8BindingError::Allocation("activation snapshot member"))?;
            prototype.define_property(scope, name.into(), &descriptor);
        }
        let tag = v8::Symbol::get_to_string_tag(scope);
        let name = v8::String::new(scope, "UserActivation")
            .ok_or(V8BindingError::Allocation("activation snapshot tag"))?;
        prototype.define_own_property(
            scope,
            tag.into(),
            name.into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        );
        global.set_private(scope, key, prototype.into());
        prototype
    };
    let object = v8::Object::new(scope);
    object.set_prototype(scope, prototype.into());
    let bits = v8::Integer::new(scope, i32::from(value[0]) | i32::from(value[1]) * 2);
    let key = private(scope, "oryn.native.userActivation.snapshot");
    object.set_private(scope, key, bits.into());
    Ok(object)
}

#[cfg(test)]
pub(super) mod test_transport {
    use crate::message_wire::{MessageRequest, MessageResponse, MessageTransport};
    pub(crate) struct Messages {
        expected: (u64, u64),
        requests: std::sync::Mutex<Vec<MessageRequest>>,
    }
    impl Messages {
        pub(crate) fn new(generation: u64, epoch: u64) -> Self {
            Self {
                expected: (generation, epoch),
                requests: Default::default(),
            }
        }
        pub(crate) fn assert_lifecycle(&self, retired: usize) {
            let requests = self.requests.lock().unwrap();
            let mut registrations = 0;
            let mut retirements = 0;
            for request in requests.iter() {
                let identity = match request {
                    MessageRequest::RegisterRootDocument {
                        document_generation,
                        stream_epoch,
                    } => {
                        registrations += 1;
                        (*document_generation, *stream_epoch)
                    }
                    MessageRequest::RetireRootDocument {
                        document_generation,
                        stream_epoch,
                    } => {
                        retirements += 1;
                        (*document_generation, *stream_epoch)
                    }
                    other => panic!("unexpected activation fixture request: {other:?}"),
                };
                assert_eq!(identity, self.expected, "activation lifetime changed");
            }
            assert!(registrations > 0, "activation did not register its creator");
            assert_eq!(
                retirements, retired,
                "retirement count before isolate teardown"
            );
        }
    }
    impl MessageTransport for Messages {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            let identity = match &request {
                MessageRequest::RegisterRootDocument {
                    document_generation,
                    stream_epoch,
                }
                | MessageRequest::RetireRootDocument {
                    document_generation,
                    stream_epoch,
                } => Some((*document_generation, *stream_epoch)),
                _ => None,
            };
            self.requests
                .lock()
                .map_err(|_| "activation fixture recorder poisoned")?
                .push(request);
            // Never assert inside a transport called by V8 guaranteed-finalizer
            // teardown: validate its protocol in the test after callbacks finish.
            if identity == Some(self.expected) {
                Ok(MessageResponse::Done)
            } else {
                Err("unexpected activation fixture lifetime request".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Backend(AtomicBool);
    impl crate::user_activation::ActivationBackend for Backend {
        fn notify(&self, _: crate::worker_wire::Creator) -> Result<(), String> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn transient(&self, _: crate::worker_wire::Creator, consume: bool) -> Result<bool, String> {
            Ok(if consume {
                self.0.swap(false, Ordering::SeqCst)
            } else {
                self.0.load(Ordering::SeqCst)
            })
        }
    }
    use super::test_transport::Messages;
    fn check(scope: &mut v8::PinScope, source: &str) {
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
    fn activation_object_is_native_same_object_and_live_across_consumption() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 31).unwrap();
        let messages = Arc::new(Messages::new(31, 0));
        state.set_message_transport(messages.clone());
        state.set_activation_backend(Arc::new(Backend(AtomicBool::new(false))));
        check(
            scope,
            "globalThis.activation=navigator.userActivation; activation===navigator.userActivation && activation instanceof UserActivation && Object.prototype.toString.call(activation)==='[object UserActivation]' && !activation.isActive && !activation.hasBeenActive",
        );
        check(
            scope,
            "dispatchEvent(new Event('click')); !activation.isActive && !activation.hasBeenActive",
        );
        check(
            scope,
            "(() => {try {new UserActivation();return false;} catch(e){return e instanceof TypeError;}})()",
        );
        check(
            scope,
            "(() => {const get=Object.getOwnPropertyDescriptor(UserActivation.prototype,'isActive').get;try {get.call(Object.create(UserActivation.prototype));return false;} catch(e){return e instanceof TypeError;}})()",
        );
        state.notify_user_activation();
        check(scope, "activation.isActive && activation.hasBeenActive");
        assert!(state.consume_transient_user_activation());
        check(scope, "!activation.isActive && activation.hasBeenActive");
        messages.assert_lifecycle(0);
        state.notify_user_activation();
        state.retire_execution();
        check(scope, "!activation.isActive && !activation.hasBeenActive");
        messages.assert_lifecycle(1);
    }
}
