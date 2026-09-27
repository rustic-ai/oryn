//! CLI boundary shared by the selected runtime and its exec-only driver launcher.
use super::{fixture::Fixture, server::Driver, service::Service};
use clap::Args;
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,
    #[arg(long)]
    pub port: u16,
    #[arg(long)]
    pub auth_token_file: PathBuf,
    #[arg(long)]
    pub configuration: PathBuf,
    #[arg(long)]
    pub configuration_sha256: String,
    #[arg(long, default_value_t = 1)]
    pub max_sessions: usize,
    /// Explicit private regular-file diagnostic capability; never discovered from environment.
    #[arg(long)]
    pub diagnostic_fd: Option<i32>,
}
impl ServeArgs {
    pub fn forwarded_args(&self) -> Vec<std::ffi::OsString> {
        let mut args = vec![
            "webdriver".into(),
            "--host".into(),
            self.host.to_string().into(),
            "--port".into(),
            self.port.to_string().into(),
            "--auth-token-file".into(),
            self.auth_token_file.clone().into(),
            "--configuration".into(),
            self.configuration.clone().into(),
            "--configuration-sha256".into(),
            self.configuration_sha256.clone().into(),
            "--max-sessions".into(),
            self.max_sessions.to_string().into(),
        ];
        if let Some(fd) = self.diagnostic_fd {
            args.extend(["--diagnostic-fd".into(), fd.to_string().into()]);
        }
        args
    }
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(fd) = self.diagnostic_fd {
            oryn_native::native_timing::install_inherited(fd)?;
        }
        oryn_native::native_timing::point("runtime_service_entry", serde_json::Value::Null);
        let mut initialization = oryn_native::native_timing::Scope::start(
            "runtime_initialization",
            serde_json::Value::Null,
        );
        let fixture = Fixture::load(&self.configuration, &self.configuration_sha256)?;
        let (browser, options) = fixture.configure(
            oryn_native::Browser::try_with_config(Default::default())?,
            oryn_native::ContextOptions::loopback_test(),
        )?;
        let service =
            Service::bind(SocketAddr::new(self.host, self.port), &self.auth_token_file).await?;
        let mut workers = oryn_native::native_timing::Scope::start(
            "runtime_prepare_workers",
            serde_json::Value::Null,
        );
        browser.prepare_workers()?;
        workers.finish(true);
        let driver = Arc::new(Driver::new(browser, options, self.max_sessions)?);
        initialization.finish(true);
        // This is entry into serving, not proof that a readiness request succeeded.
        oryn_native::native_timing::point("runtime_service_run_entered", serde_json::Value::Null);
        service.run(driver, stop_signal()).await?;
        oryn_native::native_timing::point("runtime_service_returned", serde_json::Value::Null);
        Ok(())
    }
}

async fn stop_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => return,
            };
        tokio::select! { _=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: ServeArgs,
    }
    #[test]
    fn forwarding_preserves_paths_as_single_arguments() {
        use clap::Parser;
        let parsed = Wrapper::try_parse_from([
            "driver",
            "--port",
            "8123",
            "--auth-token-file",
            "/tmp/private token",
            "--configuration",
            "/tmp/fixture with spaces.json",
            "--configuration-sha256",
            &"a".repeat(64),
            "--diagnostic-fd",
            "71",
        ])
        .unwrap();
        let args = parsed.args.forwarded_args();
        assert!(args.contains(&std::ffi::OsString::from("/tmp/private token")));
        assert!(args.contains(&std::ffi::OsString::from("/tmp/fixture with spaces.json")));
        assert_eq!(args[0], "webdriver");
        assert_eq!(
            &args[args.len() - 2..],
            &[
                std::ffi::OsString::from("--diagnostic-fd"),
                std::ffi::OsString::from("71")
            ]
        );
    }
}
