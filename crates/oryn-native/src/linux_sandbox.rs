//! Fail-closed Linux page-worker sandbox installation.

#![cfg(target_os = "linux")]

use std::{fs, io, mem, os::fd::RawFd};

const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;

#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

pub fn install() -> io::Result<()> {
    install_namespaces().map_err(|error| stage_error("namespace isolation", error))?;
    prctl_no_new_privs().map_err(|error| stage_error("no_new_privs", error))?;
    install_landlock().map_err(|error| stage_error("Landlock", error))?;
    install_seccomp_denials().map_err(|error| stage_error("seccomp", error))?;
    Ok(())
}

fn stage_error(stage: &str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("Linux worker sandbox {stage} failed: {error}"),
    )
}

fn install_namespaces() -> io::Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    cvt(unsafe { libc::unshare(libc::CLONE_NEWUSER) })?;
    // The mapping files are the final filesystem writes performed by the
    // worker. Landlock is installed immediately afterwards.
    let _ = fs::write("/proc/self/setgroups", "deny\n");
    fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"))?;
    fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"))?;
    cvt(unsafe {
        libc::unshare(
            libc::CLONE_NEWNS | libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS,
        )
    })?;
    Ok(())
}

fn prctl_no_new_privs() -> io::Result<()> {
    cvt(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) })?;
    Ok(())
}

fn install_landlock() -> io::Result<()> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<LandlockRulesetAttr>(),
            0,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < 1 {
        return Err(io::Error::last_os_error());
    }
    let attr = LandlockRulesetAttr {
        handled_access_fs: LANDLOCK_ACCESS_FS_EXECUTE
            | LANDLOCK_ACCESS_FS_WRITE_FILE
            | LANDLOCK_ACCESS_FS_READ_FILE
            | LANDLOCK_ACCESS_FS_READ_DIR
            | LANDLOCK_ACCESS_FS_REMOVE_DIR
            | LANDLOCK_ACCESS_FS_REMOVE_FILE
            | LANDLOCK_ACCESS_FS_MAKE_CHAR
            | LANDLOCK_ACCESS_FS_MAKE_DIR
            | LANDLOCK_ACCESS_FS_MAKE_REG
            | LANDLOCK_ACCESS_FS_MAKE_SOCK
            | LANDLOCK_ACCESS_FS_MAKE_FIFO
            | LANDLOCK_ACCESS_FS_MAKE_BLOCK
            | LANDLOCK_ACCESS_FS_MAKE_SYM
            | if abi >= 2 {
                LANDLOCK_ACCESS_FS_REFER
            } else {
                0
            }
            | if abi >= 3 {
                LANDLOCK_ACCESS_FS_TRUNCATE
            } else {
                0
            },
    };
    let ruleset = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            mem::size_of::<LandlockRulesetAttr>(),
            0,
        )
    } as RawFd;
    if ruleset < 0 {
        return Err(io::Error::last_os_error());
    }
    let restricted = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0) };
    let close_result = unsafe { libc::close(ruleset) };
    if restricted != 0 {
        return Err(io::Error::last_os_error());
    }
    cvt(close_result)?;
    Ok(())
}

fn install_seccomp_denials() -> io::Result<()> {
    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JMP_JEQ_K: u16 = 0x15;
    const BPF_RET_K: u16 = 0x06;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
    const AUDIT_ARCH_AARCH64: u32 = 0xc000_00b7;

    let architecture = if cfg!(target_arch = "x86_64") {
        AUDIT_ARCH_X86_64
    } else if cfg!(target_arch = "aarch64") {
        AUDIT_ARCH_AARCH64
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported Linux worker architecture",
        ));
    };
    let denied = [
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_openat,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
    ];
    // AArch64 never exposed the legacy open(2), creat(2), fork(2), or
    // vfork(2) syscall entry points. Its libc routes the corresponding APIs
    // through openat(2), clone(2), and clone3(2), while x86-64 retains the
    // legacy numbers. Keep the filter buildable and accurate on both
    // production architectures.
    #[cfg(target_arch = "x86_64")]
    let legacy_denied = &[
        libc::SYS_open,
        libc::SYS_creat,
        libc::SYS_fork,
        libc::SYS_vfork,
    ][..];
    #[cfg(target_arch = "aarch64")]
    let legacy_denied: &[libc::c_long] = &[];
    let mut filter = vec![
        stmt(BPF_LD_W_ABS, 4),
        jump(BPF_JMP_JEQ_K, architecture, 1, 0),
        stmt(BPF_RET_K, libc::EPERM as u32 | SECCOMP_RET_ERRNO),
        stmt(BPF_LD_W_ABS, 0),
    ];
    for syscall in denied.into_iter().chain(legacy_denied.iter().copied()) {
        filter.push(jump(BPF_JMP_JEQ_K, syscall as u32, 0, 1));
        filter.push(stmt(BPF_RET_K, libc::EPERM as u32 | SECCOMP_RET_ERRNO));
    }
    filter.push(stmt(BPF_RET_K, SECCOMP_RET_ALLOW));
    let program = libc::sock_fprog {
        len: filter.len().try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "seccomp program is too large")
        })?,
        filter: filter.as_mut_ptr(),
    };
    cvt(unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &program as *const libc::sock_fprog,
        )
    })?;
    Ok(())
}

const fn stmt(code: u16, value: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k: value,
    }
}

const fn jump(code: u16, value: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt,
        jf,
        k: value,
    }
}

fn cvt(value: libc::c_int) -> io::Result<libc::c_int> {
    if value == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}
