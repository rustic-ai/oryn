//! Page-worker process entry shared by the dedicated macOS helper and the
//! Linux multicall production binary.

use std::{
    io::{BufReader, stdin, stdout},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};
#[cfg(unix)]
use std::{os::fd::FromRawFd, os::unix::net::UnixStream};

/// Enter the untrusted page-worker role before any application runtime or
/// background thread is created.
pub fn run() -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(fd) = inherited_fd("--diagnostic-fd")? {
        if fd != 5 {
            return Err(std::io::Error::other(
                "worker diagnostic descriptor must use dedicated slot 5",
            ));
        }
        crate::native_timing::install_inherited(fd)?;
    }
    crate::native_timing::point("worker_process_entry", serde_json::Value::Null);
    let mut timing =
        crate::native_timing::Scope::start("worker_bootstrap", serde_json::Value::Null);
    let trusted_binary_sha256 = std::env::var("ORYN_TRUSTED_WORKER_SHA256").ok();
    clear_environment();
    if let Some(binary_sha256) = trusted_binary_sha256 {
        crate::worker::initialize_binary_sha256(binary_sha256)?;
    } else {
        crate::worker::precompute_binary_sha256()?;
    }
    #[cfg(target_os = "linux")]
    crate::linux_sandbox::install()?;
    // The transform role retains normal identity, environment and sandbox
    // initialization, but never creates Tokio or V8 background threads.
    if xslt_helper_role(std::env::args_os().skip(1))? {
        return crate::xslt_process::serve_helper();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    timing.finish(true);
    let result = runtime.block_on(async_main());
    // A final self sample before returning, not kernel terminal CPU accounting.
    crate::native_timing::point(
        "worker_serve_returned",
        serde_json::json!({"success":result.is_ok()}),
    );
    result
}

async fn async_main() -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(control_fd) = inherited_fd("--agent-control-fd")? {
        let service_fd = inherited_fd("--agent-service-fd")?
            .ok_or_else(|| std::io::Error::other("agent service descriptor required"))?;
        if control_fd < 3 || service_fd < 3 || control_fd == service_fd {
            return Err(std::io::Error::other(
                "agent descriptors must be distinct capability descriptors",
            ));
        }
        // SAFETY: F_GETFD only checks validity and does not assume ownership.
        if unsafe { libc::fcntl(control_fd, libc::F_GETFD) } < 0
            || unsafe { libc::fcntl(service_fd, libc::F_GETFD) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: dedicated socket capabilities are transferred by the verified
        // parent before exec, exactly as for the page-worker broker descriptors.
        let control = unsafe { UnixStream::from_raw_fd(control_fd) };
        let services = unsafe { UnixStream::from_raw_fd(service_fd) };
        return crate::agent_process::serve_child(control, services);
    }
    if let Some(path) = argument_value("--security-probe") {
        security_probe(&path)?;
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--runtime-info") {
        let mut runtime = crate::runtime_build_info();
        runtime.execution_mode = "sandboxed_worker".into();
        runtime.sandboxed = cfg!(any(target_os = "macos", target_os = "linux"));
        runtime.worker_protocol_version = Some(crate::worker::WORKER_PROTOCOL_VERSION);
        runtime.worker_binary_sha256 = Some(crate::worker::current_binary_sha256()?);
        runtime.sandbox_state = if cfg!(target_os = "macos") {
            "app_sandbox".into()
        } else if cfg!(target_os = "linux") {
            "linux_namespaces_landlock_seccomp".into()
        } else {
            "process_isolated_unsandboxed".into()
        };
        println!("{}", serde_json::to_string(&runtime)?);
        return Ok(());
    }
    #[cfg(unix)]
    if let Some(fd) = inherited_fd("--broker-fd")? {
        // SAFETY: the parent duplicates one end of a fresh Unix socketpair to
        // this descriptor immediately before exec and transfers ownership.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        let broker =
            crate::network::SharedNetworkBroker::new(crate::network::IpcNetworkBroker::new(stream));
        let context = if let Some(context_fd) = inherited_fd("--context-fd")? {
            // SAFETY: the descriptor is the context-service socket transferred
            // by the verified parent at exec time.
            let stream = unsafe { UnixStream::from_raw_fd(context_fd) };
            Some(crate::context_service::SharedContextClient::new(
                crate::context_service::IpcContextClient::new(stream),
            ))
        } else {
            None
        };
        return crate::worker::serve_with_services(
            BufReader::new(stdin().lock()),
            stdout().lock(),
            Some(broker),
            context,
        )
        .await;
    }
    crate::worker::serve(BufReader::new(stdin().lock()), stdout().lock()).await
}

fn clear_environment() {
    // The worker receives every capability explicitly over IPC. This must run
    // before Tokio or V8 can create any background threads.
    let names = std::env::vars_os()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    for name in names {
        // SAFETY: worker entry is the first operation in the process role, so
        // no other thread can concurrently inspect libc's environ.
        unsafe { std::env::remove_var(name) };
    }
}

fn argument_value(name: &str) -> Option<String> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments.next();
        }
    }
    None
}

fn security_probe(path: &str) -> std::io::Result<()> {
    let remote = SocketAddr::from(([1, 1, 1, 1], 443));
    let result = serde_json::json!({
        "filesystem_read_succeeded": std::fs::read(Path::new(path)).is_ok(),
        "outgoing_connection_succeeded":
            TcpStream::connect_timeout(&remote, Duration::from_millis(500)).is_ok(),
        "listener_succeeded": TcpListener::bind(("127.0.0.1", 0)).is_ok(),
        "secret_environment_present": ([
            "AZURE_OPENAI_API_KEY",
            "OPENAI_API_KEY",
            "OLLAMA_API_KEY",
            "HOME",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
        ].iter().any(|name| std::env::var_os(name).is_some())),
    });
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(unix)]
fn inherited_fd(name: &str) -> std::io::Result<Option<std::os::fd::RawFd>> {
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == name {
            let value = args.next().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("missing {name} value"),
                )
            })?;
            return value.parse().map(Some).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid inherited fd: {error}"),
                )
            });
        }
    }
    Ok(None)
}

/// A helper receives only its bounded standard streams, never agent/page
/// descriptors or a second process role supplied alongside the helper flag.
fn xslt_helper_role(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> std::io::Result<bool> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let arguments = if arguments
        .first()
        .is_some_and(|value| value == "__oryn-page-worker")
    {
        &arguments[1..]
    } else {
        &arguments[..]
    };
    if !arguments.iter().any(|value| value == "--xslt-helper") {
        return Ok(false);
    }
    if arguments.len() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "XSLT helper role cannot carry page or agent arguments",
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod xslt_role_tests {
    use super::xslt_helper_role;
    fn role(arguments: &[&str]) -> std::io::Result<bool> {
        xslt_helper_role(
            arguments
                .iter()
                .map(|value| std::ffi::OsString::from(*value)),
        )
    }
    #[test]
    fn helper_role_preserves_dedicated_and_linux_multicall_dispatch() {
        assert!(role(&["--xslt-helper"]).unwrap());
        assert!(role(&["__oryn-page-worker", "--xslt-helper"]).unwrap());
        assert!(!role(&["--runtime-info"]).unwrap());
        assert!(!role(&["__oryn-page-worker", "--broker-fd", "3"]).unwrap());
    }
    #[test]
    fn helper_role_rejects_mixed_capabilities_and_duplicate_roles() {
        for arguments in [
            vec!["--xslt-helper", "--agent-control-fd", "3"],
            vec!["--broker-fd", "3", "--xslt-helper"],
            vec!["--xslt-helper", "--runtime-info"],
            vec!["--xslt-helper", "--xslt-helper"],
            vec!["__oryn-page-worker", "--xslt-helper", "--context-fd", "4"],
        ] {
            assert_eq!(
                role(&arguments).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }
}
