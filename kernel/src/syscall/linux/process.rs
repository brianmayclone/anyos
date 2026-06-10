use super::*;
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicU32, Ordering};

static WAIT4_DEBUG_SEQ: AtomicU32 = AtomicU32::new(0);

const LINUX_FUTEX_WAIT_SLOTS: usize = 64;
const LINUX_FUTEX_WAITERS_PER_SLOT: usize = 8;
const LINUX_FUTEX_WAKE_BATCH: usize = LINUX_FUTEX_WAIT_SLOTS * LINUX_FUTEX_WAITERS_PER_SLOT;

const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;
const FUTEX_REQUEUE: u64 = 3;
const FUTEX_CMP_REQUEUE: u64 = 4;
const FUTEX_WAKE_OP: u64 = 5;
const FUTEX_WAIT_BITSET: u64 = 9;
const FUTEX_WAKE_BITSET: u64 = 10;
const FUTEX_CMD_MASK: u64 = 0x7f;

#[derive(Clone, Copy)]
struct LinuxFutexSlot {
    uaddr: u64,
    tids: [u32; LINUX_FUTEX_WAITERS_PER_SLOT],
}

impl LinuxFutexSlot {
    const EMPTY: Self = Self {
        uaddr: 0,
        tids: [0; LINUX_FUTEX_WAITERS_PER_SLOT],
    };
}

static LINUX_FUTEX_WAITERS: Spinlock<[LinuxFutexSlot; LINUX_FUTEX_WAIT_SLOTS]> =
    Spinlock::new([LinuxFutexSlot::EMPTY; LINUX_FUTEX_WAIT_SLOTS]);

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
    const CLONE_FS: u64 = 0x0000_0200;
    const CLONE_FILES: u64 = 0x0000_0400;
    const CLONE_SIGHAND: u64 = 0x0000_0800;
    const CLONE_THREAD: u64 = 0x0001_0000;
    const CLONE_SYSVSEM: u64 = 0x0004_0000;
    const CLONE_SETTLS: u64 = 0x0008_0000;
    const CLONE_PARENT_SETTID: u64 = 0x0010_0000;
    const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;
    const CLONE_CHILD_SETTID: u64 = 0x0100_0000;
    const SUPPORTED_THREAD_FLAGS: u64 = CSIGNAL
        | CLONE_VM
        | CLONE_FS
        | CLONE_FILES
        | CLONE_SIGHAND
        | CLONE_THREAD
        | CLONE_SYSVSEM
        | CLONE_SETTLS
        | CLONE_PARENT_SETTID
        | CLONE_CHILD_CLEARTID
        | CLONE_CHILD_SETTID;

    let signal = flags & CSIGNAL;
    if signal != 0 && signal != SIGCHLD {
        return linux_err(EINVAL);
    }

    if (flags & (CLONE_VM | CLONE_THREAD | CLONE_SETTLS)) != 0 || tls != 0 {
        if (flags & !SUPPORTED_THREAD_FLAGS) != 0 {
            crate::serial_verbose_println!(
                "lxe linux clone: unsupported thread flags={:#x} child_stack={:#x} tls={:#x}",
                flags,
                child_stack,
                tls
            );
            return linux_err(ENOSYS);
        }
        if (flags & CLONE_THREAD) != 0 && ((flags & CLONE_VM) == 0 || (flags & CLONE_SIGHAND) == 0)
        {
            return linux_err(EINVAL);
        }
        if (flags & CLONE_SIGHAND) != 0 && (flags & CLONE_VM) == 0 {
            return linux_err(EINVAL);
        }
        if child_stack == 0
            || child_stack >= 0x0000_8000_0000_0000
            || !handlers::helpers::is_user_range_accessible(child_stack.saturating_sub(8), 8)
        {
            return linux_err(EFAULT);
        }
        if (flags & CLONE_PARENT_SETTID) != 0
            && (parent_tidptr == 0
                || !handlers::helpers::is_user_range_accessible(parent_tidptr, 4))
        {
            return linux_err(EFAULT);
        }
        let child_set_tidptr = if (flags & CLONE_CHILD_SETTID) != 0 {
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

        let fs_base = if (flags & CLONE_SETTLS) != 0 {
            tls
        } else {
            crate::task::scheduler::current_thread_linux_fs_base()
        };
        let child_regs = crate::task::loader::ForkChildRegs {
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            cs: regs.cs,
            rflags: regs.rflags,
            rsp: child_stack,
            ss: regs.ss,
            fs_base,
            child_tidptr: child_set_tidptr,
        };
        let child_tid =
            crate::task::scheduler::create_linux_clone_thread(child_regs, clear_child_tidptr);
        crate::serial_verbose_println!(
            "lxe linux clone-thread: tid={} flags={:#x} child_stack={:#x} parent_tidptr={:#x} child_tidptr={:#x} tls={:#x} -> child={}",
            crate::task::scheduler::current_tid(),
            flags,
            child_stack,
            parent_tidptr,
            child_tidptr,
            tls,
            child_tid
        );
        if child_tid == 0 {
            return linux_err(ENOMEM);
        }
        if (flags & CLONE_PARENT_SETTID) != 0 && parent_tidptr != 0 {
            unsafe {
                write_u32(parent_tidptr, 0, child_tid);
            }
        }
        return child_tid as u64;
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

pub(super) fn linux_clone3(regs: &SyscallRegs, args_ptr: u64, size: u64) -> u64 {
    const CLONE_ARGS_MIN_SIZE: u64 = 64;
    const CLONE_ARGS_KNOWN_SIZE: u64 = 88;

    if args_ptr == 0 || size < CLONE_ARGS_MIN_SIZE {
        return linux_err(EINVAL);
    }
    let readable = size.min(CLONE_ARGS_KNOWN_SIZE);
    if !handlers::helpers::is_user_range_accessible(args_ptr, readable) {
        return linux_err(EFAULT);
    }

    let read_arg = |offset: u64| -> u64 {
        if offset + 8 <= readable {
            unsafe { read_u64(args_ptr, offset) }
        } else {
            0
        }
    };

    let flags = read_arg(0);
    let pidfd = read_arg(8);
    let child_tid = read_arg(16);
    let parent_tid = read_arg(24);
    let exit_signal = read_arg(32);
    let stack = read_arg(40);
    let stack_size = read_arg(48);
    let tls = read_arg(56);
    let set_tid = read_arg(64);
    let set_tid_size = read_arg(72);
    let cgroup = read_arg(80);

    if pidfd != 0 || set_tid != 0 || set_tid_size != 0 || cgroup != 0 {
        crate::serial_verbose_println!(
            "lxe linux clone3: unsupported pidfd={:#x} set_tid={:#x} set_tid_size={} cgroup={:#x}",
            pidfd,
            set_tid,
            set_tid_size,
            cgroup
        );
        return linux_err(ENOSYS);
    }

    let child_stack = if stack != 0 && stack_size != 0 {
        match stack.checked_add(stack_size) {
            Some(value) => value,
            None => return linux_err(EINVAL),
        }
    } else {
        stack
    };
    let clone_flags = flags | (exit_signal & 0xff);
    crate::serial_verbose_println!(
        "lxe linux clone3: flags={:#x} exit_signal={} stack={:#x} size={:#x} tls={:#x}",
        flags,
        exit_signal,
        stack,
        stack_size,
        tls
    );
    linux_clone(regs, clone_flags, child_stack, parent_tid, child_tid, tls)
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

    let argv = match read_exec_string_array(argv_ptr, MAX_EXEC_ARG_ENTRIES) {
        Ok(argv) => argv,
        Err(errno) => return linux_err(errno),
    };
    let envp = match read_exec_string_array(envp_ptr, MAX_EXEC_ARG_ENTRIES) {
        Ok(envp) => envp,
        Err(errno) => return linux_err(errno),
    };
    let (exec_linux_path, exec_resolved, argv) =
        match resolve_linux_exec_target(linux_path.clone(), resolved.clone(), argv) {
            Ok(target) => target,
            Err(errno) => return linux_err(errno),
        };

    crate::serial_verbose_println!(
        "lxe linux execve: tid={} linux='{}' resolved='{}' argv0='{}' argc={} envc={}",
        crate::task::scheduler::current_tid(),
        exec_linux_path,
        exec_resolved,
        argv.first().map(|s| s.as_str()).unwrap_or(""),
        argv.len(),
        envp.len()
    );

    let err = crate::task::loader::exec_current_linux_process(&exec_resolved, &argv, &envp);
    crate::serial_verbose_println!(
        "lxe linux execve: failed linux='{}' resolved='{}': {}",
        exec_linux_path,
        exec_resolved,
        err
    );
    if err.contains("read program file") || err.contains("path not found") {
        linux_err(ENOENT)
    } else {
        linux_err(ENOEXEC)
    }
}

fn resolve_linux_exec_target(
    mut linux_path: String,
    mut resolved: String,
    mut argv: Vec<String>,
) -> Result<(String, String, Vec<String>), i32> {
    if argv.is_empty() {
        argv.push(linux_path.clone());
    }

    for _ in 0..4 {
        let data = crate::fs::vfs::read_file_to_vec(&resolved).map_err(|_| ENOENT)?;
        let Some((interpreter, interpreter_arg)) = parse_shebang(&data) else {
            return Ok((linux_path, resolved, argv));
        };
        if !interpreter.starts_with('/') {
            return Err(ENOEXEC);
        }

        let interpreter_linux = crate::fs::path::normalize(&interpreter);
        let interpreter_translated = linux_translate_absolute_path(&interpreter_linux);
        let interpreter_resolved =
            linux_resolve_translated_path(&interpreter_translated, true, false)?;

        let script_linux_path = linux_path;
        let mut script_argv = Vec::new();
        script_argv.push(interpreter_linux.clone());
        if let Some(arg) = interpreter_arg {
            script_argv.push(arg);
        }
        script_argv.push(script_linux_path.clone());
        for arg in argv.iter().skip(1) {
            script_argv.push(arg.clone());
        }

        crate::serial_verbose_println!(
            "lxe linux execve: shebang script='{}' interp='{}' resolved='{}' argc={}",
            script_linux_path,
            interpreter_linux,
            interpreter_resolved,
            script_argv.len()
        );

        linux_path = interpreter_linux;
        resolved = interpreter_resolved;
        argv = script_argv;
    }

    Err(ELOOP)
}

fn parse_shebang(data: &[u8]) -> Option<(String, Option<String>)> {
    if data.len() < 2 || data[0] != b'#' || data[1] != b'!' {
        return None;
    }

    let max = data.len().min(256);
    let mut end = 2;
    while end < max && data[end] != b'\n' {
        end += 1;
    }

    let mut start = 2;
    while start < end && matches!(data[start], b' ' | b'\t') {
        start += 1;
    }
    while end > start && matches!(data[end - 1], b' ' | b'\t' | b'\r') {
        end -= 1;
    }
    if start >= end {
        return None;
    }

    let mut interp_end = start;
    while interp_end < end && !matches!(data[interp_end], b' ' | b'\t' | b'\r') {
        interp_end += 1;
    }

    let interpreter = core::str::from_utf8(&data[start..interp_end]).ok()?;
    let mut arg_start = interp_end;
    while arg_start < end && matches!(data[arg_start], b' ' | b'\t') {
        arg_start += 1;
    }
    let interpreter_arg = if arg_start < end {
        Some(String::from(
            core::str::from_utf8(&data[arg_start..end]).ok()?,
        ))
    } else {
        None
    };

    Some((String::from(interpreter), interpreter_arg))
}

/// Maximum argv/envp entries accepted by execve. Linux's MAX_ARG_STRINGS is
/// effectively unbounded (0x7FFFFFFF); the real constraint is ARG_MAX bytes.
/// 64 (the old value) broke any `ls *.deb` / `dpkg --unpack <many>` /
/// `apt-extracttemplates <pkglist>` with more than 64 entries. This is a loop
/// bound; the byte limit below is the binding constraint.
pub(super) const MAX_EXEC_ARG_ENTRIES: usize = 256 * 1024;

/// Maximum combined argv+envp string bytes. Linux ARG_MAX is min(RLIMIT_STACK/4,
/// ...) ≈ 2 MiB for an 8 MiB stack; we use 2 MiB. The initial-stack builder
/// additionally bounds-checks against the actually mapped 8 MiB stack, so this
/// can never overflow the stack — it only rejects oversized arg lists cleanly.
pub(super) const MAX_EXEC_ARG_BYTES: usize = 2 * 1024 * 1024;

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
        if total > MAX_EXEC_ARG_BYTES {
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

    let still_running = child_tid == u32::MAX - 1 || code == u32::MAX - 1;
    if log_wait4 || (!still_running && child_tid != u32::MAX) {
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
    if still_running {
        if log_wait4 {
            crate::serial_verbose_println!(
                "lxe linux wait4: WNOHANG-none tid={} pid={} child={}",
                crate::task::scheduler::current_tid(),
                pid,
                child_tid
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
    // Actually relinquish the CPU. glibc/pthreads and many apt/dpkg spin-then-
    // yield loops (lock fallbacks, futex spins) call sched_yield() expecting to
    // give up the timeslice; a no-op makes them busy-spin and waste CPU.
    crate::task::scheduler::schedule();
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

pub(super) fn linux_getpriority(which: u64, who: i32) -> u64 {
    const PRIO_PROCESS: u64 = 0;
    const PRIO_PGRP: u64 = 1;
    const PRIO_USER: u64 = 2;

    match which {
        PRIO_PROCESS => {
            if who > 0 && !crate::task::scheduler::thread_exists(who as u32) {
                return linux_err(ESRCH);
            }
        }
        PRIO_PGRP | PRIO_USER => {}
        _ => return linux_err(EINVAL),
    }
    0
}

pub(super) fn linux_setpriority(which: u64, who: i32, prio: i32) -> u64 {
    const PRIO_PROCESS: u64 = 0;
    const PRIO_PGRP: u64 = 1;
    const PRIO_USER: u64 = 2;

    if !(-20..=19).contains(&prio) {
        return linux_err(EINVAL);
    }
    match which {
        PRIO_PROCESS => {
            let tid = if who > 0 {
                who as u32
            } else {
                crate::task::scheduler::current_tid()
            };
            if !crate::task::scheduler::thread_exists(tid) {
                return linux_err(ESRCH);
            }
            let priority =
                (80 - prio).clamp(1, crate::task::abi::LINUX_MAX_USER_PRIORITY as i32) as u8;
            crate::task::scheduler::set_thread_priority(tid, priority);
        }
        PRIO_PGRP | PRIO_USER => {}
        _ => return linux_err(EINVAL),
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

pub(super) fn linux_clock_getres(clock_id: u64, ts_ptr: u64) -> u64 {
    const CLOCK_REALTIME: u64 = 0;
    const CLOCK_MONOTONIC: u64 = 1;
    const CLOCK_PROCESS_CPUTIME_ID: u64 = 2;
    const CLOCK_THREAD_CPUTIME_ID: u64 = 3;
    const CLOCK_MONOTONIC_RAW: u64 = 4;
    const CLOCK_REALTIME_COARSE: u64 = 5;
    const CLOCK_MONOTONIC_COARSE: u64 = 6;
    const CLOCK_BOOTTIME: u64 = 7;

    match clock_id {
        CLOCK_REALTIME
        | CLOCK_MONOTONIC
        | CLOCK_PROCESS_CPUTIME_ID
        | CLOCK_THREAD_CPUTIME_ID
        | CLOCK_MONOTONIC_RAW
        | CLOCK_REALTIME_COARSE
        | CLOCK_MONOTONIC_COARSE
        | CLOCK_BOOTTIME => {}
        _ => return linux_err(EINVAL),
    }
    if ts_ptr != 0 {
        if !handlers::helpers::is_user_range_accessible(ts_ptr, 16) {
            return linux_err(EFAULT);
        }
        let nsec = if clock_id == CLOCK_REALTIME_COARSE || clock_id == CLOCK_MONOTONIC_COARSE {
            1_000_000
        } else {
            1_000_000_000 / crate::arch::hal::timer_frequency_hz().max(1)
        };
        unsafe {
            write_u64(ts_ptr, 0, 0);
            write_u64(ts_ptr, 8, nsec.max(1));
        }
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
    if info_ptr == 0 || !handlers::helpers::is_user_range_accessible(info_ptr, 112) {
        return linux_err(EFAULT);
    }
    let swap = crate::memory::swap::stats();
    unsafe {
        core::ptr::write_bytes(info_ptr as *mut u8, 0, 112);
        write_u64(info_ptr, 0, crate::arch::hal::timer_current_ticks() as u64);
        write_u64(info_ptr, 8, 1);
        write_u64(info_ptr, 16, 1);
        write_u64(info_ptr, 24, 1);
        write_u64(info_ptr, 32, crate::memory::physical::total_frames() as u64);
        write_u64(
            info_ptr,
            40,
            crate::memory::physical::free_frame_count() as u64,
        );
        write_u64(info_ptr, 48, 0);
        write_u64(info_ptr, 56, 0);
        write_u64(info_ptr, 64, swap.total_pages);
        write_u64(info_ptr, 72, swap.free_pages);
        write_u16(info_ptr, 80, 1);
        write_u32(info_ptr, 104, crate::memory::FRAME_SIZE as u32);
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
        3 => 1, // PR_GET_DUMPABLE
        4 => 0, // PR_SET_DUMPABLE
        15 => {
            if arg2 == 0 {
                return linux_err(EFAULT);
            }
            if let Some(name) = handlers::helpers::read_user_str_safe(arg2) {
                let tid = crate::task::scheduler::current_tid();
                crate::task::scheduler::set_thread_name(tid, name);
                0
            } else {
                linux_err(EFAULT)
            }
        }
        16 => {
            if arg2 == 0 {
                return linux_err(EFAULT);
            }
            let name_buf = crate::task::scheduler::current_thread_name();
            let len = name_buf.iter().position(|&b| b == 0).unwrap_or(32);
            let limit = len.min(15);
            let mut write_buf = [0u8; 16];
            write_buf[..limit].copy_from_slice(&name_buf[..limit]);
            write_buf[limit] = 0;
            if !super::handlers::helpers::copy_to_user_bytes(arg2, &write_buf, limit + 1) {
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

fn futex_read_u32(uaddr: u64) -> Result<u32, i32> {
    if (uaddr & 0x3) != 0 {
        return Err(EINVAL);
    }
    if !handlers::helpers::is_user_range_accessible(uaddr, 4) {
        return Err(EFAULT);
    }
    Ok(unsafe { core::ptr::read_volatile(uaddr as *const u32) })
}

fn futex_add_waiter_locked(
    slots: &mut [LinuxFutexSlot; LINUX_FUTEX_WAIT_SLOTS],
    uaddr: u64,
    tid: u32,
) -> bool {
    let mut target_idx = None;
    let mut empty_idx = None;
    for (idx, slot) in slots.iter().enumerate() {
        if slot.uaddr == uaddr {
            target_idx = Some(idx);
            break;
        }
        if slot.uaddr == 0 && empty_idx.is_none() {
            empty_idx = Some(idx);
        }
    }

    let idx = match target_idx.or(empty_idx) {
        Some(idx) => idx,
        None => return false,
    };
    if slots[idx].uaddr == 0 {
        slots[idx].uaddr = uaddr;
    }

    for waiter in slots[idx].tids.iter() {
        if *waiter == tid {
            return true;
        }
    }
    for waiter in slots[idx].tids.iter_mut() {
        if *waiter == 0 {
            *waiter = tid;
            return true;
        }
    }
    false
}

/// Remove `tid` from whichever futex slot it is queued on. Needed after a
/// blocking wait returns: FUTEX_REQUEUE may have moved the waiter to a
/// different uaddr, so removal must search by tid — removing by the original
/// uaddr would leak the entry and let a later wake on the requeue target hit
/// a thread that is no longer waiting.
fn futex_remove_tid_locked(
    slots: &mut [LinuxFutexSlot; LINUX_FUTEX_WAIT_SLOTS],
    tid: u32,
) -> bool {
    for slot in slots.iter_mut() {
        if slot.uaddr == 0 {
            continue;
        }
        let mut removed = false;
        for waiter in slot.tids.iter_mut() {
            if *waiter == tid {
                *waiter = 0;
                removed = true;
            }
        }
        if removed {
            if slot.tids.iter().all(|t| *t == 0) {
                slot.uaddr = 0;
            }
            return true;
        }
    }
    false
}

fn futex_remove_waiter_locked(
    slots: &mut [LinuxFutexSlot; LINUX_FUTEX_WAIT_SLOTS],
    uaddr: u64,
    tid: u32,
) -> bool {
    for slot in slots.iter_mut() {
        if slot.uaddr != uaddr {
            continue;
        }
        let mut removed = false;
        for waiter in slot.tids.iter_mut() {
            if *waiter == tid {
                *waiter = 0;
                removed = true;
            }
        }
        if slot.tids.iter().all(|tid| *tid == 0) {
            slot.uaddr = 0;
        }
        return removed;
    }
    false
}

pub(crate) fn futex_wake_addr(uaddr: u64, max_waiters: u32) -> u32 {
    if uaddr == 0 || max_waiters == 0 {
        return 0;
    }

    let limit = (max_waiters as usize).min(LINUX_FUTEX_WAKE_BATCH);
    let mut tids = [0u32; LINUX_FUTEX_WAKE_BATCH];
    let mut count = 0usize;
    {
        let mut slots = LINUX_FUTEX_WAITERS.lock();
        for slot in slots.iter_mut() {
            if slot.uaddr != uaddr {
                continue;
            }
            for waiter in slot.tids.iter_mut() {
                if *waiter != 0 && count < limit {
                    tids[count] = *waiter;
                    count += 1;
                    *waiter = 0;
                }
            }
            if slot.tids.iter().all(|tid| *tid == 0) {
                slot.uaddr = 0;
            }
            break;
        }
    }

    for tid in tids.iter().take(count) {
        crate::task::scheduler::wake_thread(*tid);
    }
    count as u32
}

fn futex_relative_deadline(timeout_ptr: u64) -> Result<Option<u32>, i32> {
    if timeout_ptr == 0 {
        return Ok(None);
    }
    if !handlers::helpers::is_user_range_accessible(timeout_ptr, 16) {
        return Err(EFAULT);
    }
    let sec = unsafe { core::ptr::read_unaligned(timeout_ptr as *const i64) };
    let nsec = unsafe { core::ptr::read_unaligned((timeout_ptr + 8) as *const i64) };
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return Err(EINVAL);
    }

    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u128;
    let ns = (sec as u128)
        .saturating_mul(1_000_000_000)
        .saturating_add(nsec as u128);
    let ticks = ns.saturating_mul(hz).saturating_add(999_999_999) / 1_000_000_000;
    let ticks = ticks.min(u32::MAX as u128) as u32;
    Ok(Some(
        crate::arch::hal::timer_current_ticks().wrapping_add(ticks),
    ))
}

fn linux_futex_wait(uaddr: u64, expected: u64, timeout_ptr: u64) -> u64 {
    let expected = expected as u32;
    match futex_read_u32(uaddr) {
        Ok(value) if value == expected => {}
        Ok(_) => return linux_err(EAGAIN),
        Err(errno) => return linux_err(errno),
    }

    let deadline = match futex_relative_deadline(timeout_ptr) {
        Ok(deadline) => deadline,
        Err(errno) => return linux_err(errno),
    };
    let tid = crate::task::scheduler::current_tid();

    {
        let mut slots = LINUX_FUTEX_WAITERS.lock();
        if !futex_add_waiter_locked(&mut slots, uaddr, tid) {
            return linux_err(ENOMEM);
        }
        match futex_read_u32(uaddr) {
            Ok(value) if value == expected => {
                if let Some(wake_at) = deadline {
                    crate::task::scheduler::prepare_to_block_until(wake_at);
                } else {
                    crate::task::scheduler::prepare_to_block_current();
                }
            }
            Ok(_) => {
                futex_remove_waiter_locked(&mut slots, uaddr, tid);
                return linux_err(EAGAIN);
            }
            Err(errno) => {
                futex_remove_waiter_locked(&mut slots, uaddr, tid);
                return linux_err(errno);
            }
        }
    }

    crate::task::scheduler::schedule();

    // Search by tid, not uaddr: FUTEX_REQUEUE may have moved this waiter to
    // a different futex while it was blocked.
    let timed_out = {
        let mut slots = LINUX_FUTEX_WAITERS.lock();
        futex_remove_tid_locked(&mut slots, tid)
    };
    if timed_out && deadline.is_some() {
        return linux_err(ETIMEDOUT);
    }
    0
}

/// FUTEX_REQUEUE / FUTEX_CMP_REQUEUE: wake up to `max_wake` waiters of
/// `uaddr`, move up to `max_requeue` of the remaining ones onto `uaddr2`.
/// Waiters that cannot be requeued (target slot full) are woken instead —
/// futex semantics allow spurious wakes but never lost waiters.
fn futex_requeue(
    uaddr: u64,
    max_wake: u32,
    max_requeue: u32,
    uaddr2: u64,
    expected: Option<u32>,
) -> u64 {
    if uaddr == 0 || uaddr2 == 0 || uaddr == uaddr2 {
        return linux_err(EINVAL);
    }
    if let Some(expected) = expected {
        match futex_read_u32(uaddr) {
            Ok(value) if value == expected => {}
            Ok(_) => return linux_err(EAGAIN),
            Err(errno) => return linux_err(errno),
        }
    }

    let wake_limit = (max_wake as usize).min(LINUX_FUTEX_WAKE_BATCH);
    let mut wake_tids = [0u32; LINUX_FUTEX_WAKE_BATCH];
    let mut woken = 0usize;
    let mut requeued = 0u32;
    {
        let mut slots = LINUX_FUTEX_WAITERS.lock();
        // Detach all waiters from the source futex first; the slot borrow
        // must end before futex_add_waiter_locked touches the target slot.
        let mut pending = [0u32; LINUX_FUTEX_WAITERS_PER_SLOT];
        let mut pending_count = 0usize;
        for slot in slots.iter_mut() {
            if slot.uaddr != uaddr {
                continue;
            }
            for waiter in slot.tids.iter_mut() {
                if *waiter != 0 {
                    pending[pending_count] = *waiter;
                    pending_count += 1;
                    *waiter = 0;
                }
            }
            slot.uaddr = 0;
            break;
        }
        for &tid in pending.iter().take(pending_count) {
            if woken < wake_limit {
                wake_tids[woken] = tid;
                woken += 1;
            } else if requeued < max_requeue && futex_add_waiter_locked(&mut slots, uaddr2, tid) {
                requeued += 1;
            } else if woken < LINUX_FUTEX_WAKE_BATCH {
                wake_tids[woken] = tid;
                woken += 1;
            }
        }
    }
    for tid in wake_tids.iter().take(woken) {
        crate::task::scheduler::wake_thread(*tid);
    }
    if expected.is_some() {
        // FUTEX_CMP_REQUEUE returns woken + requeued.
        woken as u64 + requeued as u64
    } else {
        woken as u64
    }
}

/// FUTEX_WAKE_OP: atomically read-modify-write *uaddr2 per the encoded op,
/// wake `max_wake1` waiters on uaddr, and if the old value of *uaddr2
/// satisfies the encoded comparison, wake `max_wake2` waiters on uaddr2.
/// glibc uses this for pthread condition variable signalling.
fn linux_futex_wake_op(uaddr: u64, max_wake1: u32, max_wake2: u32, uaddr2: u64, val3: u64) -> u64 {
    const FUTEX_OP_OPARG_SHIFT: u32 = 8;
    let mut op = ((val3 >> 28) & 0xf) as u32;
    let cmp = ((val3 >> 24) & 0xf) as u32;
    let mut oparg = ((val3 >> 12) & 0xfff) as u32;
    let cmparg = (val3 & 0xfff) as u32;
    if op & FUTEX_OP_OPARG_SHIFT != 0 {
        oparg = 1u32.wrapping_shl(oparg & 31);
        op &= !FUTEX_OP_OPARG_SHIFT;
    }

    // The RMW runs under the futex lock so it is at least serialized against
    // every other futex operation in the system.
    let old = {
        let _slots = LINUX_FUTEX_WAITERS.lock();
        let old = match futex_read_u32(uaddr2) {
            Ok(value) => value,
            Err(errno) => return linux_err(errno),
        };
        let new = match op {
            0 => oparg,                   // FUTEX_OP_SET
            1 => old.wrapping_add(oparg), // FUTEX_OP_ADD
            2 => old | oparg,             // FUTEX_OP_OR
            3 => old & !oparg,            // FUTEX_OP_ANDN
            4 => old ^ oparg,             // FUTEX_OP_XOR
            _ => return linux_err(ENOSYS),
        };
        if !handlers::helpers::copy_to_user_bytes(uaddr2, &new.to_le_bytes(), 4) {
            return linux_err(EFAULT);
        }
        old
    };

    let mut woken = futex_wake_addr(uaddr, max_wake1) as u64;
    let old_signed = old as i32;
    let cmparg_signed = cmparg as i32;
    let cond = match cmp {
        0 => old == cmparg,          // FUTEX_OP_CMP_EQ
        1 => old != cmparg,          // FUTEX_OP_CMP_NE
        2 => old_signed < cmparg_signed,  // FUTEX_OP_CMP_LT
        3 => old_signed <= cmparg_signed, // FUTEX_OP_CMP_LE
        4 => old_signed > cmparg_signed,  // FUTEX_OP_CMP_GT
        5 => old_signed >= cmparg_signed, // FUTEX_OP_CMP_GE
        _ => return linux_err(ENOSYS),
    };
    if cond {
        woken += futex_wake_addr(uaddr2, max_wake2) as u64;
    }
    woken
}

pub(super) fn linux_futex(
    uaddr: u64,
    op: u64,
    val: u64,
    timeout_ptr: u64,
    uaddr2: u64,
    val3: u64,
) -> u64 {
    let cmd = op & FUTEX_CMD_MASK;
    match cmd {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => linux_futex_wait(uaddr, val, timeout_ptr),
        FUTEX_WAKE | FUTEX_WAKE_BITSET => futex_wake_addr(uaddr, val as u32) as u64,
        // For REQUEUE ops the 4th argument is val2 (max requeue count), not
        // a timeout pointer.
        FUTEX_REQUEUE => futex_requeue(uaddr, val as u32, timeout_ptr as u32, uaddr2, None),
        FUTEX_CMP_REQUEUE => {
            futex_requeue(uaddr, val as u32, timeout_ptr as u32, uaddr2, Some(val3 as u32))
        }
        // For WAKE_OP the 4th argument is val2 (max wakes on uaddr2).
        FUTEX_WAKE_OP => linux_futex_wake_op(uaddr, val as u32, timeout_ptr as u32, uaddr2, val3),
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
