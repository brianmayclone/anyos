use super::*;
use core::sync::atomic::{AtomicU32, Ordering};

static WAIT4_DEBUG_SEQ: AtomicU32 = AtomicU32::new(0);

#[cfg(target_arch = "x86_64")]
#[inline]
fn current_hw_fs_base() -> u64 {
    unsafe { crate::arch::x86::power::rdmsr(0xC000_0100) }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn current_hw_fs_base() -> u64 {
    0
}

fn log_errno_probe(tag: &str, fs_base: u64) {
    if fs_base < 0x400 || fs_base > 0x0000_8000_0000_0000 {
        return;
    }
    let mut hits = 0usize;
    let start = fs_base - 0x200;
    let end = fs_base + 0x400;
    let mut addr = start;
    while addr < end && hits < 12 {
        if handlers::helpers::is_user_range_accessible(addr, 4) {
            let value = unsafe { *(addr as *const u32) };
            if value == EINTR as u32 || value == ECHILD as u32 {
                crate::serial_verbose_println!(
                    "lxe linux errno-probe {}: fs={:#x} off={:+#x} value={}",
                    tag,
                    fs_base,
                    (addr as i64).wrapping_sub(fs_base as i64),
                    value
                );
                hits += 1;
            }
        }
        addr += 4;
    }
}

#[inline]
fn should_log_wait4_debug() -> bool {
    let seq = WAIT4_DEBUG_SEQ.fetch_add(1, Ordering::Relaxed);
    seq < 64 || (seq & 0x7f) == 0
}

pub(super) fn linux_unsupported_syscall(
    regs: &SyscallRegs,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> u64 {
    crate::serial_verbose_println!(
        "lxe linux: unsupported syscall nr={} rip={:#x} args={:#x},{:#x},{:#x},{:#x},{:#x},{:#x}",
        regs.rax,
        regs.rip,
        a1,
        a2,
        a3,
        a4,
        a5,
        a6
    );
    linux_err(ENOSYS)
}

pub(super) fn linux_clone(
    regs: &SyscallRegs,
    flags: u64,
    child_stack: u64,
    parent_tidptr: u64,
    child_tidptr: u64,
    tls: u64,
) -> u64 {
    const CSIGNAL: u64 = 0xff;
    const SIGCHLD: u64 = 17;
    const CLONE_VM: u64 = 0x0000_0100;
    const CLONE_PARENT_SETTID: u64 = 0x0010_0000;
    const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;
    const CLONE_CHILD_SETTID: u64 = 0x0100_0000;
    const CLONE_THREAD: u64 = 0x0001_0000;
    const CLONE_SETTLS: u64 = 0x0008_0000;

    let signal = flags & CSIGNAL;
    if signal != 0 && signal != SIGCHLD {
        return linux_err(EINVAL);
    }
    if (flags & (CLONE_VM | CLONE_THREAD | CLONE_SETTLS)) != 0 || tls != 0 {
        crate::serial_verbose_println!(
            "lxe linux clone: unsupported flags={:#x} child_stack={:#x} tls={:#x}",
            flags,
            child_stack,
            tls
        );
        return linux_err(ENOSYS);
    }
    if child_stack != 0 {
        crate::serial_verbose_println!(
            "lxe linux clone: unsupported custom child_stack={:#x} flags={:#x}",
            child_stack,
            flags
        );
        return linux_err(ENOSYS);
    }

    if (flags & CLONE_PARENT_SETTID) != 0
        && (parent_tidptr == 0 || !handlers::helpers::is_user_range_accessible(parent_tidptr, 4))
    {
        return linux_err(EFAULT);
    }
    let child_tidptr = if (flags & CLONE_CHILD_SETTID) != 0 {
        if child_tidptr == 0 || !handlers::helpers::is_user_range_accessible(child_tidptr, 4) {
            return linux_err(EFAULT);
        }
        child_tidptr
    } else {
        0
    };
    let clear_child_tidptr = if (flags & CLONE_CHILD_CLEARTID) != 0 {
        if child_tidptr == 0 || !handlers::helpers::is_user_range_accessible(child_tidptr, 4) {
            return linux_err(EFAULT);
        }
        child_tidptr
    } else {
        0
    };

    #[cfg(target_arch = "x86_64")]
    let child_tid = handlers::sys_fork_with_child_tidptr(regs, child_tidptr, clear_child_tidptr);
    #[cfg(not(target_arch = "x86_64"))]
    let child_tid = handlers::sys_fork(regs);
    crate::serial_verbose_println!(
        "lxe linux clone: tid={} flags={:#x} child_stack={:#x} child_tidptr={:#x} clear_child_tidptr={:#x} -> child={}",
        crate::task::scheduler::current_tid(),
        flags,
        child_stack,
        child_tidptr,
        clear_child_tidptr,
        child_tid
    );
    if child_tid == u32::MAX {
        return linux_err(ENOMEM);
    }
    if (flags & CLONE_PARENT_SETTID) != 0 && parent_tidptr != 0 {
        unsafe {
            write_u32(parent_tidptr, 0, child_tid);
        }
    }
    child_tid as u64
}

pub(super) fn linux_fork(regs: &SyscallRegs) -> u64 {
    let child_tid = handlers::sys_fork(regs);
    crate::serial_verbose_println!(
        "lxe linux fork: tid={} -> child={}",
        crate::task::scheduler::current_tid(),
        child_tid
    );
    if child_tid == u32::MAX {
        linux_err(ENOMEM)
    } else {
        child_tid as u64
    }
}

pub(super) fn linux_vfork(regs: &SyscallRegs) -> u64 {
    // vfork's shared-address-space/suspended-parent semantics are stricter
    // than fork. A real fork is conservative for early userland tools and
    // avoids corrupting the parent while execve support matures.
    linux_fork(regs)
}

pub(super) fn linux_set_tid_address(tidptr: u64) -> u64 {
    if tidptr != 0 && !handlers::helpers::is_user_range_accessible(tidptr, 4) {
        return linux_err(EFAULT);
    }
    crate::task::scheduler::set_current_thread_linux_clear_child_tid(tidptr);
    crate::task::scheduler::current_tid() as u64
}

pub(super) fn linux_execve(filename_ptr: u64, argv_ptr: u64, envp_ptr: u64) -> u64 {
    let filename = match handlers::helpers::read_user_str_safe(filename_ptr) {
        Some(path) if !path.is_empty() => path,
        Some(_) => return linux_err(ENOENT),
        None => return linux_err(EFAULT),
    };

    let linux_path = linux_absolute_path(filename);
    let translated = linux_translate_absolute_path(&linux_path);
    let resolved = match linux_resolve_translated_path(&translated, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };

    let mut argv = match read_exec_string_array(argv_ptr, 64) {
        Ok(argv) => argv,
        Err(errno) => return linux_err(errno),
    };
    if argv.is_empty() {
        argv.push(linux_path.clone());
    }
    let envp = match read_exec_string_array(envp_ptr, 128) {
        Ok(envp) => envp,
        Err(errno) => return linux_err(errno),
    };

    crate::serial_verbose_println!(
        "lxe linux execve: tid={} linux='{}' resolved='{}' argv0='{}' argc={} envc={}",
        crate::task::scheduler::current_tid(),
        linux_path,
        resolved,
        argv.first().map(|s| s.as_str()).unwrap_or(""),
        argv.len(),
        envp.len()
    );

    let err = crate::task::loader::exec_current_linux_process(&resolved, &argv, &envp);
    crate::serial_verbose_println!(
        "lxe linux execve: failed linux='{}' resolved='{}': {}",
        linux_path,
        resolved,
        err
    );
    if err.contains("read program file") || err.contains("path not found") {
        linux_err(ENOENT)
    } else {
        linux_err(ENOEXEC)
    }
}

fn read_exec_string_array(ptr: u64, max_entries: usize) -> Result<Vec<String>, i32> {
    if ptr == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::<String>::new();
    let mut total = 0usize;
    for i in 0..max_entries {
        let slot = ptr.checked_add((i as u64) * 8).ok_or(EFAULT)?;
        if !handlers::helpers::is_user_range_accessible(slot, 8) {
            return Err(EFAULT);
        }
        let string_ptr = unsafe { read_u64(slot, 0) };
        if string_ptr == 0 {
            return Ok(out);
        }
        let s = handlers::helpers::read_user_str_safe(string_ptr).ok_or(EFAULT)?;
        total = total.checked_add(s.len() + 1).ok_or(E2BIG)?;
        if total > 128 * 1024 {
            return Err(E2BIG);
        }
        out.push(String::from(s));
    }
    Err(E2BIG)
}

pub(super) fn linux_wait4(
    rip: u64,
    pid: i64,
    status_ptr: u64,
    options: u64,
    rusage_ptr: u64,
) -> u64 {
    const WNOHANG: u64 = 1;
    const WUNTRACED: u64 = 2;
    const WCONTINUED: u64 = 8;
    const WNOTHREAD: u64 = 0x2000_0000;
    const WALL: u64 = 0x4000_0000;
    const WCLONE: u64 = 0x8000_0000;
    const SUPPORTED_WAIT_OPTIONS: u64 =
        WNOHANG | WUNTRACED | WCONTINUED | WNOTHREAD | WALL | WCLONE;
    if (options & !SUPPORTED_WAIT_OPTIONS) != 0 {
        return linux_err(EINVAL);
    }
    if status_ptr != 0 && !handlers::helpers::is_user_range_accessible(status_ptr, 4) {
        return linux_err(EFAULT);
    }
    if rusage_ptr != 0 && !handlers::helpers::is_user_range_accessible(rusage_ptr, 144) {
        return linux_err(EFAULT);
    }

    let log_wait4 = should_log_wait4_debug();
    if log_wait4 {
        crate::serial_verbose_println!(
            "lxe linux wait4: enter tid={} rip={:#x} pid={} options={:#x} status={:#x} rusage={:#x} hw_fs={:#x}",
            crate::task::scheduler::current_tid(),
            rip,
            pid,
            options,
            status_ptr,
            rusage_ptr,
            current_hw_fs_base()
        );
        crate::task::scheduler::debug_wait4_snapshot("enter", pid, options);
    }

    let wnohang = (options & WNOHANG) != 0;
    let (child_tid, code) = if pid == -1 || pid == 0 || pid < -1 {
        if wnohang {
            crate::task::scheduler::try_waitpid_any()
        } else {
            crate::task::scheduler::waitpid_any()
        }
    } else if pid > 0 {
        let code = if wnohang {
            crate::task::scheduler::try_waitpid(pid as u32)
        } else {
            crate::task::scheduler::waitpid(pid as u32)
        };
        (pid as u32, code)
    } else {
        if log_wait4 {
            crate::serial_verbose_println!(
                "lxe linux wait4: immediate ECHILD tid={} pid={} options={:#x}",
                crate::task::scheduler::current_tid(),
                pid,
                options
            );
            crate::task::scheduler::debug_wait4_snapshot("bad-pid", pid, options);
        }
        return linux_err(ECHILD);
    };

    if log_wait4 || (child_tid != u32::MAX && child_tid != u32::MAX - 1) {
        crate::serial_verbose_println!(
            "lxe linux wait4: result tid={} pid={} -> child={} code={}",
            crate::task::scheduler::current_tid(),
            pid,
            child_tid,
            code
        );
        crate::task::scheduler::debug_wait4_snapshot("result", pid, options);
    }

    if child_tid == u32::MAX || code == u32::MAX {
        let ret = linux_err(ECHILD);
        if log_wait4 {
            log_errno_probe("wait4-echild", current_hw_fs_base());
            crate::serial_verbose_println!(
                "lxe linux wait4: ECHILD tid={} pid={} raw_ret={:#x} signed_ret={} hw_fs={:#x}",
                crate::task::scheduler::current_tid(),
                pid,
                ret,
                ret as i64,
                current_hw_fs_base()
            );
            crate::task::scheduler::debug_wait4_snapshot("echild", pid, options);
        }
        return ret;
    }
    if child_tid == u32::MAX - 1 || code == u32::MAX - 1 {
        if log_wait4 {
            crate::serial_verbose_println!(
                "lxe linux wait4: WNOHANG-none tid={} pid={}",
                crate::task::scheduler::current_tid(),
                pid
            );
        }
        return 0;
    }
    if code == u32::MAX - 2 {
        crate::serial_verbose_println!(
            "lxe linux wait4: stopped tid={} pid={} child={}",
            crate::task::scheduler::current_tid(),
            pid,
            child_tid
        );
        return 0;
    }

    let status_value = (code & 0xff) << 8;
    if status_ptr != 0 {
        unsafe {
            write_u32(status_ptr, 0, status_value);
        }
    }
    if rusage_ptr != 0 {
        unsafe {
            core::ptr::write_bytes(rusage_ptr as *mut u8, 0, 144);
        }
    }
    crate::task::scheduler::current_signal_clear_pending(crate::ipc::signal::SIGCHLD);
    crate::serial_verbose_println!(
        "lxe linux wait4: ok tid={} child={} code={} status_ptr={:#x} status={:#x} ret={:#x} hw_fs={:#x}",
        crate::task::scheduler::current_tid(),
        child_tid,
        code,
        status_ptr,
        status_value,
        child_tid as u64,
        current_hw_fs_base()
    );
    child_tid as u64
}

pub(super) fn linux_kill(pid: i64, sig: u64) -> u64 {
    let tid = if pid > 0 {
        pid as u32
    } else if pid == 0 {
        crate::task::scheduler::current_tid()
    } else {
        return if sig == 0 { 0 } else { linux_err(ESRCH) };
    };
    if sig == 0 {
        return if crate::task::scheduler::thread_exists(tid) {
            0
        } else {
            linux_err(ESRCH)
        };
    }
    if handlers::sys_kill(tid, sig as u32) == u32::MAX {
        linux_err(ESRCH)
    } else {
        0
    }
}

pub(super) fn linux_tgkill(_tgid: i32, tid: i32, sig: u64) -> u64 {
    if tid <= 0 {
        return linux_err(EINVAL);
    }
    linux_kill(tid as i64, sig)
}

pub(super) fn linux_setpgid(pid: i32, pgid: i32) -> u64 {
    if pid < 0 {
        return linux_err(ESRCH);
    }
    if pgid < 0 {
        return linux_err(EINVAL);
    }
    if pid != 0 && !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }
    0
}

pub(super) fn linux_getpgid(pid: i32) -> u64 {
    if pid < 0 {
        return linux_err(ESRCH);
    }
    if pid == 0 {
        return crate::task::scheduler::current_tid() as u64;
    }
    if !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }
    pid as u64
}

pub(super) fn linux_setsid() -> u64 {
    crate::task::scheduler::current_tid() as u64
}

pub(super) fn linux_getsid(pid: i32) -> u64 {
    linux_getpgid(pid)
}

pub(super) fn linux_rt_sigsuspend(_mask: u64, sigsetsize: u64) -> u64 {
    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    linux_err(EINTR)
}

pub(super) fn linux_sigaltstack(_ss: u64, old_ss: u64) -> u64 {
    if old_ss != 0 {
        if !handlers::helpers::is_user_range_accessible(old_ss, 24) {
            return linux_err(EFAULT);
        }
        unsafe {
            core::ptr::write_bytes(old_ss as *mut u8, 0, 24);
        }
    }
    0
}

pub(super) fn linux_sched_yield() -> u64 {
    0
}

fn linux_affinity_bytes() -> usize {
    let bytes = (crate::arch::hal::MAX_CPUS + 7) / 8;
    align_up(bytes.max(1), core::mem::size_of::<u64>())
}

fn linux_online_cpu_mask() -> u64 {
    let cpu_count = crate::arch::hal::cpu_count().clamp(1, 64);
    if cpu_count == 64 {
        u64::MAX
    } else {
        (1u64 << cpu_count) - 1
    }
}

pub(super) fn linux_sched_getaffinity(pid: i32, cpusetsize: u64, mask_ptr: u64) -> u64 {
    if pid < 0 {
        return linux_err(EINVAL);
    }
    if pid > 0 && !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }

    let needed = linux_affinity_bytes();
    if cpusetsize < needed as u64 {
        return linux_err(EINVAL);
    }
    if mask_ptr == 0 {
        return linux_err(EFAULT);
    }
    if !handlers::helpers::is_user_range_accessible(mask_ptr, needed as u64) {
        return linux_err(EFAULT);
    }

    unsafe {
        core::ptr::write_bytes(mask_ptr as *mut u8, 0, needed);
        write_u64(mask_ptr, 0, linux_online_cpu_mask());
    }
    needed as u64
}

pub(super) fn linux_sched_setaffinity(pid: i32, cpusetsize: u64, mask_ptr: u64) -> u64 {
    if pid < 0 {
        return linux_err(EINVAL);
    }
    if pid > 0 && !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }

    let needed = linux_affinity_bytes();
    if cpusetsize < needed as u64 {
        return linux_err(EINVAL);
    }
    if mask_ptr == 0 || !handlers::helpers::is_user_range_accessible(mask_ptr, needed as u64) {
        return linux_err(EFAULT);
    }

    let requested_mask = unsafe { read_u64(mask_ptr, 0) };
    if (requested_mask & linux_online_cpu_mask()) == 0 {
        return linux_err(EINVAL);
    }

    0
}

pub(super) fn linux_getrusage(_who: u64, usage_ptr: u64) -> u64 {
    if usage_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(usage_ptr as *mut u8, 0, 144);
    }
    0
}

pub(super) fn linux_times(tms_ptr: u64) -> u64 {
    if tms_ptr != 0 {
        if !handlers::helpers::is_user_range_accessible(tms_ptr, 32) {
            return linux_err(EFAULT);
        }
        unsafe {
            core::ptr::write_bytes(tms_ptr as *mut u8, 0, 32);
        }
    }
    crate::arch::hal::timer_current_ticks() as u64
}

pub(super) fn linux_uname(buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 65 * 6);
    }
    write_linux_uts_field(buf_ptr, 0, b"Linux");
    write_linux_uts_field(buf_ptr, 1, b"anyos");
    write_linux_uts_field(buf_ptr, 2, b"3.2.0-lxe");
    write_linux_uts_field(buf_ptr, 3, b"#1 anyOS lxe Linux ABI");
    write_linux_uts_field(buf_ptr, 4, b"x86_64");
    write_linux_uts_field(buf_ptr, 5, b"anyos");
    0
}

pub(super) fn linux_getrandom(buf_ptr: u64, len: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let len = len.min(256);
    let mut seed = crate::arch::hal::timer_current_ticks() as u64
        ^ ((crate::task::scheduler::current_tid() as u64) << 32)
        ^ 0xD1CF_600D_51A7_EA5Eu64;
    unsafe {
        for i in 0..len {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *((buf_ptr + i) as *mut u8) = (seed >> ((i & 7) * 8)) as u8;
        }
    }
    len
}

#[inline]
fn linux_signal_mask(mask: u64) -> u32 {
    use crate::ipc::signal::{SIGKILL, SIGSTOP};
    let mut internal = 0u32;
    for sig in 1..32u32 {
        if (mask & (1u64 << (sig - 1))) != 0 {
            internal |= 1u32 << sig;
        }
    }
    internal & !((1u32 << SIGKILL) | (1u32 << SIGSTOP))
}

#[inline]
fn internal_signal_mask(mask: u32) -> u64 {
    let mut linux = 0u64;
    for sig in 1..32u32 {
        if (mask & (1u32 << sig)) != 0 {
            linux |= 1u64 << (sig - 1);
        }
    }
    linux
}

pub(super) fn linux_rt_sigaction(sig: u64, act: u64, oldact: u64, sigsetsize: u64) -> u64 {
    use crate::ipc::signal::{SIGKILL, SIGSTOP};

    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if sig == 0 || sig >= 32 || sig == SIGKILL as u64 || sig == SIGSTOP as u64 {
        return linux_err(EINVAL);
    }
    if act != 0 && !handlers::helpers::is_user_range_accessible(act, 32) {
        return linux_err(EFAULT);
    }
    if oldact != 0 && !handlers::helpers::is_user_range_accessible(oldact, 32) {
        return linux_err(EFAULT);
    }

    let sig = sig as u32;
    let (old_handler, old_flags, old_restorer, old_mask) =
        crate::task::scheduler::current_signal_action(sig);
    if oldact != 0 {
        unsafe {
            write_u64(oldact, 0, old_handler);
            write_u64(oldact, 8, old_flags);
            write_u64(oldact, 16, old_restorer);
            write_u64(oldact, 24, internal_signal_mask(old_mask as u32));
        }
    }

    if act != 0 {
        let handler = unsafe { read_u64(act, 0) };
        let flags = unsafe { read_u64(act, 8) };
        let restorer = unsafe { read_u64(act, 16) };
        let raw_mask = unsafe { read_u64(act, 24) };
        let mask = linux_signal_mask(raw_mask);
        crate::task::scheduler::current_signal_set_action(
            sig,
            handler,
            flags,
            restorer,
            mask as u64,
        );
        crate::serial_verbose_println!(
            "lxe linux rt_sigaction: tid={} sig={} handler={:#x} flags={:#x} restorer={:#x} mask={:#x}->{:#x} old={:#x}",
            crate::task::scheduler::current_tid(),
            sig,
            handler,
            flags,
            restorer,
            raw_mask,
            mask,
            old_handler
        );
    }

    0
}

pub(super) fn linux_rt_sigprocmask(how: u64, set: u64, oldset: u64, sigsetsize: u64) -> u64 {
    const SIG_BLOCK: u64 = 0;
    const SIG_UNBLOCK: u64 = 1;
    const SIG_SETMASK: u64 = 2;

    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if set != 0 && !handlers::helpers::is_user_range_accessible(set, 8) {
        return linux_err(EFAULT);
    }
    if oldset != 0 && !handlers::helpers::is_user_range_accessible(oldset, 8) {
        return linux_err(EFAULT);
    }
    if set != 0 && how != SIG_BLOCK && how != SIG_UNBLOCK && how != SIG_SETMASK {
        return linux_err(EINVAL);
    }

    let old_mask = crate::task::scheduler::current_signal_get_blocked();
    if oldset != 0 {
        unsafe {
            write_u64(oldset, 0, internal_signal_mask(old_mask));
        }
    }
    if set != 0 {
        let set_mask = linux_signal_mask(unsafe { read_u64(set, 0) });
        let new_mask = match how {
            SIG_BLOCK => old_mask | set_mask,
            SIG_UNBLOCK => old_mask & !set_mask,
            SIG_SETMASK => set_mask,
            _ => old_mask,
        };
        crate::task::scheduler::current_signal_set_blocked(new_mask);
        crate::serial_verbose_println!(
            "lxe linux rt_sigprocmask: tid={} how={} set={:#x} old={:#x} new={:#x}",
            crate::task::scheduler::current_tid(),
            how,
            set_mask,
            old_mask,
            new_mask
        );
    }
    0
}

pub(super) fn linux_gettimeofday(tv_ptr: u64, tz_ptr: u64) -> u64 {
    if tv_ptr != 0 {
        let sec = linux_now_seconds();
        if !handlers::helpers::is_user_range_accessible(tv_ptr, 16) {
            return linux_err(EFAULT);
        }
        unsafe {
            write_u64(tv_ptr, 0, sec);
            write_u64(tv_ptr, 8, 0);
        }
    }
    if tz_ptr != 0 {
        if !handlers::helpers::is_user_range_accessible(tz_ptr, 8) {
            return linux_err(EFAULT);
        }
        unsafe {
            write_u64(tz_ptr, 0, 0);
        }
    }
    0
}

pub(super) fn linux_time(time_ptr: u64) -> u64 {
    let sec = linux_now_seconds();
    if time_ptr != 0 {
        if !handlers::helpers::is_user_range_accessible(time_ptr, 8) {
            return linux_err(EFAULT);
        }
        unsafe {
            write_u64(time_ptr, 0, sec);
        }
    }
    sec
}

pub(super) fn linux_clock_gettime(clock_id: u64, ts_ptr: u64) -> u64 {
    const CLOCK_REALTIME: u64 = 0;
    const CLOCK_MONOTONIC: u64 = 1;
    const CLOCK_MONOTONIC_RAW: u64 = 4;
    const CLOCK_REALTIME_COARSE: u64 = 5;
    const CLOCK_MONOTONIC_COARSE: u64 = 6;
    const CLOCK_BOOTTIME: u64 = 7;

    if ts_ptr == 0 || !handlers::helpers::is_user_range_accessible(ts_ptr, 16) {
        return linux_err(EFAULT);
    }
    let (sec, nsec) = match clock_id {
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE => (linux_now_seconds(), 0),
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME => {
            linux_monotonic_time()
        }
        _ => return linux_err(EINVAL),
    };
    unsafe {
        write_u64(ts_ptr, 0, sec);
        write_u64(ts_ptr, 8, nsec);
    }
    0
}

pub(super) fn linux_nanosleep(req_ptr: u64, rem_ptr: u64) -> u64 {
    linux_sleep_timespec(req_ptr, rem_ptr, false, 0)
}

pub(super) fn linux_clock_nanosleep(clock_id: u64, flags: u64, req_ptr: u64, rem_ptr: u64) -> u64 {
    const CLOCK_REALTIME: u64 = 0;
    const CLOCK_MONOTONIC: u64 = 1;
    const CLOCK_MONOTONIC_RAW: u64 = 4;
    const TIMER_ABSTIME: u64 = 1;

    if flags & !TIMER_ABSTIME != 0 {
        return linux_err(EINVAL);
    }
    match clock_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW => {}
        _ => return linux_err(EINVAL),
    }

    linux_sleep_timespec(req_ptr, rem_ptr, (flags & TIMER_ABSTIME) != 0, clock_id)
}

fn linux_sleep_timespec(req_ptr: u64, _rem_ptr: u64, absolute: bool, clock_id: u64) -> u64 {
    if req_ptr == 0 || !handlers::helpers::is_user_range_accessible(req_ptr, 16) {
        return linux_err(EFAULT);
    }

    let sec = unsafe { read_u64(req_ptr, 0) as i64 };
    let nsec = unsafe { read_u64(req_ptr, 8) as i64 };
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return linux_err(EINVAL);
    }

    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u128;
    let now_ticks = crate::arch::hal::timer_current_ticks();
    let sleep_ticks = if absolute {
        let target_ns = (sec as u128)
            .saturating_mul(1_000_000_000)
            .saturating_add(nsec as u128);
        let now_ns = if clock_id == 0 {
            (linux_now_seconds() as u128).saturating_mul(1_000_000_000)
        } else {
            (now_ticks as u128).saturating_mul(1_000_000_000) / hz
        };
        if target_ns <= now_ns {
            0
        } else {
            ns_to_ticks(target_ns - now_ns, hz)
        }
    } else {
        let duration_ns = (sec as u128)
            .saturating_mul(1_000_000_000)
            .saturating_add(nsec as u128);
        ns_to_ticks(duration_ns, hz)
    };

    if sleep_ticks != 0 {
        let capped_ticks = sleep_ticks.min(u32::MAX as u128 / 2) as u32;
        crate::task::scheduler::sleep_until(now_ticks.wrapping_add(capped_ticks));
    }
    0
}

fn ns_to_ticks(ns: u128, hz: u128) -> u128 {
    if ns == 0 {
        0
    } else {
        ns.saturating_mul(hz).saturating_add(999_999_999) / 1_000_000_000
    }
}

pub(super) fn linux_sysinfo(info_ptr: u64) -> u64 {
    if info_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(info_ptr as *mut u8, 0, 112);
        write_u64(info_ptr, 0, crate::arch::hal::timer_current_ticks() as u64);
        write_u64(info_ptr, 8, 1);
        write_u64(info_ptr, 16, 1);
        write_u64(info_ptr, 24, 1);
        write_u64(info_ptr, 32, 0);
        write_u64(info_ptr, 40, 0);
        write_u64(info_ptr, 48, 0);
        write_u64(info_ptr, 56, 0);
        write_u16(info_ptr, 104, 1);
    }
    0
}

pub(super) fn linux_getgroups(size: i32, list_ptr: u64) -> u64 {
    if size < 0 {
        return linux_err(EINVAL);
    }
    if size == 0 {
        return 1;
    }
    if list_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u32(list_ptr, 0, handlers::sys_getgid());
    }
    1
}

pub(super) fn linux_setgroups(size: i32, list_ptr: u64) -> u64 {
    if !(0..=1024).contains(&size) {
        return linux_err(EINVAL);
    }
    if size > 0 {
        let bytes = (size as u64).saturating_mul(core::mem::size_of::<u32>() as u64);
        if list_ptr == 0 || !handlers::helpers::is_user_range_accessible(list_ptr, bytes) {
            return linux_err(EFAULT);
        }
    }
    0
}

pub(super) fn linux_setre_id(real: u32, effective: u32, uid: bool) -> u64 {
    linux_setres_id(real, effective, u32::MAX, uid)
}

pub(super) fn linux_setres_id(real: u32, effective: u32, saved: u32, uid: bool) -> u64 {
    for value in [real, effective, saved] {
        if value != u32::MAX && value > u16::MAX as u32 {
            return linux_err(EINVAL);
        }
    }

    let (old_real, old_effective, old_saved, _old_fs) = current_linux_ids(uid);
    let privileged = old_effective == 0;
    if !privileged {
        for value in [real, effective, saved] {
            if value != u32::MAX
                && value != old_real
                && value != old_effective
                && value != old_saved
            {
                return linux_err(EPERM);
            }
        }
    }

    let next_real = linux_id_or_old(real, old_real);
    let next_effective = linux_id_or_old(effective, old_effective);
    let next_saved = linux_id_or_old(saved, old_saved);
    set_current_linux_ids(
        uid,
        Some(next_real),
        Some(next_effective),
        Some(next_saved),
        Some(next_effective),
    );
    0
}

pub(super) fn linux_getres_id(real_ptr: u64, effective_ptr: u64, saved_ptr: u64, uid: bool) -> u64 {
    if real_ptr == 0 || effective_ptr == 0 || saved_ptr == 0 {
        return linux_err(EFAULT);
    }
    let (real, effective, saved, _fs) = current_linux_ids(uid);
    unsafe {
        write_u32(real_ptr, 0, real);
        write_u32(effective_ptr, 0, effective);
        write_u32(saved_ptr, 0, saved);
    }
    0
}

pub(super) fn linux_setfs_id(id: u32, uid: bool) -> u64 {
    let (real, effective, saved, old) = current_linux_ids(uid);
    if id == u32::MAX || id == old {
        return old as u64;
    }
    if id > u16::MAX as u32 {
        return old as u64;
    }
    if effective == 0 || id == real || id == effective || id == saved || id == old {
        set_current_linux_ids(uid, None, None, None, Some(id as u16));
        return old as u64;
    }
    old as u64
}

pub(super) fn current_linux_id(uid: bool) -> u32 {
    let (_real, effective, _saved, _fs) = current_linux_ids(uid);
    effective as u32
}

pub(super) fn current_linux_real_id(uid: bool) -> u32 {
    let (real, _effective, _saved, _fs) = current_linux_ids(uid);
    real as u32
}

fn current_linux_ids(uid: bool) -> (u32, u32, u32, u32) {
    let (real, effective, saved, fs) = crate::task::scheduler::current_thread_linux_ids(uid);
    (real as u32, effective as u32, saved as u32, fs as u32)
}

pub(super) fn linux_set_root_or_current(id: u32, uid: bool) -> u64 {
    if id > u16::MAX as u32 {
        return linux_err(EINVAL);
    }
    let (real, effective, saved, _fs) = current_linux_ids(uid);
    if effective == 0 {
        return set_current_linux_ids(
            uid,
            Some(id as u16),
            Some(id as u16),
            Some(id as u16),
            Some(id as u16),
        );
    }
    if id == real || id == effective || id == saved {
        set_current_linux_ids(uid, None, Some(id as u16), None, Some(id as u16));
        0
    } else {
        linux_err(EPERM)
    }
}

fn linux_id_or_old(value: u32, old: u32) -> u16 {
    if value == u32::MAX {
        old as u16
    } else {
        value as u16
    }
}

fn set_current_linux_ids(
    uid: bool,
    real: Option<u16>,
    effective: Option<u16>,
    saved: Option<u16>,
    fs: Option<u16>,
) -> u64 {
    let tid = crate::task::scheduler::current_tid();
    crate::task::scheduler::set_process_linux_ids(tid, uid, real, effective, saved, fs);
    0
}

pub(super) fn linux_capget(header_ptr: u64, data_ptr: u64) -> u64 {
    if header_ptr == 0 {
        return linux_err(EFAULT);
    }
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    unsafe {
        write_u32(header_ptr, 0, LINUX_CAPABILITY_VERSION_3);
    }
    if data_ptr != 0 {
        let effective = if current_linux_id(true) == 0 {
            u32::MAX
        } else {
            0
        };
        unsafe {
            write_u32(data_ptr, 0, effective);
            write_u32(data_ptr, 4, effective);
            write_u32(data_ptr, 8, 0);
            write_u32(data_ptr, 12, effective);
            write_u32(data_ptr, 16, effective);
            write_u32(data_ptr, 20, 0);
        }
    }
    0
}

pub(super) fn linux_capset(_header_ptr: u64, _data_ptr: u64) -> u64 {
    if current_linux_id(true) == 0 {
        0
    } else {
        linux_err(EPERM)
    }
}

pub(super) fn linux_prctl(option: u64, arg2: u64) -> u64 {
    match option {
        3 => 1,  // PR_GET_DUMPABLE
        4 => 0,  // PR_SET_DUMPABLE
        15 => 0, // PR_SET_NAME
        16 => {
            if arg2 == 0 {
                return linux_err(EFAULT);
            }
            let name = b"lxe\0";
            if !super::handlers::helpers::copy_to_user_bytes(arg2, name, name.len()) {
                return linux_err(EFAULT);
            }
            0
        }
        38 => 0, // PR_SET_NO_NEW_PRIVS
        39 => 0, // PR_GET_NO_NEW_PRIVS
        _ => linux_err(EINVAL),
    }
}

pub(super) fn linux_prlimit64(_pid: i32, resource: u64, _new_limit: u64, old_limit: u64) -> u64 {
    if old_limit == 0 {
        return 0;
    }
    let (cur, max) = match resource {
        3 => (8 * 1024 * 1024, 8 * 1024 * 1024), // RLIMIT_STACK
        7 => (64, 64),                           // RLIMIT_NOFILE
        _ => (u64::MAX, u64::MAX),
    };
    unsafe {
        write_u64(old_limit, 0, cur);
        write_u64(old_limit, 8, max);
    }
    0
}

pub(super) fn linux_setrlimit(resource: u64, limit_ptr: u64) -> u64 {
    if limit_ptr == 0 || !handlers::helpers::is_user_range_accessible(limit_ptr, 16) {
        return linux_err(EFAULT);
    }
    crate::serial_verbose_println!("lxe linux setrlimit: resource={} -> ok", resource);
    0
}

pub(super) fn linux_futex(_uaddr: u64, op: u64, _val: u64) -> u64 {
    let cmd = op & 0x7F;
    match cmd {
        0 => linux_err(EAGAIN), // FUTEX_WAIT
        1 => 0,                 // FUTEX_WAKE
        _ => linux_err(ENOSYS),
    }
}

pub(super) fn linux_now_seconds() -> u64 {
    crate::time::wall_clock_unix_secs()
}

fn linux_monotonic_time() -> (u64, u64) {
    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u128;
    let ticks = crate::arch::hal::timer_current_ticks() as u128;
    let total_ns = ticks.saturating_mul(1_000_000_000) / hz;
    (
        (total_ns / 1_000_000_000) as u64,
        (total_ns % 1_000_000_000) as u64,
    )
}
