#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    RawV8,
    DenoCore,
}

/// Thread-affine JavaScript host owned by one `PageRuntime`.
///
/// This trait intentionally has no `Send` bound. Typed page handles cross
/// threads; V8 isolates and persistent handles never do.
pub trait JavaScriptHost {
    fn kind(&self) -> HostKind;
    fn execute(&mut self, source: &str) -> Result<(), HostError>;
    fn evaluate_string(&mut self, source: &str) -> Result<String, HostError>;
    fn terminate(&mut self);
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("JavaScript host is not enabled in this build")]
    Disabled,
    #[error("JavaScript execution failed: {0}")]
    Execution(String),
    #[error("JavaScript exceeded the configured {resource} limit")]
    LimitExceeded { resource: String },
}

pub struct DisabledHost;

impl JavaScriptHost for DisabledHost {
    fn kind(&self) -> HostKind {
        HostKind::RawV8
    }

    fn execute(&mut self, _source: &str) -> Result<(), HostError> {
        Err(HostError::Disabled)
    }

    fn evaluate_string(&mut self, _source: &str) -> Result<String, HostError> {
        Err(HostError::Disabled)
    }

    fn terminate(&mut self) {}
}

#[cfg(feature = "v8-host")]
mod raw_v8 {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Once,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use serde::Serialize;
    use url::Url;

    use super::{HostError, HostKind, JavaScriptHost};
    use crate::network::{NetworkRequest, SharedNetworkBroker};

    static INITIALIZE_V8: Once = Once::new();

    pub struct RawV8Host {
        isolate: v8::OwnedIsolate,
        context: v8::Global<v8::Context>,
        execution_wall_limit: Duration,
    }

    struct ExecutionDeadline {
        cancel: Option<mpsc::Sender<()>>,
        expired: Arc<AtomicBool>,
    }

    impl ExecutionDeadline {
        fn arm(isolate: &v8::OwnedIsolate, limit: Duration) -> Self {
            let handle = isolate.thread_safe_handle();
            let (sender, receiver) = mpsc::channel();
            let expired = Arc::new(AtomicBool::new(false));
            let thread_expired = expired.clone();
            std::thread::spawn(move || {
                if receiver.recv_timeout(limit).is_err() {
                    thread_expired.store(true, Ordering::Release);
                    handle.terminate_execution();
                }
            });
            Self {
                cancel: Some(sender),
                expired,
            }
        }

        fn expired(&self) -> bool {
            self.expired.load(Ordering::Acquire)
        }
    }

    impl Drop for ExecutionDeadline {
        fn drop(&mut self) {
            if let Some(sender) = self.cancel.take() {
                let _ = sender.send(());
            }
        }
    }

    #[derive(Clone)]
    struct ModuleSources(BTreeMap<String, String>);

    #[derive(Clone)]
    struct FetchState {
        broker: SharedNetworkBroker,
        base_url: Url,
    }

    #[derive(Serialize)]
    struct FetchResult {
        url: String,
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl RawV8Host {
        pub fn new() -> Self {
            Self::create(None, 256 * 1024 * 1024)
        }

        pub fn new_with_network(broker: SharedNetworkBroker, base_url: Url) -> Self {
            Self::create(Some(FetchState { broker, base_url }), 256 * 1024 * 1024)
        }

        pub fn new_with_network_and_heap_limit(
            broker: SharedNetworkBroker,
            base_url: Url,
            max_heap_bytes: usize,
        ) -> Self {
            Self::create(Some(FetchState { broker, base_url }), max_heap_bytes)
        }

        pub fn new_with_heap_limit(max_heap_bytes: usize) -> Self {
            Self::create(None, max_heap_bytes)
        }

        #[cfg(test)]
        pub fn new_with_wall_limit(execution_wall_limit: Duration) -> Self {
            let mut host = Self::create(None, 256 * 1024 * 1024);
            host.execution_wall_limit = execution_wall_limit;
            host
        }

        fn create(fetch_state: Option<FetchState>, max_heap_bytes: usize) -> Self {
            INITIALIZE_V8.call_once(|| {
                let platform = v8::new_default_platform(0, false).make_shared();
                v8::V8::initialize_platform(platform);
                v8::V8::initialize();
            });
            let mut isolate =
                v8::Isolate::new(v8::CreateParams::default().heap_limits(0, max_heap_bytes));
            if let Some(fetch_state) = fetch_state {
                isolate.set_slot(fetch_state);
            }
            let context = {
                v8::scope!(let scope, &mut isolate);
                let context = v8::Context::new(scope, Default::default());
                if scope.get_slot::<FetchState>().is_some() {
                    let context_scope = &mut v8::ContextScope::new(scope, context);
                    let function = v8::Function::new(context_scope, fetch_callback)
                        .expect("allocate native fetch callback");
                    let name = v8::String::new(context_scope, "__oryn_fetch_sync")
                        .expect("allocate native fetch name");
                    context
                        .global(context_scope)
                        .set(context_scope, name.into(), function.into());
                }
                v8::Global::new(scope, context)
            };
            Self {
                isolate,
                context,
                execution_wall_limit: Duration::from_millis(9_500),
            }
        }

        pub fn execute_module(
            &mut self,
            resource_name: &str,
            source: &str,
            dependencies: &BTreeMap<String, String>,
        ) -> Result<(), HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            self.isolate.set_slot(ModuleSources(dependencies.clone()));
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &self.context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let mut source = module_source(scope, resource_name, source)?;
            let module = v8::script_compiler::compile_module(scope, &mut source)
                .ok_or_else(|| HostError::Execution("module compilation failed".into()))?;
            let instantiated = module
                .instantiate_module(scope, resolve_module)
                .ok_or_else(|| HostError::Execution("module linking failed".into()))?;
            if !instantiated {
                return Err(HostError::Execution("module linking failed".into()));
            }
            if module.evaluate(scope).is_none() {
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                return Err(HostError::Execution("module evaluation failed".into()));
            }
            scope.perform_microtask_checkpoint();
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            Ok(())
        }
    }

    fn fetch_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<String, String> {
            let state = scope
                .get_slot::<FetchState>()
                .cloned()
                .ok_or_else(|| "network broker unavailable".to_string())?;
            let input = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "fetch URL is not string-convertible".to_string())?;
            let method = args
                .get(1)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "GET".into());
            let body = args
                .get(2)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default()
                .into_bytes();
            let header_json = args
                .get(3)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "[]".into());
            let mut request_headers =
                serde_json::from_str::<Vec<(String, String)>>(&header_json).unwrap_or_default();
            let mut url = Url::parse(&input)
                .or_else(|_| state.base_url.join(&input))
                .map_err(|error| format!("invalid fetch URL: {error}"))?;
            let request_origin = origin(&state.base_url);
            for redirect in 0..=10_u64 {
                if !request_headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("origin"))
                {
                    request_headers.push(("origin".into(), request_origin.clone()));
                }
                let response = state
                    .broker
                    .lock()
                    .map_err(|_| "network broker lock poisoned".to_string())?
                    .send(NetworkRequest {
                        request_id: 1_000_000 + redirect,
                        method: method.clone(),
                        url: url.to_string(),
                        headers: request_headers.clone(),
                        body: body.clone(),
                        resolved_addrs: Vec::new(),
                    })
                    .map_err(|error| error.to_string())?;
                if (300..400).contains(&response.status) {
                    let location = response
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                        .map(|(_, value)| value)
                        .ok_or_else(|| "fetch redirect omitted Location".to_string())?;
                    url = url
                        .join(location)
                        .map_err(|error| format!("invalid fetch redirect: {error}"))?;
                    continue;
                }
                if origin(&url) != request_origin {
                    let allowed = response
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"))
                        .map(|(_, value)| value == "*" || value == &request_origin)
                        .unwrap_or(false);
                    if !allowed {
                        return Err("cross-origin fetch denied by CORS".into());
                    }
                }
                return serde_json::to_string(&FetchResult {
                    url: url.to_string(),
                    status: response.status,
                    headers: response.headers,
                    body: String::from_utf8_lossy(&response.body).into_owned(),
                })
                .map_err(|error| error.to_string());
            }
            Err("fetch redirect limit exceeded".into())
        })();
        match result {
            Ok(result) => {
                if let Some(value) = v8::String::new(scope, &result) {
                    return_value.set(value.into());
                }
            }
            Err(error) => {
                if let Some(error) = v8::String::new(scope, &error) {
                    scope.throw_exception(error.into());
                }
            }
        }
    }

    fn origin(url: &Url) -> String {
        let port = url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            port
        )
    }

    fn module_source<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        resource_name: &str,
        source: &str,
    ) -> Result<v8::script_compiler::Source, HostError> {
        let source = v8::String::new(scope, source)
            .ok_or_else(|| HostError::Execution("module source allocation failed".into()))?;
        let name = v8::String::new(scope, resource_name)
            .ok_or_else(|| HostError::Execution("module name allocation failed".into()))?;
        let origin = v8::ScriptOrigin::new(
            scope,
            name.into(),
            0,
            0,
            false,
            -1,
            None,
            false,
            false,
            true,
            None,
        );
        Ok(v8::script_compiler::Source::new(source, Some(&origin)))
    }

    #[allow(clippy::unnecessary_wraps)]
    fn resolve_module<'s>(
        context: v8::Local<'s, v8::Context>,
        specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
        _referrer: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Module>> {
        v8::callback_scope!(unsafe scope, context);
        let specifier = specifier.to_rust_string_lossy(scope);
        let source = scope
            .get_slot::<ModuleSources>()?
            .0
            .get(&specifier)?
            .clone();
        let mut source = module_source(scope, &specifier, &source).ok()?;
        v8::script_compiler::compile_module(scope, &mut source)
    }

    impl Default for RawV8Host {
        fn default() -> Self {
            Self::new()
        }
    }

    impl JavaScriptHost for RawV8Host {
        fn kind(&self) -> HostKind {
            HostKind::RawV8
        }

        fn execute(&mut self, source: &str) -> Result<(), HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &self.context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            let source = v8::String::new(scope, source)
                .ok_or_else(|| HostError::Execution("source allocation failed".into()))?;
            let Some(script) = v8::Script::compile(scope, source, None) else {
                let detail = scope
                    .stack_trace()
                    .or_else(|| scope.exception())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "compilation failed".into());
                return Err(HostError::Execution(detail));
            };
            if script.run(scope).is_none() {
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                let detail = scope
                    .stack_trace()
                    .or_else(|| scope.exception())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "execution failed".into());
                return Err(HostError::Execution(detail));
            }
            scope.perform_microtask_checkpoint();
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            Ok(())
        }

        fn evaluate_string(&mut self, source: &str) -> Result<String, HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &self.context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let source = v8::String::new(scope, source)
                .ok_or_else(|| HostError::Execution("source allocation failed".into()))?;
            let script = v8::Script::compile(scope, source, None)
                .ok_or_else(|| HostError::Execution("compilation failed".into()))?;
            let value = match script.run(scope) {
                Some(value) => value,
                None if _deadline.expired() || scope.is_execution_terminating() => {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                None => return Err(HostError::Execution("execution failed".into())),
            };
            let value = value
                .to_string(scope)
                .ok_or_else(|| HostError::Execution("result is not string-convertible".into()))?;
            Ok(value.to_rust_string_lossy(scope))
        }

        fn terminate(&mut self) {
            self.isolate.terminate_execution();
        }
    }
}

#[cfg(feature = "v8-host")]
pub use raw_v8::RawV8Host;

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use std::time::Duration;

    use super::{HostError, JavaScriptHost, RawV8Host};

    #[test]
    fn v8_watchdog_terminates_execution_before_process_replacement() {
        let mut host = RawV8Host::new_with_wall_limit(Duration::from_millis(25));
        let result = host.evaluate_string("for(;;){}");
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        assert_eq!(host.evaluate_string("1 + 1").unwrap(), "2");
    }
}

#[cfg(feature = "deno-host")]
mod deno {
    use super::{HostError, HostKind, JavaScriptHost};

    pub struct DenoCoreHost {
        runtime: deno_core::JsRuntime,
    }

    impl DenoCoreHost {
        pub fn new() -> Self {
            Self {
                runtime: deno_core::JsRuntime::new(deno_core::RuntimeOptions::default()),
            }
        }
    }

    impl Default for DenoCoreHost {
        fn default() -> Self {
            Self::new()
        }
    }

    impl JavaScriptHost for DenoCoreHost {
        fn kind(&self) -> HostKind {
            HostKind::DenoCore
        }

        fn execute(&mut self, source: &str) -> Result<(), HostError> {
            self.runtime
                .execute_script("oryn:probe", source.to_string())
                .map(|_| ())
                .map_err(|error| HostError::Execution(error.to_string()))
        }

        fn evaluate_string(&mut self, _source: &str) -> Result<String, HostError> {
            Err(HostError::Execution(
                "string evaluation is not implemented by the comparison host".into(),
            ))
        }

        fn terminate(&mut self) {
            self.runtime.v8_isolate().terminate_execution();
        }
    }
}

#[cfg(feature = "deno-host")]
pub use deno::DenoCoreHost;
