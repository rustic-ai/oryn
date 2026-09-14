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
    let trusted_binary_sha256 = std::env::var("ORYN_TRUSTED_WORKER_SHA256").ok();
    clear_environment();
    if let Some(binary_sha256) = trusted_binary_sha256 {
        crate::worker::initialize_binary_sha256(binary_sha256)?;
    } else {
        crate::worker::precompute_binary_sha256()?;
    }
    #[cfg(target_os = "linux")]
    crate::linux_sandbox::install()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> std::io::Result<()> {
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
