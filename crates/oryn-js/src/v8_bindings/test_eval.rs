//! Test-only JavaScript diagnostics. Failed assertions still fail; retain the
//! actual engine message and source instead of reducing it to Option::unwrap.

#[track_caller]
pub(super) fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, code).expect("test JavaScript source allocation");
    run(scope, source)
}

#[track_caller]
pub(super) fn run<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::String>,
) -> v8::Local<'s, v8::Value> {
    let value = {
        v8::tc_scope!(let caught, scope);
        let result =
            v8::Script::compile(caught, source, None).and_then(|script| script.run(caught));
        match result {
            Some(value) => v8::Global::new(caught, value),
            None => {
                let message = caught
                    .message()
                    .map(|message| {
                        let text = message.get(caught);
                        let line = message.get_line_number(caught).unwrap_or(0);
                        format!("line {line}: {}", text.to_rust_string_lossy(caught))
                    })
                    .unwrap_or_else(|| "no JavaScript message".into());
                let code = source.to_rust_string_lossy(caught);
                panic!(
                    "test JavaScript failed ({message}, terminating={}):\n{code}",
                    caught.has_terminated()
                );
            }
        }
    };
    v8::Local::new(scope, &value)
}

/// Register a test document with the real in-memory message authority.
pub(super) fn register_document(state: &super::V8BindingState, origin: &str) {
    use crate::message_wire::{MessageHub, MessageRequest, MessageResponse, MessageTransport};
    struct Transport {
        hub: std::sync::Mutex<MessageHub>,
        origin: String,
    }
    impl MessageTransport for Transport {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            self.hub.lock().unwrap().handle(
                1,
                request,
                &std::collections::BTreeMap::from([(1, self.origin.clone())]),
            )
        }
    }
    state.set_message_transport(std::sync::Arc::new(Transport {
        hub: std::sync::Mutex::new(MessageHub::default()),
        origin: origin.into(),
    }));
}
