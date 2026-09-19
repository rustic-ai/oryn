//! Test-only JavaScript diagnostics. Failed assertions still fail; retain the
//! actual engine message and source instead of reducing it to Option::unwrap.
use super::*;

#[track_caller]
pub(super) fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
    let value = {
        v8::tc_scope!(let caught, scope);
        let source = v8::String::new(caught, code).expect("test JavaScript source allocation");
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
                panic!(
                    "test JavaScript failed ({message}, terminating={}):\n{code}",
                    caught.has_terminated()
                );
            }
        }
    };
    v8::Local::new(scope, &value)
}
