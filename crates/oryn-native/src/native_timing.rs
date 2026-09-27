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
            assert_eq!(rows.len(), 7);
            assert!(
                rows.iter()
                    .all(|row| row["pid"].as_u64().unwrap() != std::process::id() as u64)
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row["outcome"] == "abandoned")
                    .count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row["outcome"] == "returned")
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
