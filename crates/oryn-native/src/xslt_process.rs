//! One-request XSLT processes. Only a parent service/executor may call `run`;
//! sandboxed page workers request this service over their existing capability.
//! The helper role enters the normal worker sandbox before `serve_helper` and
//! never initializes V8 or Tokio. No stylesheet-controlled path is executed.

use oryn_js::platform::{XsltRequest, XsltResult};
use serde::Serialize;
use std::io::{self, Read, Write};

const MAX_JSON_BYTES: usize = 24 * 1024 * 1024;
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_URL_BYTES: usize = 64 * 1024;
const MAX_RESOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESOURCES: usize = 64;
const MAX_PARAMETERS: usize = 1024;
const MAX_ERROR_BYTES: usize = 4096;

type Response = Result<XsltResult, String>;

pub(crate) fn run(request: XsltRequest, cancelled: impl Fn() -> bool) -> Response {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        process::run(request, cancelled)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (request, cancelled);
        Err("XSLT helper requires the macOS or Linux worker sandbox".into())
    }
}

/// Called after environment clearing, binary identity initialization and the
/// Linux sandbox installation, before any runtime or V8 background threads.
pub(crate) fn serve_helper() -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        serve_once(
            io::stdin().lock(),
            io::stdout().lock(),
            oryn_js::xslt_engine::run_request,
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "XSLT helper requires the macOS or Linux worker sandbox",
        ))
    }
}

fn validate_request(request: &XsltRequest) -> Result<(), String> {
    if request.style.len() > MAX_DOCUMENT_BYTES {
        return Err("XSLT stylesheet exceeds 4 MiB".into());
    }
    if request
        .source
        .as_ref()
        .is_some_and(|source| source.len() > MAX_SOURCE_BYTES)
    {
        return Err("XSLT source exceeds 16 MiB".into());
    }
    if request.parameters.len() > MAX_PARAMETERS {
        return Err("XSLT parameter count exceeds 1024".into());
    }
    if request.resources.len() > MAX_RESOURCES {
        return Err("XSLT resource count exceeds 64".into());
    }
    if request.url.len() > MAX_URL_BYTES {
        return Err("XSLT document URL exceeds 64 KiB".into());
    }
    for (requested_url, final_url, body) in &request.resources {
        if requested_url.len() > MAX_URL_BYTES || final_url.len() > MAX_URL_BYTES {
            return Err("XSLT resource URL exceeds 64 KiB".into());
        }
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("XSLT resource body exceeds 4 MiB".into());
        }
    }
    let bytes = request
        .resources
        .iter()
        .try_fold(0_usize, |total, (_, _, body)| total.checked_add(body.len()));
    if bytes.is_none_or(|bytes| bytes > MAX_RESOURCE_BYTES) {
        return Err("XSLT resources exceed 16 MiB".into());
    }
    Ok(())
}

fn validate_result(has_source: bool, result: XsltResult) -> Response {
    if let Some((body, kind)) = &result.output
        && (body.len() > MAX_OUTPUT_BYTES || !matches!(kind, 0..=2))
    {
        return Err("XSLT helper returned an invalid output size or kind".into());
    }
    if (has_source && result.valid != result.output.is_some())
        || (!has_source && result.output.is_some())
    {
        return Err("XSLT helper returned an inconsistent result".into());
    }
    Ok(result)
}

fn bounded_error(mut error: String) -> String {
    if error.len() > MAX_ERROR_BYTES {
        let mut end = MAX_ERROR_BYTES - 3;
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        error.truncate(end);
        error.push_str("...");
    }
    error
}

struct JsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("XSLT JSON exceeds 24 MiB"));
        }
        // Reserve only admitted bytes; escaped JSON can exceed the source size.
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = JsonBuffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(|error| error.to_string())?;
    Ok(output.bytes)
}

fn serve_once(
    input: impl Read,
    mut output: impl Write,
    execute: impl FnOnce(&XsltRequest) -> Response,
) -> io::Result<()> {
    let mut input_bytes = Vec::new();
    let response = input
        .take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut input_bytes)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            if input_bytes.len() > MAX_JSON_BYTES {
                return Err("XSLT request JSON exceeds 24 MiB".into());
            }
            let request: XsltRequest = serde_json::from_slice(&input_bytes)
                .map_err(|error| format!("invalid XSLT request: {error}"))?;
            validate_request(&request)?;
            let has_source = request.source.is_some();
            execute(&request).and_then(|result| validate_result(has_source, result))
        })
        .map_err(bounded_error);
    let bytes = encode(&response, MAX_JSON_BYTES)
        .or_else(|_| {
            encode(
                &Response::Err("XSLT response JSON exceeds 24 MiB".into()),
                MAX_JSON_BYTES,
            )
        })
        .map_err(io::Error::other)?;
    output.write_all(&bytes)?;
    output.flush()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod process {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs::{File, Metadata, OpenOptions},
        os::{
            fd::AsRawFd,
            unix::{
                fs::{MetadataExt, OpenOptionsExt},
                process::CommandExt,
            },
        },
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    const MAX_HELPERS: usize = 4;
    const WALL_LIMIT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(2);
    const IO_CHUNK: usize = 64 * 1024;
    static ACTIVE_HELPERS: AtomicUsize = AtomicUsize::new(0);

    struct Admission<'a>(&'a AtomicUsize);

    impl<'a> Admission<'a> {
        fn acquire(count: &'a AtomicUsize) -> Result<Self, String> {
            count
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current < MAX_HELPERS).then(|| current + 1)
                })
                .map_err(|_| "XSLT helper capacity exhausted".to_string())?;
            Ok(Self(count))
        }
    }

    impl Drop for Admission<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn check(deadline: Instant, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if cancelled() {
            Err("XSLT transform cancelled".into())
        } else if Instant::now() >= deadline {
            Err("XSLT helper exceeded 10 second deadline".into())
        } else {
            Ok(())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Identity {
        dev: u64,
        ino: u64,
        len: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }

    impl Identity {
        fn of(metadata: &Metadata) -> io::Result<Self> {
            if !metadata.is_file() {
                return Err(io::Error::other("XSLT helper binary is not a regular file"));
            }
            Ok(Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
                len: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
        }
    }

    struct BinarySeal {
        file: File,
        path: PathBuf,
        identity: Identity,
    }

    // A single cached inode/digest avoids hashing the same immutable release
    // binary for every transform. A changed ctime, mtime, inode or size misses.
    static VERIFIED_BINARY: OnceLock<Mutex<Option<(Identity, String)>>> = OnceLock::new();

    impl BinarySeal {
        fn open(
            path: &Path,
            expected: &str,
            deadline: Instant,
            cancelled: &impl Fn() -> bool,
        ) -> Result<Self, String> {
            check(deadline, cancelled)?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(|error| format!("XSLT binary open: {error}"))?;
            let identity = Identity::of(&file.metadata().map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            let cache = VERIFIED_BINARY.get_or_init(|| Mutex::new(None));
            let cached = cache
                .lock()
                .map_err(|_| "XSLT binary cache poisoned".to_string())?
                .as_ref()
                .is_some_and(|(prior, hash)| prior == &identity && hash == expected);
            if !cached {
                let mut digest = Sha256::new();
                let mut bytes = [0_u8; IO_CHUNK];
                loop {
                    check(deadline, cancelled)?;
                    let count = match file.read(&mut bytes) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        value => value.map_err(|error| error.to_string())?,
                    };
                    if count == 0 {
                        break;
                    }
                    digest.update(&bytes[..count]);
                }
                if format!("{:x}", digest.finalize()) != expected {
                    return Err("XSLT helper binary digest changed before launch".into());
                }
            }
            let seal = Self {
                file,
                path: path.to_owned(),
                identity,
            };
            seal.verify()?;
            if !cached {
                *cache
                    .lock()
                    .map_err(|_| "XSLT binary cache poisoned".to_string())? =
                    Some((seal.identity.clone(), expected.to_owned()));
            }
            Ok(seal)
        }

        fn verify(&self) -> Result<(), String> {
            let open_identity =
                Identity::of(&self.file.metadata().map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?;
            // symlink_metadata also rejects replacement with a symlink.
            let path_identity = Identity::of(
                &std::fs::symlink_metadata(&self.path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if open_identity != self.identity || path_identity != self.identity {
                return Err(
                    "XSLT helper binary identity changed during launch or execution".into(),
                );
            }
            Ok(())
        }
    }

    fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
        // SAFETY: these descriptor queries change flags only on our owned pipe.
        unsafe {
            let flags = libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL);
            if flags < 0
                || libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn descriptor_ceiling() -> io::Result<i32> {
        let mut ceiling = 0_i32;
        let mut size = std::mem::size_of_val(&ceiling);
        // SAFETY: sysctl writes one integer to the correctly sized local buffer.
        let result = unsafe {
            libc::sysctlbyname(
                c"kern.maxfilesperproc".as_ptr(),
                (&mut ceiling as *mut i32).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if size != std::mem::size_of_val(&ceiling) || !(3..=1_048_576).contains(&ceiling) {
            return Err(io::Error::other(
                "unsupported XSLT helper descriptor ceiling",
            ));
        }
        Ok(ceiling)
    }

    fn restrict_child(command: &mut Command) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        let ceiling = descriptor_ceiling()?;
        // SAFETY: after fork this closure uses only descriptor and resource-limit
        // syscalls, does not allocate, and leaves the exec-error pipe open until
        // exec. No parent capability descriptor survives the exec boundary.
        unsafe {
            command.pre_exec(move || {
                let core = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_CORE, &core) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut cpu = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(libc::RLIMIT_CPU, &mut cpu) != 0 {
                    return Err(io::Error::last_os_error());
                }
                cpu.rlim_cur = cpu.rlim_cur.min(10);
                cpu.rlim_max = cpu.rlim_max.min(10);
                if libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0 {
                    return Err(io::Error::last_os_error());
                }
                #[cfg(target_os = "linux")]
                if libc::syscall(
                    libc::SYS_close_range,
                    3_u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                #[cfg(target_os = "macos")]
                for fd in 3..ceiling {
                    if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::EBADF) {
                            return Err(error);
                        }
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }

    /// Child owns its PID through successful wait, including every early return
    /// between spawn, attestation, pipe configuration and response decoding.
    struct OwnedHelper(Option<Child>);

    impl OwnedHelper {
        fn stop(&mut self) -> io::Result<()> {
            let Some(child) = self.0.as_mut() else {
                return Ok(());
            };
            drop(child.stdin.take());
            drop(child.stdout.take());
            drop(child.stderr.take());
            let observed = loop {
                match child.try_wait() {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    result => break result,
                }
            };
            let mut custody_error = None;
            let alive = match observed {
                Ok(status) => status.is_none(),
                Err(error) => {
                    custody_error = Some(format!("poll: {error}"));
                    true
                }
            };
            let mut kill_error = None;
            if alive && let Err(error) = child.kill() {
                kill_error = Some(error);
            }
            let waited = loop {
                match child.wait() {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    result => break result,
                }
            };
            match waited {
                Ok(_) => {
                    self.0.take();
                    // A child can exit between try_wait and kill. Successful
                    // reap establishes custody even when that kill raced exit.
                    match custody_error {
                        Some(error) => Err(io::Error::other(error)),
                        None => Ok(()),
                    }
                }
                Err(error) => Err(io::Error::other(match kill_error {
                    Some(kill) => format!("kill: {kill}; reap: {error}"),
                    None => format!("reap: {error}"),
                })),
            }
        }
    }

    impl Drop for OwnedHelper {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    struct PipeTransfer<'a, W, R> {
        input: &'a [u8],
        written: usize,
        writer: Option<W>,
        reader: R,
        output: Vec<u8>,
        eof: bool,
        limit: usize,
    }

    impl<'a, W: Write, R: Read> PipeTransfer<'a, W, R> {
        fn new(input: &'a [u8], writer: W, reader: R, limit: usize) -> Self {
            Self {
                input,
                written: 0,
                writer: Some(writer),
                reader,
                output: Vec::new(),
                eof: false,
                limit,
            }
        }

        fn step(&mut self) -> Result<bool, String> {
            let mut progress = false;
            if let Some(writer) = self.writer.as_mut() {
                let end = self.written.saturating_add(IO_CHUNK).min(self.input.len());
                if self.written < end {
                    match writer.write(&self.input[self.written..end]) {
                        Ok(0) => {
                            return Err(
                                "XSLT helper input pipe closed before request completed".into()
                            );
                        }
                        Ok(count) => {
                            self.written += count;
                            progress = true;
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => return Err(format!("XSLT helper input: {error}")),
                    }
                }
                if self.written == self.input.len() {
                    drop(self.writer.take()); // EOF terminates the single request.
                    progress = true;
                }
            }
            if !self.eof {
                let mut bytes = [0_u8; IO_CHUNK];
                match self.reader.read(&mut bytes) {
                    Ok(0) => {
                        self.eof = true;
                        progress = true;
                    }
                    Ok(count) => {
                        if count > self.limit.saturating_sub(self.output.len()) {
                            return Err("XSLT response JSON exceeds 24 MiB".into());
                        }
                        self.output.extend_from_slice(&bytes[..count]);
                        progress = true;
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(format!("XSLT helper output: {error}")),
                }
            }
            Ok(progress)
        }

        fn complete(&self) -> bool {
            self.writer.is_none() && self.eof
        }
    }

    fn exchange(
        child: &mut Child,
        input: &[u8],
        deadline: Instant,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<u8>, String> {
        let writer = child.stdin.take().ok_or("XSLT helper stdin missing")?;
        let reader = child.stdout.take().ok_or("XSLT helper stdout missing")?;
        nonblocking(&writer).map_err(|error| error.to_string())?;
        nonblocking(&reader).map_err(|error| error.to_string())?;
        let mut transfer = PipeTransfer::new(input, writer, reader, MAX_JSON_BYTES);
        loop {
            check(deadline, cancelled)?;
            let progress = transfer.step()?;
            let status = child
                .try_wait()
                .map_err(|error| format!("XSLT helper wait: {error}"))?;
            if let Some(status) = status {
                if !status.success() {
                    return Err(format!("XSLT helper exited unsuccessfully: {status}"));
                }
                if transfer.complete() {
                    return Ok(transfer.output);
                }
            }
            if !progress {
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }

    pub(super) fn run(request: XsltRequest, cancelled: impl Fn() -> bool) -> Response {
        let deadline = Instant::now() + WALL_LIMIT;
        check(deadline, &cancelled)?;
        validate_request(&request)?;
        let _admission = Admission::acquire(&ACTIVE_HELPERS)?;
        let input = encode(&request, MAX_JSON_BYTES)?;
        let has_source = request.source.is_some();
        drop(request);
        check(deadline, &cancelled)?;
        // This path applies macOS signature/manifest verification and Linux's
        // immutable multicall executable selection, including its role prefix.
        let (mut command, path, hash) = crate::worker_client::prepare_agent_command()
            .map_err(|error| {
                // The JS synchronous value adapter exposes NetworkError for
                // failed helper completions. Preserve a payload-free native
                // category so unavailable signed infrastructure is not mistaken
                // for a stylesheet fetch or transform algorithm failure.
                crate::native_timing::count("xslt.helper_discovery_failure", 1);
                format!("XSLT helper discovery: {error}")
            })?;
        let seal = BinarySeal::open(&path, &hash, deadline, &cancelled)?;
        command
            .arg("--xslt-helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        restrict_child(&mut command)
            .map_err(|error| format!("XSLT helper restrictions: {error}"))?;
        check(deadline, &cancelled)?;
        let child = command
            .spawn()
            .map_err(|error| format!("XSLT helper launch: {error}"))?;
        let mut owner = OwnedHelper(Some(child));
        let outcome = (|| {
            let child = owner.0.as_mut().ok_or("XSLT helper ownership missing")?;
            crate::worker_client::verify_agent_process(child.id(), &path)
                .map_err(|error| format!("XSLT helper process verification: {error}"))?;
            seal.verify()?;
            check(deadline, &cancelled)?;
            let output = exchange(child, &input, deadline, &cancelled)?;
            seal.verify()?;
            check(deadline, &cancelled)?;
            let response: Response = serde_json::from_slice(&output)
                .map_err(|error| format!("invalid XSLT helper response: {error}"))?;
            let result = response
                .map_err(bounded_error)
                .and_then(|result| validate_result(has_source, result));
            check(deadline, &cancelled)?;
            result
        })();
        let cleanup = owner.stop();
        match (outcome, cleanup) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(format!("XSLT helper cleanup: {error}")),
            (Err(primary), Err(error)) => Err(format!("{primary}; XSLT helper cleanup: {error}")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::net::UnixStream;

        #[test]
        fn verified_binary_replacement_is_rejected() {
            struct Directory(PathBuf);
            impl Drop for Directory {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let directory = Directory(
                std::env::temp_dir().join(format!("oryn-xslt-seal-{}", uuid::Uuid::new_v4())),
            );
            std::fs::create_dir(&directory.0).unwrap();
            let path = directory.0.join("worker");
            std::fs::write(&path, b"trusted helper").unwrap();
            let digest = format!("{:x}", Sha256::digest(b"trusted helper"));
            let seal =
                BinarySeal::open(&path, &digest, Instant::now() + WALL_LIMIT, &|| false).unwrap();
            let replacement = directory.0.join("replacement");
            std::fs::write(&replacement, b"different helper").unwrap();
            std::fs::rename(replacement, &path).unwrap();
            assert!(seal.verify().unwrap_err().contains("identity changed"));
            assert!(
                BinarySeal::open(&path, &digest, Instant::now() + WALL_LIMIT, &|| false).is_err()
            );
        }

        #[test]
        fn launcher_does_not_inherit_an_unmarked_capability_descriptor() {
            use std::os::fd::{FromRawFd, OwnedFd};
            let (socket, _peer) = UnixStream::pair().unwrap();
            // SAFETY: duplicate our live socket into a high, non-CLOEXEC slot;
            // OwnedFd takes sole ownership of that new descriptor.
            let raw = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_DUPFD, 200) };
            assert!(raw >= 200);
            let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "test ! -e /dev/fd/\"$1\"", "fd-probe"])
                .arg(descriptor.as_raw_fd().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            restrict_child(&mut command).unwrap();
            let mut owner = OwnedHelper(Some(command.spawn().unwrap()));
            let status = owner.0.as_mut().unwrap().wait().unwrap();
            owner.stop().unwrap();
            assert!(
                status.success(),
                "an unrelated parent descriptor crossed exec"
            );
        }

        #[test]
        fn admission_rejects_excess_work_and_recovers_on_drop() {
            let count = AtomicUsize::new(0);
            let mut leases = (0..MAX_HELPERS)
                .map(|_| Admission::acquire(&count).unwrap())
                .collect::<Vec<_>>();
            assert!(Admission::acquire(&count).is_err());
            leases.pop();
            let replacement = Admission::acquire(&count).unwrap();
            assert_eq!(count.load(Ordering::Acquire), MAX_HELPERS);
            drop(replacement);
            drop(leases);
            assert_eq!(count.load(Ordering::Acquire), 0);
        }

        #[test]
        fn duplex_transfer_drains_output_before_helper_consumes_large_input() {
            fn pipe() -> (File, File) {
                use std::os::fd::FromRawFd;
                let mut descriptors = [-1; 2];
                // SAFETY: pipe initializes two fresh owned descriptors.
                assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
                let ends = unsafe {
                    (File::from_raw_fd(descriptors[0]), File::from_raw_fd(descriptors[1]))
                };
                for end in [&ends.0, &ends.1] {
                    assert_eq!(unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) }, 0);
                }
                ends
            }
            // Production exchanges ChildStdin/ChildStdout pipes, not sockets.
            // Exercise that descriptor behavior with the same opposing order.
            let (mut helper_input, writer) = pipe();
            let (reader, mut helper_output) = pipe();
            nonblocking(&writer).unwrap();
            nonblocking(&reader).unwrap();
            let input = vec![b'i'; 2 * 1024 * 1024];
            let expected_output = vec![b'o'; 2 * 1024 * 1024];
            let helper = std::thread::spawn(move || -> io::Result<Vec<u8>> {
                // Both messages exceed pipe/socket capacity. Serial parent
                // write-then-read would deadlock at this opposite ordering.
                helper_output.write_all(&vec![b'o'; 2 * 1024 * 1024])?;
                drop(helper_output);
                let mut received = Vec::new();
                helper_input.read_to_end(&mut received)?;
                Ok(received)
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = (|| {
                let mut transfer = PipeTransfer::new(&input, writer, reader, MAX_JSON_BYTES);
                while !transfer.complete() {
                    check(deadline, &|| false).map_err(|error| format!(
                        "{error}; written={} output={} eof={} writer_open={}",
                        transfer.written, transfer.output.len(), transfer.eof,
                        transfer.writer.is_some(),
                    ))?;
                    if !transfer.step()? {
                        std::thread::sleep(POLL_INTERVAL);
                    }
                }
                Ok::<_, String>(transfer.output)
            })();
            // All parent pipe ends have closed before joining, even on failure.
            let received = helper.join().unwrap();
            assert_eq!(result.unwrap(), expected_output);
            assert_eq!(received.unwrap(), input);
        }

        #[test]
        fn transfer_rejects_oversized_output_and_truncated_input() {
            let mut transfer =
                PipeTransfer::new(b"request", io::sink(), io::Cursor::new(vec![0_u8; 9]), 8);
            assert!(transfer.step().unwrap_err().contains("response JSON"));
            struct Closed;
            impl Write for Closed {
                fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                    Ok(0)
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            let mut transfer = PipeTransfer::new(b"request", Closed, io::empty(), MAX_JSON_BYTES);
            assert!(
                transfer
                    .step()
                    .unwrap_err()
                    .contains("before request completed")
            );
        }

        #[test]
        fn cancellation_and_expired_deadline_have_distinct_errors() {
            assert_eq!(
                check(Instant::now() + WALL_LIMIT, &|| true).unwrap_err(),
                "XSLT transform cancelled"
            );
            assert_eq!(
                check(Instant::now(), &|| false).unwrap_err(),
                "XSLT helper exceeded 10 second deadline"
            );
        }

        #[test]
        fn stalled_owned_pipes_cancel_or_time_out_and_reap() {
            for cancel in [false, true] {
                let child = Command::new("/bin/sh")
                    .args(["-c", "exec /bin/sleep 30"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let pid = child.id() as libc::pid_t;
                let mut owner = OwnedHelper(Some(child));
                let trigger = Instant::now() + Duration::from_millis(20);
                let deadline = if cancel {
                    Instant::now() + WALL_LIMIT
                } else {
                    trigger
                };
                // The child never consumes stdin. A blocking write would hide
                // both cancellation and the wall deadline behind a full pipe.
                let outcome = exchange(
                    owner.0.as_mut().unwrap(),
                    &vec![b'x'; 1024 * 1024],
                    deadline,
                    &|| cancel && Instant::now() >= trigger,
                );
                owner.stop().unwrap();
                assert_eq!(
                    outcome.unwrap_err(),
                    if cancel {
                        "XSLT transform cancelled"
                    } else {
                        "XSLT helper exceeded 10 second deadline"
                    }
                );
                let mut status = 0;
                // SAFETY: observe only the exact child PID already reaped above.
                assert_eq!(
                    unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
                    -1
                );
                assert_eq!(
                    io::Error::last_os_error().raw_os_error(),
                    Some(libc::ECHILD)
                );
            }
        }

        #[test]
        fn owned_child_is_killed_and_reaped_on_early_return() {
            // A fixed host utility exercises ownership, never production worker
            // discovery. exec ensures the one child has no descendant to leak.
            let child = Command::new("/bin/sh")
                .args(["-c", "exec /bin/sleep 30"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id() as libc::pid_t;
            drop(OwnedHelper(Some(child)));
            let mut status = 0;
            // SAFETY: WNOHANG observes the exact PID we owned and already reaped.
            let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            assert_eq!(waited, -1);
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> XsltRequest {
        XsltRequest {
            style: "style".into(),
            source: None,
            parameters: Vec::new(),
            url: "https://example.test/".into(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn semantic_limits_reject_before_engine_execution() {
        let mut cases = Vec::new();
        let mut oversized = request();
        oversized.style = "s".repeat(MAX_DOCUMENT_BYTES + 1);
        cases.push(oversized);
        let mut oversized = request();
        oversized.source = Some("s".repeat(MAX_SOURCE_BYTES + 1));
        cases.push(oversized);
        let mut oversized = request();
        oversized.parameters =
            vec![(String::new(), String::new(), String::new()); MAX_PARAMETERS + 1];
        cases.push(oversized);
        let mut oversized = request();
        oversized.resources =
            vec![(String::new(), String::new(), String::new()); MAX_RESOURCES + 1];
        cases.push(oversized);
        let mut oversized = request();
        oversized.resources =
            vec![(String::new(), String::new(), "s".repeat(MAX_DOCUMENT_BYTES)); 5];
        cases.push(oversized);
        for oversized in cases {
            let input = encode(&oversized, MAX_JSON_BYTES).unwrap();
            let mut output = Vec::new();
            serve_once(input.as_slice(), &mut output, |_| {
                panic!("rejected input reached engine")
            })
            .unwrap();
            let response: Response = serde_json::from_slice(&output).unwrap();
            assert!(response.is_err());
        }
    }

    #[test]
    fn large_source_is_admitted_but_parameter_escaping_counts_toward_wire_budget() {
        let mut request = request();
        request.source = Some("s".repeat(MAX_DOCUMENT_BYTES + 1));
        assert!(validate_request(&request).is_ok());
        request.source = None;
        request.parameters.push((
            String::new(),
            "name".into(),
            "\u{0}".repeat(MAX_DOCUMENT_BYTES + 1),
        ));
        assert!(validate_request(&request).is_ok());
        assert!(encode(&request, MAX_JSON_BYTES).is_err());
    }

    #[test]
    fn malformed_and_excess_json_never_reach_engine() {
        for input in [b"{} {}".to_vec(), vec![b' '; MAX_JSON_BYTES + 1]] {
            let mut output = Vec::new();
            serve_once(input.as_slice(), &mut output, |_| {
                panic!("invalid JSON reached engine")
            })
            .unwrap();
            assert!(
                serde_json::from_slice::<Response>(&output)
                    .unwrap()
                    .is_err()
            );
        }
    }

    #[test]
    fn escaped_output_budget_produces_bounded_error_envelope() {
        let mut request = request();
        request.source = Some("source".into());
        let input = encode(&request, MAX_JSON_BYTES).unwrap();
        let mut output = Vec::new();
        serve_once(input.as_slice(), &mut output, |_| {
            Ok(XsltResult {
                valid: true,
                output: Some(("\u{0}".repeat(MAX_DOCUMENT_BYTES + 1), 1)),
            })
        })
        .unwrap();
        assert!(output.len() < MAX_ERROR_BYTES);
        assert!(
            serde_json::from_slice::<Response>(&output)
                .unwrap()
                .unwrap_err()
                .contains("response JSON")
        );
    }

    #[test]
    fn validation_transform_and_engine_error_preserve_protocol_meaning() {
        for source in [None, Some("source".into())] {
            let mut request = request();
            request.source = source;
            let expected = XsltResult {
                valid: true,
                output: request.source.as_ref().map(|_| ("<result/>".into(), 1)),
            };
            let input = encode(&request, MAX_JSON_BYTES).unwrap();
            let mut output = Vec::new();
            serve_once(input.as_slice(), &mut output, |_| Ok(expected.clone())).unwrap();
            assert_eq!(
                serde_json::from_slice::<Response>(&output)
                    .unwrap()
                    .unwrap(),
                expected
            );
        }
        let input = encode(&request(), MAX_JSON_BYTES).unwrap();
        let mut output = Vec::new();
        serve_once(input.as_slice(), &mut output, |_| {
            Err("specific engine failure".into())
        })
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Response>(&output)
                .unwrap()
                .unwrap_err(),
            "specific engine failure"
        );
        assert!(
            validate_result(
                false,
                XsltResult {
                    valid: true,
                    output: Some(("wrong".into(), 1))
                }
            )
            .is_err()
        );
        assert!(
            validate_result(
                true,
                XsltResult {
                    valid: false,
                    output: Some(("wrong".into(), 1))
                }
            )
            .is_err()
        );
        assert!(
            validate_result(
                true,
                XsltResult {
                    valid: true,
                    output: Some(("wrong".into(), 3))
                }
            )
            .is_err()
        );
        assert!(
            validate_result(
                true,
                XsltResult {
                    valid: true,
                    output: Some(("x".repeat(MAX_OUTPUT_BYTES + 1), 0))
                }
            )
            .is_err()
        );
    }
}
