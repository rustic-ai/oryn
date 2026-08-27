use std::{
    io::{BufReader, stdin, stdout},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};
#[cfg(unix)]
use std::{os::fd::FromRawFd, os::unix::net::UnixStream};

fn main() -> std::io::Result<()> {
    clear_environment();
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
        println!(
            "{}",
            serde_json::to_string(&oryn_native::runtime_build_info())?
        );
        return Ok(());
    }
    #[cfg(unix)]
    if let Some(fd) = broker_fd()? {
        // SAFETY: the parent duplicates one end of a fresh Unix socketpair to
        // this descriptor immediately before exec and transfers ownership.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        let broker = oryn_native::network::SharedNetworkBroker::new(
            oryn_native::network::IpcNetworkBroker::new(stream),
        );
        return oryn_native::worker::serve_with_broker(
            BufReader::new(stdin().lock()),
            stdout().lock(),
            Some(broker),
        )
        .await;
    }
    oryn_native::worker::serve(BufReader::new(stdin().lock()), stdout().lock()).await
}

fn clear_environment() {
    // The worker receives every capability explicitly over IPC. Clear even
    // variables macOS may inject while starting an App Sandbox executable.
    // This happens before Tokio or V8 can create any background threads.
    let names = std::env::vars_os()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    for name in names {
        // SAFETY: this is the first operation in main, before any thread is
        // created, so no other code can concurrently inspect libc's environ.
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
fn broker_fd() -> std::io::Result<Option<std::os::fd::RawFd>> {
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--broker-fd" {
            let value = args.next().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing broker fd")
            })?;
            return value.parse().map(Some).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid broker fd: {error}"),
                )
            });
        }
    }
    Ok(None)
}
