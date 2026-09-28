//! Optional, payload-free native diagnostics. Never authority or protocol success.
//! The caller supplies one private append-only regular-file capability. Writes
//! are bounded single syscalls, with non-waiting local and cross-process locks.
//! CPU values are process-lifetime counters, not per-thread or pure-JS time.
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_RECORDS: u64 = 4096;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 32 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static SCOPES: AtomicU64 = AtomicU64::new(1);

#[cfg(unix)]
mod unix {
    use super::*;
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::sync::{Mutex, OnceLock};

    struct Sink {
        file: File,
        writing: Mutex<()>,
    }
    static SINK: OnceLock<Sink> = OnceLock::new();

    fn validate(fd: RawFd) -> std::io::Result<()> {
        if fd < 3 {
            return Err(std::io::Error::other(
                "diagnostic descriptor must be a dedicated capability",
            ));
        }
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstat initializes this exact ABI buffer; descriptor is borrowed.
        if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0
            || stat.st_mode & libc::S_IFMT != libc::S_IFREG
            || stat.st_mode & 0o777 != 0o600
            || stat.st_uid != unsafe { libc::geteuid() }
            || stat.st_nlink != 1
            || flags & libc::O_APPEND == 0
            || flags & libc::O_ACCMODE == libc::O_RDONLY
        {
            return Err(std::io::Error::other(
                "diagnostic descriptor must be private owned regular append file",
            ));
        }
        Ok(())
    }

    pub(super) fn install(fd: RawFd) -> std::io::Result<()> {
        validate(fd)?;
        // Do not let the ambient argument descriptor escape any later exec.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Keep the source safely above broker/context destination slots 3/4/5.
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 64) };
        if duplicate < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(duplicate) };
        SINK.set(Sink {
            file,
            writing: Mutex::new(()),
        })
        .map_err(|_| std::io::Error::other("native diagnostics already installed"))
    }

    pub(super) fn fd() -> Option<RawFd> {
        // The retained File is never closed or replaced while this process runs.
        Some(SINK.get()?.file.as_raw_fd())
    }

    fn locked_write(file: &File, bytes: &[u8], limit: u64) -> bool {
        if bytes.len() > MAX_RECORD_BYTES || bytes.last() != Some(&b'\n') {
            return false;
        }
        let fd = file.as_raw_fd();
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_WRLCK as _;
        lock.l_whence = libc::SEEK_SET as _;
        lock.l_start = 0;
        lock.l_len = 1;
        // POSIX record locks are per-process even for inherited descriptions.
        // F_SETLK never waits: contention is explicitly missing diagnostics.
        if unsafe { libc::fcntl(fd, libc::F_SETLK, &lock) } < 0 {
            return false;
        }
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let allowed = if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } == 0 {
            let size = unsafe { stat.assume_init() }.st_size;
            size >= 0
                && (size as u64)
                    .checked_add(bytes.len() as u64)
                    .is_some_and(|end| end <= limit)
        } else {
            false
        };
        let written = allowed
            && unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) }
                == bytes.len() as isize;
        lock.l_type = libc::F_UNLCK as _;
        let _ = unsafe { libc::fcntl(fd, libc::F_SETLK, &lock) };
        written
    }

    pub(super) fn write(bytes: &[u8]) -> bool {
        let Some(sink) = SINK.get() else { return false };
        let Ok(_guard) = sink.writing.try_lock() else {
            return false;
        };
        locked_write(&sink.file, bytes, MAX_BYTES)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs::OpenOptions;
        use std::io::Read;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        fn temporary() -> (std::path::PathBuf, File) {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "oryn-native-timing-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let file = OpenOptions::new()
                .read(true)
                .append(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            (path, file)
        }

        #[test]
        fn only_private_append_regular_capabilities_are_accepted() {
            let (path, file) = temporary();
            assert!(validate(file.as_raw_fd()).is_ok());
            file.set_permissions(std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(validate(file.as_raw_fd()).is_err());
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .unwrap();
            let read_only = File::open(&path).unwrap();
            assert!(validate(read_only.as_raw_fd()).is_err());
            let (socket, _) = std::os::unix::net::UnixStream::pair().unwrap();
            assert!(validate(socket.as_raw_fd()).is_err());
            assert!(validate(-1).is_err());
            assert!(validate(2).is_err());
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn bounded_append_preserves_prior_bytes_and_rejects_overflow() {
            let (path, file) = temporary();
            assert!(locked_write(&file, b"one\n", 8));
            assert!(locked_write(&file, b"two\n", 8));
            assert!(!locked_write(&file, b"x\n", 8));
            assert!(!locked_write(&file, b"unterminated", 100));
            let mut content = String::new();
            File::open(&path)
                .unwrap()
                .read_to_string(&mut content)
                .unwrap();
            assert_eq!(content, "one\ntwo\n");
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn explicit_exec_capability_collects_pipe_processes_and_does_not_escape_exec() {
            use std::os::unix::process::CommandExt;
            use std::process::{Command, Stdio};
            const MODE: &str = "ORYN_NATIVE_TIMING_TEST_MODE";
            const FILTER: &str = "native_timing::unix::tests::explicit_exec_capability_collects_pipe_processes_and_does_not_escape_exec";
            match std::env::var(MODE).ok().as_deref() {
                Some("closed") => {
                    let fd: i32 = std::env::var("ORYN_NATIVE_TIMING_TEST_DUP")
                        .unwrap()
                        .parse()
                        .unwrap();
                    // The exact diagnostic inode must not survive a later exec.
                    let expected: u64 = std::env::var("ORYN_NATIVE_TIMING_TEST_INODE")
                        .unwrap()
                        .parse()
                        .unwrap();
                    for fd in [5, fd] {
                        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                        if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } == 0 {
                            assert_ne!(unsafe { stat.assume_init() }.st_ino as u64, expected);
                        }
                    }
                    return;
                }
                Some("emit") | Some("locked") => {
                    install(5).unwrap();
                    let sink = SINK.get().unwrap();
                    if std::env::var(MODE).unwrap() == "locked" {
                        assert!(!write(b"blocked\n"));
                        return;
                    }
                    super::super::point("exec_child", Value::Null);
                    let pid = std::process::id();
                    for (kind, version) in [
                        ("oryn_context_rpc_progress", 1),
                        ("oryn_context_parent_summary", 2),
                    ] {
                        let mut bytes =
                            serde_json::to_vec(&json!({"kind":kind,"version":version,"pid":pid}))
                                .unwrap();
                        bytes.push(b'\n');
                        super::super::aggregate(&bytes);
                    }
                    {
                        let _unfinished = super::super::Scope::start("abandoned_test", Value::Null);
                    }
                    let mut scope = super::super::Scope::start("completed_test", Value::Null);
                    scope.finish(true);
                    scope.finish(true); // idempotent finish must not duplicate.
                    let mut command =
                        super::super::CommandScope::start("command_test", json!({"command_id":42}));
                    std::thread::spawn(|| {
                        let _outside = super::super::stage("foreign_thread");
                        super::super::count("foreign_thread", 1);
                    })
                    .join()
                    .unwrap();
                    for _ in 0..10_000 {
                        let _poll = super::super::stage("poll");
                        super::super::count("empty_poll", 1);
                    }
                    {
                        let mut nested = super::super::CommandScope::start("nested", Value::Null);
                        let stage = super::super::stage("work");
                        super::super::count("items", 7);
                        drop(stage);
                        nested.finish(true);
                    }
                    command.set_identity(json!({"command_id":42,"context_id":11}));
                    command.finish(true);
                    command.finish(true);
                    let abandoned =
                        super::super::CommandScope::start("abandoned_command", Value::Null);
                    let unfinished = super::super::stage("unfinished");
                    drop(abandoned);
                    drop(unfinished); // stale guard after command end must be inert
                    let coalescer = super::super::TurnCoalescer::default();
                    let before_turns = sink.file.metadata().unwrap().len();
                    for index in 0..32 {
                        // This integration assertion isolates count-triggered
                        // writes from host scheduling delays. The real 250ms
                        // boundary has separate deterministic state tests.
                        coalescer.state.lock().unwrap().pending_since_ns = Some(u64::MAX);
                        let mut turn = super::super::CommandScope::start(
                            "native_actor_turn",
                            json!({"turn":"poll","page_id":42}),
                        );
                        let stage = super::super::stage("poll");
                        super::super::count("empty_poll", 1);
                        drop(stage);
                        turn.finish_into(true, &coalescer);
                        super::super::OPERATIONS.with(|slot| assert!(slot.borrow().is_none()));
                        if index < 31 {
                            assert_eq!(sink.file.metadata().unwrap().len(), before_turns);
                        }
                    }
                    assert!(sink.file.metadata().unwrap().len() > before_turns);
                    // A partial retirement tail is emitted exactly once.
                    let mut tail = super::super::CommandScope::start(
                        "native_actor_turn",
                        json!({"turn":"poll","page_id":42}),
                    );
                    tail.finish_into(true, &coalescer);
                    coalescer.retire();
                    let retired_size = sink.file.metadata().unwrap().len();
                    coalescer.retire();
                    drop(coalescer);
                    assert_eq!(sink.file.metadata().unwrap().len(), retired_size);
                    // Lost-only state must produce a final explicit loss record,
                    // even if no subsequent turn ever obtains the try-lock.
                    let lost = super::super::TurnCoalescer::default();
                    let held = lost.state.lock().unwrap();
                    let mut missed =
                        super::super::CommandScope::start("native_actor_turn", Value::Null);
                    missed.finish_into(true, &lost);
                    lost.retire(); // contended retirement defers to exclusive Drop
                    drop(held);
                    drop(lost);
                    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                    assert_eq!(
                        unsafe { libc::fstat(sink.file.as_raw_fd(), stat.as_mut_ptr()) },
                        0
                    );
                    let inode = unsafe { stat.assume_init() }.st_ino;
                    let output = Command::new(std::env::current_exe().unwrap())
                        .args(["--exact", FILTER, "--nocapture"])
                        .env(MODE, "closed")
                        .env(
                            "ORYN_NATIVE_TIMING_TEST_DUP",
                            sink.file.as_raw_fd().to_string(),
                        )
                        .env("ORYN_NATIVE_TIMING_TEST_INODE", inode.to_string())
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return;
                }
                None => {}
                _ => panic!("invalid test mode"),
            }
            let (path, file) = temporary();
            let source = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
            assert!(source >= 64);
            let source = unsafe { File::from_raw_fd(source) };
            let launch = |mode: &str| {
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args(["--exact", FILTER, "--nocapture"])
                    .env(MODE, mode)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let fd = source.as_raw_fd();
                // SAFETY: only async-signal-safe descriptor duplication before exec.
                unsafe {
                    command.pre_exec(move || {
                        if libc::dup2(fd, 5) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let result = command.output().unwrap();
                assert!(
                    result.status.success(),
                    "stdout={} stderr={}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
            };
            let mut lock: libc::flock = unsafe { std::mem::zeroed() };
            lock.l_type = libc::F_WRLCK as _;
            lock.l_whence = libc::SEEK_SET as _;
            lock.l_len = 1;
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) },
                0
            );
            launch("locked");
            assert_eq!(file.metadata().unwrap().len(), 0);
            lock.l_type = libc::F_UNLCK as _;
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) },
                0
            );
            launch("emit");
            let rows: Vec<Value> = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(rows.len(), 12);
            let aggregates: Vec<_> = rows
                .iter()
                .filter(|row| row["event"] == "aggregate")
                .collect();
            assert_eq!(
                aggregates.len(),
                5,
                "polls write only at scope or coalescer boundaries"
            );
            let aggregate = aggregates
                .iter()
                .find(|r| r["phase"] == "command_test")
                .unwrap();
            assert_eq!(aggregate["identity"]["command_id"], 42);
            assert_eq!(aggregate["identity"]["context_id"], 11);
            assert_eq!(aggregate["aggregation"]["complete"], true);
            assert_eq!(aggregate["aggregation"]["scope_returned"], true);
            assert!(aggregate["aggregation"].get("command_returned").is_none());
            let operations = aggregate["aggregation"]["operations"].as_array().unwrap();
            assert!(!operations.iter().any(|row| row["name"] == "foreign_thread"));
            assert_eq!(
                operations.iter().find(|r| r["name"] == "poll").unwrap()["calls"],
                10_000
            );
            assert_eq!(
                operations
                    .iter()
                    .find(|r| r["name"] == "empty_poll")
                    .unwrap()["count"],
                10_000
            );
            assert_eq!(
                operations.iter().find(|r| r["name"] == "nested").unwrap()["calls"],
                1
            );
            let abandoned = aggregates
                .iter()
                .find(|r| r["phase"] == "abandoned_command")
                .unwrap();
            assert_eq!(abandoned["outcome"], "abandoned");
            assert_eq!(abandoned["aggregation"]["complete"], false);
            assert_eq!(abandoned["aggregation"]["scope_returned"], false);
            assert_eq!(abandoned["aggregation"]["unclosed_stages"], 1);
            let merged = aggregates
                .iter()
                .find(|row| row["aggregation"]["merged_turns"] == 32)
                .unwrap();
            assert_eq!(merged["aggregation"]["flush_reason"], "turn_limit");
            assert_eq!(
                merged["aggregation"]["scope_ids"].as_array().unwrap().len(),
                32
            );
            let lost = aggregates
                .iter()
                .find(|row| row["aggregation"]["merged_turns"] == 0)
                .unwrap();
            assert_eq!(lost["aggregation"]["coalescer_missed_turns"], 1);
            assert_eq!(lost["aggregation"]["complete"], false);
            assert_eq!(lost["aggregation"]["identity_unavailable"], true);
            assert!(
                rows.iter()
                    .all(|row| row["pid"].as_u64().unwrap() != std::process::id() as u64)
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row["event"] == "end" && row["outcome"] == "abandoned")
                    .count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row["event"] == "end" && row["outcome"] == "returned")
                    .count(),
                1
            );
            assert!(
                rows.iter()
                    .filter(|row| row["kind"] == "oryn_native_timing")
                    .all(|row| row["terminal_cpu_complete"] == false)
            );
            assert_eq!(
                rows.iter()
                    .filter(
                        |row| row["kind"] == "oryn_context_parent_summary" && row["version"] == 2
                    )
                    .count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row["kind"] == "oryn_context_rpc_progress" && row["version"] == 1)
                    .count(),
                1
            );
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn transfer_duplicate_is_close_on_exec_and_above_protocol_slots() {
            let (path, file) = temporary();
            let duplicate = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
            assert!(duplicate >= 64);
            let owned = unsafe { File::from_raw_fd(duplicate) };
            assert_ne!(
                unsafe { libc::fcntl(owned.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
            assert!(validate(owned.as_raw_fd()).is_ok());
            std::fs::remove_file(path).unwrap();
        }
    }
}

/// Optional CLI capability setup. No environment variable grants are consulted.
pub fn install_inherited(fd: i32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        unix::install(fd)
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        Err(std::io::Error::other("native diagnostic fd requires Unix"))
    }
}

pub(crate) fn inherited_fd() -> Option<i32> {
    #[cfg(unix)]
    {
        unix::fd()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(crate) fn enabled() -> bool {
    inherited_fd().is_some()
}

fn cpu() -> (Option<u128>, Option<u128>) {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let usage = unsafe { usage.assume_init() };
            let convert = |value: libc::timeval| {
                let seconds = u128::try_from(value.tv_sec).ok()?;
                let micros = u128::try_from(value.tv_usec).ok()?;
                (micros < 1_000_000).then_some(seconds * 1_000_000_000 + micros * 1000)
            };
            return (convert(usage.ru_utime), convert(usage.ru_stime));
        }
    }
    (None, None)
}

fn birth(pid: u32) -> Option<[u64; 2]> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let written = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if written == size {
            let info = unsafe { info.assume_init() };
            if info.pbi_pid == pid && info.pbi_start_tvusec < 1_000_000 {
                return Some([info.pbi_start_tvsec, info.pbi_start_tvusec]);
            }
        }
    }
    let _ = pid;
    None
}

fn monotonic() -> Option<u128> {
    #[cfg(unix)]
    {
        let mut time = std::mem::MaybeUninit::<libc::timespec>::zeroed();
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, time.as_mut_ptr()) } == 0 {
            let time = unsafe { time.assume_init() };
            if time.tv_sec >= 0 && (0..1_000_000_000).contains(&time.tv_nsec) {
                return Some(time.tv_sec as u128 * 1_000_000_000 + time.tv_nsec as u128);
            }
        }
    }
    None
}

fn encode(
    phase: &'static str,
    event: &'static str,
    scope: Option<u64>,
    elapsed: Option<u128>,
    outcome: Option<&'static str>,
    identity: Value,
) -> Value {
    let cpu_sample_start_ns = monotonic();
    let (user, system) = cpu();
    let cpu_sample_end_ns = monotonic();
    let pid = std::process::id();
    #[cfg(unix)]
    let parent_pid = unsafe { libc::getppid() };
    #[cfg(not(unix))]
    let parent_pid = 0;
    json!({"kind":"oryn_native_timing","version":1,"phase":phase,"event":event,
        "pid":pid,"parent_pid":parent_pid,"kernel_birth":birth(pid),
        "clock_domain":"clock_monotonic","elapsed_clock":"rust_instant","monotonic_ns":monotonic(),"unix_time_ns":SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|v|v.as_nanos()),
        "scope_id":scope,"parent_scope_id":null,"elapsed_ns":elapsed,"outcome":outcome,
        "cpu_user_ns":user,"cpu_system_ns":system,"cpu_scope":"process_lifetime",
        "cpu_sample_start_ns":cpu_sample_start_ns,"cpu_sample_end_ns":cpu_sample_end_ns,
        "terminal_cpu_complete":false,"identity":identity})
}

fn emit(mut record: Value) {
    if !enabled() {
        return;
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if sequence > MAX_RECORDS {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if sequence == MAX_RECORDS {
        record = encode(
            "diagnostic_capacity",
            "truncated",
            None,
            None,
            None,
            Value::Null,
        );
    }
    record["sequence"] = json!(sequence);
    record["prior_dropped_records"] = json!(DROPPED.load(Ordering::Relaxed));
    if let Ok(mut bytes) = serde_json::to_vec(&record) {
        bytes.push(b'\n');
        #[cfg(unix)]
        if !unix::write(&bytes) {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Existing context diagnostics retain their own cumulative/delta schemas.
pub(crate) fn aggregate(bytes: &[u8]) {
    if !enabled() {
        return;
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if sequence >= MAX_RECORDS {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        if sequence == MAX_RECORDS {
            let mut marker = encode(
                "diagnostic_capacity",
                "truncated",
                None,
                None,
                None,
                Value::Null,
            );
            marker["sequence"] = json!(sequence);
            marker["prior_dropped_records"] = json!(DROPPED.load(Ordering::Relaxed));
            if let Ok(mut bytes) = serde_json::to_vec(&marker) {
                bytes.push(b'\n');
                #[cfg(unix)]
                let _ = unix::write(&bytes);
            }
        }
        return;
    }
    #[cfg(unix)]
    if !unix::write(bytes) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn point(phase: &'static str, identity: Value) {
    if enabled() {
        emit(encode(phase, "point", None, None, None, identity));
    }
}

pub(crate) fn worker_identity(
    process_session: u64,
    context_id: u64,
    page_id: u64,
    generation: u64,
    worker_pid: Option<u32>,
    role: &str,
) -> Value {
    json!({"process_session":process_session,"context_id":context_id,"page_id":page_id,
        "route_generation":generation,"worker_pid":worker_pid,
        "worker_kernel_birth":worker_pid.and_then(birth),"role":role})
}

const MAX_OPERATIONS: usize = 64;
const MAX_STAGE_DEPTH: usize = 64;
const MAX_OPERATION_NAME: usize = 64;

#[derive(Default)]
struct Operation {
    name: &'static str,
    calls: u64,
    inclusive_ns: u64,
    exclusive_ns: u64,
    count: u64,
}

struct Frame {
    id: u64,
    operation: usize,
    started_ns: u64,
    children_ns: u64,
}

struct Operations {
    scope: u64,
    origin: Instant,
    start_monotonic_ns: Option<u128>,
    rows: Vec<Operation>,
    stack: Vec<Frame>,
    next_frame: u64,
    dropped_operations: u64,
    nesting_errors: u64,
    unclosed_stages: u64,
    saturated: bool,
}

fn add_timing(total: &mut u64, value: u64, saturated: &mut bool) {
    match total.checked_add(value) {
        Some(sum) => *total = sum,
        None => {
            *total = u64::MAX;
            *saturated = true;
        }
    }
}

impl Operations {
    fn new(scope: u64) -> Self {
        Self {
            scope,
            origin: Instant::now(),
            start_monotonic_ns: monotonic(),
            rows: Vec::new(),
            stack: Vec::new(),
            next_frame: 0,
            dropped_operations: 0,
            nesting_errors: 0,
            unclosed_stages: 0,
            saturated: false,
        }
    }

    fn now(&mut self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or_else(|_| {
            self.saturated = true;
            u64::MAX
        })
    }

    fn operation(&mut self, name: &'static str) -> Option<usize> {
        if let Some(index) = self.rows.iter().position(|row| row.name == name) {
            return Some(index);
        }
        if name.is_empty()
            || name.len() > MAX_OPERATION_NAME
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
            || self.rows.len() == MAX_OPERATIONS
        {
            self.dropped_operations = self.dropped_operations.saturating_add(1);
            return None;
        }
        self.rows.push(Operation {
            name,
            ..Operation::default()
        });
        Some(self.rows.len() - 1)
    }

    fn begin(&mut self, name: &'static str, now: u64) -> Option<(u64, u64)> {
        if self.stack.len() == MAX_STAGE_DEPTH || self.next_frame == u64::MAX {
            self.dropped_operations = self.dropped_operations.saturating_add(1);
            return None;
        }
        let operation = self.operation(name)?;
        self.next_frame += 1;
        add_timing(&mut self.rows[operation].calls, 1, &mut self.saturated);
        self.stack.push(Frame {
            id: self.next_frame,
            operation,
            started_ns: now,
            children_ns: 0,
        });
        Some((self.scope, self.next_frame))
    }

    fn pop(&mut self, now: u64) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if now < frame.started_ns || frame.children_ns > now.saturating_sub(frame.started_ns) {
            self.nesting_errors = self.nesting_errors.saturating_add(1);
        }
        let inclusive = now.saturating_sub(frame.started_ns);
        let exclusive = inclusive.saturating_sub(frame.children_ns);
        let row = &mut self.rows[frame.operation];
        add_timing(&mut row.inclusive_ns, inclusive, &mut self.saturated);
        add_timing(&mut row.exclusive_ns, exclusive, &mut self.saturated);
        if let Some(parent) = self.stack.last_mut() {
            add_timing(&mut parent.children_ns, inclusive, &mut self.saturated);
        }
    }

    fn end(&mut self, token: (u64, u64), now: u64) {
        if token.0 != self.scope {
            return;
        }
        let Some(index) = self.stack.iter().position(|frame| frame.id == token.1) else {
            return;
        };
        // A moved/dropped parent must not panic or corrupt a later command.
        // Close descendants at the same instant and label the partial account.
        if index + 1 != self.stack.len() {
            self.nesting_errors = self.nesting_errors.saturating_add(1);
            self.unclosed_stages = self
                .unclosed_stages
                .saturating_add((self.stack.len() - index - 1) as u64);
        }
        while self.stack.len() > index {
            self.pop(now);
        }
    }

    fn increment(&mut self, name: &'static str, value: u64) {
        if let Some(index) = self.operation(name) {
            add_timing(&mut self.rows[index].count, value, &mut self.saturated);
        }
    }

    fn close_stages(&mut self, now: u64) {
        self.unclosed_stages = self.unclosed_stages.saturating_add(self.stack.len() as u64);
        while !self.stack.is_empty() {
            self.pop(now);
        }
    }

    fn complete(&self) -> bool {
        self.dropped_operations == 0
            && self.nesting_errors == 0
            && self.unclosed_stages == 0
            && !self.saturated
    }

    fn summary(mut self, now: u64) -> Value {
        self.close_stages(now);
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|row| {
                json!({
                    "name":row.name,"calls":row.calls,"inclusive_ns":row.inclusive_ns,
                    "exclusive_ns":row.exclusive_ns,"count":row.count,
                })
            })
            .collect();
        json!({"version":1,"thread_scope":"synchronous_current_thread",
            "complete":self.complete(),"dropped_operations":self.dropped_operations,
            "nesting_errors":self.nesting_errors,"unclosed_stages":self.unclosed_stages,
            "saturated":self.saturated,"operations":rows})
    }
}

thread_local! {
    static OPERATIONS: std::cell::RefCell<Option<Operations>> = const { std::cell::RefCell::new(None) };
}

/// A synchronous, thread-bound nested stage. Dropping it only updates memory.
/// Do not hold across an await or transfer work to another thread: this measures
/// elapsed wall time in the current command, not CPU or asynchronous causality.
#[must_use = "bind the stage guard for the operation's lifetime"]
pub struct Stage {
    token: Option<(u64, u64)>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

fn stage_guard(token: Option<(u64, u64)>) -> Stage {
    Stage {
        token,
        _thread_bound: std::marker::PhantomData,
    }
}

/// Time a named operation only inside an enabled CommandScope. Names must be
/// short static labels, never URLs, scripts, or other payloads.
pub fn stage(name: &'static str) -> Stage {
    if !enabled() {
        return stage_guard(None);
    }
    let token = OPERATIONS
        .try_with(|cell| {
            let mut current = cell.try_borrow_mut().ok()?;
            let operations = current.as_mut()?;
            let now = operations.now();
            operations.begin(name, now)
        })
        .ok()
        .flatten();
    stage_guard(token)
}

impl Drop for Stage {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        let _ = OPERATIONS.try_with(|cell| {
            let Ok(mut current) = cell.try_borrow_mut() else {
                return;
            };
            if let Some(operations) = current.as_mut() {
                let now = operations.now();
                operations.end(token, now);
            }
        });
    }
}

/// Add a diagnostic counter without a clock read, serialization, or file write.
pub fn count(name: &'static str, value: u64) {
    if !enabled() {
        return;
    }
    let _ = OPERATIONS.try_with(|cell| {
        let Ok(mut current) = cell.try_borrow_mut() else {
            return;
        };
        if let Some(operations) = current.as_mut() {
            operations.increment(name, value);
        }
    });
}

/// Aggregate one outer synchronous native scope. Nested scopes join as stages.
/// Exactly one bounded record is attempted at finish/drop, through the existing
/// descriptor capability, non-waiting locks and global diagnostic limits.
/// A returned actor turn or checkpoint is not command completion: scope_returned
/// describes only this synchronous boundary, never protocol or custody success.
#[must_use = "bind the command guard until command completion"]
pub struct CommandScope {
    root: Option<(u64, &'static str, Value)>,
    joined: Option<Stage>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl CommandScope {
    pub fn start(phase: &'static str, identity: Value) -> Self {
        let mut result = Self {
            root: None,
            joined: None,
            _thread_bound: std::marker::PhantomData,
        };
        if !enabled() {
            return result;
        }
        let _ = OPERATIONS.try_with(|cell| {
            let Ok(mut current) = cell.try_borrow_mut() else {
                return;
            };
            if let Some(operations) = current.as_mut() {
                let now = operations.now();
                result.joined = Some(stage_guard(operations.begin(phase, now)));
            } else {
                let scope = SCOPES.fetch_add(1, Ordering::Relaxed);
                *current = Some(Operations::new(scope));
                result.root = Some((scope, phase, identity));
            }
        });
        result
    }

    /// The outer command owns identity; nested scopes cannot replace it.
    pub fn set_identity(&mut self, identity: Value) {
        if let Some((_, _, current)) = self.root.as_mut() {
            *current = identity;
        }
    }

    pub fn finish(&mut self, success: bool) {
        self.close(if success { "returned" } else { "error" });
    }

    /// Close every TLS stage now, then hand only completed totals to a bounded
    /// registration-owned coalescer. No timing span survives this call.
    pub fn finish_into(&mut self, success: bool, coalescer: &TurnCoalescer) {
        if let Some(completed) = self.completed(if success { "returned" } else { "error" }) {
            coalescer.submit(completed);
        }
    }

    fn completed(&mut self, outcome: &'static str) -> Option<CompletedTurn> {
        self.joined.take();
        let (scope, phase, identity) = self.root.take()?;
        let operations = OPERATIONS
            .try_with(|cell| {
                let mut current = cell.try_borrow_mut().ok()?;
                if current.as_ref()?.scope != scope {
                    return None;
                }
                current.take()
            })
            .ok()
            .flatten();
        if let Some(mut operations) = operations {
            let now = operations.now();
            operations.close_stages(now);
            Some(CompletedTurn {
                phase,
                identity,
                outcome,
                elapsed_ns: now,
                end_monotonic_ns: monotonic(),
                operations,
            })
        } else {
            let mut record = encode(
                phase,
                "aggregate",
                Some(scope),
                None,
                Some(outcome),
                identity,
            );
            record["aggregation"] =
                json!({"version":1,"complete":false,"error":"state_unavailable"});
            record["aggregation"]["scope_returned"] = json!(outcome == "returned");
            emit(record);
            None
        }
    }

    fn close(&mut self, outcome: &'static str) {
        if let Some(completed) = self.completed(outcome) {
            emit(completed.record());
        }
    }
}

impl Drop for CommandScope {
    fn drop(&mut self) {
        self.close("abandoned");
    }
}

struct CompletedTurn {
    phase: &'static str,
    identity: Value,
    outcome: &'static str,
    elapsed_ns: u64,
    end_monotonic_ns: Option<u128>,
    operations: Operations,
}

impl CompletedTurn {
    fn record(self) -> Value {
        let mut record = encode(
            self.phase,
            "aggregate",
            Some(self.operations.scope),
            Some(self.elapsed_ns as u128),
            Some(self.outcome),
            self.identity,
        );
        record["aggregation"] = self.operations.summary(self.elapsed_ns);
        record["aggregation"]["scope_returned"] = json!(self.outcome == "returned");
        record
    }
}

const COALESCE_TURNS: u64 = 32;
const COALESCE_NS: u64 = 250_000_000;
const COALESCE_KEYS: usize = 4;

struct TurnGroup {
    turn: CompletedTurn,
    scope_ids: Vec<u64>,
    turns: u64,
    returned_turns: u64,
    error_turns: u64,
    abandoned_turns: u64,
    sum_elapsed_ns: u64,
    first_start_ns: Option<u128>,
    last_end_ns: Option<u128>,
    reason: &'static str,
}

impl TurnGroup {
    fn new(turn: CompletedTurn) -> Self {
        Self {
            scope_ids: vec![turn.operations.scope],
            turns: 1,
            returned_turns: u64::from(turn.outcome == "returned"),
            error_turns: u64::from(turn.outcome == "error"),
            abandoned_turns: u64::from(turn.outcome == "abandoned"),
            sum_elapsed_ns: turn.elapsed_ns,
            first_start_ns: turn.operations.start_monotonic_ns,
            last_end_ns: turn.end_monotonic_ns,
            turn,
            reason: "pending",
        }
    }

    fn merge(&mut self, next: CompletedTurn) {
        self.turns += 1; // The whole pending batch flushes after at most 32 turns.
        self.returned_turns += u64::from(next.outcome == "returned");
        self.error_turns += u64::from(next.outcome == "error");
        self.abandoned_turns += u64::from(next.outcome == "abandoned");
        self.scope_ids.push(next.operations.scope);
        self.first_start_ns = self
            .first_start_ns
            .zip(next.operations.start_monotonic_ns)
            .map(|(a, b)| a.min(b));
        self.last_end_ns = self
            .last_end_ns
            .zip(next.end_monotonic_ns)
            .map(|(a, b)| a.max(b));
        let into = &mut self.turn.operations;
        let from = next.operations;
        add_timing(
            &mut self.sum_elapsed_ns,
            next.elapsed_ns,
            &mut into.saturated,
        );
        add_timing(
            &mut into.dropped_operations,
            from.dropped_operations,
            &mut into.saturated,
        );
        add_timing(
            &mut into.nesting_errors,
            from.nesting_errors,
            &mut into.saturated,
        );
        add_timing(
            &mut into.unclosed_stages,
            from.unclosed_stages,
            &mut into.saturated,
        );
        into.saturated |= from.saturated;
        for row in from.rows {
            if let Some(index) = into.operation(row.name) {
                let target = &mut into.rows[index];
                add_timing(&mut target.calls, row.calls, &mut into.saturated);
                add_timing(
                    &mut target.inclusive_ns,
                    row.inclusive_ns,
                    &mut into.saturated,
                );
                add_timing(
                    &mut target.exclusive_ns,
                    row.exclusive_ns,
                    &mut into.saturated,
                );
                add_timing(&mut target.count, row.count, &mut into.saturated);
            }
        }
        if next.outcome != "returned" {
            self.turn.outcome = next.outcome;
        }
    }

    fn record(self, missed: u64, coalescer_id: u64) -> Value {
        let span = self
            .first_start_ns
            .zip(self.last_end_ns)
            .and_then(|(first, last)| last.checked_sub(first));
        let mut record = encode(
            self.turn.phase,
            "aggregate",
            None,
            Some(self.sum_elapsed_ns as u128),
            Some(self.turn.outcome),
            self.turn.identity,
        );
        let mut aggregation = self.turn.operations.summary(0);
        aggregation["scope_returned"] = json!(self.turn.outcome == "returned");
        aggregation["coalesced"] = json!(true);
        aggregation["merged_turns"] = json!(self.turns);
        aggregation["returned_turns"] = json!(self.returned_turns);
        aggregation["error_turns"] = json!(self.error_turns);
        aggregation["abandoned_turns"] = json!(self.abandoned_turns);
        aggregation["sum_of_turn_elapsed_ns"] = json!(self.sum_elapsed_ns);
        aggregation["first_scope_id"] = json!(self.scope_ids.first());
        aggregation["last_scope_id"] = json!(self.scope_ids.last());
        aggregation["scope_ids"] = json!(self.scope_ids);
        aggregation["first_turn_start_monotonic_ns"] = json!(self.first_start_ns);
        aggregation["last_turn_end_monotonic_ns"] = json!(self.last_end_ns);
        aggregation["span_including_idle_gaps_ns"] = json!(span);
        aggregation["elapsed_semantics"] = json!(
            "sum of completed synchronous turns; span includes idle gaps; neither is CPU or command completion"
        );
        aggregation["cpu_semantics"] =
            json!("process-lifetime sample at flush; no per-turn CPU attribution");
        aggregation["flush_reason"] = json!(self.reason);
        aggregation["coalescer_missed_turns"] = json!(missed);
        aggregation["coalescer_id"] = json!(coalescer_id);
        aggregation["loss_attribution"] = json!(
            "coalescer-wide; missed turn identities unknown, not attributed to this group's owner"
        );
        if missed != 0 {
            aggregation["complete"] = json!(false);
        }
        record["aggregation"] = aggregation;
        record
    }
}

#[derive(Default)]
struct CoalescedTurns {
    base_identity: Option<Value>,
    groups: Vec<TurnGroup>,
    pending_turns: u64,
    pending_since_ns: Option<u64>,
}

impl CoalescedTurns {
    fn drain(&mut self, reason: &'static str) -> Vec<TurnGroup> {
        let mut groups = std::mem::take(&mut self.groups);
        for group in &mut groups {
            group.reason = reason;
        }
        self.base_identity = None;
        self.pending_turns = 0;
        self.pending_since_ns = None;
        groups
    }

    fn push(&mut self, turn: CompletedTurn, now: u64, retired: bool) -> Vec<TurnGroup> {
        // Every identity field except the explicit turn kind participates in
        // the epoch. In particular owner, page, route and registration cannot
        // silently merge when a page is reused or a navigation owner changes.
        let mut base = turn.identity.clone();
        if let Some(object) = base.as_object_mut() {
            object.remove("turn");
        }
        let mut ready = Vec::new();
        if self
            .base_identity
            .as_ref()
            .is_some_and(|value| value != &base)
        {
            ready.extend(self.drain("identity_change"));
        }
        let matching = self.groups.iter().position(|group| {
            group.turn.phase == turn.phase && group.turn.identity == turn.identity
        });
        if matching.is_none() && self.groups.len() == COALESCE_KEYS {
            ready.extend(self.drain("turn_key_capacity"));
        }
        self.base_identity = Some(base);
        self.pending_since_ns.get_or_insert(now);
        let exceptional = turn.outcome != "returned" || !turn.operations.complete();
        let matching = self.groups.iter().position(|group| {
            group.turn.phase == turn.phase && group.turn.identity == turn.identity
        });
        if let Some(index) = matching {
            self.groups[index].merge(turn);
        } else {
            self.groups.push(TurnGroup::new(turn));
        }
        self.pending_turns += 1;
        let reason = if retired {
            Some("registration_retired")
        } else if exceptional
            || self
                .groups
                .iter()
                .any(|group| !group.turn.operations.complete())
        {
            Some("incomplete_or_error")
        } else if self.pending_turns >= COALESCE_TURNS {
            Some("turn_limit")
        } else if now.saturating_sub(self.pending_since_ns.unwrap_or(now)) >= COALESCE_NS {
            Some("time_limit")
        } else {
            None
        };
        if let Some(reason) = reason {
            ready.extend(self.drain(reason));
        }
        ready
    }
}

/// Registration-owned, bounded storage of *completed* actor-turn summaries.
/// Up to four exact turn-kind keys share one identity epoch and 32-turn/250ms
/// budget. Time is checked when a turn completes; there is no timer, wakeup, or
/// scheduling effect. Call retire on registration retirement; Drop also drains.
pub struct TurnCoalescer {
    id: u64,
    state: std::sync::Mutex<CoalescedTurns>,
    missed: AtomicU64,
    retired: std::sync::atomic::AtomicBool,
    origin: Instant,
}

impl Default for TurnCoalescer {
    fn default() -> Self {
        Self {
            id: SCOPES.fetch_add(1, Ordering::Relaxed),
            state: Default::default(),
            missed: AtomicU64::new(0),
            retired: std::sync::atomic::AtomicBool::new(false),
            origin: Instant::now(),
        }
    }
}

impl TurnCoalescer {
    fn missed(&self) {
        let _ = self
            .missed
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
    }

    fn submit(&self, turn: CompletedTurn) {
        let ready = match self.state.try_lock() {
            Ok(mut state) => state.push(
                turn,
                u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX),
                self.retired.load(Ordering::Acquire),
            ),
            Err(_) => {
                self.missed();
                return;
            }
        };
        // No JSON encoding, CPU sampling, file locks or writes under state lock.
        self.publish(ready, false, "submission");
    }

    fn publish(&self, ready: Vec<TurnGroup>, terminal: bool, reason: &'static str) {
        if ready.is_empty() && !terminal {
            return;
        }
        let missed = self.missed.swap(0, Ordering::AcqRel);
        if ready.is_empty() && missed != 0 {
            let mut record = encode(
                "native_actor_turn",
                "aggregate",
                None,
                None,
                None,
                Value::Null,
            );
            record["aggregation"] = json!({"version":1,"coalesced":true,"complete":false,
                "merged_turns":0,"coalescer_missed_turns":missed,"flush_reason":reason,
                "scope_returned":null,"identity_unavailable":true,"coalescer_id":self.id,
                "loss_attribution":"coalescer-wide; missed turn identities unknown"});
            emit(record);
        } else {
            for (index, group) in ready.into_iter().enumerate() {
                // Charge losses once, rather than duplicating them per key.
                emit(group.record(if index == 0 { missed } else { 0 }, self.id));
            }
        }
    }

    /// Idempotent, nonwaiting retirement. Any concurrently finishing turn sees
    /// retirement and flushes immediately; final Drop preserves a contended tail.
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        let ready = match self.state.try_lock() {
            Ok(mut state) => state.drain("registration_retired"),
            Err(_) => return,
        };
        self.publish(ready, true, "registration_retired");
    }
}

impl Drop for TurnCoalescer {
    fn drop(&mut self) {
        // Exclusive ownership means no lock wait, including a poisoned mutex.
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ready = state.drain("coalescer_drop");
        self.publish(ready, true, "coalescer_drop");
    }
}

pub struct Scope {
    phase: &'static str,
    scope: u64,
    started: Instant,
    identity: Value,
    completed: bool,
    active: bool,
}
impl Scope {
    pub fn start(phase: &'static str, identity: Value) -> Self {
        let started = Instant::now();
        let active = enabled();
        let scope = SCOPES.fetch_add(1, Ordering::Relaxed);
        if active {
            emit(encode(
                phase,
                "begin",
                Some(scope),
                None,
                None,
                identity.clone(),
            ));
        }
        Self {
            phase,
            scope,
            started,
            identity,
            completed: false,
            active,
        }
    }
    /// Add identities allocated during this scope; begin/end still join by PID and scope_id.
    pub fn set_identity(&mut self, identity: Value) {
        self.identity = identity;
    }
    pub fn finish(&mut self, success: bool) {
        if self.completed {
            return;
        }
        self.completed = true;
        if self.active {
            emit(encode(
                self.phase,
                "end",
                Some(self.scope),
                Some(self.started.elapsed().as_nanos()),
                Some(if success { "returned" } else { "error" }),
                self.identity.clone(),
            ));
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.active && !self.completed {
            emit(encode(
                self.phase,
                "end",
                Some(self.scope),
                Some(self.started.elapsed().as_nanos()),
                Some("abandoned"),
                self.identity.clone(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation<'a>(summary: &'a Value, name: &str) -> &'a Value {
        summary["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()
    }

    fn completed_turn(id: u64, kind: &str, owner: u64, elapsed: u64) -> CompletedTurn {
        let mut operations = Operations::new(id);
        operations.start_monotonic_ns = Some(id as u128 * 1_000);
        let parent = operations.begin("parent", 0).unwrap();
        let child = operations.begin("child", elapsed / 4).unwrap();
        operations.end(child, elapsed / 2);
        operations.end(parent, elapsed);
        operations.increment("empty_poll", 1);
        CompletedTurn {
            phase: "native_actor_turn",
            identity: json!({"turn":kind,"page_id":7,"route_generation":9,"retained_navigation_owner":owner}),
            outcome: "returned",
            elapsed_ns: elapsed,
            end_monotonic_ns: Some(id as u128 * 1_000 + elapsed as u128),
            operations,
        }
    }

    #[test]
    fn completed_turn_coalescing_sums_nested_accounts_with_bounded_mixed_keys() {
        let mut state = CoalescedTurns::default();
        for id in 1..32 {
            let kind = if id % 2 == 0 { "poll" } else { "dispatch" };
            assert!(
                state
                    .push(completed_turn(id, kind, 1, 100), id, false)
                    .is_empty()
            );
        }
        let ready = state.push(completed_turn(32, "poll", 1, 100), 32, false);
        assert_eq!(ready.len(), 2);
        assert_eq!(
            ready
                .iter()
                .map(|group| group.scope_ids.len())
                .sum::<usize>(),
            32
        );
        assert!(state.groups.is_empty());
        for group in ready {
            assert_eq!(group.reason, "turn_limit");
            let record = group.record(0, 7);
            let summary = &record["aggregation"];
            assert_eq!(summary["merged_turns"], 16);
            assert_eq!(summary["returned_turns"], 16);
            assert_eq!(summary["sum_of_turn_elapsed_ns"], 1600);
            assert_eq!(summary["span_including_idle_gaps_ns"], 30_100);
            assert_eq!(summary["complete"], true);
            assert_eq!(operation(summary, "parent")["calls"], 16);
            assert_eq!(operation(summary, "parent")["inclusive_ns"], 1600);
            assert_eq!(operation(summary, "parent")["exclusive_ns"], 1200);
            assert_eq!(operation(summary, "child")["inclusive_ns"], 400);
            assert_eq!(operation(summary, "empty_poll")["count"], 16);
            assert!(record["scope_id"].is_null());
            assert_eq!(record["elapsed_ns"], 1600);
            assert_eq!(record["terminal_cpu_complete"], false);
        }
    }

    #[test]
    fn completed_turn_time_identity_key_capacity_and_error_flushes_are_explicit() {
        let mut state = CoalescedTurns::default();
        assert!(
            state
                .push(completed_turn(1, "poll", 1, 10), 0, false)
                .is_empty()
        );
        assert!(
            state
                .push(completed_turn(2, "poll", 1, 10), COALESCE_NS - 1, false)
                .is_empty()
        );
        let ready = state.push(completed_turn(3, "poll", 1, 10), COALESCE_NS, false);
        assert_eq!(ready[0].reason, "time_limit");
        assert_eq!(ready[0].turns, 3);
        state.push(completed_turn(4, "poll", 1, 10), 0, false);
        let old = state.push(completed_turn(5, "poll", 2, 10), 1, false);
        assert_eq!(old[0].reason, "identity_change");
        assert_eq!(old[0].turn.identity["retained_navigation_owner"], 1);
        assert_eq!(
            state.groups[0].turn.identity["retained_navigation_owner"],
            2
        );
        let mut changed_route = completed_turn(6, "poll", 2, 10);
        changed_route.identity["route_generation"] = json!(10);
        assert_eq!(
            state.push(changed_route, 2, false)[0].reason,
            "identity_change"
        );
        state.drain("test_reset");
        for (id, kind) in ["a", "b", "c", "d"].into_iter().enumerate() {
            assert!(
                state
                    .push(completed_turn(id as u64, kind, 1, 10), 0, false)
                    .is_empty()
            );
        }
        let old = state.push(completed_turn(5, "e", 1, 10), 0, false);
        assert_eq!(old.len(), 4);
        assert!(old.iter().all(|group| group.reason == "turn_key_capacity"));
        assert_eq!(state.groups.len(), 1);
        let mut error = completed_turn(6, "e", 1, 10);
        error.outcome = "error";
        let ready = state.push(error, 0, false);
        assert_eq!(ready[0].reason, "incomplete_or_error");
        assert_eq!(ready[0].turns, 2);
        let record = ready.into_iter().next().unwrap().record(0, 7);
        assert_eq!(record["aggregation"]["scope_returned"], false);
        assert_eq!(record["aggregation"]["error_turns"], 1);
        assert!(state.groups.is_empty());
    }

    #[test]
    fn completed_turn_unknown_clocks_saturation_and_retired_tail_are_not_hidden() {
        let mut state = CoalescedTurns::default();
        let mut first = completed_turn(1, "poll", 1, 100);
        first.operations.start_monotonic_ns = None;
        first.operations.increment("counter", u64::MAX);
        state.push(first, 0, false);
        let mut next = completed_turn(2, "poll", 1, 100);
        next.end_monotonic_ns = None;
        next.operations.increment("counter", 1);
        let ready = state.push(next, 1, false);
        assert_eq!(ready[0].reason, "incomplete_or_error");
        let record = ready.into_iter().next().unwrap().record(2, 55);
        let summary = &record["aggregation"];
        assert_eq!(summary["complete"], false);
        assert_eq!(summary["saturated"], true);
        assert_eq!(summary["coalescer_missed_turns"], 2);
        assert_eq!(summary["coalescer_id"], 55);
        assert!(summary["first_turn_start_monotonic_ns"].is_null());
        assert!(summary["last_turn_end_monotonic_ns"].is_null());
        assert!(summary["span_including_idle_gaps_ns"].is_null());
        state.push(completed_turn(3, "poll", 1, 100), 0, false);
        assert_eq!(state.drain("registration_retired").len(), 1);
        assert!(state.drain("coalescer_drop").is_empty());
        let late = state.push(completed_turn(4, "poll", 1, 100), 0, true);
        assert_eq!(late[0].reason, "registration_retired");
        assert!(state.groups.is_empty());
    }

    #[test]
    fn nested_and_recursive_stages_have_exact_inclusive_and_exclusive_accounts() {
        let mut operations = Operations::new(1);
        let root = operations.begin("parse", 10).unwrap();
        let child = operations.begin("parse", 20).unwrap();
        let leaf = operations.begin("decode", 25).unwrap();
        operations.end(leaf, 30);
        operations.end(child, 40);
        let sibling = operations.begin("decode", 45).unwrap();
        operations.end(sibling, 55);
        operations.end(root, 70);
        operations.increment("bytes", 17);
        operations.increment("bytes", 23);
        let summary = operations.summary(80);
        assert_eq!(summary["complete"], true);
        assert_eq!(operation(&summary, "parse")["calls"], 2);
        assert_eq!(operation(&summary, "parse")["inclusive_ns"], 80);
        assert_eq!(operation(&summary, "parse")["exclusive_ns"], 45);
        assert_eq!(operation(&summary, "decode")["calls"], 2);
        assert_eq!(operation(&summary, "decode")["inclusive_ns"], 15);
        assert_eq!(operation(&summary, "decode")["exclusive_ns"], 15);
        assert_eq!(operation(&summary, "bytes")["count"], 40);
        assert_eq!(operation(&summary, "bytes")["calls"], 0);
    }

    #[test]
    fn out_of_order_and_unclosed_stages_are_partial_and_stale_guards_are_inert() {
        let mut operations = Operations::new(2);
        let parent = operations.begin("parent", 0).unwrap();
        let child = operations.begin("child", 10).unwrap();
        operations.end(parent, 30);
        operations.end(child, 40); // already closed, never counted a second time
        let summary = operations.summary(50);
        assert_eq!(summary["complete"], false);
        assert_eq!(summary["nesting_errors"], 1);
        assert_eq!(summary["unclosed_stages"], 1);
        assert_eq!(operation(&summary, "parent")["exclusive_ns"], 10);
        assert_eq!(operation(&summary, "child")["inclusive_ns"], 20);

        let mut next = Operations::new(3);
        let fresh = next.begin("fresh", 0).unwrap();
        next.end(parent, 100); // a guard from another command cannot close this one
        assert_eq!(next.stack.len(), 1);
        next.end(fresh, 10);
        assert_eq!(next.summary(20)["complete"], true);

        let mut abandoned = Operations::new(4);
        abandoned.begin("open", 5);
        let summary = abandoned.summary(25);
        assert_eq!(summary["complete"], false);
        assert_eq!(summary["unclosed_stages"], 1);
        assert_eq!(operation(&summary, "open")["inclusive_ns"], 20);
    }

    #[test]
    fn stage_depth_names_and_counter_overflow_are_bounded_and_explicit() {
        let mut operations = Operations::new(1);
        let mut tokens = Vec::new();
        for time in 0..MAX_STAGE_DEPTH {
            tokens.push(operations.begin("recursive", time as u64).unwrap());
        }
        assert!(operations.begin("too_deep", 64).is_none());
        for token in tokens.into_iter().rev() {
            operations.end(token, 100);
        }
        assert_eq!(operations.rows.len(), 1);
        operations.increment("counter", u64::MAX);
        operations.increment("counter", 1);
        assert!(operations.begin("payload with spaces", 100).is_none());
        assert!(operations.begin("", 100).is_none());
        let summary = operations.summary(100);
        assert_eq!(summary["complete"], false);
        assert_eq!(summary["dropped_operations"], 3);
        assert_eq!(summary["saturated"], true);
        assert_eq!(operation(&summary, "counter")["count"], u64::MAX);

        let mut names = Operations::new(2);
        for index in 0..MAX_OPERATIONS {
            // Leaked static strings exist only in this bounded test fixture.
            let label = Box::leak(format!("operation_{index:02}").into_boxed_str());
            names.increment(label, 1);
        }
        names.increment("one_too_many", 1);
        let summary = names.summary(0);
        assert_eq!(
            summary["operations"].as_array().unwrap().len(),
            MAX_OPERATIONS
        );
        assert_eq!(summary["dropped_operations"], 1);
        assert!(serde_json::to_vec(&summary).unwrap().len() < MAX_RECORD_BYTES / 2);
    }

    #[test]
    fn disabled_aggregation_has_no_state_and_no_record_attempts() {
        // Capability installation is isolated in the Unix subprocess test.
        assert!(!enabled());
        let before = SEQUENCE.load(Ordering::Relaxed);
        let mut command = CommandScope::start("disabled", Value::Null);
        let stage = stage("disabled_stage");
        count("empty_poll", 100);
        drop(stage);
        command.finish(true);
        drop(command);
        OPERATIONS.with(|state| assert!(state.borrow().is_none()));
        assert_eq!(SEQUENCE.load(Ordering::Relaxed), before);
    }

    #[test]
    fn cpu_and_clock_records_are_lifetime_counters_not_claimed_terminal_usage() {
        let value = encode("worker_entry", "point", None, None, None, Value::Null);
        assert_eq!(value["cpu_scope"], "process_lifetime");
        assert_eq!(value["terminal_cpu_complete"], false);
        assert!(value["elapsed_ns"].is_null());
        assert!(value["scope_id"].is_null());
        #[cfg(unix)]
        assert!(value["cpu_user_ns"].is_number() && value["monotonic_ns"].is_number());
    }
    #[test]
    fn worker_identity_keeps_process_and_environment_generations_separate() {
        let value = worker_identity(11, 22, 33, 44, Some(std::process::id()), "page");
        assert_eq!(value["process_session"], 11);
        assert_eq!(value["context_id"], 22);
        assert_eq!(value["page_id"], 33);
        assert_eq!(value["route_generation"], 44);
        assert_eq!(value["worker_pid"], std::process::id());
    }
}
