use super::*;

pub(super) fn linux_unsupported_syscall(
    regs: &SyscallRegs,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> u64 {
    crate::serial_println!(
        "licof linux: unsupported syscall nr={} rip={:#x} args={:#x},{:#x},{:#x},{:#x},{:#x},{:#x}",
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
    const CLONE_CHILD_SETTID: u64 = 0x0100_0000;
    const CLONE_THREAD: u64 = 0x0001_0000;
    const CLONE_SETTLS: u64 = 0x0008_0000;

    let signal = flags & CSIGNAL;
    if signal != 0 && signal != SIGCHLD {
        return linux_err(EINVAL);
    }
    if (flags & (CLONE_VM | CLONE_THREAD | CLONE_SETTLS)) != 0 || tls != 0 {
        crate::serial_println!(
            "licof linux clone: unsupported flags={:#x} child_stack={:#x} tls={:#x}",
            flags,
            child_stack,
            tls
        );
        return linux_err(ENOSYS);
    }
    if child_stack != 0 {
        crate::serial_println!(
            "licof linux clone: unsupported custom child_stack={:#x} flags={:#x}",
            child_stack,
            flags
        );
        return linux_err(ENOSYS);
    }

    if (flags & CLONE_PARENT_SETTID) != 0
        && (parent_tidptr == 0
            || !handlers::helpers::is_user_range_accessible(parent_tidptr, 4))
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

    #[cfg(target_arch = "x86_64")]
    let child_tid = handlers::sys_fork_with_child_tidptr(regs, child_tidptr);
    #[cfg(not(target_arch = "x86_64"))]
    let child_tid = handlers::sys_fork(regs);
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

    let err = crate::task::loader::exec_current_linux_process(&resolved, &argv, &envp);
    crate::serial_println!(
        "licof linux execve: failed linux='{}' resolved='{}': {}",
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

pub(super) fn linux_wait4(pid: i64, status_ptr: u64, options: u64, rusage_ptr: u64) -> u64 {
    const WNOHANG: u64 = 1;
    if (options & !WNOHANG) != 0 {
        return linux_err(EINVAL);
    }

    let wnohang = (options & WNOHANG) != 0;
    let (child_tid, code) = if pid == -1 {
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
        return linux_err(ECHILD);
    };

    if child_tid == u32::MAX || code == u32::MAX {
        return linux_err(ECHILD);
    }
    if child_tid == u32::MAX - 1 || code == u32::MAX - 1 {
        return 0;
    }
    if code == u32::MAX - 2 {
        return 0;
    }

    if status_ptr != 0 {
        unsafe {
            write_u32(status_ptr, 0, (code & 0xff) << 8);
        }
    }
    if rusage_ptr != 0 {
        unsafe {
            core::ptr::write_bytes(rusage_ptr as *mut u8, 0, 144);
        }
    }
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

pub(super) fn linux_tgkill(_tgid: u64, tid: u64, sig: u64) -> u64 {
    if tid == 0 {
        return linux_err(EINVAL);
    }
    linux_kill(tid as i64, sig)
}

pub(super) fn linux_setpgid(pid: u64, _pgid: u64) -> u64 {
    if pid != 0 && !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }
    0
}

pub(super) fn linux_getpgid(pid: u64) -> u64 {
    if pid == 0 {
        return crate::task::scheduler::current_tid() as u64;
    }
    if !crate::task::scheduler::thread_exists(pid as u32) {
        return linux_err(ESRCH);
    }
    pid
}

pub(super) fn linux_setsid() -> u64 {
    crate::task::scheduler::current_tid() as u64
}

pub(super) fn linux_getsid(pid: u64) -> u64 {
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
    write_linux_uts_field(buf_ptr, 2, b"3.2.0-licof");
    write_linux_uts_field(buf_ptr, 3, b"#1 anyOS licof Linux ABI");
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

pub(super) fn linux_rt_sigaction(_sig: u64, _act: u64, oldact: u64, sigsetsize: u64) -> u64 {
    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if oldact != 0 {
        unsafe {
            core::ptr::write_bytes(oldact as *mut u8, 0, 32);
        }
    }
    0
}

pub(super) fn linux_rt_sigprocmask(_how: u64, _set: u64, oldset: u64, sigsetsize: u64) -> u64 {
    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if oldset != 0 {
        unsafe {
            write_u64(oldset, 0, 0);
        }
    }
    0
}

pub(super) fn linux_gettimeofday(tv_ptr: u64) -> u64 {
    if tv_ptr != 0 {
        let sec = linux_now_seconds();
        unsafe {
            write_u64(tv_ptr, 0, sec);
            write_u64(tv_ptr, 8, 0);
        }
    }
    0
}

pub(super) fn linux_time(time_ptr: u64) -> u64 {
    let sec = linux_now_seconds();
    if time_ptr != 0 {
        unsafe {
            write_u64(time_ptr, 0, sec);
        }
    }
    sec
}

pub(super) fn linux_clock_gettime(_clock_id: u64, ts_ptr: u64) -> u64 {
    if ts_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u64(ts_ptr, 0, linux_now_seconds());
        write_u64(ts_ptr, 8, 0);
    }
    0
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

pub(super) fn linux_socket(domain: u64, type_: u64, protocol: u64) -> u64 {
    const AF_UNIX: u64 = 1;
    const SOCK_TYPE_MASK: u64 = 0xF;
    const SOCK_STREAM: u64 = 1;
    if domain == AF_UNIX && (type_ & SOCK_TYPE_MASK) == SOCK_STREAM {
        return linux_err(EAFNOSUPPORT);
    }
    crate::serial_println!(
        "licof linux socket: unsupported domain={} type={:#x} protocol={} -> EAFNOSUPPORT",
        domain,
        type_,
        protocol
    );
    linux_err(EAFNOSUPPORT)
}

pub(super) fn linux_getgroups(size: u64, list_ptr: u64) -> u64 {
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

pub(super) fn linux_setgroups(size: u64, _list_ptr: u64) -> u64 {
    if handlers::sys_getuid() != 0 {
        return linux_err(EPERM);
    }
    if size > 1024 {
        return linux_err(EINVAL);
    }
    0
}

pub(super) fn linux_setres_id(real: u64, effective: u64, saved: u64, uid: bool) -> u64 {
    for value in [real, effective, saved] {
        if value != u32::MAX as u64 && value != 0 && value != current_linux_id(uid) as u64 {
            return linux_err(EPERM);
        }
    }
    0
}

pub(super) fn linux_getres_id(real_ptr: u64, effective_ptr: u64, saved_ptr: u64, uid: bool) -> u64 {
    if real_ptr == 0 || effective_ptr == 0 || saved_ptr == 0 {
        return linux_err(EFAULT);
    }
    let id = current_linux_id(uid);
    unsafe {
        write_u32(real_ptr, 0, id);
        write_u32(effective_ptr, 0, id);
        write_u32(saved_ptr, 0, id);
    }
    0
}

pub(super) fn linux_setfs_id(id: u64, uid: bool) -> u64 {
    let old = current_linux_id(uid);
    if id != u32::MAX as u64 && id != 0 && id != old as u64 {
        return linux_err(EPERM);
    }
    old as u64
}

pub(super) fn current_linux_id(uid: bool) -> u32 {
    if uid {
        handlers::sys_getuid()
    } else {
        handlers::sys_getgid()
    }
}

pub(super) fn linux_set_root_or_current(id: u64, uid: bool) -> u64 {
    if id == 0 || id == current_linux_id(uid) as u64 {
        0
    } else {
        linux_err(EPERM)
    }
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
        let effective = if handlers::sys_getuid() == 0 {
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
    if handlers::sys_getuid() == 0 {
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
            let name = b"licof\0";
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

pub(super) fn linux_prlimit64(_pid: u64, resource: u64, _new_limit: u64, old_limit: u64) -> u64 {
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
    crate::serial_println!("licof linux setrlimit: resource={} -> ok", resource);
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
    let ticks = crate::arch::hal::timer_current_ticks() as u64;
    let hz = crate::arch::hal::timer_frequency_hz() as u64;
    if hz == 0 {
        0
    } else {
        ticks / hz
    }
}
