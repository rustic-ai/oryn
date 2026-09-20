//! Independent agent child mode using the verified worker executable and
//! capability sockets. Control and services never carry browsing-context IDs.
use crate::agent_ipc::*;
use crate::agent_registry::AgentProcess;
use oryn_js::worker_wire::*;
use std::{
    io,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(any(feature = "v8-host", test))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn failure(error: impl std::fmt::Display) -> WorkerError {
    WorkerError::new("OperationError", error.to_string())
}

/// One native isolate serializes callbacks, while the cancellation socket stays
/// independent of the request mutex so teardown can wake a blocked import.
#[cfg(any(feature = "v8-host", test))]
struct ChildServices {
    stream: Mutex<UnixStream>,
    cancel: UnixStream,
    next: AtomicU64,
    retired: AtomicBool,
}
#[cfg(any(feature = "v8-host", test))]
impl ChildServices {
    fn new(stream: UnixStream) -> io::Result<Self> {
        Ok(Self {
            cancel: stream.try_clone()?,
            stream: Mutex::new(stream),
            next: AtomicU64::new(1),
            retired: AtomicBool::new(false),
        })
    }
}
#[cfg(any(feature = "v8-host", test))]
impl WorkerTransport for ChildServices {
    fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(failure("agent service route retired"));
        }
        let id = self
            .next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| failure("agent service IDs exhausted"))?;
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| failure("agent service stream poisoned"))?;
        if self.retired.load(Ordering::Acquire) {
            return Err(failure("agent service route retired"));
        }
        write_frame(&mut *stream, &ServiceRequest { id, request }).map_err(|error| {
            self.retire();
            failure(error)
        })?;
        let mut response: ServiceResponse = read_frame(&mut *stream).map_err(|error| {
            self.retire();
            failure(error)
        })?;
        if response.id != id {
            self.retire();
            return Err(failure("agent service response correlation mismatch"));
        }
        if let Ok(AgentServiceResponse::SharedMemory(memory)) = &mut response.result
            && memory.descriptor.is_some()
        {
            memory.native_descriptor = Some(Arc::new(
                crate::shared_memory_fd::receive(&stream, id).map_err(|error| {
                    self.retire();
                    failure(error)
                })?,
            ));
        }
        write_frame(&mut *stream, &ServiceReceipt { received_id: id }).map_err(|error| {
            self.retire();
            failure(error)
        })?;
        response.result
    }
    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        let _ = self.cancel.shutdown(Shutdown::Both);
    }
}

struct ServicePublication<'a> {
    backend: &'a dyn WorkerTransport,
    scope: Option<ServiceResponseScope>,
    response: &'a ServiceResponse,
}
impl Drop for ServicePublication<'_> {
    fn drop(&mut self) {
        if let (Some(scope), Ok(response)) = (self.scope.take(), &self.response.result) {
            let _ = self.backend.reclaim_response(scope, response);
        }
    }
}
fn publish_service_response(
    stream: &mut UnixStream,
    backend: &dyn WorkerTransport,
    scope: ServiceResponseScope,
    response: &ServiceResponse,
) -> io::Result<()> {
    let mut custody = ServicePublication {
        backend,
        scope: Some(scope),
        response,
    };
    write_frame(stream, response)?;
    if let Ok(AgentServiceResponse::SharedMemory(memory)) = &response.result
        && let Some(fd) = &memory.native_descriptor
    {
        crate::shared_memory_fd::send(stream, response.id, fd.as_ref())?;
    }
    let receipt: ServiceReceipt = read_frame(stream)?;
    if receipt.received_id != response.id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent response receipt correlation mismatch",
        ));
    }
    custody.scope = None;
    Ok(())
}

/// Invoked only after worker_process installed the OS sandbox and cleared the
/// environment. The process remains admission-paused until ParentFrame::Start.
#[cfg(feature = "v8-host")]
pub fn serve_child(mut control: UnixStream, services: UnixStream) -> io::Result<()> {
    let ParentFrame::Bootstrap { version, bootstrap } = read_frame(&mut control)? else {
        return Err(io::Error::other("agent bootstrap required"));
    };
    if version != VERSION {
        return Err(io::Error::other("agent protocol mismatch"));
    }
    let bootstrap = *bootstrap;
    let digest = bootstrap_digest(&bootstrap).map_err(io::Error::other)?;
    let id = bootstrap.id;
    let service = Arc::new(ChildServices::new(services)?);
    let child_service = service.clone();
    let process = crate::agent_thread::AgentThreadProcess::prepare(bootstrap, move |bootstrap| {
        crate::host::NativeAgentRuntime::create(bootstrap, child_service)
    })
    .map_err(io::Error::other)?;
    let process = Arc::new(Mutex::new(process));
    let suspension_epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let pump_epoch = suspension_epoch.clone();
    write_frame(
        &mut control,
        &ChildFrame::Prepared {
            version: VERSION,
            agent: id,
            process_id: std::process::id(),
            binary_sha256: crate::worker::current_binary_sha256()?,
            bootstrap_sha256: digest,
            sandbox: if cfg!(target_os = "macos") {
                "app_sandbox"
            } else if cfg!(target_os = "linux") {
                "linux_namespaces_landlock_seccomp"
            } else {
                "process_isolated_unsandboxed"
            }
            .into(),
            build_profile: crate::runtime_build_info().build_profile,
        },
    )?;
    let stopped = Arc::new(AtomicBool::new(false));
    let pump_stopped = stopped.clone();
    let pump_process = process.clone();
    let mut writer = control.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(2)))?;
    let cancel_control = control.try_clone()?;
    let pump = std::thread::Builder::new()
        .name("oryn-agent-events".into())
        .spawn(move || {
            let mut last_suspension = None;
            while !pump_stopped.load(Ordering::Acquire) {
                let suspension = pump_process.lock().ok().and_then(|mut process| {
                    process
                        .suspension_status()
                        .ok()
                        .map(|suspended| (pump_epoch.load(Ordering::Acquire), suspended))
                });
                if suspension != last_suspension {
                    if let Some((epoch, suspended)) = suspension
                        && write_frame(&mut writer, &ChildFrame::Suspension { epoch, suspended })
                            .is_err()
                    {
                        break;
                    }
                    last_suspension = suspension;
                }
                let result = pump_process
                    .lock()
                    .map_err(|_| failure("agent process poisoned"))
                    .and_then(|mut process| process.poll(32));
                match result {
                    Ok(events) if !events.is_empty() => {
                        let closed = events
                            .iter()
                            .any(|event| matches!(event, AgentEvent::Closed));
                        if write_frame(&mut writer, &ChildFrame::Events(events)).is_err() || closed
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = write_frame(&mut writer, &ChildFrame::Failure(error));
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            pump_stopped.store(true, Ordering::Release);
            let _ = cancel_control.shutdown(Shutdown::Both);
        })?;
    let result = (|| -> io::Result<()> {
        while !stopped.load(Ordering::Acquire) {
            let command = match read_frame::<ParentFrame>(&mut control) {
                Ok(command) => command,
                Err(error) => {
                    if stopped.load(Ordering::Acquire)
                        || matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                        )
                    {
                        break;
                    }
                    return Err(error);
                }
            };
            let mut process = process
                .lock()
                .map_err(|_| io::Error::other("agent process poisoned"))?;
            match command {
                ParentFrame::Start => process.start().map_err(io::Error::other)?,
                ParentFrame::Suspension { epoch, suspended } => {
                    process.set_suspended(suspended).map_err(io::Error::other)?;
                    suspension_epoch.store(epoch, Ordering::Release);
                }
                ParentFrame::Command(command) if matches!(command.as_ref(), AgentCommand::Stop) => {
                    stopped.store(true, Ordering::Release);
                    break;
                }
                ParentFrame::Command(command) => {
                    process.send(*command).map_err(io::Error::other)?
                }
                ParentFrame::Bootstrap { .. } => {
                    return Err(io::Error::other("duplicate agent bootstrap"));
                }
            }
        }
        Ok(())
    })();
    stopped.store(true, Ordering::Release);
    service.retire();
    let _ = control.shutdown(Shutdown::Both);
    let termination = process
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .terminate()
        .map_err(io::Error::other);
    let joined = pump
        .join()
        .map_err(|_| io::Error::other("agent event pump panicked"));
    result.and(termination).and(joined)
}

struct ParentState {
    commands: std::collections::VecDeque<(ParentFrame, usize)>,
    command_bytes: usize,
    events: std::collections::VecDeque<(AgentEvent, usize)>,
    event_bytes: usize,
    admitted: bool,
    stopping: bool,
    closed: bool,
    failure: Option<WorkerError>,
    suspension_epoch: u64,
    suspend_requested: bool,
    suspend_acknowledged: bool,
    suspension_control: Option<ParentFrame>,
}
struct ParentQueue {
    state: Mutex<ParentState>,
    changed: std::sync::Condvar,
    max_messages: usize,
    max_bytes: usize,
}
pub struct AgentChildProcess {
    placement: crate::execution_placement::ExecutionPlacement,
    child: Option<std::process::Child>,
    control: UnixStream,
    services: UnixStream,
    backend: Arc<dyn WorkerTransport>,
    queue: Arc<ParentQueue>,
    threads: Vec<std::thread::JoinHandle<()>>,
}
struct LaunchGuard(Option<std::process::Child>);
impl Drop for LaunchGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl AgentChildProcess {
    /// The returned process has verified its bootstrap but has not executed any
    /// author source. Registry admission must call AgentProcess::start afterward.
    pub fn prepare(
        bootstrap: AgentBootstrap,
        backend: Arc<dyn WorkerTransport>,
    ) -> Result<Self, WorkerError> {
        use std::os::{fd::AsRawFd, unix::process::CommandExt};
        if bootstrap.limits.queue_messages == 0 || bootstrap.limits.queue_bytes == 0 {
            return Err(failure("empty agent mailbox budget"));
        }
        let (mut command, path, binary) =
            crate::worker_client::prepare_agent_command().map_err(failure)?;
        let (mut control, child_control) = UnixStream::pair().map_err(failure)?;
        let (services, child_services) = UnixStream::pair().map_err(failure)?;
        let control_fd = child_control.as_raw_fd();
        let service_fd = child_services.as_raw_fd();
        command
            .arg("--agent-control-fd")
            .arg("197")
            .arg("--agent-service-fd")
            .arg("198");
        // SAFETY: only async-signal-safe descriptor operations execute after
        // fork. Copy both sources first so destination/source collisions cannot
        // silently alias the two authority channels.
        unsafe {
            command.pre_exec(move || {
                let first = libc::fcntl(control_fd, libc::F_DUPFD_CLOEXEC, 200);
                if first < 0 {
                    return Err(io::Error::last_os_error());
                }
                let second = libc::fcntl(service_fd, libc::F_DUPFD_CLOEXEC, 200);
                if second < 0 {
                    libc::close(first);
                    return Err(io::Error::last_os_error());
                }
                if libc::dup2(first, 197) < 0 || libc::dup2(second, 198) < 0 {
                    return Err(io::Error::last_os_error());
                }
                libc::close(first);
                libc::close(second);
                Ok(())
            });
        }
        let child = command.spawn().map_err(failure)?;
        let pid = child.id();
        let mut guard = LaunchGuard(Some(child));
        crate::worker_client::verify_agent_process(pid, &path).map_err(failure)?;
        drop(child_control);
        drop(child_services);
        control
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(failure)?;
        control
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(failure)?;
        let digest = bootstrap_digest(&bootstrap)?;
        write_frame(
            &mut control,
            &ParentFrame::Bootstrap {
                version: VERSION,
                bootstrap: Box::new(bootstrap.clone()),
            },
        )
        .map_err(failure)?;
        let prepared = read_frame::<ChildFrame>(&mut control).map_err(failure)?;
        let expected_sandbox = if cfg!(target_os = "macos") {
            "app_sandbox"
        } else if cfg!(target_os = "linux") {
            "linux_namespaces_landlock_seccomp"
        } else {
            "process_isolated_unsandboxed"
        };
        match prepared {
            ChildFrame::Prepared {
                version,
                agent,
                process_id,
                binary_sha256,
                bootstrap_sha256,
                sandbox,
                build_profile,
            } if version == VERSION
                && agent == bootstrap.id
                && process_id == pid
                && binary_sha256 == binary
                && bootstrap_sha256 == digest
                && sandbox == expected_sandbox
                && (!cfg!(target_os = "macos") || build_profile == "release") => {}
            _ => {
                return Err(failure(
                    "agent handshake does not match verified launch and bootstrap",
                ));
            }
        }
        control.set_read_timeout(None).map_err(failure)?;
        control
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(failure)?;
        let queue = Arc::new(ParentQueue {
            state: Mutex::new(ParentState {
                commands: Default::default(),
                command_bytes: 0,
                events: Default::default(),
                event_bytes: 0,
                admitted: false,
                stopping: false,
                closed: false,
                failure: None,
                suspension_epoch: 0,
                suspend_requested: false,
                suspend_acknowledged: false,
                suspension_control: None,
            }),
            changed: std::sync::Condvar::new(),
            max_messages: bootstrap.limits.queue_messages,
            max_bytes: bootstrap.limits.queue_bytes.min(MAX_FRAME),
        });
        let mut process = Self {
            placement: crate::execution_placement::ExecutionPlacement {
                mode: "child_process".into(),
                process_id: pid,
                attested: true,
                binary_sha256: Some(binary),
                bootstrap_sha256: Some(digest),
            },
            child: guard.0.take(),
            control,
            services,
            backend,
            queue,
            threads: Vec::new(),
        };
        if let Err(error) = process.start_io() {
            let _ = process.terminate();
            return Err(error);
        }
        Ok(process)
    }
    fn enqueue(&self, frame: ParentFrame) -> Result<(), WorkerError> {
        let bytes = crate::agent_thread::wire_size(&frame, self.queue.max_bytes)?;
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| failure("agent mailbox poisoned"))?;
        if state.stopping || state.closed {
            return Err(failure("agent process retired"));
        }
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        let total = state
            .command_bytes
            .checked_add(bytes)
            .ok_or_else(|| failure("agent command byte overflow"))?;
        if state.commands.len() >= self.queue.max_messages || total > self.queue.max_bytes {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "agent command mailbox full",
            ));
        }
        state.commands.push_back((frame, bytes));
        state.command_bytes = total;
        self.queue.changed.notify_all();
        Ok(())
    }
    fn start_io(&mut self) -> Result<(), WorkerError> {
        let mut writer = self.control.try_clone().map_err(failure)?;
        let queue = self.queue.clone();
        self.threads.push(
            std::thread::Builder::new()
                .name("oryn-agent-command-writer".into())
                .spawn(move || {
                    loop {
                        let frame = {
                            let state = match queue.state.lock() {
                                Ok(state) => state,
                                Err(_) => break,
                            };
                            let mut state = match queue.changed.wait_while(state, |state| {
                                !state.stopping
                                    && !state.closed
                                    && state.failure.is_none()
                                    && state.commands.is_empty()
                                    && state.suspension_control.is_none()
                            }) {
                                Ok(state) => state,
                                Err(_) => break,
                            };
                            if state.stopping || state.closed || state.failure.is_some() {
                                break;
                            }
                            if let Some(frame) = state.suspension_control.take() {
                                frame
                            } else {
                                let Some((frame, bytes)) = state.commands.pop_front() else {
                                    continue;
                                };
                                state.command_bytes -= bytes;
                                frame
                            }
                        };
                        if let Err(error) = write_frame(&mut writer, &frame) {
                            if let Ok(mut state) = queue.state.lock()
                                && !state.stopping
                            {
                                state.failure = Some(failure(error));
                            }
                            queue.changed.notify_all();
                            let _ = writer.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                })
                .map_err(failure)?,
        );
        let mut reader = self.control.try_clone().map_err(failure)?;
        let queue = self.queue.clone();
        self.threads.push(
            std::thread::Builder::new()
                .name("oryn-agent-event-reader".into())
                .spawn(move || {
                    loop {
                        let frame = read_frame::<ChildFrame>(&mut reader);
                        let mut state = match queue.state.lock() {
                            Ok(state) => state,
                            Err(_) => break,
                        };
                        if state.stopping {
                            break;
                        }
                        match frame {
                            Ok(ChildFrame::Events(events)) => {
                                let staged = events
                                    .into_iter()
                                    .map(|event| {
                                        crate::agent_thread::wire_size(&event, queue.max_bytes)
                                            .map(|bytes| (event, bytes))
                                    })
                                    .collect::<Result<Vec<_>, _>>();
                                match staged {
                                    Ok(events) => {
                                        let bytes = events
                                            .iter()
                                            .try_fold(state.event_bytes, |sum, (_, bytes)| {
                                                sum.checked_add(*bytes)
                                            });
                                        if state.events.len().saturating_add(events.len())
                                            > queue.max_messages
                                            || bytes.is_none_or(|bytes| bytes > queue.max_bytes)
                                        {
                                            state.failure = Some(WorkerError::new(
                                                "QuotaExceededError",
                                                "agent event mailbox full",
                                            ));
                                        } else {
                                            state.event_bytes = bytes.unwrap();
                                            for (event, bytes) in events {
                                                state.closed |= matches!(event, AgentEvent::Closed);
                                                state.events.push_back((event, bytes));
                                            }
                                        }
                                    }
                                    Err(error) => state.failure = Some(error),
                                }
                            }
                            Ok(ChildFrame::Suspension { epoch, suspended }) => {
                                if epoch == state.suspension_epoch {
                                    state.suspend_acknowledged =
                                        state.suspend_requested && suspended;
                                }
                            }
                            Ok(ChildFrame::Failure(error)) => state.failure = Some(error),
                            Ok(ChildFrame::Prepared { .. }) => {
                                state.failure = Some(failure("duplicate agent prepared frame"))
                            }
                            Err(error) => {
                                if !state.closed {
                                    state.failure = Some(failure(error));
                                }
                                break;
                            }
                        }
                        queue.changed.notify_all();
                        if state.failure.is_some() || state.closed {
                            break;
                        }
                    }
                    queue.changed.notify_all();
                    let _ = reader.shutdown(Shutdown::Both);
                })
                .map_err(failure)?,
        );
        let mut stream = self.services.try_clone().map_err(failure)?;
        let backend = self.backend.clone();
        let queue = self.queue.clone();
        self.threads.push(
            std::thread::Builder::new()
                .name("oryn-agent-service-broker".into())
                .spawn(move || {
                    let mut previous = 0u64;
                    while let Ok(request) = read_frame::<ServiceRequest>(&mut stream) {
                        if queue
                            .state
                            .lock()
                            .map(|state| state.stopping || state.closed)
                            .unwrap_or(true)
                        {
                            break;
                        }
                        if request.id <= previous {
                            let _ = stream.shutdown(Shutdown::Both);
                            break;
                        }
                        previous = request.id;
                        // The backend closes over the actual registry AgentId; no
                        // request field may substitute a page, context or principal.
                        let scope = ServiceResponseScope::capture(&request.request);
                        let result = backend.request(request.request);
                        let response = ServiceResponse {
                            id: request.id,
                            result,
                        };
                        if let Err(error) = publish_service_response(
                            &mut stream,
                            backend.as_ref(),
                            scope,
                            &response,
                        ) {
                            if let Ok(mut state) = queue.state.lock() {
                                state.failure = Some(failure(error));
                            }
                            queue.changed.notify_all();
                            let _ = stream.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                })
                .map_err(failure)?,
        );
        Ok(())
    }
}
impl AgentProcess for AgentChildProcess {
    fn execution_placement(&self) -> Option<crate::execution_placement::ExecutionPlacement> {
        Some(self.placement.clone())
    }
    fn start(&mut self) -> Result<(), WorkerError> {
        {
            let state = self
                .queue
                .state
                .lock()
                .map_err(|_| failure("agent mailbox poisoned"))?;
            if state.stopping || state.closed {
                return Err(failure("agent process retired"));
            }
            if state.admitted {
                return Ok(());
            }
        }
        self.enqueue(ParentFrame::Start)?;
        self.queue
            .state
            .lock()
            .map_err(|_| failure("agent mailbox poisoned"))?
            .admitted = true;
        Ok(())
    }
    fn set_suspended(&mut self, suspended: bool) -> Result<(), WorkerError> {
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| failure("agent mailbox poisoned"))?;
        if state.stopping || state.closed {
            return Err(failure("agent retired"));
        }
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        state.suspension_epoch = state
            .suspension_epoch
            .checked_add(1)
            .ok_or_else(|| failure("agent suspension epoch exhausted"))?;
        state.suspend_requested = suspended;
        state.suspend_acknowledged = false;
        state.suspension_control = Some(ParentFrame::Suspension {
            epoch: state.suspension_epoch,
            suspended,
        });
        self.queue.changed.notify_all();
        Ok(())
    }
    fn suspension_status(&mut self) -> Result<bool, WorkerError> {
        let state = self
            .queue
            .state
            .lock()
            .map_err(|_| failure("agent mailbox poisoned"))?;
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.stopping || state.closed {
            return Err(failure("agent retired"));
        }
        Ok(state.suspend_requested && state.suspend_acknowledged)
    }
    fn send(&mut self, command: AgentCommand) -> Result<(), WorkerError> {
        if let AgentCommand::Message { value, .. } = &command {
            value
                .validate()
                .map_err(|error| WorkerError::new("DataCloneError", error))?;
        }
        self.enqueue(ParentFrame::Command(Box::new(command)))
    }
    fn poll(&mut self, max: usize) -> Result<Vec<AgentEvent>, WorkerError> {
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| failure("agent mailbox poisoned"))?;
        // Child Events frames precede Failure on the ordered control stream.
        if state.events.is_empty()
            && let Some(error) = &state.failure
        {
            return Err(error.clone());
        }
        let mut events = Vec::new();
        for _ in 0..max {
            let Some((event, bytes)) = state.events.pop_front() else {
                break;
            };
            state.event_bytes -= bytes;
            events.push(event);
        }
        Ok(events)
    }
    fn terminate(&mut self) -> Result<(), WorkerError> {
        {
            let mut state = self
                .queue
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.stopping = true;
            self.queue.changed.notify_all();
        }
        self.backend.retire();
        let _ = self.control.shutdown(Shutdown::Both);
        let _ = self.services.shutdown(Shutdown::Both);
        if let Some(child) = &mut self.child {
            match child.try_wait().map_err(failure)? {
                Some(_) => {}
                None => {
                    child.kill().map_err(failure)?;
                    child.wait().map_err(failure)?;
                }
            }
            self.child = None;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut panicked = false;
        loop {
            let mut index = 0;
            while index < self.threads.len() {
                if self.threads[index].is_finished() {
                    let thread = self.threads.swap_remove(index);
                    panicked |= thread.join().is_err();
                } else {
                    index += 1;
                }
            }
            if self.threads.is_empty() {
                let drained = self
                    .backend
                    .drain(deadline.saturating_duration_since(std::time::Instant::now()));
                return if panicked {
                    Err(failure("agent IPC thread panicked"))
                } else {
                    drained
                };
            }
            if std::time::Instant::now() >= deadline {
                return Err(failure("agent broker cancellation has not drained"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for AgentChildProcess {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_retirement_wakes_a_blocked_reply_without_request_lock() {
        let (client, mut parent) = UnixStream::pair().unwrap();
        let client = Arc::new(ChildServices::new(client).unwrap());
        let caller = client.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = caller.request(AgentServiceRequest::PlatformPoll { max: 1 });
            done.send(result.is_err()).unwrap();
        });
        let request: ServiceRequest = read_frame(&mut parent).unwrap();
        assert_eq!(request.id, 1);
        client.retire();
        assert!(completed.recv_timeout(Duration::from_secs(1)).unwrap());
        thread.join().unwrap();
    }
    #[test]
    fn service_correlation_mismatch_revokes_the_endpoint() {
        let (client, mut parent) = UnixStream::pair().unwrap();
        let client = ChildServices::new(client).unwrap();
        let thread = std::thread::spawn(move || {
            let request: ServiceRequest = read_frame(&mut parent).unwrap();
            write_frame(
                &mut parent,
                &ServiceResponse {
                    id: request.id + 1,
                    result: Ok(AgentServiceResponse::Done),
                },
            )
            .unwrap();
        });
        assert!(
            client
                .request(AgentServiceRequest::PlatformPoll { max: 1 })
                .is_err()
        );
        assert!(client.retired.load(Ordering::Acquire));
        thread.join().unwrap();
    }
    struct Backend;
    impl WorkerTransport for Backend {
        fn request(&self, _: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
            Ok(AgentServiceResponse::Done)
        }
    }
    #[test]
    fn terminal_child_failure_follows_received_error_frames() {
        let (control, _) = UnixStream::pair().unwrap();
        let (services, _) = UnixStream::pair().unwrap();
        let queue = Arc::new(ParentQueue {
            state: Mutex::new(ParentState {
                commands: Default::default(),
                command_bytes: 0,
                events: Default::default(),
                event_bytes: 0,
                admitted: true,
                stopping: false,
                closed: false,
                failure: Some(WorkerError::new(
                    "TypeError",
                    "original child startup failure",
                )),
                suspension_epoch: 0,
                suspend_requested: false,
                suspend_acknowledged: false,
                suspension_control: None,
            }),
            changed: std::sync::Condvar::new(),
            max_messages: 2,
            max_bytes: 4096,
        });
        {
            let mut state = queue.state.lock().unwrap();
            for line in [1, 2] {
                let event = AgentEvent::Error {
                    message: "source failure".encode_utf16().collect(),
                    filename: "worker.js".into(),
                    line,
                    column: 1,
                };
                let bytes = crate::agent_thread::wire_size(&event, 4096).unwrap();
                state.event_bytes += bytes;
                state.events.push_back((event, bytes));
            }
        }
        let mut process = AgentChildProcess {
            placement: crate::execution_placement::ExecutionPlacement::thread(),
            child: None,
            control,
            services,
            backend: Arc::new(Backend),
            queue,
            threads: Vec::new(),
        };
        assert!(process.poll(0).unwrap().is_empty());
        for expected in [1, 2] {
            assert!(
                matches!(process.poll(1).unwrap().as_slice(),[AgentEvent::Error{line,..}] if *line==expected)
            );
        }
        assert_eq!(process.queue.state.lock().unwrap().event_bytes, 0);
        for _ in 0..2 {
            assert_eq!(
                process.poll(1).unwrap_err().message,
                "original child startup failure"
            );
        }
        process.terminate().unwrap();
    }

    #[test]
    fn parent_command_admission_is_bounded_and_does_not_write_socket_synchronously() {
        let (control, _) = UnixStream::pair().unwrap();
        let (services, _) = UnixStream::pair().unwrap();
        let queue = Arc::new(ParentQueue {
            state: Mutex::new(ParentState {
                commands: Default::default(),
                command_bytes: 0,
                events: Default::default(),
                event_bytes: 0,
                admitted: false,
                stopping: false,
                closed: false,
                failure: None,
                suspension_epoch: 0,
                suspend_requested: false,
                suspend_acknowledged: false,
                suspension_control: None,
            }),
            changed: std::sync::Condvar::new(),
            max_messages: 1,
            max_bytes: 1024,
        });
        let mut process = AgentChildProcess {
            placement: crate::execution_placement::ExecutionPlacement::thread(),
            child: None,
            control,
            services,
            backend: Arc::new(Backend),
            queue,
            threads: Vec::new(),
        };
        process.start().unwrap();
        assert_eq!(process.queue.state.lock().unwrap().commands.len(), 1);
        process.set_suspended(true).unwrap();
        {
            let state = process.queue.state.lock().unwrap();
            assert_eq!(state.commands.len(), 1);
            assert!(matches!(
                state.suspension_control.as_ref(),
                Some(ParentFrame::Suspension {
                    suspended: true,
                    ..
                })
            ));
        }
        assert!(!process.suspension_status().unwrap());
        process.set_suspended(false).unwrap();
        assert!(matches!(
            process
                .queue
                .state
                .lock()
                .unwrap()
                .suspension_control
                .as_ref(),
            Some(ParentFrame::Suspension {
                epoch: 2,
                suspended: false
            })
        ));
        assert_eq!(
            process.send(AgentCommand::Stop).unwrap_err().name,
            "QuotaExceededError"
        );
        process.terminate().unwrap();
        assert!(process.start().is_err());
    }
}

#[cfg(test)]
mod service_publication_tests {
    use super::*;
    struct Custodian(std::sync::atomic::AtomicUsize);
    impl WorkerTransport for Custodian {
        fn request(&self, _: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
            unreachable!()
        }
        fn reclaim_response(
            &self,
            _: ServiceResponseScope,
            response: &AgentServiceResponse,
        ) -> Result<(), WorkerError> {
            assert!(matches!(response, AgentServiceResponse::Done));
            self.0.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }
    #[test]
    fn decoded_but_unacknowledged_response_retains_parent_custody() {
        for mismatched in [false, true] {
            let (mut parent, mut child) = UnixStream::pair().unwrap();
            let reader = std::thread::spawn(move || {
                let response: ServiceResponse = read_frame(&mut child).unwrap();
                if mismatched {
                    write_frame(
                        &mut child,
                        &ServiceReceipt {
                            received_id: response.id + 1,
                        },
                    )
                    .unwrap();
                }
            });
            let backend = Custodian(0.into());
            assert!(
                publish_service_response(
                    &mut parent,
                    &backend,
                    ServiceResponseScope::default(),
                    &ServiceResponse {
                        id: 3,
                        result: Ok(AgentServiceResponse::Done)
                    }
                )
                .is_err()
            );
            reader.join().unwrap();
            assert_eq!(backend.0.load(Ordering::Acquire), 1);
        }
    }
    #[test]
    fn accepted_response_transfers_custody_exactly_once() {
        let (mut parent, mut child) = UnixStream::pair().unwrap();
        let reader = std::thread::spawn(move || {
            let response: ServiceResponse = read_frame(&mut child).unwrap();
            write_frame(
                &mut child,
                &ServiceReceipt {
                    received_id: response.id,
                },
            )
            .unwrap();
        });
        let backend = Custodian(0.into());
        publish_service_response(
            &mut parent,
            &backend,
            ServiceResponseScope::default(),
            &ServiceResponse {
                id: 9,
                result: Ok(AgentServiceResponse::Done),
            },
        )
        .unwrap();
        reader.join().unwrap();
        assert_eq!(backend.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn failed_write_reclaims_without_waiting_for_receipt() {
        let (mut parent, child) = UnixStream::pair().unwrap();
        drop(child);
        let backend = Custodian(0.into());
        assert!(
            publish_service_response(
                &mut parent,
                &backend,
                ServiceResponseScope::default(),
                &ServiceResponse {
                    id: 1,
                    result: Ok(AgentServiceResponse::Done)
                }
            )
            .is_err()
        );
        assert_eq!(backend.0.load(Ordering::Acquire), 1);
    }
}
