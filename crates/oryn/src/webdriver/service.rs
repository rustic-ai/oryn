//! Authenticated listener ownership and orderly native shutdown.
use super::{CommandError, auth::DriverAuth, http::load_auth, server::Driver};
use std::{
    future::{Future, IntoFuture},
    net::SocketAddr,
    path::Path,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::oneshot};

/// The listener is bound before authentication is configured so an ephemeral
/// port is authenticated using its actual authority, never the requested port 0.
pub struct Service {
    listener: TcpListener,
    auth: Arc<DriverAuth>,
    address: SocketAddr,
}

impl Service {
    pub async fn bind(address: SocketAddr, credential_file: &Path) -> Result<Self, CommandError> {
        if !address.ip().is_loopback() {
            return Err(CommandError::invalid(
                "driver listener must use a loopback IP",
            ));
        }
        let listener = TcpListener::bind(address).await.map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let auth = Arc::new(load_auth(address, credential_file).map_err(CommandError::invalid)?);
        Ok(Self {
            listener,
            auth,
            address,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Caller retains and awaits this future until cleanup completes. The stop
    /// signal carries no credentials and cannot cancel driver-owned commands.
    pub async fn run(
        self,
        driver: Arc<Driver>,
        stop: impl Future<Output = ()>,
    ) -> Result<(), CommandError> {
        let (stop_http, http_stopped) = oneshot::channel::<()>();
        let serving = axum::serve(self.listener, driver.clone().router(self.auth))
            .with_graceful_shutdown(async {
                let _ = http_stopped.await;
            })
            .into_future();
        tokio::pin!(serving);
        tokio::pin!(stop);
        let (transport, cleanup) = tokio::select! {
            result=&mut serving=>{
                (result,driver.shutdown().await)
            },
            ()=&mut stop=>{
                // Stop accepting first, then revoke session/command admission.
                // Native cleanup and HTTP response completion are both awaited.
                let _=stop_http.send(());
                let cleanup=driver.shutdown().await;
                (serving.await,cleanup)
            },
        };
        combine_results(transport.map_err(io_error), cleanup)
    }
}

fn io_error(error: std::io::Error) -> CommandError {
    CommandError {
        code: "unknown error",
        message: format!("driver transport failed: {error}"),
    }
}

fn combine_results(
    transport: Result<(), CommandError>,
    cleanup: Result<(), CommandError>,
) -> Result<(), CommandError> {
    match (transport, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(transport), Err(cleanup)) => Err(CommandError {
            code: "unknown error",
            message: format!("{}; cleanup: {}", transport.message, cleanup.message),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn actual_bound_authority_authenticates_and_shutdown_releases_listener() {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        struct Credential(std::path::PathBuf);
        impl Drop for Credential {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let credential = Credential(std::env::temp_dir().join(format!("oryn-service-{suffix}")));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&credential.0)
            .unwrap();
        file.write_all("a".repeat(64).as_bytes()).unwrap();
        drop(file);
        let service = Service::bind("127.0.0.1:0".parse().unwrap(), &credential.0)
            .await
            .unwrap();
        let address = service.address();
        assert_ne!(address.port(), 0);
        let driver = Arc::new(
            Driver::new(
                oryn_native::Browser::new(),
                oryn_native::ContextOptions::loopback_test(),
                1,
            )
            .unwrap(),
        );
        let (stop, stopped) = oneshot::channel();
        let serving = tokio::spawn(service.run(driver, async {
            let _ = stopped.await;
        }));
        for (authorization, expected) in [
            (String::new(), "403"),
            (
                format!("Authorization: Bearer {}\r\n", "a".repeat(64)),
                "200",
            ),
        ] {
            let mut connection = tokio::net::TcpStream::connect(address).await.unwrap();
            connection.write_all(format!("GET /status HTTP/1.1\r\nHost: {address}\r\n{authorization}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
            let mut response = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                connection.read_to_string(&mut response),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected}")),
                "{response}"
            );
        }
        stop.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }

    #[tokio::test]
    async fn public_listener_is_rejected_before_opening_credentials() {
        let result = Service::bind(
            "0.0.0.0:0".parse().unwrap(),
            Path::new("missing-token-file"),
        )
        .await;
        assert_eq!(
            result.err().unwrap().message,
            "driver listener must use a loopback IP"
        );
    }

    #[test]
    fn transport_failure_does_not_mask_cleanup_failure() {
        let failure = combine_results(
            Err(CommandError::invalid("transport")),
            Err(CommandError::invalid("retirement")),
        )
        .unwrap_err();
        assert!(failure.message.contains("transport"));
        assert!(failure.message.contains("retirement"));
        assert!(combine_results(Ok(()), Err(CommandError::invalid("retirement"))).is_err());
        assert!(combine_results(Ok(()), Ok(())).is_ok());
    }
}
