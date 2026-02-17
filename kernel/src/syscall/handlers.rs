//! Syscall handler implementations for all supported system calls.
//!
//! Each `pub fn sys_*` corresponds to one syscall number and is called from
//! [`super::syscall_dispatch`]. Handlers validate user pointers, perform the
//! requested kernel operation, and return a result code in EAX.

use alloc::string::String;

// =========================================================================
// Helpers
// =========================================================================

/// Make a relative path absolute using the current thread's working directory.
/// Returns the input unchanged if already absolute.
fn resolve_path(path: &str) -> String {
    if path.starts_with('/') {
        String::from(path)
    } else {
        let mut cwd_buf = [0u8; 256];
        let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
        let cwd = core::str::from_utf8(&cwd_buf[..cwd_len]).unwrap_or("/");
        if cwd == "/" || cwd.is_empty() {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd, path)
        }
    }
}

/// Validate that a user pointer is in user address space (below kernel half).
/// Returns false if the pointer is NULL, in kernel space, or if ptr+len overflows.
#[inline]
fn is_valid_user_ptr(ptr: u64, len: u64) -> bool {
    if ptr == 0 {
        return false;
    }
    // User space is below 0x0000_8000_0000_0000 (canonical lower half)
    let end = ptr.checked_add(len);
    match end {
        Some(e) => e <= 0x0000_8000_0000_0000,
        None => false, // overflow
    }
}

/// Read a null-terminated string from user memory (max 256 bytes).
/// Returns None if the pointer is invalid.
fn read_user_str_safe(ptr: u32) -> Option<&'static str> {
    if !is_valid_user_ptr(ptr as u64, 1) {
        return None;
    }
    let p = ptr as *const u8;
    let mut len = 0usize;
    unsafe {
        while len < 256 && *p.add(len) != 0 {
            len += 1;
        }
        Some(core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len)))
    }
}

/// Read a null-terminated string from user memory (max 256 bytes).
/// Returns "" if the pointer is invalid (NULL or kernel space).
unsafe fn read_user_str(ptr: u32) -> &'static str {
    if !is_valid_user_ptr(ptr as u64, 1) {
        return "";
    }
    let p = ptr as *const u8;
    let mut len = 0usize;
    while len < 256 && *p.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len))
}

// =========================================================================
// Process management (SYS_EXIT, SYS_WRITE, SYS_READ, SYS_GETPID, etc.)
// =========================================================================

/// sys_exit - Terminate the current process
pub fn sys_exit(status: u32) -> u32 {
    // Single atomic lock acquisition — no TOCTOU gaps between reads
    let (tid, pd, can_destroy) = crate::task::scheduler::current_exit_info();
    crate::debug_println!("sys_exit({}) TID={}", status, tid);

    // Clean up shared memory mappings while still in user PD context.
    // Must happen BEFORE switching CR3, so unmap_page operates on the
    // correct page tables via recursive mapping.
    if pd.is_some() {
        crate::ipc::shared_memory::cleanup_process(tid);
    }

    // Clean up environment variables for this process
    if let Some(pd_phys) = pd {
        crate::task::env::cleanup(pd_phys.as_u64());
    }

    if let Some(pd_phys) = pd {
        unsafe {
            let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3);
        }
        if can_destroy {
            crate::memory::virtual_mem::destroy_user_page_directory(pd_phys);
        }
    }

    crate::task::scheduler::exit_current(status);
    0 // unreachable
}

/// sys_kill - Kill a thread by TID
pub fn sys_kill(tid: u32) -> u32 {
    if tid == 0 { return u32::MAX; }
    crate::debug_println!("sys_kill({})", tid);
    crate::task::scheduler::kill_thread(tid)
}

/// sys_write - Write to a file descriptor
/// fd=1 -> stdout (pipe if configured, else serial), fd=2 -> stderr (same), fd>=3 -> VFS file
pub fn sys_write(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    if len > 0x1000_0000 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }
    if fd == 1 || fd == 2 {
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
        if pipe_id != 0 {
            crate::ipc::pipe::write(pipe_id, buf);
        }
        // Acquire the serial output lock so entire messages are atomic —
        // without this, user-space println! bytes interleave with kernel serial_println!
        let lock_state = crate::drivers::serial::output_lock_acquire();
        for &byte in buf {
            crate::drivers::serial::write_byte(byte);
        }
        crate::drivers::serial::output_lock_release(lock_state);
        len
    } else if fd >= 3 {
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        match crate::fs::vfs::write(fd, buf) {
            Ok(n) => {
                crate::task::scheduler::record_io_write(n as u64);
                n as u32
            }
            Err(_) => u32::MAX,
        }
    } else {
        u32::MAX
    }
}

/// sys_read - Read from a file descriptor
/// fd=0 -> stdin (reads from stdin_pipe if set), fd>=3 -> VFS file
pub fn sys_read(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if fd == 0 {
        let pipe = crate::task::scheduler::current_thread_stdin_pipe();
        if pipe != 0 {
            if buf_ptr == 0 || len == 0 {
                return 0;
            }
            if len > 0x1000_0000 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
                return u32::MAX;
            }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
            return crate::ipc::pipe::read(pipe, buf) as u32;
        }
        0 // no stdin pipe
    } else if fd >= 3 {
        if buf_ptr == 0 || len == 0 {
            return 0;
        }
        if len > 0x1000_0000 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
            return u32::MAX;
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
        match crate::fs::vfs::read(fd, buf) {
            Ok(n) => {
                crate::task::scheduler::record_io_read(n as u64);
                n as u32
            }
            Err(_) => u32::MAX,
        }
    } else {
        u32::MAX
    }
}

/// sys_open - Open a file. arg1=path_ptr (null-terminated), arg2=flags, arg3=unused
/// Returns file descriptor or u32::MAX on error.
pub fn sys_open(path_ptr: u32, flags: u32, _arg3: u32) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let file_flags = crate::fs::file::FileFlags {
        read: true,
        write: (flags & 1) != 0,
        append: (flags & 2) != 0,
        create: (flags & 4) != 0,
        truncate: (flags & 8) != 0,
    };
    let resolved = resolve_path(path);
    let result = match crate::fs::vfs::open(&resolved, file_flags) {
        Ok(fd) => fd,
        Err(_) => u32::MAX,
    };
    crate::debug_println!("  open({:?}) -> fd={}", resolved, if result == u32::MAX { -1i32 } else { result as i32 });
    result
}

/// sys_close - Close a file descriptor
pub fn sys_close(fd: u32) -> u32 {
    if fd < 3 {
        return 0;
    }
    match crate::fs::vfs::close(fd) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// sys_getpid - Get current process ID
pub fn sys_getpid() -> u32 {
    crate::task::scheduler::current_tid()
}

/// sys_yield - Yield the CPU to another thread
pub fn sys_yield() -> u32 {
    crate::task::scheduler::schedule();
    0
}

/// sys_sleep - Sleep for N milliseconds (blocking sleep with timer wake).
/// The thread is blocked and does not consume CPU until the timer wakes it.
pub fn sys_sleep(ms: u32) -> u32 {
    if ms == 0 {
        return 0;
    }
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let ticks = (ms as u64 * pit_hz as u64 / 1000) as u32;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let now = crate::arch::x86::pit::get_ticks();
    let wake_at = now.wrapping_add(ticks);
    crate::task::scheduler::sleep_until(wake_at);
    0
}

/// sys_sbrk - Grow/shrink the process heap
pub fn sys_sbrk(increment: i32) -> u32 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    let old_brk = crate::task::scheduler::current_thread_brk();
    if old_brk == 0 {
        return u32::MAX;
    }
    if increment == 0 {
        return old_brk;
    }

    let page_size = 4096u32;

    if increment > 0 {
        let new_brk = old_brk + increment as u32;
        let old_page_end = (old_brk + page_size - 1) & !(page_size - 1);
        let new_page_end = (new_brk + page_size - 1) & !(page_size - 1);

        let mut addr = old_page_end;
        let mut pages_mapped = 0u32;
        while addr < new_page_end {
            // Skip pages already mapped (another thread sharing this PD may have mapped them)
            if !virtual_mem::is_page_mapped(VirtAddr::new(addr as u64)) {
                if let Some(phys) = physical::alloc_frame() {
                    virtual_mem::map_page(VirtAddr::new(addr as u64), phys, 0x02 | 0x04);
                    unsafe { core::ptr::write_bytes(addr as *mut u8, 0, page_size as usize); }
                    pages_mapped += 1;
                } else {
                    return u32::MAX;
                }
            }
            addr += page_size;
        }

        if pages_mapped > 0 {
            crate::task::scheduler::adjust_current_user_pages(pages_mapped as i32);
        }

        // set_current_thread_brk also syncs brk across all sibling threads
        // sharing the same page directory (with cli to prevent timer deadlock).
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    } else {
        let decrement = (-increment) as u32;
        let new_brk = old_brk.saturating_sub(decrement);
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    }
}

/// sys_waitpid - Wait for a process to exit. Returns exit code.
pub fn sys_waitpid(tid: u32) -> u32 {
    crate::task::scheduler::waitpid(tid)
}

/// sys_try_waitpid - Non-blocking check if process exited.
/// Returns exit code if terminated, or u32::MAX-1 if still running.
pub fn sys_try_waitpid(tid: u32) -> u32 {
    crate::task::scheduler::try_waitpid(tid)
}

/// sys_spawn - Spawn a new process from a filesystem path.
/// arg1=path_ptr, arg2=stdout_pipe_id (0=none), arg3=args_ptr (0=none), arg4=stdin_pipe_id (0=none)
/// Returns TID or u32::MAX on error.
pub fn sys_spawn(path_ptr: u32, stdout_pipe: u32, args_ptr: u32, stdin_pipe: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let args = if args_ptr != 0 {
        unsafe { read_user_str(args_ptr) }
    } else {
        ""
    };
    crate::debug_println!("sys_spawn: path='{}' pipe={} args_ptr={:#x} stdin_pipe={}", path, stdout_pipe, args_ptr, stdin_pipe);
    let raw_name = path.rsplit('/').next().unwrap_or(path);
    // Strip ".app" suffix so process name is clean (e.g. "Calculator" not "Calculator.app")
    let name = if raw_name.ends_with(".app") {
        &raw_name[..raw_name.len() - 4]
    } else {
        raw_name
    };
    match crate::task::loader::load_and_run_with_args(path, name, args) {
        Ok(tid) => {
            // Inherit parent's cwd
            let mut cwd_buf = [0u8; 256];
            let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
            if cwd_len > 0 {
                if let Ok(cwd) = core::str::from_utf8(&cwd_buf[..cwd_len]) {
                    crate::task::scheduler::set_thread_cwd(tid, cwd);
                }
            }
            if stdout_pipe != 0 {
                crate::task::scheduler::set_thread_stdout_pipe(tid, stdout_pipe);
            }
            if stdin_pipe != 0 {
                crate::task::scheduler::set_thread_stdin_pipe(tid, stdin_pipe);
            }
            crate::debug_println!("sys_spawn: returning TID={}", tid);
            tid
        }
        Err(e) => {
            crate::serial_println!("sys_spawn: FAILED: {}", e);
            u32::MAX
        }
    }
}

/// sys_getargs - Get command-line arguments for the current process.
/// arg1=buf_ptr, arg2=buf_size. Returns bytes written.
pub fn sys_getargs(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    crate::task::scheduler::current_thread_args(buf) as u32
}

// =========================================================================
// Filesystem (SYS_READDIR, SYS_STAT)
// =========================================================================

/// sys_readdir - Read directory entries.
/// arg1=path_ptr (null-terminated), arg2=buf_ptr, arg3=buf_size
/// Each entry: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes] = 64 bytes
/// Returns number of entries, or u32::MAX on error.
pub fn sys_readdir(path_ptr: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => {
            let entry_size = 64u32;
            if buf_ptr != 0 && buf_size > 0
                && is_valid_user_ptr(buf_ptr as u64, buf_size as u64)
            {
                let max_entries = (buf_size / entry_size) as usize;
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize)
                };
                for (i, entry) in entries.iter().enumerate().take(max_entries) {
                    let off = i * entry_size as usize;
                    buf[off] = match entry.file_type {
                        crate::fs::file::FileType::Regular => 0,
                        crate::fs::file::FileType::Directory => 1,
                        crate::fs::file::FileType::Device => 2,
                    };
                    let name_bytes = entry.name.as_bytes();
                    let name_len = name_bytes.len().min(55);
                    buf[off + 1] = name_len as u8;
                    buf[off + 2] = 0;
                    buf[off + 3] = 0;
                    let size = entry.size as u32;
                    buf[off + 4..off + 8].copy_from_slice(&size.to_le_bytes());
                    buf[off + 8..off + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);
                    buf[off + 8 + name_len] = 0;
                }
            }
            entries.len() as u32
        }
        Err(_) => u32::MAX,
    }
}

/// sys_stat - Get file information.
/// arg1=path_ptr (null-terminated), arg2=stat_buf_ptr: output [type:u32, size:u32] = 8 bytes
/// Returns 0 on success, u32::MAX on error.
pub fn sys_stat(path_ptr: u32, buf_ptr: u32) -> u32 {
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);

    // Use vfs::stat() which does a directory-entry lookup (no file I/O)
    match crate::fs::vfs::stat(&path) {
        Ok((file_type, size)) => {
            if buf_ptr != 0 {
                let type_val: u32 = match file_type {
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                    _ => 0, // Regular
                };
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = type_val;
                    *buf.add(1) = size;
                }
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_lseek - Seek within an open file.
/// arg1=fd, arg2=offset (signed i32), arg3=whence (0=SET, 1=CUR, 2=END)
/// Returns new position, or u32::MAX on error.
pub fn sys_lseek(fd: u32, offset: u32, whence: u32) -> u32 {
    if fd < 3 { return 0; }
    match crate::fs::vfs::lseek(fd, offset as i32, whence) {
        Ok(pos) => pos,
        Err(_) => u32::MAX,
    }
}

/// sys_fstat - Get file information by fd.
/// arg1=fd, arg2=stat_buf_ptr: output [type:u32, size:u32, position:u32] = 12 bytes
/// Returns 0 on success, u32::MAX on error.
pub fn sys_fstat(fd: u32, buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
    if fd < 3 {
        // stdin/stdout/stderr: character device, size 0
        unsafe {
            let buf = buf_ptr as *mut u32;
            *buf = 2; // device
            *buf.add(1) = 0;
            *buf.add(2) = 0;
        }
        return 0;
    }
    match crate::fs::vfs::fstat(fd) {
        Ok((file_type, size, position)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = match file_type {
                    crate::fs::file::FileType::Regular => 0,
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                };
                *buf.add(1) = size;
                *buf.add(2) = position;
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_getcwd - Get current working directory.
/// arg1=buf_ptr, arg2=buf_size. Returns length written.
pub fn sys_getcwd(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return u32::MAX;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    let len = crate::task::scheduler::current_thread_cwd(buf);
    if len == 0 {
        // Fallback
        if buf_size >= 2 {
            buf[0] = b'/';
            buf[1] = 0;
        }
        return 1;
    }
    len as u32
}

/// sys_chdir - Change current working directory.
/// arg1=path_ptr (null-terminated). Returns 0 on success, u32::MAX on error.
pub fn sys_chdir(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);
    // Verify the directory exists
    match crate::fs::vfs::read_dir(&path) {
        Ok(_) => {
            let tid = crate::task::scheduler::current_tid();
            crate::task::scheduler::set_thread_cwd(tid, &path);
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_isatty - Check if a file descriptor refers to a terminal.
/// Returns 1 for stdin/stdout/stderr, 0 otherwise.
pub fn sys_isatty(fd: u32) -> u32 {
    if fd <= 2 { 1 } else { 0 }
}

/// sys_mkdir - Create a directory. arg1=path_ptr (null-terminated).
pub fn sys_mkdir(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });
    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// sys_unlink - Delete a file. arg1=path_ptr (null-terminated).
pub fn sys_unlink(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });
    match crate::fs::vfs::delete(&path) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// sys_truncate - Truncate a file to zero. arg1=path_ptr (null-terminated).
pub fn sys_truncate(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });
    match crate::fs::vfs::truncate(&path) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

// =========================================================================
// Mount/Unmount (SYS_MOUNT, SYS_UMOUNT, SYS_LIST_MOUNTS)
// =========================================================================

/// sys_mount - Mount a filesystem.
/// arg1=mount_path_ptr, arg2=device_path_ptr, arg3=fs_type (0=fat, 1=iso9660)
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_mount(mount_path_ptr: u32, device_path_ptr: u32, fs_type: u32) -> u32 {
    if mount_path_ptr == 0 { return u32::MAX; }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    let device_path = if device_path_ptr != 0 {
        String::from(unsafe { read_user_str(device_path_ptr) })
    } else {
        String::new()
    };
    match crate::fs::vfs::mount_fs(&mount_path, &device_path, fs_type) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// sys_umount - Unmount a filesystem.
/// arg1=mount_path_ptr
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_umount(mount_path_ptr: u32) -> u32 {
    if mount_path_ptr == 0 { return u32::MAX; }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    match crate::fs::vfs::umount_fs(&mount_path) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// sys_list_mounts - List all mount points.
/// arg1=buf_ptr: output buffer
/// arg2=buf_len: buffer capacity
/// Returns number of bytes written, or u32::MAX on error.
///
/// Output format: "mount_path\tfs_type\n" for each mount, null-terminated.
pub fn sys_list_mounts(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 { return u32::MAX; }
    if !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) { return u32::MAX; }

    let mounts = crate::fs::vfs::list_mounts();
    let mut output = String::new();
    for (path, fs_type, _dev_id) in &mounts {
        output.push_str(path);
        output.push('\t');
        output.push_str(fs_type);
        output.push('\n');
    }

    let bytes = output.as_bytes();
    let to_copy = bytes.len().min(buf_len as usize - 1);
    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, to_copy);
        *dst.add(to_copy) = 0; // null-terminate
    }
    to_copy as u32
}

// =========================================================================
// System Information (SYS_TIME, SYS_UPTIME, SYS_SYSINFO)
// =========================================================================

/// sys_time - Get current date/time.
/// arg1=buf_ptr: output [year_lo:u8, year_hi:u8, month:u8, day:u8, hour:u8, min:u8, sec:u8, pad:u8]
pub fn sys_time(buf_ptr: u32) -> u32 {
    let (year, month, day, hour, min, sec) = crate::drivers::rtc::read_datetime();
    if buf_ptr != 0 {
        unsafe {
            let buf = buf_ptr as *mut u8;
            let year_bytes = (year as u16).to_le_bytes();
            *buf = year_bytes[0];
            *buf.add(1) = year_bytes[1];
            *buf.add(2) = month as u8;
            *buf.add(3) = day as u8;
            *buf.add(4) = hour as u8;
            *buf.add(5) = min as u8;
            *buf.add(6) = sec as u8;
            *buf.add(7) = 0;
        }
    }
    0
}

/// sys_uptime - Get system uptime in PIT ticks (see `pit::TICK_HZ`).
pub fn sys_uptime() -> u32 {
    crate::arch::x86::pit::get_ticks()
}

/// sys_tick_hz - Get the PIT tick rate in Hz.
pub fn sys_tick_hz() -> u32 {
    crate::arch::x86::pit::TICK_HZ
}

/// sys_dmesg - Read kernel log ring buffer.
/// arg1=buf_ptr, arg2=buf_size. Returns bytes written.
pub fn sys_dmesg(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    crate::drivers::serial::read_log(buf) as u32
}

/// sys_sysinfo - Get system information.
/// arg1=cmd: 0=memory, 1=threads, 2=cpus, 3=cpu_load
/// arg2=buf_ptr, arg3=buf_size
pub fn sys_sysinfo(cmd: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    match cmd {
        0 => {
            // Memory: [total_frames:u32, free_frames:u32, heap_used:u32, heap_total:u32] = 16 bytes
            if buf_ptr != 0 && buf_size >= 8 {
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = crate::memory::physical::total_frames() as u32;
                    *buf.add(1) = crate::memory::physical::free_frames() as u32;
                    if buf_size >= 16 {
                        let (heap_used, heap_total) = crate::memory::heap::heap_stats();
                        *buf.add(2) = heap_used as u32;
                        *buf.add(3) = heap_total as u32;
                    }
                }
            }
            0
        }
        1 => {
            // Thread list: 56 bytes each
            // [tid:u32, prio:u8, state:u8, arch:u8, pad:u8, name:24bytes,
            //  user_pages:u32, cpu_ticks:u32, io_read_bytes:u64, io_write_bytes:u64]
            let threads = crate::task::scheduler::list_threads();
            if buf_ptr != 0 && buf_size > 0 {
                let entry_size = 56usize;
                let max = (buf_size as usize) / entry_size;
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize)
                };
                for (i, t) in threads.iter().enumerate().take(max) {
                    let off = i * entry_size;
                    buf[off..off + 4].copy_from_slice(&t.tid.to_le_bytes());
                    buf[off + 4] = t.priority;
                    buf[off + 5] = match t.state {
                        "ready" => 0, "running" => 1, "blocked" => 2, "dead" => 3, _ => 255,
                    };
                    buf[off + 6] = t.arch_mode; // 0=x86_64, 1=x86
                    buf[off + 7] = 0;
                    let name_bytes = t.name.as_bytes();
                    let n = name_bytes.len().min(23);
                    buf[off + 8..off + 8 + n].copy_from_slice(&name_bytes[..n]);
                    buf[off + 8 + n] = 0;
                    // user_pages at offset 32
                    buf[off + 32..off + 36].copy_from_slice(&t.user_pages.to_le_bytes());
                    // cpu_ticks at offset 36
                    buf[off + 36..off + 40].copy_from_slice(&t.cpu_ticks.to_le_bytes());
                    // io_read_bytes at offset 40, io_write_bytes at offset 48
                    buf[off + 40..off + 48].copy_from_slice(&t.io_read_bytes.to_le_bytes());
                    buf[off + 48..off + 56].copy_from_slice(&t.io_write_bytes.to_le_bytes());
                }
            }
            threads.len() as u32
        }
        2 => crate::arch::x86::smp::cpu_count() as u32,
        3 => {
            // CPU load (extended):
            //   [0] total_sched_ticks (u32)
            //   [1] total_idle_ticks  (u32)
            //   [2] num_cpus          (u32)
            //   [3] reserved          (u32)
            //   [4..4+num_cpus*2] per_cpu_total[i], per_cpu_idle[i] pairs
            // Minimum 16 bytes for header, +8 per CPU
            let num_cpus = crate::arch::x86::smp::cpu_count() as usize;
            if buf_ptr != 0 && buf_size >= 16 {
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = crate::task::scheduler::total_sched_ticks();
                    *buf.add(1) = crate::task::scheduler::idle_sched_ticks();
                    *buf.add(2) = num_cpus as u32;
                    *buf.add(3) = 0;
                    // Per-CPU data if buffer is large enough
                    for i in 0..num_cpus {
                        let off = 4 + i * 2;
                        if (off + 2) * 4 <= buf_size as usize {
                            *buf.add(off) = crate::task::scheduler::per_cpu_total_ticks(i);
                            *buf.add(off + 1) = crate::task::scheduler::per_cpu_idle_ticks(i);
                        }
                    }
                }
            }
            0
        }
        4 => {
            // Hardware info: 96-byte struct
            //   [0..48]  CPU brand string (null-terminated)
            //   [48..64] CPU vendor string (null-terminated)
            //   [64..68] TSC frequency in MHz (u32 LE)
            //   [68..72] CPU count (u32 LE)
            //   [72..76] Boot mode: 0=BIOS, 1=UEFI (u32 LE)
            //   [76..80] Total physical memory in MiB (u32 LE)
            //   [80..84] Free physical memory in MiB (u32 LE)
            //   [84..88] Framebuffer width (u32 LE)
            //   [88..92] Framebuffer height (u32 LE)
            //   [92..96] Framebuffer BPP (u32 LE)
            if buf_ptr == 0 || buf_size < 96 { return u32::MAX; }
            if !is_valid_user_ptr(buf_ptr as u64, 96) { return u32::MAX; }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 96) };
            buf.fill(0);

            // CPU brand (48 bytes) and vendor (16 bytes)
            let brand = crate::arch::x86::cpuid::brand();
            let vendor = crate::arch::x86::cpuid::vendor();
            buf[0..48].copy_from_slice(brand);
            buf[48..64].copy_from_slice(vendor);

            // TSC MHz
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            buf[64..68].copy_from_slice(&tsc_mhz.to_le_bytes());

            // CPU count
            let ncpu = crate::arch::x86::smp::cpu_count() as u32;
            buf[68..72].copy_from_slice(&ncpu.to_le_bytes());

            // Boot mode
            let bmode = crate::boot_mode() as u32;
            buf[72..76].copy_from_slice(&bmode.to_le_bytes());

            // Physical memory in MiB
            let total_mib = (crate::memory::physical::total_frames() as u32 * 4) / 1024;
            let free_mib = (crate::memory::physical::free_frames() as u32 * 4) / 1024;
            buf[76..80].copy_from_slice(&total_mib.to_le_bytes());
            buf[80..84].copy_from_slice(&free_mib.to_le_bytes());

            // Framebuffer info
            if let Some(fb) = crate::drivers::framebuffer::info() {
                buf[84..88].copy_from_slice(&(fb.width as u32).to_le_bytes());
                buf[88..92].copy_from_slice(&(fb.height as u32).to_le_bytes());
                buf[92..96].copy_from_slice(&(fb.bpp as u32).to_le_bytes());
            }

            96
        }
        _ => u32::MAX,
    }
}

// =========================================================================
// Networking (SYS_NET_*)
// =========================================================================

/// sys_net_config - Get or set network configuration.
/// arg1=cmd (0=get, 1=set), arg2=buf_ptr (24 bytes: ip4+mask4+gw4+dns4+mac6+link1+pad1)
pub fn sys_net_config(cmd: u32, buf_ptr: u32) -> u32 {
    match cmd {
        0 => {
            if buf_ptr == 0 { return u32::MAX; }
            let cfg = crate::net::config();
            let link_up = crate::drivers::network::e1000::is_link_up();
            unsafe {
                let buf = buf_ptr as *mut u8;
                core::ptr::copy_nonoverlapping(cfg.ip.0.as_ptr(), buf, 4);
                core::ptr::copy_nonoverlapping(cfg.mask.0.as_ptr(), buf.add(4), 4);
                core::ptr::copy_nonoverlapping(cfg.gateway.0.as_ptr(), buf.add(8), 4);
                core::ptr::copy_nonoverlapping(cfg.dns.0.as_ptr(), buf.add(12), 4);
                core::ptr::copy_nonoverlapping(cfg.mac.0.as_ptr(), buf.add(16), 6);
                *buf.add(22) = if link_up { 1 } else { 0 };
                *buf.add(23) = 0;
            }
            0
        }
        1 => {
            if buf_ptr == 0 { return u32::MAX; }
            unsafe {
                let buf = buf_ptr as *const u8;
                let mut ip = [0u8; 4]; let mut mask = [0u8; 4];
                let mut gw = [0u8; 4]; let mut dns = [0u8; 4];
                core::ptr::copy_nonoverlapping(buf, ip.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(4), mask.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(8), gw.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(12), dns.as_mut_ptr(), 4);
                crate::net::set_config(
                    crate::net::types::Ipv4Addr(ip), crate::net::types::Ipv4Addr(mask),
                    crate::net::types::Ipv4Addr(gw), crate::net::types::Ipv4Addr(dns),
                );
            }
            0
        }
        2 => {
            // Disable NIC
            crate::drivers::network::e1000::set_enabled(false);
            0
        }
        3 => {
            // Enable NIC
            crate::drivers::network::e1000::set_enabled(true);
            0
        }
        4 => {
            // Query enabled state
            if crate::drivers::network::e1000::is_enabled() { 1 } else { 0 }
        }
        5 => {
            // Query hardware availability
            if crate::drivers::network::e1000::is_available() { 1 } else { 0 }
        }
        _ => u32::MAX,
    }
}

/// sys_net_ping - ICMP ping. arg1=ip_ptr(4 bytes), arg2=seq, arg3=timeout_ticks
/// Returns RTT in ticks, or u32::MAX on timeout.
pub fn sys_net_ping(ip_ptr: u32, seq: u32, timeout: u32) -> u32 {
    if ip_ptr == 0 { return u32::MAX; }
    let mut ip_bytes = [0u8; 4];
    unsafe { core::ptr::copy_nonoverlapping(ip_ptr as *const u8, ip_bytes.as_mut_ptr(), 4); }
    let ip = crate::net::types::Ipv4Addr(ip_bytes);
    match crate::net::icmp::ping(ip, seq as u16, timeout) {
        Some((rtt, _ttl)) => rtt,
        None => u32::MAX,
    }
}

/// sys_net_dhcp - DHCP discovery. arg1=buf_ptr (16 bytes: ip+mask+gw+dns)
/// Returns 0 on success, applies config automatically.
pub fn sys_net_dhcp(buf_ptr: u32) -> u32 {
    match crate::net::dhcp::discover() {
        Ok(result) => {
            crate::net::set_config(result.ip, result.mask, result.gateway, result.dns);
            if buf_ptr != 0 {
                unsafe {
                    let buf = buf_ptr as *mut u8;
                    core::ptr::copy_nonoverlapping(result.ip.0.as_ptr(), buf, 4);
                    core::ptr::copy_nonoverlapping(result.mask.0.as_ptr(), buf.add(4), 4);
                    core::ptr::copy_nonoverlapping(result.gateway.0.as_ptr(), buf.add(8), 4);
                    core::ptr::copy_nonoverlapping(result.dns.0.as_ptr(), buf.add(12), 4);
                }
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_net_dns - DNS resolve. arg1=hostname_ptr, arg2=result_ptr(4 bytes)
pub fn sys_net_dns(hostname_ptr: u32, result_ptr: u32) -> u32 {
    let hostname = unsafe { read_user_str(hostname_ptr) };
    match crate::net::dns::resolve(hostname) {
        Ok(ip) => {
            if result_ptr != 0 {
                unsafe { core::ptr::copy_nonoverlapping(ip.0.as_ptr(), result_ptr as *mut u8, 4); }
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

// =========================================================================
// TCP Networking (SYS_TCP_*)
// =========================================================================

/// sys_tcp_connect - Connect to a remote host.
/// arg1=params_ptr: [ip:4, port:u16, pad:u16, timeout:u32] = 12 bytes
/// Returns socket_id or u32::MAX on error.
pub fn sys_tcp_connect(params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let params = unsafe { core::slice::from_raw_parts(params_ptr as *const u8, 12) };
    let ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let port = u16::from_le_bytes([params[4], params[5]]);
    let timeout = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let timeout_ticks = if timeout == 0 { pit_hz } else { timeout * pit_hz / 1000 };
    crate::net::tcp::connect(ip, port, timeout_ticks)
}

/// sys_tcp_send - Send data on TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes sent or u32::MAX on error.
pub fn sys_tcp_send(socket_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 { return 0; }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    crate::net::tcp::send(socket_id, buf, 1000) // 10s timeout
}

/// sys_tcp_recv - Receive data from TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes received, 0=EOF, u32::MAX=error.
pub fn sys_tcp_recv(socket_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 { return u32::MAX; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    crate::net::tcp::recv(socket_id, buf, 3000) // 30s timeout
}

/// sys_tcp_close - Close TCP connection. arg1=socket_id.
pub fn sys_tcp_close(socket_id: u32) -> u32 {
    crate::net::tcp::close(socket_id)
}

/// sys_tcp_status - Get TCP connection state. arg1=socket_id.
/// Returns state enum as u32, or u32::MAX if not found.
pub fn sys_tcp_status(socket_id: u32) -> u32 {
    crate::net::tcp::status(socket_id)
}

/// sys_tcp_recv_available - Check bytes available to read.
/// Returns: >0 = bytes available, 0 = no data, u32::MAX-1 = EOF, u32::MAX = error.
pub fn sys_tcp_recv_available(socket_id: u32) -> u32 {
    crate::net::tcp::recv_available(socket_id)
}

/// sys_tcp_shutdown_wr - Half-close (send FIN, don't block).
/// arg1=socket_id. Returns 0 on success.
pub fn sys_tcp_shutdown_wr(socket_id: u32) -> u32 {
    crate::net::tcp::shutdown_write(socket_id)
}

/// sys_net_poll - Process pending network packets.
/// Triggers E1000 RX ring processing and TCP packet dispatch.
pub fn sys_net_poll() -> u32 {
    crate::net::poll();
    0
}

// =========================================================================
// UDP Networking (SYS_UDP_*)
// =========================================================================

/// sys_udp_bind - Bind to a UDP port (creates receive queue).
/// arg1=port. Returns 0 on success, u32::MAX if already bound or invalid.
pub fn sys_udp_bind(port: u32) -> u32 {
    if port == 0 || port > 65535 { return u32::MAX; }
    if crate::net::udp::bind(port as u16) { 0 } else { u32::MAX }
}

/// sys_udp_unbind - Unbind a UDP port.
/// arg1=port. Returns 0.
pub fn sys_udp_unbind(port: u32) -> u32 {
    if port > 65535 { return u32::MAX; }
    crate::net::udp::unbind(port as u16);
    0
}

/// sys_udp_sendto - Send a UDP datagram.
/// arg1=params_ptr: [dst_ip:4, dst_port:u16, src_port:u16, data_ptr:u32, data_len:u32, flags:u32] = 20 bytes
/// flags: bit 0 = force broadcast (bypass SO_BROADCAST check).
/// Returns bytes sent or u32::MAX on error.
pub fn sys_udp_sendto(params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let params = unsafe { core::slice::from_raw_parts(params_ptr as *const u8, 20) };

    let dst_ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let dst_port = u16::from_le_bytes([params[4], params[5]]);
    let src_port = u16::from_le_bytes([params[6], params[7]]);
    let data_ptr = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let data_len = u32::from_le_bytes([params[12], params[13], params[14], params[15]]);
    let flags = u32::from_le_bytes([params[16], params[17], params[18], params[19]]);

    if data_ptr == 0 || data_len == 0 { return 0; }
    if data_len > 1472 { return u32::MAX; } // Max UDP payload (1500 - 20 IP - 8 UDP)

    let data = unsafe { core::slice::from_raw_parts(data_ptr as *const u8, data_len as usize) };

    let ok = if flags & 1 != 0 {
        // Force broadcast flag — skip SO_BROADCAST check
        crate::net::udp::send_unchecked(dst_ip, src_port, dst_port, data)
    } else {
        crate::net::udp::send(dst_ip, src_port, dst_port, data)
    };

    if ok { data_len } else { u32::MAX }
}

/// sys_udp_recvfrom - Receive a UDP datagram on a bound port.
/// arg1=port, arg2=buf_ptr, arg3=buf_len.
/// Writes header [src_ip:4, src_port:u16, payload_len:u16] (8 bytes) then payload.
/// Returns total bytes written (8 + payload), 0 = no data/timeout, u32::MAX = error.
pub fn sys_udp_recvfrom(port: u32, buf_ptr: u32, buf_len: u32) -> u32 {
    if port == 0 || port > 65535 || buf_ptr == 0 || buf_len < 8 {
        return u32::MAX;
    }

    let port16 = port as u16;
    let timeout_ms = crate::net::udp::get_timeout_ms(port16);

    let dgram = if timeout_ms == 0 {
        // Non-blocking: poll once then try
        crate::net::poll();
        crate::net::udp::recv(port16)
    } else {
        let timeout_ticks = timeout_ms * crate::arch::x86::pit::TICK_HZ / 1000;
        crate::net::udp::recv_timeout(port16, if timeout_ticks == 0 { 1 } else { timeout_ticks })
    };

    match dgram {
        Some(d) => {
            let payload_len = d.data.len().min((buf_len as usize).saturating_sub(8));
            let total = 8 + payload_len;
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };

            // Header: src_ip (4 bytes)
            buf[0..4].copy_from_slice(&d.src_ip.0);
            // Header: src_port (u16 LE)
            buf[4..6].copy_from_slice(&d.src_port.to_le_bytes());
            // Header: payload_len (u16 LE)
            buf[6..8].copy_from_slice(&(payload_len as u16).to_le_bytes());
            // Payload
            buf[8..8 + payload_len].copy_from_slice(&d.data[..payload_len]);

            total as u32
        }
        None => 0,
    }
}

/// sys_udp_set_opt - Set a per-port socket option.
/// arg1=port, arg2=opt (1=SO_BROADCAST, 2=SO_RCVTIMEO), arg3=val.
/// Returns 0 on success, u32::MAX on error.
pub fn sys_udp_set_opt(port: u32, opt: u32, val: u32) -> u32 {
    if port == 0 || port > 65535 { return u32::MAX; }
    if crate::net::udp::set_opt(port as u16, opt, val) { 0 } else { u32::MAX }
}

/// sys_net_arp - Get ARP table. arg1=buf_ptr, arg2=buf_size
/// Each entry: [ip:4, mac:6, pad:2] = 12 bytes. Returns entry count.
pub fn sys_net_arp(buf_ptr: u32, buf_size: u32) -> u32 {
    let entries = crate::net::arp::entries();
    if buf_ptr != 0 && buf_size > 0 {
        let max = (buf_size / 12) as usize;
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        for (i, (ip, mac)) in entries.iter().enumerate().take(max) {
            let off = i * 12;
            buf[off..off + 4].copy_from_slice(&ip.0);
            buf[off + 4..off + 10].copy_from_slice(&mac.0);
            buf[off + 10] = 0;
            buf[off + 11] = 0;
        }
    }
    entries.len() as u32
}

/// sys_screen_size - Get screen dimensions from GPU driver.
pub fn sys_screen_size(buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
    match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some((w, h, _pitch, _addr)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = w;
                *buf.add(1) = h;
            }
            0
        }
        None => {
            // Fallback to boot framebuffer info
            match crate::drivers::framebuffer::info() {
                Some(fb) => {
                    unsafe {
                        let buf = buf_ptr as *mut u32;
                        *buf = fb.width;
                        *buf.add(1) = fb.height;
                    }
                    0
                }
                None => u32::MAX,
            }
        }
    }
}

// =========================================================================
// Display / GPU
// =========================================================================

/// sys_set_resolution - Change display resolution via GPU driver.
pub fn sys_set_resolution(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return u32::MAX;
    }
    match crate::drivers::gpu::with_gpu(|g| g.set_mode(width, height, 32)) {
        Some(Some(_)) => {
            // Update kernel-side cursor bounds for the new resolution
            crate::drivers::gpu::update_cursor_bounds(width, height);
            // Notify all subscribers about the resolution change
            crate::ipc::event_bus::system_emit(
                crate::ipc::event_bus::EventData::new(
                    crate::ipc::event_bus::EVT_RESOLUTION_CHANGED,
                    width, height, 0, 0,
                ),
            );
            0
        }
        _ => u32::MAX,
    }
}

/// sys_list_resolutions - List supported display resolutions.
/// Writes (width, height) pairs as u32 pairs to buf. Returns number of modes.
pub fn sys_list_resolutions(buf_ptr: u32, buf_len: u32) -> u32 {
    let modes = crate::drivers::gpu::with_gpu(|g| {
        let m = g.supported_modes();
        // Copy to a fixed-size buffer to return outside the lock
        let mut result = [(0u32, 0u32); 16];
        let count = m.len().min(16);
        for i in 0..count {
            result[i] = m[i];
        }
        (result, count)
    });

    match modes {
        Some((mode_list, count)) => {
            if buf_ptr != 0 && buf_len > 0 {
                let max_entries = (buf_len as usize / 8).min(count); // 8 bytes per (u32, u32)
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    for i in 0..max_entries {
                        *buf.add(i * 2) = mode_list[i].0;
                        *buf.add(i * 2 + 1) = mode_list[i].1;
                    }
                }
            }
            count as u32
        }
        None => 0, // No GPU driver registered
    }
}

/// sys_gpu_info - Get GPU driver info. Writes driver name to buf. Returns name length.
pub fn sys_gpu_info(buf_ptr: u32, buf_len: u32) -> u32 {
    let name = crate::drivers::gpu::with_gpu(|g| {
        let mut s = alloc::string::String::new();
        s.push_str(g.name());
        s
    });

    match name {
        Some(n) => {
            if buf_ptr != 0 && buf_len > 0 {
                let bytes = n.as_bytes();
                let copy_len = bytes.len().min(buf_len as usize - 1);
                unsafe {
                    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len + 1);
                    buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
                    buf[copy_len] = 0; // null-terminate
                }
            }
            n.len() as u32
        }
        None => 0,
    }
}

// =========================================================================
// Audio
// =========================================================================

/// SYS_AUDIO_WRITE: Write PCM data to audio output.
/// arg1 = pointer to PCM data buffer, arg2 = length in bytes.
/// Returns number of bytes written.
pub fn sys_audio_write(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return 0;
    }
    let data = unsafe {
        core::slice::from_raw_parts(buf_ptr as *const u8, buf_len as usize)
    };
    crate::drivers::audio::write_pcm(data) as u32
}

/// SYS_AUDIO_CTL: Audio control operations.
/// arg1 = command, arg2 = argument.
///   0 = stop playback
///   1 = set volume (arg2 = 0-100)
///   2 = get volume (returns 0-100)
///   3 = get status (returns 1 if playing, 0 if not)
///   4 = is available (returns 1 if audio hw present)
pub fn sys_audio_ctl(cmd: u32, arg: u32) -> u32 {
    match cmd {
        0 => { crate::drivers::audio::stop(); 0 }
        1 => { crate::drivers::audio::set_volume(arg as u8); 0 }
        2 => crate::drivers::audio::get_volume() as u32,
        3 => if crate::drivers::audio::is_playing() { 1 } else { 0 },
        4 => if crate::drivers::audio::is_available() { 1 } else { 0 },
        _ => u32::MAX,
    }
}

// =========================================================================
// Font / AA Drawing / Wallpaper
// =========================================================================

/// SYS_GPU_HAS_ACCEL: Query if GPU acceleration is available.
pub fn sys_gpu_has_accel() -> u32 {
    use core::sync::atomic::Ordering;
    if crate::GPU_ACCEL.load(Ordering::Relaxed) { 1 } else { 0 }
}

// =========================================================================
// Device management (existing)
// =========================================================================

/// sys_devlist - List devices. Each entry is 64 bytes:
///   [0..32]  path (null-terminated)
///   [32..56] driver name (null-terminated, 24 bytes)
///   [56]     driver_type (0=Block,1=Char,2=Network,3=Display,4=Input,5=Audio,6=Output,7=Sensor,8=Bus,9=Unknown)
///   [57..64] padding (zeroed)
pub fn sys_devlist(buf_ptr: u32, buf_size: u32) -> u32 {
    let devices = crate::drivers::hal::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 64usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, (path, name, dtype)) in devices.iter().enumerate().take(max_entries.min(count)) {
            let offset = i * entry_size;
            // Zero the entry first
            for b in &mut buf[offset..offset + entry_size] { *b = 0; }
            // Path [0..32]
            let path_bytes = path.as_bytes();
            let plen = path_bytes.len().min(31);
            buf[offset..offset + plen].copy_from_slice(&path_bytes[..plen]);
            // Driver name [32..56]
            let name_bytes = name.as_bytes();
            let nlen = name_bytes.len().min(23);
            buf[offset + 32..offset + 32 + nlen].copy_from_slice(&name_bytes[..nlen]);
            // Driver type [56]
            buf[offset + 56] = match dtype {
                crate::drivers::hal::DriverType::Block => 0,
                crate::drivers::hal::DriverType::Char => 1,
                crate::drivers::hal::DriverType::Network => 2,
                crate::drivers::hal::DriverType::Display => 3,
                crate::drivers::hal::DriverType::Input => 4,
                crate::drivers::hal::DriverType::Audio => 5,
                crate::drivers::hal::DriverType::Output => 6,
                crate::drivers::hal::DriverType::Sensor => 7,
                crate::drivers::hal::DriverType::Bus => 8,
                crate::drivers::hal::DriverType::Unknown => 9,
            };
        }
    }
    count as u32
}

pub fn sys_devopen(path_ptr: u32, _flags: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let devices = crate::drivers::hal::list_devices();
    if devices.iter().any(|(p, _, _)| p == path) { 0 } else { u32::MAX }
}

// =========================================================================
// Pipes (SYS_PIPE_*)
// =========================================================================

/// sys_pipe_create - Create a new named pipe. arg1=name_ptr (null-terminated).
/// Returns pipe_id (always > 0).
pub fn sys_pipe_create(name_ptr: u32) -> u32 {
    let name = if name_ptr != 0 {
        unsafe { read_user_str(name_ptr) }
    } else {
        "unnamed"
    };
    crate::ipc::pipe::create(name)
}

/// sys_pipe_read - Read from a pipe. Returns bytes read, or u32::MAX if not found.
pub fn sys_pipe_read(pipe_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 || !is_valid_user_ptr(buf_ptr as u64, len as u64) { return 0; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    crate::ipc::pipe::read(pipe_id, buf)
}

/// sys_pipe_close - Destroy a pipe and free its buffer.
pub fn sys_pipe_close(pipe_id: u32) -> u32 {
    crate::ipc::pipe::close(pipe_id);
    0
}

/// sys_pipe_write - Write data to a pipe. Returns bytes written.
pub fn sys_pipe_write(pipe_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 || !is_valid_user_ptr(buf_ptr as u64, len as u64) { return 0; }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    crate::ipc::pipe::write(pipe_id, buf)
}

/// sys_pipe_open - Open an existing pipe by name. Returns pipe_id or 0 if not found.
pub fn sys_pipe_open(name_ptr: u32) -> u32 {
    if name_ptr == 0 { return 0; }
    let name = unsafe { read_user_str(name_ptr) };
    crate::ipc::pipe::open(name)
}

/// sys_pipe_list - List all open pipes. Each entry is 80 bytes:
///   [0..4]   pipe_id (u32 LE)
///   [4..8]   buffered_bytes (u32 LE)
///   [8..72]  name (64 bytes, null-terminated)
///   [72..80] padding (zeroed)
/// Returns total pipe count.
pub fn sys_pipe_list(buf_ptr: u32, buf_size: u32) -> u32 {
    let pipes = crate::ipc::pipe::list();
    let count = pipes.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 80usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, pipe) in pipes.iter().enumerate().take(max_entries.min(count)) {
            let offset = i * entry_size;
            // Zero the entry first
            for b in &mut buf[offset..offset + entry_size] { *b = 0; }
            // pipe_id [0..4]
            buf[offset..offset + 4].copy_from_slice(&pipe.id.to_le_bytes());
            // buffered [4..8]
            buf[offset + 4..offset + 8].copy_from_slice(&(pipe.buffered as u32).to_le_bytes());
            // name [8..72]
            let nlen = pipe.name_len.min(63);
            buf[offset + 8..offset + 8 + nlen].copy_from_slice(&pipe.name[..nlen]);
        }
    }
    count as u32
}

// =========================================================================
// DLL (SYS_DLL_LOAD)
// =========================================================================

/// sys_dll_load - Load/map a DLL into the current process.
/// arg1=path_ptr (null-terminated), arg2=path_len (unused, null-terminated).
/// Returns base virtual address of the DLL, or 0 on failure.
pub fn sys_dll_load(path_ptr: u32, _path_len: u32) -> u32 {
    if path_ptr == 0 { return 0; }
    let path = unsafe { read_user_str(path_ptr) };
    // Try existing loaded DLLs first
    if let Some(base) = crate::task::dll::get_dll_base(path) {
        return base as u32;
    }
    // Try loading from filesystem (dload)
    match crate::task::dll::load_dll_dynamic(path) {
        Some(base) => base as u32,
        None => 0,
    }
}

/// Write a u32 value to a shared DLIB page.
/// arg1 = dll_base_vaddr (lower 32 bits), arg2 = offset, arg3 = value.
/// Used by compositor to write theme field to uisys shared RO pages.
pub fn sys_set_dll_u32(dll_base_lo: u32, offset: u32, value: u32) -> u32 {
    let dll_base = dll_base_lo as u64;
    if crate::task::dll::set_dll_u32(dll_base, offset as u64, value) {
        0
    } else {
        u32::MAX
    }
}

pub fn sys_devclose(_handle: u32) -> u32 { 0 }
pub fn sys_devread(_handle: u32, _buf_ptr: u32, _len: u32) -> u32 { u32::MAX }
pub fn sys_devwrite(_handle: u32, _buf_ptr: u32, _len: u32) -> u32 { u32::MAX }
/// sys_devioctl - Send ioctl to a device by driver type.
/// handle = DriverType as u32 (0=Block,1=Char,2=Network,3=Display,4=Input,5=Audio,6=Output,7=Sensor)
pub fn sys_devioctl(dtype: u32, cmd: u32, arg: u32) -> u32 {
    use crate::drivers::hal::{DriverType, device_ioctl_by_type};
    let driver_type = match dtype {
        0 => DriverType::Block,
        1 => DriverType::Char,
        2 => DriverType::Network,
        3 => DriverType::Display,
        4 => DriverType::Input,
        5 => DriverType::Audio,
        6 => DriverType::Output,
        7 => DriverType::Sensor,
        _ => return u32::MAX,
    };
    match device_ioctl_by_type(driver_type, cmd, arg) {
        Ok(val) => val,
        Err(_) => u32::MAX,
    }
}
pub fn sys_irqwait(_irq: u32) -> u32 { 0 }

// =========================================================================
// Event bus (SYS_EVT_*)
// =========================================================================

use crate::ipc::event_bus::{self, EventData};

/// Subscribe to system events. ebx=filter (0=all). Returns sub_id.
pub fn sys_evt_sys_subscribe(filter: u32) -> u32 {
    event_bus::system_subscribe(filter)
}

/// Poll system event. ebx=sub_id, ecx=buf_ptr (20 bytes). Returns 1 if event, 0 if empty.
pub fn sys_evt_sys_poll(sub_id: u32, buf_ptr: u32) -> u32 {
    if let Some(evt) = event_bus::system_poll(sub_id) {
        if buf_ptr != 0 && is_valid_user_ptr(buf_ptr as u64, 20) {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u32, 5) };
            buf.copy_from_slice(&evt.words);
        }
        1
    } else {
        0
    }
}

/// Unsubscribe from system events. ebx=sub_id.
pub fn sys_evt_sys_unsubscribe(sub_id: u32) -> u32 {
    event_bus::system_unsubscribe(sub_id);
    0
}

/// Create a module channel. ebx=name_ptr, ecx=name_len. Returns channel_id.
pub fn sys_evt_chan_create(name_ptr: u32, name_len: u32) -> u32 {
    let len = (name_len as usize).min(256);
    let name_bytes = unsafe { core::slice::from_raw_parts(name_ptr as *const u8, len) };
    event_bus::channel_create(name_bytes)
}

/// Subscribe to module channel. ebx=chan_id, ecx=filter. Returns sub_id.
pub fn sys_evt_chan_subscribe(chan_id: u32, filter: u32) -> u32 {
    event_bus::channel_subscribe(chan_id, filter)
}

/// Emit to module channel. ebx=chan_id, ecx=event_ptr (20 bytes). Returns 0.
pub fn sys_evt_chan_emit(chan_id: u32, event_ptr: u32) -> u32 {
    if event_ptr == 0 { return u32::MAX; }
    let words = unsafe { core::slice::from_raw_parts(event_ptr as *const u32, 5) };
    let evt = EventData { words: [words[0], words[1], words[2], words[3], words[4]] };
    event_bus::channel_emit(chan_id, evt);
    0
}

/// Poll module channel. ebx=chan_id, ecx=sub_id, edx=buf_ptr. Returns 1/0.
pub fn sys_evt_chan_poll(chan_id: u32, sub_id: u32, buf_ptr: u32) -> u32 {
    if let Some(evt) = event_bus::channel_poll(chan_id, sub_id) {
        if buf_ptr != 0 && is_valid_user_ptr(buf_ptr as u64, 20) {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u32, 5) };
            buf.copy_from_slice(&evt.words);
        }
        1
    } else {
        0
    }
}

/// Unsubscribe from module channel. ebx=chan_id, ecx=sub_id.
pub fn sys_evt_chan_unsubscribe(chan_id: u32, sub_id: u32) -> u32 {
    event_bus::channel_unsubscribe(chan_id, sub_id);
    0
}

/// Destroy a module channel. ebx=chan_id.
pub fn sys_evt_chan_destroy(chan_id: u32) -> u32 {
    event_bus::channel_destroy(chan_id);
    0
}

/// Emit to a specific subscriber (unicast). ebx=chan_id, r10=sub_id, rdx=event_ptr.
pub fn sys_evt_chan_emit_to(chan_id: u32, sub_id: u32, event_ptr: u32) -> u32 {
    if event_ptr == 0 { return u32::MAX; }
    let words = unsafe { core::slice::from_raw_parts(event_ptr as *const u32, 5) };
    let evt = EventData { words: [words[0], words[1], words[2], words[3], words[4]] };
    event_bus::channel_emit_to(chan_id, sub_id, evt);
    0
}

// =========================================================================
// Shared memory (SYS_SHM_CREATE, SYS_SHM_MAP, SYS_SHM_UNMAP, SYS_SHM_DESTROY)
// =========================================================================

/// Create a shared memory region. ebx=size (bytes, rounded up to page).
/// Returns shm_id (>0) on success, 0 on failure.
pub fn sys_shm_create(size: u32) -> u32 {
    let tid = crate::task::scheduler::current_tid();
    match crate::ipc::shared_memory::create(size as usize, tid) {
        Some(id) => id,
        None => 0,
    }
}

/// Map a shared memory region into the caller's address space.
/// ebx=shm_id. Returns virtual address (u32) or 0 on failure.
pub fn sys_shm_map(shm_id: u32) -> u32 {
    crate::ipc::shared_memory::map_into_current(shm_id) as u32
}

/// Unmap a shared memory region from the caller's address space.
/// ebx=shm_id. Returns 0 on success, u32::MAX on failure.
pub fn sys_shm_unmap(shm_id: u32) -> u32 {
    if crate::ipc::shared_memory::unmap_from_current(shm_id) {
        0
    } else {
        u32::MAX
    }
}

/// Destroy a shared memory region (owner only).
/// ebx=shm_id. Returns 0 on success, u32::MAX on failure.
pub fn sys_shm_destroy(shm_id: u32) -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if crate::ipc::shared_memory::destroy(shm_id, tid) {
        0
    } else {
        u32::MAX
    }
}

// =========================================================================
// Compositor-privileged syscalls
// =========================================================================

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// TID of the registered compositor process. 0 = none registered.
static COMPOSITOR_TID: AtomicU32 = AtomicU32::new(0);

/// Page directory (CR3) of the registered compositor. 0 = none.
/// Used to identify compositor child threads (render thread etc.)
/// that share the same address space.
static COMPOSITOR_PD: AtomicU64 = AtomicU64::new(0);

/// Check if the current thread belongs to the compositor process.
/// Returns true if the calling thread is the compositor's management thread
/// OR any child thread sharing the same page directory (e.g. render thread).
///
/// Lock-free: reads CR3 directly instead of acquiring the SCHEDULER lock.
/// This is critical because the render thread calls GPU commands at 60Hz
/// and each call checks is_compositor() — lock contention would be severe.
fn is_compositor() -> bool {
    let comp_pd = COMPOSITOR_PD.load(Ordering::Relaxed);
    if comp_pd == 0 {
        return false;
    }
    let current_cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
    }
    // CR3 bits [12..] are the physical page directory address; mask off flags in low 12 bits
    (current_cr3 & !0xFFF) == comp_pd
}

/// Register calling process as the compositor. First caller wins.
/// Returns 0 on success, u32::MAX if already registered.
pub fn sys_register_compositor() -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if COMPOSITOR_TID.compare_exchange(0, tid, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        // Store the compositor's page directory so child threads (render thread)
        // are also recognized as compositor by is_compositor().
        if let Some(pd) = crate::task::scheduler::current_thread_page_directory() {
            COMPOSITOR_PD.store(pd.as_u64(), Ordering::SeqCst);
        }

        // Boost compositor to realtime priority so UI never stutters
        crate::task::scheduler::set_thread_priority(tid, 127);
        crate::serial_println!("[OK] Compositor registered (TID={}, priority=127)", tid);
        0
    } else {
        u32::MAX // Already registered
    }
}

/// Take over cursor from kernel splash mode. Compositor-only.
/// Disables the kernel's IRQ-driven cursor tracking, drains stale mouse events,
/// and returns the splash cursor position packed as (x << 16) | (y & 0xFFFF).
/// The compositor uses this to initialize its logical cursor to match the HW cursor.
pub fn sys_cursor_takeover() -> u32 {
    if !is_compositor() {
        return 0;
    }
    let (x, y) = crate::drivers::gpu::splash_cursor_position();
    crate::drivers::gpu::disable_splash_cursor();
    crate::drivers::input::mouse::clear_buffer();
    crate::serial_println!("Compositor cursor takeover: splash pos ({}, {})", x, y);
    ((x as u16 as u32) << 16) | (y as u16 as u32)
}

/// Map the GPU framebuffer into the compositor's address space.
/// ebx=out_info_ptr (pointer to FbMapInfo struct, 16 bytes).
/// Returns 0 on success, u32::MAX on failure.
///
/// FbMapInfo layout: { fb_vaddr: u32, width: u32, height: u32, pitch: u32 }
pub fn sys_map_framebuffer(out_info_ptr: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    // Get framebuffer info from GPU driver
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return u32::MAX,
    };

    // Map 16 MiB of VRAM into the compositor's address space at 0x20000000
    // (covers all resolutions up to 1920x1080 double-buffered)
    let fb_user_base: u64 = 0x2000_0000;
    let fb_map_size: usize = 16 * 1024 * 1024;
    let pages = fb_map_size / crate::memory::FRAME_SIZE;

    for i in 0..pages {
        let phys_addr = crate::memory::address::PhysAddr::new(
            fb_phys as u64 + (i * crate::memory::FRAME_SIZE) as u64,
        );
        let virt_addr = crate::memory::address::VirtAddr::new(
            fb_user_base + (i * crate::memory::FRAME_SIZE) as u64,
        );
        // Present + Writable + User + Write-Through (0x0F)
        crate::memory::virtual_mem::map_page(virt_addr, phys_addr, 0x0F);
    }

    // Write FbMapInfo struct to user memory
    if out_info_ptr != 0 {
        let info = unsafe { &mut *(out_info_ptr as *mut [u32; 4]) };
        info[0] = fb_user_base as u32;
        info[1] = width;
        info[2] = height;
        info[3] = pitch;
    }

    crate::serial_println!(
        "[OK] Framebuffer mapped to compositor at {:#010x} ({}x{}, pitch={}, phys={:#x})",
        fb_user_base, width, height, pitch, fb_phys
    );
    0
}

/// Submit GPU acceleration commands from the compositor.
/// ebx=cmd_buf_ptr, ecx=cmd_count.
/// Returns number of commands executed, or u32::MAX on error.
///
/// Each command is 36 bytes: { cmd_type: u32, args: [u32; 8] }
/// Command types: 1=UPDATE, 2=FILL_RECT, 3=COPY_RECT, 4=CURSOR_MOVE,
///                5=CURSOR_SHOW, 6=DEFINE_CURSOR, 7=FLIP
pub fn sys_gpu_command(cmd_buf_ptr: u32, cmd_count: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if cmd_count == 0 || cmd_buf_ptr == 0 {
        return 0;
    }

    let count = cmd_count.min(256) as usize; // Cap at 256 commands per call
    let byte_size = count * 36; // 9 u32s * 4 bytes each
    if !is_valid_user_ptr(cmd_buf_ptr as u64, byte_size as u64) {
        return 0;
    }
    let cmds = unsafe {
        core::slice::from_raw_parts(cmd_buf_ptr as *const [u32; 9], count)
    };

    let mut executed = 0u32;
    for cmd in cmds {
        let cmd_type = cmd[0];
        let ok = crate::drivers::gpu::with_gpu(|g| {
            match cmd_type {
                1 => { // UPDATE(x, y, w, h)
                    g.update_rect(cmd[1], cmd[2], cmd[3], cmd[4]);
                    true
                }
                2 => { // FILL_RECT(x, y, w, h, color)
                    g.accel_fill_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5])
                }
                3 => { // COPY_RECT(sx, sy, dx, dy, w, h)
                    g.accel_copy_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5], cmd[6])
                }
                4 => { // CURSOR_MOVE(x, y)
                    // When kernel-side cursor tracking is active (IRQ-driven),
                    // skip compositor CURSOR_MOVE to avoid dual-tracking jitter.
                    if !crate::drivers::gpu::is_splash_cursor_active() {
                        g.move_cursor(cmd[1], cmd[2]);
                    }
                    true
                }
                5 => { // CURSOR_SHOW(visible)
                    g.show_cursor(cmd[1] != 0);
                    true
                }
                6 => { // DEFINE_CURSOR(w, h, hotx, hoty, pixels_ptr_lo, pixels_ptr_hi, pixel_count)
                    let w = cmd[1];
                    let h = cmd[2];
                    let hotx = cmd[3];
                    let hoty = cmd[4];
                    let ptr = (cmd[5] as u64) | ((cmd[6] as u64) << 32);
                    let count = cmd[7] as usize;
                    if w == 0 || h == 0 || count == 0 || ptr == 0 {
                        false
                    } else if count != (w * h) as usize {
                        false
                    } else {
                        let pixels = unsafe {
                            core::slice::from_raw_parts(ptr as *const u32, count)
                        };
                        g.define_cursor(w, h, hotx, hoty, pixels);
                        true
                    }
                }
                7 => { // FLIP
                    g.flip();
                    true
                }
                _ => false,
            }
        });
        if ok == Some(true) {
            executed += 1;
        }
    }

    executed
}

/// Poll raw input events for the compositor.
/// ebx=buf_ptr (array of RawInputEvent), ecx=max_events.
/// Returns number of events written.
///
/// RawInputEvent layout (20 bytes): { event_type: u32, arg0-arg3: u32 }
/// Event types:
///   1 = KEY_DOWN:     arg0=scancode, arg1=char_value, arg2=modifiers
///   2 = KEY_UP:       arg0=scancode, arg1=char_value, arg2=modifiers
///   3 = MOUSE_MOVE:   arg0=dx(i32), arg1=dy(i32)
///   4 = MOUSE_BUTTON: arg0=buttons, arg1=1(down)/0(up)
///   5 = MOUSE_SCROLL: arg0=dz(i32)
pub fn sys_input_poll(buf_ptr: u32, max_events: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if buf_ptr == 0 || max_events == 0 {
        return 0;
    }

    let max = max_events.min(256) as usize;
    let byte_size = max * 20; // 5 u32s * 4 bytes each
    if !is_valid_user_ptr(buf_ptr as u64, byte_size as u64) {
        return 0;
    }
    let events = unsafe {
        core::slice::from_raw_parts_mut(buf_ptr as *mut [u32; 5], max)
    };
    let mut count = 0usize;

    // Drain keyboard events
    while count < max {
        match crate::drivers::input::keyboard::read_event() {
            Some(key_evt) => {
                let event_type: u32 = if key_evt.pressed { 1 } else { 2 };
                let char_val = match key_evt.key {
                    crate::drivers::input::keyboard::Key::Char(c) => c as u32,
                    crate::drivers::input::keyboard::Key::Enter => 0x0D,
                    crate::drivers::input::keyboard::Key::Backspace => 0x08,
                    crate::drivers::input::keyboard::Key::Tab => 0x09,
                    crate::drivers::input::keyboard::Key::Escape => 0x1B,
                    crate::drivers::input::keyboard::Key::Space => 0x20,
                    crate::drivers::input::keyboard::Key::Delete => 0x7F,
                    _ => 0,
                };
                let mods = (key_evt.modifiers.shift as u32)
                    | ((key_evt.modifiers.ctrl as u32) << 1)
                    | ((key_evt.modifiers.alt as u32) << 2)
                    | ((key_evt.modifiers.caps_lock as u32) << 3);

                events[count] = [event_type, key_evt.scancode as u32, char_val, mods, 0];
                count += 1;
            }
            None => break,
        }
    }

    // Drain mouse events
    while count < max {
        match crate::drivers::input::mouse::read_event() {
            Some(mouse_evt) => {
                use crate::drivers::input::mouse::MouseEventType;
                let (event_type, arg0, arg1, arg2, arg3) = match mouse_evt.event_type {
                    MouseEventType::Move => {
                        (3u32, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, 0)
                    }
                    MouseEventType::ButtonDown => {
                        let btns = (mouse_evt.buttons.left as u32)
                            | ((mouse_evt.buttons.right as u32) << 1)
                            | ((mouse_evt.buttons.middle as u32) << 2);
                        (4, btns, 1, mouse_evt.dx as u32, mouse_evt.dy as u32)
                    }
                    MouseEventType::ButtonUp => {
                        let btns = (mouse_evt.buttons.left as u32)
                            | ((mouse_evt.buttons.right as u32) << 1)
                            | ((mouse_evt.buttons.middle as u32) << 2);
                        (4, btns, 0, mouse_evt.dx as u32, mouse_evt.dy as u32)
                    }
                    MouseEventType::Scroll => {
                        (5, mouse_evt.dz as u32, 0, 0, 0)
                    }
                };
                events[count] = [event_type, arg0, arg1, arg2, arg3];
                count += 1;
            }
            None => break,
        }
    }

    count as u32
}

/// SYS_THREAD_CREATE: Create a new thread in the current process.
/// arg1 = entry_rip, arg2 = user_rsp, arg3 = name_ptr, arg4 = name_len, arg5 = priority (0=inherit)
/// Returns TID of new thread, or 0 on error.
pub fn sys_thread_create(entry_rip: u32, user_rsp: u32, name_ptr: u32, name_len: u32, priority: u32) -> u32 {
    let entry = entry_rip as u64;
    let rsp = user_rsp as u64;

    // Basic validation: entry must be in user space, rsp must be in user space and aligned
    if entry == 0 || entry >= 0x0000_8000_0000_0000 {
        return 0;
    }
    if rsp == 0 || rsp >= 0x0000_8000_0000_0000 || rsp & 7 != 0 {
        return 0;
    }

    // Read thread name from user space (max 31 chars)
    let mut name_buf = [0u8; 32];
    let len = (name_len as usize).min(31);
    if name_ptr != 0 && len > 0 {
        let src = name_ptr as *const u8;
        // Validate pointer is in user space
        if (src as u64) < 0x0000_8000_0000_0000 {
            unsafe {
                core::ptr::copy_nonoverlapping(src, name_buf.as_mut_ptr(), len);
            }
        }
    }
    let name = core::str::from_utf8(&name_buf[..len]).unwrap_or("thread");

    // Priority: 0 means inherit from parent (handled by scheduler), 1-255 = explicit
    let pri = if priority > 0 && priority <= 255 { priority as u8 } else { 0 };

    let tid = crate::task::scheduler::create_thread_in_current_process(entry, rsp, name, pri);
    crate::debug_println!("sys_thread_create: entry={:#x} rsp={:#x} name={} pri={} -> TID={}", entry, rsp, name, pri, tid);
    tid
}

/// SYS_SET_PRIORITY: Change the priority of a thread.
/// arg1 = tid (0 = self), arg2 = new priority (1-255)
/// Returns 0 on success, u32::MAX on error.
pub fn sys_set_priority(tid: u32, priority: u32) -> u32 {
    if priority == 0 || priority > 255 {
        return u32::MAX;
    }
    let target_tid = if tid == 0 {
        crate::task::scheduler::current_tid()
    } else {
        tid
    };
    if target_tid == 0 {
        return u32::MAX;
    }
    crate::task::scheduler::set_thread_priority(target_tid, priority as u8);
    0
}

/// SYS_SET_CRITICAL: Mark the calling thread as critical (won't be killed by RSP recovery).
pub fn sys_set_critical() -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if tid == 0 {
        return u32::MAX;
    }
    crate::task::scheduler::set_thread_critical(tid);
    0
}

/// SYS_CAPTURE_SCREEN: Capture the current framebuffer contents to a user buffer.
/// arg1 = buf_ptr (pointer to u32 ARGB pixels)
/// arg2 = buf_size (buffer size in bytes)
/// arg3 = info_ptr (pointer to write [width: u32, height: u32])
/// Returns: 0 on success, 1 = no GPU, 2 = buffer too small.
pub fn sys_capture_screen(buf_ptr: u32, buf_size: u32, info_ptr: u32) -> u32 {
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return 1,
    };

    let needed = width * height * 4;
    if buf_size < needed {
        return 2;
    }

    // Write dimensions to info struct
    if info_ptr != 0 {
        unsafe {
            let info = info_ptr as *mut u32;
            *info = width;
            *info.add(1) = height;
        }
    }

    // Map framebuffer physical pages into the current process at 0x30000000
    // (read-only user access: PAGE_PRESENT | PAGE_USER)
    let fb_map_base: u64 = 0x3000_0000;
    let fb_total_bytes = height as usize * pitch as usize;
    let fb_pages = (fb_total_bytes + 0xFFF) / 0x1000;

    for i in 0..fb_pages {
        let phys = crate::memory::address::PhysAddr::new(
            fb_phys as u64 + (i * 0x1000) as u64,
        );
        let virt = crate::memory::address::VirtAddr::new(
            fb_map_base + (i * 0x1000) as u64,
        );
        crate::memory::virtual_mem::map_page(virt, phys, 0x05);
    }

    // Copy pixels row by row (pitch may differ from width*4)
    unsafe {
        let src = fb_map_base as *const u8;
        let dst = buf_ptr as *mut u8;
        for y in 0..height as usize {
            let src_row = src.add(y * pitch as usize);
            let dst_row = dst.add(y * width as usize * 4);
            core::ptr::copy_nonoverlapping(src_row, dst_row, width as usize * 4);
        }
    }

    0
}

// =========================================================================
// Environment Variables (SYS_SETENV, SYS_GETENV, SYS_LISTENV)
// =========================================================================

/// sys_setenv - Set an environment variable.
/// arg1 = key_ptr (null-terminated), arg2 = val_ptr (null-terminated, or 0 to unset).
/// Returns 0 on success.
pub fn sys_setenv(key_ptr: u32, val_ptr: u32) -> u32 {
    if key_ptr == 0 { return u32::MAX; }
    let key = unsafe { read_user_str(key_ptr) };
    if key.is_empty() { return u32::MAX; }

    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return u32::MAX,
    };

    if val_ptr == 0 {
        crate::task::env::unset(pd, key);
    } else {
        let val = unsafe { read_user_str(val_ptr) };
        crate::task::env::set(pd, key, val);
    }
    0
}

/// sys_getenv - Get an environment variable.
/// arg1 = key_ptr (null-terminated), arg2 = val_buf_ptr, arg3 = val_buf_size.
/// Returns length of value (bytes written, excluding null terminator), or u32::MAX if not found.
pub fn sys_getenv(key_ptr: u32, val_buf_ptr: u32, val_buf_size: u32) -> u32 {
    if key_ptr == 0 { return u32::MAX; }
    let key = unsafe { read_user_str(key_ptr) };
    if key.is_empty() { return u32::MAX; }

    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return u32::MAX,
    };

    match crate::task::env::get(pd, key) {
        Some(val) => {
            let val_bytes = val.as_bytes();
            let copy_len = val_bytes.len().min(val_buf_size as usize);
            if val_buf_ptr != 0 && val_buf_size > 0
                && is_valid_user_ptr(val_buf_ptr as u64, val_buf_size as u64)
            {
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(val_buf_ptr as *mut u8, val_buf_size as usize)
                };
                buf[..copy_len].copy_from_slice(&val_bytes[..copy_len]);
                if copy_len < val_buf_size as usize {
                    buf[copy_len] = 0;
                }
            }
            val_bytes.len() as u32
        }
        None => u32::MAX,
    }
}

/// sys_listenv - List all environment variables.
/// arg1 = buf_ptr, arg2 = buf_size.
/// Format: "KEY=VALUE\0KEY2=VALUE2\0..." packed entries.
/// Returns total bytes needed (may exceed buf_size).
pub fn sys_listenv(buf_ptr: u32, buf_size: u32) -> u32 {
    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return 0,
    };

    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        // Just return the needed size
        let mut dummy = [0u8; 0];
        return crate::task::env::list(pd, &mut dummy) as u32;
    }

    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    crate::task::env::list(pd, buf) as u32
}
