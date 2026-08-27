#![cfg(unix)]

use std::{
    fs,
    os::{
        fd::{AsRawFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BrowserConfig, ContextOptions, RuntimeBuildInfo,
    network::{PolicyBroker, RustlsTransport, serve_ipc_broker},
    runtime::RuntimeError,
    worker::{
        HostEnvelope, WORKER_PROTOCOL_VERSION, WorkerEnvelope, WorkerLimits, WorkerRequest,
        WorkerResponse,
    },
};

const BROKER_FD: RawFd = 3;
const EXPECTED_BUNDLE_ID: &str = "ai.rustic.oryn.page-worker";
const EXPECTED_TEAM_ID: &str = "5HVA9VFF8K";
const EXPECTED_SIGNING_CERT_SHA1: &str = "9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90";
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerVerification {
    pub path: PathBuf,
    pub bundle_id: String,
    pub team_id: String,
    pub signing_cert_sha1: String,
    pub binary_sha256: String,
    pub manifest_sha256: String,
    pub app_sandbox: bool,
    pub allow_jit: bool,
    pub hardened_runtime: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkerManifest {
    schema_version: u32,
    bundle_id: String,
    team_id: String,
    signing_cert_sha1: String,
    protocol_version: u32,
    v8_version: String,
    worker_build_sha256: String,
    worker_build_bytes: u64,
    entitlement_sha256: String,
    policy_sha256: String,
    source_tree_sha256: String,
    limits: WorkerLimits,
}

pub struct WorkerClient {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    response_buffer: Vec<u8>,
    session_id: u64,
    next_message_id: u64,
    limits: WorkerLimits,
    runtime: RuntimeBuildInfo,
    binary_sha256: String,
    verification: Option<WorkerVerification>,
}

impl WorkerClient {
    pub fn spawn(
        config: &BrowserConfig,
        context: &ContextOptions,
        parent_cookies: std::sync::Arc<reqwest::cookie::Jar>,
    ) -> Result<Self, RuntimeError> {
        let worker_path = discover_worker()?;
        let verification = if requires_trusted_worker() {
            Some(verify_worker(&worker_path)?)
        } else {
            None
        };
        let launch_path = verification
            .as_ref()
            .map(|item| item.path.as_path())
            .unwrap_or(worker_path.as_path());

        let (parent_broker, child_broker) = UnixStream::pair()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let child_fd = child_broker.as_raw_fd();
        let mut command = Command::new(launch_path);
        command
            .arg("--broker-fd")
            .arg(BROKER_FD.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env_clear();
        // SAFETY: this closure runs after fork and before exec, and only
        // duplicates an already-open socket descriptor to the agreed slot.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(child_fd, BROKER_FD) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        drop(child_broker);

        let transport = RustlsTransport::with_cookie_jar(
            config.request_timeout,
            config.max_response_bytes.min(16 * 1024 * 1024),
            parent_cookies,
        )
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let policy = context.network_policy.clone();
        let frame_limit = config.max_ipc_frame_bytes;
        std::thread::Builder::new()
            .name("oryn-worker-network-broker".into())
            .spawn(move || {
                let _ = serve_ipc_broker(
                    parent_broker,
                    PolicyBroker::new(policy, transport),
                    frame_limit,
                );
            })
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;

        let input = child.stdin.take().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("worker stdin was not created".into())
        })?;
        set_nonblocking(input.as_raw_fd())
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let output = child.stdout.take().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("worker stdout was not created".into())
        })?;
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let limits = WorkerLimits::from(config);
        let nonce = handshake_nonce(session_id);
        let mut client = Self {
            child,
            input,
            output,
            response_buffer: Vec::new(),
            session_id,
            next_message_id: 1,
            limits: limits.clone(),
            runtime: crate::runtime_build_info(),
            binary_sha256: String::new(),
            verification,
        };
        let response = client.request_with_timeout(
            WorkerRequest::Initialize {
                nonce: nonce.clone(),
                limits: limits.clone(),
                context: context.clone(),
                document_generation_base: config.document_generation_base,
            },
            config.command_timeout,
        )?;
        let WorkerResponse::Ready {
            nonce: returned_nonce,
            protocol_version,
            runtime,
            binary_sha256,
            policy_digest,
            sandbox_state,
            secret_environment_present,
            limits: effective_limits,
        } = response
        else {
            client.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker did not return an initialization response".into(),
            ));
        };
        let runtime = *runtime;
        let verified_hash = client
            .verification
            .as_ref()
            .map(|item| item.binary_sha256.as_str())
            .unwrap_or(binary_sha256.as_str());
        if returned_nonce != nonce
            || protocol_version != WORKER_PROTOCOL_VERSION
            || !runtime.native_v8
            || runtime.v8_version.is_none()
            || effective_limits != limits
            || binary_sha256 != verified_hash
            || policy_digest != crate::worker::policy_digest(context)
            || sandbox_state
                != if cfg!(target_os = "macos") {
                    "app_sandbox"
                } else {
                    "process_isolated_unsandboxed"
                }
            || secret_environment_present
        {
            client.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker handshake metadata did not match the verified launch".into(),
            ));
        }
        if requires_trusted_worker() && runtime.build_profile != "release" {
            client.kill();
            return Err(RuntimeError::SandboxUnavailable(
                "production worker is not a release build".into(),
            ));
        }
        client.runtime = runtime;
        client.runtime.policy_digest = Some(policy_digest);
        client.runtime.sandbox_state = sandbox_state;
        client.binary_sha256 = binary_sha256;
        Ok(client)
    }

    pub fn request(&mut self, request: WorkerRequest) -> Result<WorkerResponse, RuntimeError> {
        let timeout = match request {
            WorkerRequest::Goto { .. }
            | WorkerRequest::Back
            | WorkerRequest::Forward
            | WorkerRequest::Refresh
            | WorkerRequest::LoadHtml { .. } => {
                Duration::from_millis(self.limits.navigation_wall_ms)
            }
            _ => Duration::from_millis(self.limits.command_wall_ms),
        };
        self.request_with_timeout(request, timeout)
    }

    pub fn runtime(&self) -> &RuntimeBuildInfo {
        &self.runtime
    }

    pub fn verification(&self) -> Option<&WorkerVerification> {
        self.verification.as_ref()
    }

    fn request_with_timeout(
        &mut self,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let message_id = self.next_message_id;
        self.next_message_id += 1;
        let envelope = HostEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            session_id: self.session_id,
            message_id,
            command: request,
        };
        let encoded = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if encoded.len() > self.limits.ipc_frame_bytes {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "IPC frame".into(),
            });
        }
        self.write_frame(&encoded, timeout)?;

        let response = self.read_frame(timeout)?;
        let response: WorkerEnvelope = serde_json::from_slice(&response)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != WORKER_PROTOCOL_VERSION
            || response.session_id != self.session_id
            || response.correlation_id != message_id
            || response.message_id != message_id
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker response correlation mismatch".into(),
            ));
        }
        match response.response {
            WorkerResponse::Error { error } => Err(*error),
            WorkerResponse::LimitExceeded { resource } => {
                self.kill();
                Err(RuntimeError::WorkerLimitExceeded { resource })
            }
            response => Ok(response),
        }
    }

    fn write_frame(&mut self, encoded: &[u8], timeout: Duration) -> Result<(), RuntimeError> {
        let mut frame = Vec::with_capacity(encoded.len() + 1);
        frame.extend_from_slice(encoded);
        frame.push(b'\n');
        let started = Instant::now();
        let mut written = 0;
        while written < frame.len() {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
            {
                return Err(RuntimeError::WorkerCrashed(format!(
                    "worker exited with {status}"
                )));
            }
            if resident_bytes(self.child.id()).is_some_and(|rss| rss > self.limits.worker_rss_bytes)
            {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "RSS".into(),
                });
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "IPC write wall time".into(),
                });
            }
            let poll_ms = remaining.min(Duration::from_millis(250)).as_millis() as i32;
            let mut descriptor = libc::pollfd {
                fd: self.input.as_raw_fd(),
                events: libc::POLLOUT | libc::POLLHUP,
                revents: 0,
            };
            // SAFETY: descriptor points to one initialized pollfd and stdin is
            // kept alive by self.input for the duration of the call.
            let result = unsafe { libc::poll(&mut descriptor, 1, poll_ms) };
            if result > 0 {
                // SAFETY: frame is a live byte slice and the descriptor is a
                // nonblocking pipe owned by self.input.
                let count = unsafe {
                    libc::write(
                        self.input.as_raw_fd(),
                        frame[written..].as_ptr().cast(),
                        frame.len() - written,
                    )
                };
                if count > 0 {
                    written += count as usize;
                    continue;
                }
                if count == 0 {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(
                        "worker stopped accepting requests".into(),
                    ));
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted
                    && error.kind() != std::io::ErrorKind::WouldBlock
                {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            } else if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
        }
        Ok(())
    }

    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, RuntimeError> {
        let started = Instant::now();
        loop {
            if let Some(newline) = self.response_buffer.iter().position(|byte| *byte == b'\n') {
                return Ok(self.response_buffer.drain(..=newline).collect());
            }
            if self.response_buffer.len() > self.limits.ipc_frame_bytes {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "IPC frame".into(),
                });
            }
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
            {
                return Err(RuntimeError::WorkerCrashed(format!(
                    "worker exited with {status}"
                )));
            }
            if resident_bytes(self.child.id()).is_some_and(|rss| rss > self.limits.worker_rss_bytes)
            {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "RSS".into(),
                });
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let poll_ms = remaining.min(Duration::from_millis(250)).as_millis() as i32;
            let mut descriptor = libc::pollfd {
                fd: self.output.as_raw_fd(),
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            // SAFETY: descriptor points to one initialized pollfd and the
            // child stdout descriptor remains owned by self.output.
            let result = unsafe { libc::poll(&mut descriptor, 1, poll_ms) };
            if result > 0 {
                let mut chunk = [0_u8; 8192];
                // SAFETY: chunk is a live writable byte buffer and the
                // descriptor remains owned by self.output.
                let read = unsafe {
                    libc::read(
                        self.output.as_raw_fd(),
                        chunk.as_mut_ptr().cast(),
                        chunk.len(),
                    )
                };
                if read > 0 {
                    self.response_buffer
                        .extend_from_slice(&chunk[..read as usize]);
                    continue;
                }
                if read == 0 {
                    return Err(RuntimeError::WorkerCrashed(
                        "worker closed its response stream".into(),
                    ));
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted
                    && error.kind() != std::io::ErrorKind::WouldBlock
                {
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn runtime_report(
    config: &BrowserConfig,
    context: &ContextOptions,
) -> Result<RuntimeBuildInfo, RuntimeError> {
    let client = WorkerClient::spawn(
        config,
        context,
        std::sync::Arc::new(reqwest::cookie::Jar::default()),
    )?;
    let mut runtime = client.runtime().clone();
    runtime.execution_mode = if client.verification().is_some() {
        "sandboxed_worker".into()
    } else {
        "process_isolated_unsandboxed".into()
    };
    runtime.sandboxed = client.verification().is_some();
    runtime.sandbox_state = if runtime.sandboxed {
        "app_sandbox".into()
    } else {
        "process_isolated_unsandboxed".into()
    };
    runtime.worker_protocol_version = Some(WORKER_PROTOCOL_VERSION);
    runtime.worker_binary_sha256 = Some(client.binary_sha256.clone());
    runtime.worker_bundle_id = client.verification().map(|item| item.bundle_id.clone());
    runtime.worker_team_id = client.verification().map(|item| item.team_id.clone());
    runtime.worker_signing_cert_sha1 = client
        .verification()
        .map(|item| item.signing_cert_sha1.clone());
    runtime.limits = Some(client.limits.clone());
    Ok(runtime)
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.request_with_timeout(WorkerRequest::Exit, Duration::from_secs(1));
        }
        self.kill();
    }
}

pub fn discover_worker() -> Result<PathBuf, RuntimeError> {
    if let Some(path) = std::env::var_os("ORYN_PAGE_WORKER") {
        return Ok(PathBuf::from(path));
    }
    let parent = std::env::current_exe()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent executable has no directory".into())
        })?;
    #[cfg(target_os = "macos")]
    {
        Ok(parent.join("OrynPageWorker.app/Contents/MacOS/oryn-page-worker"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(parent.join("oryn-page-worker"))
    }
}

pub fn verify_worker(path: &Path) -> Result<WorkerVerification, RuntimeError> {
    if !path.is_file() {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker does not exist: {}",
            path.display()
        )));
    }
    let path = fs::canonicalize(path)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let app = containing_app(&path)?;
    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(&app)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    if !verify.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            String::from_utf8_lossy(&verify.stderr).trim().to_string(),
        ));
    }
    let details = Command::new("/usr/bin/codesign")
        .args(["-dvvv"])
        .arg(&path)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let details = String::from_utf8_lossy(&details.stderr);
    let bundle_id = detail_value(&details, "Identifier=").unwrap_or_default();
    let team_id = detail_value(&details, "TeamIdentifier=").unwrap_or_default();
    if bundle_id != EXPECTED_BUNDLE_ID || team_id != EXPECTED_TEAM_ID {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker identity mismatch: bundle={bundle_id}, team={team_id}"
        )));
    }
    if !details.contains("Authority=Apple Development:")
        || !details.contains("flags=0x10000(runtime)")
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker lacks the trusted Apple Development chain or hardened runtime".into(),
        ));
    }
    let signing_cert_sha1 = signing_certificate_sha1(&path)?;
    if signing_cert_sha1 != EXPECTED_SIGNING_CERT_SHA1 {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker signing certificate mismatch: {signing_cert_sha1}"
        )));
    }
    let entitlements = Command::new("/usr/bin/codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(&path)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let entitlements = String::from_utf8_lossy(&entitlements.stdout);
    let expected = std::collections::BTreeSet::from([
        "com.apple.security.app-sandbox".to_string(),
        "com.apple.security.cs.allow-jit".to_string(),
    ]);
    if entitlement_keys(&entitlements) != expected
        || expected
            .iter()
            .any(|key| !entitlement_is_true(&entitlements, key))
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker entitlement set is not exactly app-sandbox plus allow-jit".into(),
        ));
    }

    let manifest_path = app.join("Contents/Resources/worker-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let manifest: WorkerManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let binary_sha256 = sha256_file(&path)?;
    if manifest.schema_version != 1
        || manifest.bundle_id != EXPECTED_BUNDLE_ID
        || manifest.team_id != EXPECTED_TEAM_ID
        || manifest.signing_cert_sha1 != EXPECTED_SIGNING_CERT_SHA1
        || manifest.protocol_version != WORKER_PROTOCOL_VERSION
        || manifest.limits != WorkerLimits::default()
        || manifest.v8_version.is_empty()
        || manifest.worker_build_sha256.len() != 64
        || manifest.worker_build_bytes == 0
        || manifest.entitlement_sha256 != expected_entitlement_sha256()
        || manifest.policy_sha256 != compiled_policy_sha256()
        || manifest.source_tree_sha256.len() != 64
    {
        return Err(RuntimeError::SandboxUnavailable(
            "signed worker manifest does not match the executable".into(),
        ));
    }
    Ok(WorkerVerification {
        path,
        bundle_id,
        team_id,
        signing_cert_sha1,
        binary_sha256,
        manifest_sha256: format!("{:x}", Sha256::digest(manifest_bytes)),
        app_sandbox: true,
        allow_jit: true,
        hardened_runtime: true,
    })
}

fn containing_app(path: &Path) -> Result<PathBuf, RuntimeError> {
    let macos = path.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    let contents = macos.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    let app = contents.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    if path.file_name().and_then(|name| name.to_str()) != Some("oryn-page-worker")
        || macos.file_name().and_then(|name| name.to_str()) != Some("MacOS")
        || contents.file_name().and_then(|name| name.to_str()) != Some("Contents")
        || app.extension().and_then(|value| value.to_str()) != Some("app")
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker is not at OrynPageWorker.app/Contents/MacOS/oryn-page-worker".into(),
        ));
    }
    Ok(app.to_path_buf())
}

fn requires_trusted_worker() -> bool {
    cfg!(target_os = "macos")
}

fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: fd is owned by the live ChildStdin passed by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: F_SETFL updates only descriptor status flags for the same live
    // pipe descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn detail_value(details: &str, prefix: &str) -> Option<String> {
    details
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn entitlement_keys(xml: &str) -> std::collections::BTreeSet<String> {
    xml.split("<key>")
        .skip(1)
        .filter_map(|part| part.split_once("</key>").map(|(key, _)| key.to_string()))
        .collect()
}

fn entitlement_is_true(xml: &str, key: &str) -> bool {
    xml.split_once(&format!("<key>{key}</key>"))
        .map(|(_, remainder)| {
            let value = remainder.split("<key>").next().unwrap_or(remainder);
            value.contains("<true/>") || value.contains("<true />")
        })
        .unwrap_or(false)
}

fn expected_entitlement_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            br#"{"com.apple.security.app-sandbox":true,"com.apple.security.cs.allow-jit":true}"#
        )
    )
}

fn compiled_policy_sha256() -> String {
    let mut digest = Sha256::new();
    digest.update(b"security.rs\0");
    digest.update(include_bytes!("security.rs"));
    digest.update(b"\0network.rs\0");
    digest.update(include_bytes!("network.rs"));
    format!("{:x}", digest.finalize())
}

fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let bytes =
        fs::read(path).map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn signing_certificate_sha1(path: &Path) -> Result<String, RuntimeError> {
    use sha1::{Digest, Sha1};

    let directory = std::env::temp_dir().join(format!(
        "oryn-worker-cert-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&directory)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let prefix = directory.join("certificate-");
    let extraction = Command::new("/usr/bin/codesign")
        .arg("-d")
        .arg(format!("--extract-certificates={}", prefix.display()))
        .arg(path)
        .output();
    let extraction = match extraction {
        Ok(extraction) => extraction,
        Err(error) => {
            let _ = fs::remove_dir_all(&directory);
            return Err(RuntimeError::SandboxUnavailable(error.to_string()));
        }
    };
    let leaf_path = PathBuf::from(format!("{}0", prefix.display()));
    let leaf = fs::read(&leaf_path);
    let trust = Command::new("/usr/bin/security")
        .args(["verify-cert", "-c"])
        .arg(&leaf_path)
        .args(["-p", "codeSign"])
        .output();
    let _ = fs::remove_dir_all(&directory);
    if !extraction.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            "unable to extract worker signing certificate".into(),
        ));
    }
    let leaf = leaf.map_err(|error| {
        RuntimeError::SandboxUnavailable(format!(
            "unable to read extracted signing certificate: {error}"
        ))
    })?;
    let trust = trust.map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    if !trust.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            "worker signing certificate is not trusted for code signing".into(),
        ));
    }
    Ok(format!("{:X}", Sha1::digest(leaf)))
}

fn handshake_nonce(session_id: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{session_id}-{nanos}", std::process::id())
}

fn resident_bytes(pid: u32) -> Option<usize> {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kib = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .ok()?;
    Some(kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_codesign_detail_values() {
        let details = "Identifier=ai.rustic.oryn.page-worker\nTeamIdentifier=5HVA9VFF8K\n";
        assert_eq!(
            detail_value(details, "Identifier=").as_deref(),
            Some(EXPECTED_BUNDLE_ID)
        );
        assert_eq!(
            detail_value(details, "TeamIdentifier=").as_deref(),
            Some(EXPECTED_TEAM_ID)
        );
    }

    #[test]
    fn entitlement_parser_requires_exact_true_keys() {
        let xml = "<dict><key>com.apple.security.app-sandbox</key><true/><key>com.apple.security.cs.allow-jit</key><true/></dict>";
        assert_eq!(entitlement_keys(xml).len(), 2);
        assert!(entitlement_is_true(xml, "com.apple.security.app-sandbox"));
        assert!(!entitlement_is_true(
            xml,
            "com.apple.security.network.client"
        ));
        assert_eq!(expected_entitlement_sha256().len(), 64);
        assert_eq!(compiled_policy_sha256().len(), 64);
    }

    #[test]
    fn worker_path_must_have_the_expected_bundle_layout() {
        let valid = Path::new("/tmp/OrynPageWorker.app/Contents/MacOS/oryn-page-worker");
        assert_eq!(
            containing_app(valid).unwrap(),
            PathBuf::from("/tmp/OrynPageWorker.app")
        );
        assert!(containing_app(Path::new("/tmp/oryn-page-worker")).is_err());
    }
}
