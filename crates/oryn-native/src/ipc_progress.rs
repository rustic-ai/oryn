//! Bounded, payload-free IPC progress breadcrumbs. Never protocol or authority.
//!
//! A regular-file stderr receives one unbuffered write per event. Pipes/terminals
//! are deliberately excluded: a diagnostic consumer must not backpressure IPC.
//! Missing/truncated events never imply completion. Identity is captured before
//! queued work; resource lanes must not look up whichever command runs later.
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
#[cfg(any(unix, test))]
use std::time::Instant;

const MAX_ACTIVE: usize = 256;
#[cfg(any(unix, test))]
const MAX_EVENTS: u64 = 16_384;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub(crate) struct Identity {
    process_session: u64,
    page_id: u64,
    route_generation: u64,
    message_id: Option<u64>,
    command: Option<&'static str>,
    context_id: Option<u64>,
    peer_pid: Option<u32>,
    association: &'static str,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    #[cfg(unix)]
    RequestSend,
    #[cfg(unix)]
    ContextBound,
    #[cfg(unix)]
    RequestWritten,
    #[cfg(unix)]
    ResponseRead,
    #[cfg(unix)]
    ResponseReadFailed,
    #[cfg(unix)]
    TransportRetirementBegin,
    #[cfg(unix)]
    TransportRetirementReturned,
    RequestReceived,
    RequestReturned,
    ResponseWritten,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameQueued,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameStarted,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameHeadersReturned,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameCompletionSent,
    #[cfg(unix)]
    FramePolicyBegin,
    #[cfg(unix)]
    FramePolicyReturned,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameBodyQueued,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameBodyStarted,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameBodyReturned,
    #[cfg(all(unix, feature = "v8-host"))]
    FrameBodyDelivered,
    #[cfg(unix)]
    ParentBodyBegin,
    #[cfg(unix)]
    ParentBodyReturned,
    #[cfg(unix)]
    ParentBodyLockAcquired,
    #[cfg(unix)]
    ParentBodyGrant,
    #[cfg(unix)]
    ParentChunkReceived,
    #[cfg(all(unix, feature = "v8-host"))]
    FramePolicyBound,
    #[cfg(all(unix, feature = "v8-host"))]
    DocumentCommitted,
    #[cfg(all(unix, feature = "v8-host"))]
    NavigationLayoutCompleted,
    #[cfg(all(unix, feature = "v8-host"))]
    NavigationParserSettled,
    #[cfg(all(unix, feature = "v8-host"))]
    NavigationLoadReady,
    #[cfg(all(unix, feature = "v8-host"))]
    ActionNavigationReleased,
    #[cfg(all(unix, feature = "v8-host"))]
    ActionSettled,
    #[cfg(all(unix, feature = "v8-host"))]
    CloseActorEntered,
    #[cfg(all(unix, feature = "v8-host"))]
    CloseRealmsPending,
    #[cfg(all(unix, feature = "v8-host"))]
    CloseDocumentsPending,
    #[cfg(all(unix, feature = "v8-host"))]
    CloseNativeCleanupPending,
    #[cfg(all(unix, feature = "v8-host"))]
    CloseActorReleased,
    #[cfg(unix)]
    CloseAgentsSettled,
    #[cfg(any(unix, test))]
    Truncated,
}

#[derive(Default)]
struct Registry {
    active: BTreeMap<u64, Identity>,
}
impl Registry {
    fn insert(&mut self, id: Identity) -> bool {
        if self.active.len() >= MAX_ACTIVE || self.active.contains_key(&id.page_id) {
            return false;
        }
        self.active.insert(id.page_id, id);
        true
    }
    fn remove(&mut self, id: Identity) {
        if self.active.get(&id.page_id) == Some(&id) {
            self.active.remove(&id.page_id);
        }
    }
}
fn registry() -> &'static Mutex<Registry> {
    static VALUE: OnceLock<Mutex<Registry>> = OnceLock::new();
    VALUE.get_or_init(Default::default)
}
pub(crate) struct RequestScope {
    identity: Identity,
    registered: bool,
}
impl RequestScope {
    pub(crate) fn identity(&self) -> Option<Identity> {
        Some(self.identity)
    }
}
impl Drop for RequestScope {
    fn drop(&mut self) {
        if self.registered {
            registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(self.identity);
        }
    }
}
pub(crate) fn request(
    route: &crate::worker_route::WorkerRoute,
    message_id: u64,
    command: &crate::worker::WorkerRequest,
) -> RequestScope {
    let mut command = command;
    while let crate::worker::WorkerRequest::Scoped { request, .. } = command {
        command = request;
    }
    use crate::worker::WorkerRequest;
    let command = match command {
        WorkerRequest::Goto { .. } => "goto",
        WorkerRequest::GotoResource { .. } => "goto_resource",
        WorkerRequest::ControlledNavigate { .. } => "controlled_navigation",
        WorkerRequest::Execute { .. } => "execute",
        WorkerRequest::ExecuteBatch { .. } => "execute_batch",
        WorkerRequest::Evaluate { .. } => "evaluate",
        WorkerRequest::Close => "close",
        WorkerRequest::Oil { .. } => "oil",
        WorkerRequest::Initialize { .. } => "initialize",
        WorkerRequest::ResolveTarget { .. } => "resolve_target",
        WorkerRequest::Observe { .. } => "observe",
        WorkerRequest::ObserveWith { .. } => "observe_with",
        WorkerRequest::ObserveSemantic { .. } => "observe_semantic",
        WorkerRequest::ObserveSemanticV6 { .. } => "observe_semantic_v6",
        WorkerRequest::ObserveDelta { .. } => "observe_delta",
        WorkerRequest::ObserveSemanticDelta { .. } => "observe_semantic_delta",
        _ => "other",
    };
    let identity = Identity {
        process_session: route.process_session,
        page_id: route.environment,
        route_generation: route.generation,
        message_id: Some(message_id),
        command: Some(command),
        context_id: None,
        peer_pid: None,
        association: "request",
    };
    let registered = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(identity);
    RequestScope {
        identity,
        registered,
    }
}
#[cfg(unix)]
pub(crate) fn bind_context(
    route: &crate::worker_route::WorkerRoute,
    context_id: u64,
    peer_pid: u32,
) {
    let identity = Identity {
        process_session: route.process_session,
        page_id: route.environment,
        route_generation: route.generation,
        message_id: None,
        command: None,
        context_id: Some(context_id),
        peer_pid: Some(peer_pid),
        association: "parent_context_binding",
    };
    record(Some(identity), Phase::ContextBound, None, None);
}
#[cfg(unix)]
pub(crate) fn capture(page_id: u64) -> Option<Identity> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active
        .get(&page_id)
        .copied()
        .map(|mut id| {
            id.association = "active_at_capture";
            id
        })
}
/// Parent receipt observes an active request; stream ID joins determine whether
/// it belongs to that request. Background work may have been queued earlier.
#[cfg(unix)]
pub(crate) fn observed(page_id: u64) -> Option<Identity> {
    capture(page_id).map(|mut id| {
        id.association = "active_at_parent_receipt";
        id
    })
}
#[cfg(any(unix, test))]
#[derive(serde::Serialize)]
struct Event {
    kind: &'static str,
    version: u8,
    pid: u32,
    sequence: u64,
    elapsed_ns: u128,
    unix_time_ns: Option<u128>,
    identity: Identity,
    phase: Phase,
    stream_id: Option<u64>,
    document_generation: Option<u64>,
    success: Option<bool>,
}
#[cfg(any(unix, test))]
#[derive(Default)]
struct Sink {
    sequence: u64,
}
#[cfg(any(unix, test))]
impl Sink {
    fn encode(
        &mut self,
        identity: Identity,
        phase: Phase,
        stream_id: Option<u64>,
        document_generation: Option<u64>,
        success: Option<bool>,
    ) -> Option<Vec<u8>> {
        if self.sequence > MAX_EVENTS {
            return None;
        }
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        let event = Event {
            kind: "oryn_ipc_progress",
            version: 1,
            pid: std::process::id(),
            sequence: self.sequence,
            elapsed_ns: ORIGIN.get_or_init(Instant::now).elapsed().as_nanos(),
            unix_time_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|value| value.as_nanos()),
            identity,
            phase: if self.sequence == MAX_EVENTS {
                Phase::Truncated
            } else {
                phase
            },
            stream_id,
            document_generation,
            success: if self.sequence == MAX_EVENTS {
                None
            } else {
                success
            },
        };
        self.sequence += 1;
        let mut bytes = serde_json::to_vec(&event).ok()?;
        bytes.push(b'\n');
        (bytes.len() <= 1024).then_some(bytes)
    }
}
pub(crate) fn record(
    identity: Option<Identity>,
    phase: Phase,
    stream_id: Option<u64>,
    document_generation: Option<u64>,
) {
    result(identity, phase, stream_id, document_generation, None);
}
// Mirror only low-frequency close observations into the existing private FD.
// The original captured identity is never replaced by the current request.
#[cfg(unix)]
fn close_timing_phase(phase: Phase) -> Option<&'static str> {
    match phase {
        #[cfg(feature = "v8-host")]
        Phase::CloseActorEntered => Some("close_actor_entered"),
        #[cfg(feature = "v8-host")]
        Phase::CloseRealmsPending => Some("close_realms_pending"),
        #[cfg(feature = "v8-host")]
        Phase::CloseDocumentsPending => Some("close_documents_pending"),
        #[cfg(feature = "v8-host")]
        Phase::CloseNativeCleanupPending => Some("close_native_cleanup_pending"),
        #[cfg(feature = "v8-host")]
        Phase::CloseActorReleased => Some("close_actor_released"),
        Phase::CloseAgentsSettled => Some("close_agents_settled"),
        _ => None,
    }
}

#[cfg(unix)]
fn close_timing_identity(
    identity: Identity,
    stream_id: Option<u64>,
    document_generation: Option<u64>,
    success: Option<bool>,
) -> serde_json::Value {
    // Keep the existing flat IPC identity keys for exact request/route joins.
    let mut value = serde_json::json!(identity);
    value["stream_id"] = serde_json::json!(stream_id);
    value["document_generation"] = serde_json::json!(document_generation);
    value["observed_success"] = serde_json::json!(success);
    value
}

pub(crate) fn result(
    identity: Option<Identity>,
    phase: Phase,
    stream_id: Option<u64>,
    document_generation: Option<u64>,
    success: Option<bool>,
) {
    let Some(identity) = identity else {
        return;
    };
    // No new descriptor, filesystem path, environment grant, or broker capability.
    #[cfg(unix)]
    {
        if crate::native_timing::enabled()
            && let Some(label) = close_timing_phase(phase)
        {
            // point() uses the existing non-waiting, capped file collector. It
            // never routes back into IPC progress or changes close ownership.
            crate::native_timing::point(
                label,
                close_timing_identity(identity, stream_id, document_generation, success),
            );
        }
        if !regular_file(libc::STDERR_FILENO) {
            return;
        }
        static SINK: Mutex<Sink> = Mutex::new(Sink { sequence: 0 });
        let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(bytes) = sink.encode(identity, phase, stream_id, document_generation, success) {
            let _ = write_once(libc::STDERR_FILENO, &bytes);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (identity, phase, stream_id, document_generation, success);
    }
}

#[cfg(unix)]
fn regular_file(fd: libc::c_int) -> bool {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat initializes metadata only on success; descriptor is borrowed.
    if unsafe { libc::fstat(fd, metadata.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: successful fstat initialized the complete stat value.
    let metadata = unsafe { metadata.assume_init() };
    metadata.st_mode & libc::S_IFMT == libc::S_IFREG
}
#[cfg(unix)]
fn write_once(fd: libc::c_int, bytes: &[u8]) -> bool {
    // SAFETY: bytes remains live; caller retains the descriptor. One bounded
    // write, no retry or user-space buffer. A partial write is missing evidence.
    unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) == bytes.len() as isize }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(page: u64, message: u64) -> Identity {
        Identity {
            process_session: 7,
            page_id: page,
            route_generation: 9,
            message_id: Some(message),
            command: Some("execute"),
            context_id: None,
            peer_pid: None,
            association: "request",
        }
    }
    #[test]
    fn captured_work_keeps_original_command_after_replacement() {
        let mut registry = Registry::default();
        assert!(registry.insert(id(1, 1)));
        let captured = registry.active[&1];
        registry.remove(id(1, 1));
        assert!(registry.insert(id(1, 2)));
        assert_eq!(captured.message_id, Some(1));
        registry.remove(captured);
        assert_eq!(registry.active[&1].message_id, Some(2));
    }
    #[test]
    fn registry_is_bounded_and_never_overwrites_live_identity() {
        let mut registry = Registry::default();
        for page in 0..MAX_ACTIVE as u64 {
            assert!(registry.insert(id(page, 1)));
        }
        assert!(!registry.insert(id(0, 2)));
        assert!(!registry.insert(id(MAX_ACTIVE as u64, 1)));
        registry.remove(id(0, 1));
        assert!(registry.insert(id(MAX_ACTIVE as u64, 1)));
    }
    #[test]
    fn sink_marks_truncation_once_and_does_not_grow() {
        let mut sink = Sink {
            sequence: MAX_EVENTS - 1,
        };
        let row = sink
            .encode(id(1, 1), Phase::RequestReceived, None, None, None)
            .unwrap();
        assert!(row.len() <= 1024);
        let terminal = sink
            .encode(id(1, 1), Phase::ResponseWritten, None, None, Some(true))
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&terminal).unwrap();
        assert_eq!(value["phase"], "truncated");
        assert!(
            sink.encode(id(1, 1), Phase::ResponseWritten, None, None, None)
                .is_none()
        );
    }
    #[test]
    fn encoded_failure_is_incremental_payload_free_and_correlated() {
        let bytes = Sink::default()
            .encode(
                id(4, 12),
                Phase::RequestReturned,
                Some(33),
                Some(8),
                Some(false),
            )
            .unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["identity"]["message_id"], 12);
        assert_eq!(value["identity"]["page_id"], 4);
        assert_eq!(value["stream_id"], 33);
        assert_eq!(value["success"], false);
        let text = String::from_utf8(bytes).unwrap();
        for forbidden in ["grant", "token", "url", "source", "cookie", "password"] {
            assert!(!text.contains(forbidden));
        }
    }
    #[cfg(unix)]
    #[test]
    fn sink_excludes_backpressure_descriptors_and_writes_without_flush_buffer() {
        use std::io::{Read, Seek, SeekFrom};
        use std::os::fd::AsRawFd;
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(!regular_file(socket.as_raw_fd()));
        assert!(!regular_file(-1));
        let path = std::env::temp_dir().join(format!(
            "oryn-ipc-progress-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        // Unlink immediately so panic cannot leave a diagnostic fixture behind.
        std::fs::remove_file(&path).unwrap();
        assert!(regular_file(file.as_raw_fd()));
        let bytes = Sink::default()
            .encode(id(8, 4), Phase::RequestReceived, None, None, None)
            .unwrap();
        assert!(write_once(file.as_raw_fd(), &bytes));
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut observed = Vec::new();
        file.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, bytes);
    }

    #[cfg(unix)]
    #[test]
    fn close_timing_preserves_captured_identity_and_unknown_outcome() {
        let mut original = id(4, 12);
        original.command = Some("close");
        original.context_id = Some(91);
        original.peer_pid = Some(82);
        let mut registry = Registry::default();
        assert!(registry.insert(original));
        let captured = registry.active[&4];
        registry.remove(original);
        assert!(registry.insert(id(4, 13)));
        let value = close_timing_identity(captured, Some(33), Some(8), None);
        for (key, field) in serde_json::to_value(original).unwrap().as_object().unwrap() {
            assert_eq!(&value[key], field);
        }
        assert_eq!(value["message_id"], 12);
        assert_eq!(value["stream_id"], 33);
        assert_eq!(value["document_generation"], 8);
        assert!(value["observed_success"].is_null());
        assert_eq!(close_timing_identity(captured, None, None, Some(false))["observed_success"], false);
        assert_eq!(close_timing_identity(captured, None, None, Some(true))["observed_success"], true);
        assert_eq!(value.as_object().unwrap().len(), 11);
        for forbidden in ["grant", "token", "url", "source", "cookie", "password"] {
            assert!(!value.to_string().contains(forbidden));
        }
    }

    #[cfg(unix)]
    #[test]
    fn close_timing_excludes_generic_and_high_frequency_progress() {
        for phase in [
            Phase::RequestSend,
            Phase::RequestReceived,
            Phase::RequestReturned,
            Phase::ResponseWritten,
            Phase::ContextBound,
            Phase::ParentChunkReceived,
            Phase::FramePolicyBegin,
            Phase::TransportRetirementBegin,
            Phase::Truncated,
        ] {
            assert!(close_timing_phase(phase).is_none());
        }
        assert_eq!(close_timing_phase(Phase::CloseAgentsSettled), Some("close_agents_settled"));
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[test]
    fn close_timing_collects_private_fd_with_piped_stderr_without_false_completion() {
        use std::fs::{File, OpenOptions};
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        const MODE: &str = "ORYN_CLOSE_TIMING_TEST_CHILD";
        const FILTER: &str = "ipc_progress::tests::close_timing_collects_private_fd_with_piped_stderr_without_false_completion";
        let phases = [
            (Phase::CloseActorEntered, "close_actor_entered"),
            (Phase::CloseRealmsPending, "close_realms_pending"),
            (Phase::CloseDocumentsPending, "close_documents_pending"),
            (Phase::CloseNativeCleanupPending, "close_native_cleanup_pending"),
            (Phase::CloseActorReleased, "close_actor_released"),
            (Phase::CloseAgentsSettled, "close_agents_settled"),
        ];
        if std::env::var_os(MODE).is_some() {
            assert!(!regular_file(libc::STDERR_FILENO));
            crate::native_timing::install_inherited(5).unwrap();
            let mut identity = id(4, 12);
            identity.command = Some("close");
            for (index, (phase, _)) in phases.iter().enumerate() {
                result(Some(identity), *phase, None, Some(8), (index == 5).then_some(false));
            }
            // These calls must neither write to the pipe nor consume native
            // telemetry slots. Missing identity cannot become a completed close.
            for _ in 0..20_000 {
                result(Some(identity), Phase::RequestReceived, None, None, None);
            }
            result(None, Phase::CloseActorReleased, None, None, Some(true));
            result(Some(identity), Phase::CloseAgentsSettled, None, None, Some(true));
            return;
        }
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
        }
        let path = std::env::temp_dir().join(format!(
            "oryn-close-timing-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let file = OpenOptions::new().read(true).append(true).create_new(true).mode(0o600).open(&path).unwrap();
        let _remove = Remove(path.clone());
        // Duplicate above the protocol destinations before changing child FD 5.
        let source = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
        assert!(source >= 64);
        let source = unsafe { File::from_raw_fd(source) };
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", FILTER, "--nocapture"]).env(MODE, "1");
        let fd = source.as_raw_fd();
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 5) != 5 { return Err(std::io::Error::last_os_error()); }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("oryn_ipc_progress"));
        let bytes = std::fs::read(&path).unwrap();
        let rows: Vec<serde_json::Value> = bytes.split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty()).map(|line| serde_json::from_slice(line).unwrap()).collect();
        assert_eq!(rows.len(), 7);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row["kind"], "oryn_native_timing");
            assert_eq!(row["version"], 1);
            assert_eq!(row["event"], "point");
            assert_eq!(row["sequence"], index);
            assert_eq!(row["prior_dropped_records"], 0);
            assert_eq!(row["phase"], phases[index.min(5)].1);
            assert_eq!(row["identity"]["process_session"], 7);
            assert_eq!(row["identity"]["message_id"], 12);
            assert_eq!(row["identity"]["page_id"], 4);
            assert_eq!(row["identity"]["route_generation"], 9);
            assert_eq!(row["clock_domain"], "clock_monotonic");
            assert!(row["monotonic_ns"].is_number());
            assert!(row["pid"].as_u64().unwrap() > 0);
            assert_eq!(row["cpu_scope"], "process_lifetime");
            assert_eq!(row["terminal_cpu_complete"], false);
            assert!(row["outcome"].is_null());
            assert!(row["scope_id"].is_null());
            if index < 5 { assert!(row["identity"]["observed_success"].is_null()); }
        }
        assert_eq!(rows[5]["identity"]["observed_success"], false);
        assert_eq!(rows[6]["identity"]["observed_success"], true);
    }

    #[cfg(unix)]
    #[test]
    fn observe_command_identity_is_not_other() {
        let route = crate::worker_route::WorkerRoute::issue(81, 991, 1).unwrap();
        let _request = request(
            &route,
            17,
            &crate::worker::WorkerRequest::Observe { profile: None },
        );
        assert_eq!(capture(991).unwrap().command, Some("observe"));
    }
    #[cfg(unix)]
    #[test]
    fn parent_context_mapping_does_not_fabricate_a_request_identity() {
        let mut identity = id(4, 12);
        identity.context_id = Some(91);
        identity.peer_pid = Some(82);
        identity.message_id = None;
        identity.command = None;
        identity.association = "parent_context_binding";
        let bytes = Sink::default()
            .encode(identity, Phase::ContextBound, None, None, None)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["identity"]["context_id"], 91);
        assert_eq!(value["identity"]["peer_pid"], 82);
        assert!(value["identity"]["message_id"].is_null());
        assert!(value["identity"]["command"].is_null());
    }
}

// Separate bounded budget: context traffic cannot consume frame/close evidence.
#[cfg(unix)]
pub(crate) mod context_rpc {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    const MAX_KEYS: usize = 1024;
    const MAX_RECORDS: u64 = 4096;
    static EXHAUSTED: AtomicBool = AtomicBool::new(false);
    type Key = (Option<Identity>, &'static str);
    #[derive(Clone, Copy, Default, serde::Serialize)]
    struct Totals {
        calls: u64,
        completed: u64,
        failed: u64,
        admission_write_ns: u128,
        response_wait_ns: u128,
        publication_wait_ns: u128,
        child_decode_ns: u128,
        response_boundary_clamped_calls: u64,
    }
    #[derive(Default)]
    struct Counters {
        rows: BTreeMap<Key, Totals>,
    }
    impl Counters {
        fn admit(&mut self, key: Key) -> Option<bool> {
            if !self.rows.contains_key(&key) && self.rows.len() >= MAX_KEYS {
                return None;
            }
            let row = self.rows.entry(key).or_default();
            row.calls = row.calls.saturating_add(1);
            Some(row.calls <= 2 || row.calls.is_power_of_two() || row.calls.is_multiple_of(256))
        }
        fn finish(
            &mut self,
            key: Key,
            elapsed: [u128; 3],
            success: bool,
            response_clamped: bool,
        ) -> Totals {
            let row = self.rows.get_mut(&key).expect("admitted diagnostic key");
            row.completed = row.completed.saturating_add(1);
            row.failed = row.failed.saturating_add(u64::from(!success));
            row.response_boundary_clamped_calls = row
                .response_boundary_clamped_calls
                .saturating_add(u64::from(response_clamped));
            row.admission_write_ns = row.admission_write_ns.saturating_add(elapsed[0]);
            row.response_wait_ns = row.response_wait_ns.saturating_add(elapsed[1]);
            row.publication_wait_ns = row.publication_wait_ns.saturating_add(elapsed[2]);
            *row
        }
    }
    fn counters() -> &'static Mutex<Counters> {
        static VALUE: OnceLock<Mutex<Counters>> = OnceLock::new();
        VALUE.get_or_init(Default::default)
    }
    #[derive(serde::Serialize)]
    struct RpcEvent {
        kind: &'static str,
        version: u8,
        pid: u32,
        sequence: u64,
        unix_time_ns: Option<u128>,
        identity: Option<Identity>,
        operation: &'static str,
        context_message_id: Option<u64>,
        phase: &'static str,
        rpc_elapsed_ns: u128,
        success: Option<bool>,
        totals: Totals,
        publication_received: bool,
        // Explicit observation endpoint: older records omit this field and
        // measured actor consumption, including its initial scheduling wake.
        response_boundary: &'static str,
        response_boundary_clamped: bool,
    }
    fn take_sequence(sequence: &mut u64) -> Option<(u64, bool)> {
        if *sequence > MAX_RECORDS {
            return None;
        }
        let current = *sequence;
        *sequence += 1;
        Some((current, current == MAX_RECORDS))
    }
    fn emit(rpc: &Rpc, phase: &'static str, success: Option<bool>, totals: Totals) {
        if EXHAUSTED.load(Ordering::Relaxed) {
            return;
        }
        static SEQUENCE: Mutex<u64> = Mutex::new(0);
        let mut sequence = SEQUENCE.lock().unwrap_or_else(|e| e.into_inner());
        let Some((current, truncated)) = take_sequence(&mut sequence) else {
            return;
        };
        let event = RpcEvent {
            kind: "oryn_context_rpc_progress",
            version: 1,
            pid: std::process::id(),
            sequence: current,
            unix_time_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|v| v.as_nanos()),
            identity: rpc.key.0,
            operation: rpc.key.1,
            context_message_id: rpc.message,
            phase: if truncated { "truncated" } else { phase },
            rpc_elapsed_ns: rpc.started.elapsed().as_nanos(),
            success: if truncated { None } else { success },
            totals,
            publication_received: rpc.publication_received,
            response_boundary: "reader_ready",
            response_boundary_clamped: rpc.response_boundary_clamped,
        };
        if truncated {
            EXHAUSTED.store(true, Ordering::Relaxed);
        }
        if let Ok(mut bytes) = serde_json::to_vec(&event) {
            bytes.push(b'\n');
            if bytes.len() <= 2048 && regular_file(libc::STDERR_FILENO) {
                let _ = write_once(libc::STDERR_FILENO, &bytes);
            }
            if phase == "finished" || truncated {
                crate::native_timing::aggregate(&bytes);
            }
        }
    }
    struct Rpc {
        key: Key,
        message: Option<u64>,
        started: Instant,
        boundary: Instant,
        stage: usize,
        elapsed: [u128; 3],
        selected: bool,
        success: bool,
        publication_received: bool,
        response_boundary_clamped: bool,
    }
    pub(crate) struct Scope(Option<Rpc>, Option<super::context_phases::Stamp>);
    impl Scope {
        pub(crate) fn start(
            route: Option<&crate::worker_route::WorkerRoute>,
            operation: &'static str,
        ) -> Self {
            let identity = route.map(|route| {
                capture(route.environment)
                    .filter(|id| {
                        id.process_session == route.process_session
                            && id.route_generation == route.generation
                    })
                    .unwrap_or(Identity {
                        process_session: route.process_session,
                        page_id: route.environment,
                        route_generation: route.generation,
                        message_id: None,
                        command: None,
                        context_id: None,
                        peer_pid: None,
                        association: "context_route_without_active_command",
                    })
            });
            let stamp = identity.and_then(super::context_phases::Stamp::from_identity);
            if EXHAUSTED.load(Ordering::Relaxed)
                || (!regular_file(libc::STDERR_FILENO) && !crate::native_timing::enabled())
            {
                return Self(None, stamp);
            }
            let key = (identity, operation);
            let selected = counters()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .admit(key);
            let Some(selected) = selected else {
                // Key saturation is explicit and disables further RPC work only.
                let now = Instant::now();
                let rpc = Rpc {
                    key,
                    message: None,
                    started: now,
                    boundary: now,
                    stage: 0,
                    elapsed: [0; 3],
                    selected: true,
                    success: false,
                    publication_received: false,
                    response_boundary_clamped: false,
                };
                emit(&rpc, "key_capacity_reached", None, Totals::default());
                EXHAUSTED.store(true, Ordering::Relaxed);
                return Self(None, stamp);
            };
            let now = Instant::now();
            let rpc = Rpc {
                key,
                message: None,
                started: now,
                boundary: now,
                stage: 0,
                elapsed: [0; 3],
                selected,
                success: false,
                publication_received: false,
                response_boundary_clamped: false,
            };
            if selected {
                let totals = counters().lock().unwrap_or_else(|e| e.into_inner()).rows[&key];
                emit(&rpc, "begin", None, totals);
            }
            Self(Some(rpc), stamp)
        }
        pub(crate) fn diagnostic_stamp(&self) -> Option<super::context_phases::Stamp> {
            self.1
        }
        pub(crate) fn admitted(&mut self, message: u64) {
            if let Some(rpc) = &mut self.0 {
                rpc.message = Some(message);
            }
            self.record("admitted");
        }
        fn record(&self, phase: &'static str) {
            if let Some(rpc) = &self.0
                && rpc.selected
            {
                let totals = counters().lock().unwrap_or_else(|e| e.into_inner()).rows[&rpc.key];
                emit(rpc, phase, None, totals);
            }
        }
        fn advance(&mut self, phase: &'static str) {
            if let Some(rpc) = &mut self.0 {
                let now = Instant::now();
                rpc.elapsed[rpc.stage] = rpc.elapsed[rpc.stage]
                    .saturating_add(now.duration_since(rpc.boundary).as_nanos());
                rpc.boundary = now;
                rpc.stage = (rpc.stage + 1).min(2);
            }
            self.record(phase);
        }
        pub(crate) fn written(&mut self) {
            self.advance("written");
        }
        /// Decoder/attachment time is a subset of response_wait_ns.
        pub(crate) fn decoded(&mut self, elapsed: u128) {
            if let Some(rpc) = &self.0 {
                let mut guard = counters().lock().unwrap_or_else(|e| e.into_inner());
                let row = guard
                    .rows
                    .get_mut(&rpc.key)
                    .expect("admitted diagnostic key");
                row.child_decode_ns = row.child_decode_ns.saturating_add(elapsed);
            }
        }
        pub(crate) fn response_at(&mut self, ready_at: Instant) {
            if let Some(rpc) = &mut self.0 {
                // A very fast peer can decode before the actor records written.
                // Clamp that observation to the existing boundary; never panic
                // or count overlapping time twice.
                rpc.response_boundary_clamped = ready_at < rpc.boundary;
                let ready_at = ready_at.max(rpc.boundary);
                rpc.elapsed[rpc.stage] = rpc.elapsed[rpc.stage]
                    .saturating_add(ready_at.duration_since(rpc.boundary).as_nanos());
                rpc.boundary = ready_at;
                rpc.stage = (rpc.stage + 1).min(2);
            }
            self.record("response_delivered");
        }
        pub(crate) fn published(&mut self) {
            if let Some(rpc) = &mut self.0 {
                rpc.publication_received = true;
            }
            self.record("publication_received");
        }
        pub(crate) fn finished(&mut self, success: bool) {
            if let Some(rpc) = &mut self.0 {
                rpc.success = success;
            }
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            let Some(mut rpc) = self.0.take() else { return };
            rpc.elapsed[rpc.stage] =
                rpc.elapsed[rpc.stage].saturating_add(rpc.boundary.elapsed().as_nanos());
            let totals = counters().lock().unwrap_or_else(|e| e.into_inner()).finish(
                rpc.key,
                rpc.elapsed,
                rpc.success,
                rpc.response_boundary_clamped,
            );
            if rpc.selected || !rpc.success {
                emit(&rpc, "finished", Some(rpc.success), totals);
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        fn test_identity(page: u64, message: u64) -> Identity {
            Identity {
                process_session: 1,
                page_id: page,
                route_generation: 2,
                message_id: Some(message),
                command: Some("execute"),
                context_id: None,
                peer_pid: None,
                association: "request",
            }
        }
        #[test]
        fn ipc_parent_correlation_survives_disabled_child_emission() {
            let identity = test_identity(1, 17);
            let stamp = super::super::context_phases::Stamp::from_identity(identity).unwrap();
            let mut scope = Scope(None, Some(stamp));
            scope.admitted(91);
            scope.written();
            scope.response_at(Instant::now());
            scope.published();
            scope.finished(true);
            assert_eq!(scope.diagnostic_stamp(), Some(stamp));
        }
        #[test]
        fn response_ready_timestamp_preserves_publication_interval_and_clamps_race() {
            let start = Instant::now();
            let ready = start + std::time::Duration::from_millis(7);
            let mut scope = Scope(
                Some(Rpc {
                    key: (None, "test"),
                    message: Some(1),
                    started: start,
                    boundary: start,
                    stage: 1,
                    elapsed: [3, 0, 0],
                    selected: false,
                    success: false,
                    publication_received: false,
                    response_boundary_clamped: false,
                }),
                None,
            );
            scope.response_at(ready);
            let rpc = scope.0.take().unwrap();
            assert_eq!(rpc.elapsed, [3, 7_000_000, 0]);
            assert_eq!(rpc.boundary, ready);
            assert_eq!(rpc.stage, 2);
            assert!(!rpc.response_boundary_clamped);
            // Remove the synthetic scope before Drop touches real counters.
            let _ = scope.0.take();
            scope.0 = Some(Rpc {
                key: (None, "test"),
                message: Some(2),
                started: start,
                boundary: ready,
                stage: 1,
                elapsed: [3, 0, 0],
                selected: false,
                success: false,
                publication_received: false,
                response_boundary_clamped: false,
            });
            scope.response_at(start);
            let rpc = scope.0.take().unwrap();
            assert_eq!(rpc.elapsed, [3, 0, 0]);
            assert_eq!(rpc.boundary, ready);
            assert!(rpc.response_boundary_clamped);
            let _ = scope.0.take();
        }
        #[test]
        fn counts_unsampled_calls_and_separates_wait_buckets() {
            let mut c = Counters::default();
            for n in 1u64..=9 {
                assert_eq!(c.admit((None, "Read")), Some(n <= 2 || n.is_power_of_two()));
                c.finish((None, "Read"), [2, 3, 5], n != 9, n == 9);
            }
            let row = c.rows[&(None, "Read")];
            assert_eq!((row.calls, row.completed, row.failed), (9, 9, 1));
            assert_eq!(row.response_boundary_clamped_calls, 1);
            assert_eq!(
                (
                    row.admission_write_ns,
                    row.response_wait_ns,
                    row.publication_wait_ns
                ),
                (18, 27, 45)
            );
        }
        #[test]
        fn registry_capacity_is_bounded_and_existing_keys_continue() {
            let mut c = Counters::default();
            for page in 0..MAX_KEYS as u64 {
                let mut id = test_identity(1, 1);
                id.page_id = page;
                assert_eq!(c.admit((Some(id), "Read")), Some(true));
            }
            assert_eq!(c.admit((None, "Extra")), None);
            assert_eq!(c.rows.len(), MAX_KEYS);
        }
        #[test]
        fn command_and_operation_counters_do_not_alias() {
            let mut c = Counters::default();
            let a = test_identity(1, 1);
            let b = test_identity(1, 2);
            for key in [(Some(a), "Read"), (Some(b), "Read"), (Some(a), "Write")] {
                assert_eq!(c.admit(key), Some(true));
            }
            assert_eq!(c.rows.len(), 3);
        }
        #[test]
        fn record_budget_has_one_explicit_terminal_marker() {
            let mut sequence = 0;
            for expected in 0..MAX_RECORDS {
                assert_eq!(take_sequence(&mut sequence), Some((expected, false)));
            }
            assert_eq!(take_sequence(&mut sequence), Some((MAX_RECORDS, true)));
            assert_eq!(take_sequence(&mut sequence), None);
            assert_eq!(take_sequence(&mut sequence), None);
        }
        #[cfg(feature = "v8-host")]
        #[test]
        fn readiness_stages_survive_rpc_budget_exhaustion_with_explicit_general_truncation() {
            let mut rpc_sequence = MAX_RECORDS;
            assert_eq!(take_sequence(&mut rpc_sequence), Some((MAX_RECORDS, true)));
            assert_eq!(take_sequence(&mut rpc_sequence), None);
            let original = test_identity(11, 4);
            let mut sink = super::super::Sink::default();
            use super::super::Phase;
            for (sequence, (phase, label)) in [
                (Phase::DocumentCommitted, "document_committed"),
                (
                    Phase::NavigationLayoutCompleted,
                    "navigation_layout_completed",
                ),
                (Phase::NavigationParserSettled, "navigation_parser_settled"),
                (Phase::NavigationLoadReady, "navigation_load_ready"),
                (
                    Phase::ActionNavigationReleased,
                    "action_navigation_released",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let bytes = sink.encode(original, phase, None, Some(7), None).unwrap();
                assert!(bytes.len() <= 1024);
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["sequence"], sequence);
                assert_eq!(value["phase"], label);
                assert_eq!(value["identity"]["message_id"], 4);
                assert_eq!(value["document_generation"], 7);
                assert_eq!(take_sequence(&mut rpc_sequence), None);
            }
            sink.sequence = super::super::MAX_EVENTS;
            let marker = sink
                .encode(
                    original,
                    Phase::NavigationLoadReady,
                    None,
                    Some(7),
                    Some(true),
                )
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&marker).unwrap();
            assert_eq!(value["phase"], "truncated");
            assert!(value["success"].is_null());
            assert!(
                sink.encode(
                    original,
                    Phase::ActionNavigationReleased,
                    None,
                    Some(7),
                    Some(true)
                )
                .is_none()
            );
        }
        #[test]
        fn encoded_event_has_separate_correlation_and_bounded_payload() {
            let event = RpcEvent {
                kind: "oryn_context_rpc_progress",
                version: 1,
                pid: u32::MAX,
                sequence: u64::MAX,
                unix_time_ns: Some(u128::MAX),
                identity: Some(test_identity(u64::MAX, 17)),
                operation: "AuthorizeInlineScript",
                context_message_id: Some(81),
                phase: "finished",
                rpc_elapsed_ns: u128::MAX,
                success: Some(false),
                totals: Totals {
                    calls: u64::MAX,
                    completed: u64::MAX,
                    failed: u64::MAX,
                    admission_write_ns: u128::MAX,
                    response_wait_ns: u128::MAX,
                    publication_wait_ns: u128::MAX,
                    child_decode_ns: u128::MAX,
                    response_boundary_clamped_calls: u64::MAX,
                },
                publication_received: false,
                response_boundary: "reader_ready",
                response_boundary_clamped: false,
            };
            let bytes = serde_json::to_vec(&event).unwrap();
            assert!(bytes.len() < 2048);
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["identity"]["message_id"], 17);
            assert_eq!(value["context_message_id"], 81);
            assert!(value.get("request").is_none());
            assert!(value.get("response").is_none());
            assert_eq!(value["publication_received"], false);
            assert_eq!(value["response_boundary"], "reader_ready");
            assert_eq!(value["response_boundary_clamped"], false);
        }
    }
}

/// Independent navigation budget: RPC/frame traces cannot consume these slots.
#[cfg(feature = "v8-host")]
pub(crate) mod navigation {
    use super::Identity;
    #[cfg(any(unix, test))]
    const LIMIT: u64 = 4096;

    #[derive(Clone, Copy, serde::Serialize)]
    pub(crate) struct Observation {
        pub identity: Option<Identity>,
        pub navigation_id: u64,
        // HeaderDiagnostics' transaction-only record has no request yet.
        pub navigation_request_id: Option<u64>,
        pub source_generation: u64,
        pub phase: &'static str,
        pub error_present: bool,
        pub elapsed_ns: u128,
        pub deadline_remaining_ns: Option<u128>,
    }

    #[cfg(any(unix, test))]
    fn known_phase(phase: &str) -> bool {
        matches!(
            phase,
            "transaction_bound"
                | "transaction_headers_ready"
                | "transaction_failed"
                | "navigation_policy_consumed"
                | "request_start"
                | "io_job_entered"
                | "consumer_closed_before_broker"
                | "deadline_before_broker"
                | "broker_entered"
                | "broker_returned"
                | "broker_failed"
                | "result_delivered"
                | "consumer_closed_after_broker"
                | "deadline_before_submit"
                | "resource_job_rejected"
                | "io_submit_started"
                | "io_submit_failed"
                | "io_submit_accepted"
                | "consumer_failed"
                | "admission_received"
                | "headers_received"
                | "consumer_retired"
        )
    }

    #[cfg(any(unix, test))]
    #[derive(Default)]
    struct Sink {
        sequence: u64,
    }
    #[cfg(any(unix, test))]
    impl Sink {
        fn encode(&mut self, observation: Observation) -> Option<Vec<u8>> {
            if self.sequence > LIMIT || !known_phase(observation.phase) {
                return None;
            }
            let truncated = self.sequence == LIMIT;
            let value = serde_json::json!({
                "kind": "oryn_navigation_progress", "version": 1,
                "pid": std::process::id(), "sequence": self.sequence,
                "unix_time_ns": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).ok().map(|v| v.as_nanos()),
                "truncated": truncated,
                // A cap marker describes the sink, not a completed operation.
                "observation": if truncated { None } else { Some(observation) },
            });
            self.sequence += 1;
            let mut bytes = serde_json::to_vec(&value).ok()?;
            bytes.push(b'\n');
            (bytes.len() <= 2048).then_some(bytes)
        }
    }
    pub(crate) fn record(observation: Observation) {
        #[cfg(unix)]
        {
            if !super::regular_file(libc::STDERR_FILENO) {
                return;
            }
            static SINK: std::sync::Mutex<Sink> = std::sync::Mutex::new(Sink { sequence: 0 });
            let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(bytes) = sink.encode(observation) {
                let _ = super::write_once(libc::STDERR_FILENO, &bytes);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = observation;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn observation() -> Observation {
            Observation {
                identity: Some(Identity {
                    process_session: 3,
                    page_id: 17,
                    route_generation: 19,
                    message_id: Some(23),
                    command: Some("execute"),
                    context_id: Some(29),
                    peer_pid: Some(31),
                    association: "request",
                }),
                navigation_id: 37,
                navigation_request_id: Some(2),
                source_generation: 41,
                phase: "io_submit_started",
                error_present: false,
                elapsed_ns: 43,
                deadline_remaining_ns: Some(47),
            }
        }
        #[test]
        fn navigation_mirror_preserves_original_transaction_and_request_correlation() {
            let first = observation();
            let later = Observation {
                navigation_request_id: Some(3),
                phase: "broker_entered",
                ..first
            };
            let mut sink = Sink::default();
            for expected in [first, later] {
                let bytes = sink.encode(expected).unwrap();
                let row: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(row["observation"], serde_json::to_value(expected).unwrap());
                assert!(row["unix_time_ns"].is_number());
                assert!(!row["truncated"].as_bool().unwrap());
            }
            assert_eq!(first.navigation_request_id, Some(2));
        }
        #[test]
        fn navigation_mirror_has_independent_bound_and_one_noncompletion_marker() {
            let mut sink = Sink::default();
            for _ in 0..LIMIT {
                let bytes = sink.encode(observation()).unwrap();
                assert!(bytes.len() <= 2048);
                let row: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(row["truncated"], false);
            }
            let row: serde_json::Value =
                serde_json::from_slice(&sink.encode(observation()).unwrap()).unwrap();
            assert_eq!(row["truncated"], true);
            assert!(row["observation"].is_null());
            assert!(sink.encode(observation()).is_none());
        }
        #[test]
        fn navigation_mirror_rejects_nonliteral_payload_and_preserves_unknown_identity() {
            let mut sink = Sink::default();
            assert!(
                sink.encode(Observation {
                    phase: "https://private.invalid/secret",
                    ..observation()
                })
                .is_none()
            );
            assert_eq!(sink.sequence, 0);
            let row: serde_json::Value = serde_json::from_slice(
                &sink
                    .encode(Observation {
                        identity: None,
                        navigation_request_id: None,
                        phase: "transaction_bound",
                        ..observation()
                    })
                    .unwrap(),
            )
            .unwrap();
            assert!(row["observation"]["identity"].is_null());
            assert!(row["observation"]["navigation_request_id"].is_null());
        }
    }
}

#[cfg(unix)]
#[path = "context_rpc_phases.rs"]
pub(crate) mod context_phases;

#[path = "close_admission_progress.rs"]
pub(crate) mod close_admission;
