// =============================================================================
// Syscall Handlers — complete implementation for .anyOS
// =============================================================================

use alloc::string::String;

// =========================================================================
// Helpers
// =========================================================================

/// Read a null-terminated string from user memory (max 256 bytes).
unsafe fn read_user_str(ptr: u32) -> &'static str {
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
    crate::serial_println!("sys_exit({})", status);

    // Destroy windows owned by this thread
    let tid = crate::task::scheduler::current_tid();
    crate::ui::desktop::with_desktop(|desktop| {
        desktop.close_windows_by_owner(tid);
    });

    if let Some(pd_phys) = crate::task::scheduler::current_thread_page_directory() {
        unsafe {
            let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3);
        }
        crate::memory::virtual_mem::destroy_user_page_directory(pd_phys);
    }

    crate::task::scheduler::exit_current(status);
    0 // unreachable
}

/// sys_kill - Kill a thread by TID
pub fn sys_kill(tid: u32) -> u32 {
    if tid == 0 { return u32::MAX; }
    crate::serial_println!("sys_kill({})", tid);

    // Destroy windows owned by this thread BEFORE killing it
    crate::ui::desktop::with_desktop(|desktop| {
        desktop.close_windows_by_owner(tid);
    });

    crate::task::scheduler::kill_thread(tid)
}

/// sys_write - Write to a file descriptor
/// fd=1 -> stdout (pipe if configured, else serial), fd=2 -> stderr (same), fd>=3 -> VFS file
pub fn sys_write(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if fd == 1 || fd == 2 {
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
        if pipe_id != 0 {
            // Redirect to pipe AND serial (serial for kernel debug visibility)
            crate::ipc::pipe::write(pipe_id, buf);
        }
        // Always write to serial as well (for debug)
        for &byte in buf {
            crate::drivers::serial::write_byte(byte);
        }
        len
    } else if fd >= 3 {
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        match crate::fs::vfs::write(fd, buf) {
            Ok(n) => n as u32,
            Err(_) => u32::MAX,
        }
    } else {
        u32::MAX
    }
}

/// sys_read - Read from a file descriptor
/// fd=0 -> stdin (not yet implemented), fd>=3 -> VFS file
pub fn sys_read(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if fd == 0 {
        0 // stdin: not yet implemented
    } else if fd >= 3 {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
        match crate::fs::vfs::read(fd, buf) {
            Ok(n) => n as u32,
            Err(_) => u32::MAX,
        }
    } else {
        u32::MAX
    }
}

/// sys_open - Open a file. arg1=path_ptr (null-terminated), arg2=flags, arg3=unused
/// Returns file descriptor or u32::MAX on error.
pub fn sys_open(path_ptr: u32, flags: u32, _arg3: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let file_flags = crate::fs::file::FileFlags {
        read: true,
        write: (flags & 1) != 0,
        append: (flags & 2) != 0,
        create: (flags & 4) != 0,
        truncate: (flags & 8) != 0,
    };
    match crate::fs::vfs::open(path, file_flags) {
        Ok(fd) => fd,
        Err(_) => u32::MAX,
    }
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

/// sys_sleep - Sleep for N milliseconds (busy-wait with yield)
pub fn sys_sleep(ms: u32) -> u32 {
    if ms == 0 {
        return 0;
    }
    let ticks = ms / 10;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let start = crate::arch::x86::pit::get_ticks();
    while crate::arch::x86::pit::get_ticks().wrapping_sub(start) < ticks {
        crate::task::scheduler::schedule();
    }
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
        while addr < new_page_end {
            if let Some(phys) = physical::alloc_frame() {
                virtual_mem::map_page(VirtAddr::new(addr), phys, 0x02 | 0x04);
                unsafe { core::ptr::write_bytes(addr as *mut u8, 0, page_size as usize); }
            } else {
                return u32::MAX;
            }
            addr += page_size;
        }
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

/// sys_spawn - Spawn a new process from a filesystem path.
/// arg1=path_ptr, arg2=stdout_pipe_id (0=none), arg3=args_ptr (0=none), arg4=unused
/// Returns TID or u32::MAX on error.
pub fn sys_spawn(path_ptr: u32, stdout_pipe: u32, args_ptr: u32, _arg4: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let args = if args_ptr != 0 {
        unsafe { read_user_str(args_ptr) }
    } else {
        ""
    };
    crate::serial_println!("sys_spawn: path='{}' pipe={} args_ptr={:#x}", path, stdout_pipe, args_ptr);
    let name = path.rsplit('/').next().unwrap_or(path);
    match crate::task::loader::load_and_run_with_args(path, name, args) {
        Ok(tid) => {
            if stdout_pipe != 0 {
                crate::task::scheduler::set_thread_stdout_pipe(tid, stdout_pipe);
            }
            crate::serial_println!("sys_spawn: returning TID={}", tid);
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
    if buf_ptr == 0 || buf_size == 0 {
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
    let path = unsafe { read_user_str(path_ptr) };

    match crate::fs::vfs::read_dir(path) {
        Ok(entries) => {
            let entry_size = 64u32;
            if buf_ptr != 0 && buf_size > 0 {
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
    let path = unsafe { read_user_str(path_ptr) };

    // Check directory first
    if let Ok(entries) = crate::fs::vfs::read_dir(path) {
        if buf_ptr != 0 {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = 1; // directory
                *buf.add(1) = entries.len() as u32;
            }
        }
        return 0;
    }

    // Try as a file
    if let Ok(data) = crate::fs::vfs::read_file_to_vec(path) {
        if buf_ptr != 0 {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = 0; // regular file
                *buf.add(1) = data.len() as u32;
            }
        }
        return 0;
    }

    u32::MAX
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

/// sys_uptime - Get system uptime in PIT ticks (100 Hz).
pub fn sys_uptime() -> u32 {
    crate::arch::x86::pit::get_ticks()
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
            // Thread list: 36 bytes each
            // [tid:u32, prio:u8, state:u8, pad:u16, name:24bytes, cpu_ticks:u32]
            let threads = crate::task::scheduler::list_threads();
            if buf_ptr != 0 && buf_size > 0 {
                let entry_size = 36usize;
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
                    buf[off + 6] = 0;
                    buf[off + 7] = 0;
                    let name_bytes = t.name.as_bytes();
                    let n = name_bytes.len().min(23);
                    buf[off + 8..off + 8 + n].copy_from_slice(&name_bytes[..n]);
                    buf[off + 8 + n] = 0;
                    // cpu_ticks at offset 32
                    buf[off + 32..off + 36].copy_from_slice(&t.cpu_ticks.to_le_bytes());
                }
            }
            threads.len() as u32
        }
        2 => crate::arch::x86::smp::cpu_count() as u32,
        3 => {
            // CPU load: [cpu_pct:u32, uptime_ticks:u32] = 8 bytes
            if buf_ptr != 0 && buf_size >= 8 {
                let total = crate::task::scheduler::total_sched_ticks();
                let idle = crate::task::scheduler::idle_sched_ticks();
                let pct = if total > 0 {
                    100u32.saturating_sub(idle.saturating_mul(100) / total)
                } else {
                    0
                };
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = pct;
                    *buf.add(1) = crate::arch::x86::pit::get_ticks();
                }
            }
            0
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
    if buf_ptr == 0 { return u32::MAX; }

    match cmd {
        0 => {
            let cfg = crate::net::config();
            let link_up = crate::drivers::e1000::is_link_up();
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

// =========================================================================
// Window Manager (SYS_WIN_*)
// =========================================================================

/// sys_win_create - Create a new window.
/// arg1=title_ptr, arg2=x|(y<<16), arg3=w|(h<<16), arg4=flags, arg5=0
/// flags: bit 0 = non-resizable
/// Returns window_id or u32::MAX.
pub fn sys_win_create(title_ptr: u32, pos_packed: u32, size_packed: u32, flags: u32, _a5: u32) -> u32 {
    let title = if title_ptr != 0 { unsafe { read_user_str(title_ptr) } } else { "Window" };
    let x = (pos_packed & 0xFFFF) as i32;
    let y = ((pos_packed >> 16) & 0xFFFF) as i32;
    let w = size_packed & 0xFFFF;
    let h = (size_packed >> 16) & 0xFFFF;
    let w = if w == 0 { 400 } else { w };
    let h = if h == 0 { 300 } else { h };

    let owner_tid = crate::task::scheduler::current_tid();
    match crate::ui::desktop::with_desktop(|desktop| {
        desktop.create_window_with_owner(title, x, y, w, h, flags, owner_tid)
    }) {
        Some(id) => id,
        None => u32::MAX,
    }
}

/// sys_win_destroy - Close/destroy a window.
pub fn sys_win_destroy(window_id: u32) -> u32 {
    crate::ui::desktop::with_desktop(|desktop| desktop.close_window(window_id));
    0
}

/// sys_win_set_title - Set window title.
pub fn sys_win_set_title(window_id: u32, title_ptr: u32, _len: u32) -> u32 {
    if title_ptr == 0 { return u32::MAX; }
    let title = unsafe { read_user_str(title_ptr) };
    crate::ui::desktop::with_desktop(|desktop| {
        if let Some(w) = desktop.window_content(window_id) {
            w.title = String::from(title);
            w.mark_dirty();
        }
    });
    0
}

/// sys_win_get_event - Poll for a window event.
/// arg1=window_id, arg2=event_buf_ptr (20 bytes: [type:u32, p1-p4:u32])
/// Returns 1 if event, 0 if none.
pub fn sys_win_get_event(window_id: u32, buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
    match crate::ui::desktop::with_desktop(|desktop| {
        desktop.poll_user_event(window_id)
    }) {
        Some(Some(event)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                for i in 0..5 { *buf.add(i) = event[i]; }
            }
            1
        }
        _ => 0,
    }
}

/// sys_win_fill_rect - Fill rectangle in window content.
/// arg1=window_id, arg2=params_ptr: [x:i16, y:i16, w:u16, h:u16, color:u32] = 12 bytes
pub fn sys_win_fill_rect(window_id: u32, params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let (x, y, w, h, color) = unsafe {
        let p = params_ptr as *const u8;
        (
            i16::from_le_bytes([*p, *p.add(1)]) as i32,
            i16::from_le_bytes([*p.add(2), *p.add(3)]) as i32,
            u16::from_le_bytes([*p.add(4), *p.add(5)]) as u32,
            u16::from_le_bytes([*p.add(6), *p.add(7)]) as u32,
            u32::from_le_bytes([*p.add(8), *p.add(9), *p.add(10), *p.add(11)]),
        )
    };
    crate::ui::desktop::with_desktop(|desktop| {
        if let Some(window) = desktop.window_content(window_id) {
            let rect = crate::graphics::rect::Rect::new(x, y, w, h);
            window.content.fill_rect(rect, crate::graphics::color::Color::from_u32(color));
            window.mark_dirty();
        }
    });
    0
}

/// sys_win_draw_text - Draw text in window content.
/// arg1=window_id, arg2=params_ptr: [x:i16, y:i16, color:u32, text_ptr:u32] = 12 bytes
pub fn sys_win_draw_text(window_id: u32, params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let (x, y, color, text) = unsafe {
        let p = params_ptr as *const u8;
        let x = i16::from_le_bytes([*p, *p.add(1)]) as i32;
        let y = i16::from_le_bytes([*p.add(2), *p.add(3)]) as i32;
        let color = u32::from_le_bytes([*p.add(4), *p.add(5), *p.add(6), *p.add(7)]);
        let text_ptr = u32::from_le_bytes([*p.add(8), *p.add(9), *p.add(10), *p.add(11)]);
        (x, y, color, read_user_str(text_ptr))
    };
    crate::ui::desktop::with_desktop(|desktop| {
        if let Some(window) = desktop.window_content(window_id) {
            crate::graphics::font::draw_string(
                &mut window.content, x, y, text,
                crate::graphics::color::Color::from_u32(color),
            );
            window.mark_dirty();
        }
    });
    0
}

/// sys_win_draw_text_mono - Draw text using the monospace bitmap font (8x16).
/// Same parameter format as sys_win_draw_text.
pub fn sys_win_draw_text_mono(window_id: u32, params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let (x, y, color, text) = unsafe {
        let p = params_ptr as *const u8;
        let x = i16::from_le_bytes([*p, *p.add(1)]) as i32;
        let y = i16::from_le_bytes([*p.add(2), *p.add(3)]) as i32;
        let color = u32::from_le_bytes([*p.add(4), *p.add(5), *p.add(6), *p.add(7)]);
        let text_ptr = u32::from_le_bytes([*p.add(8), *p.add(9), *p.add(10), *p.add(11)]);
        (x, y, color, read_user_str(text_ptr))
    };
    crate::ui::desktop::with_desktop(|desktop| {
        if let Some(window) = desktop.window_content(window_id) {
            crate::graphics::font::draw_string_bitmap(
                &mut window.content, x, y, text,
                crate::graphics::color::Color::from_u32(color),
            );
            window.mark_dirty();
        }
    });
    0
}

/// sys_win_present - Flush window to compositor.
pub fn sys_win_present(window_id: u32) -> u32 {
    crate::ui::desktop::with_desktop(|desktop| {
        desktop.render_window(window_id);
    });
    0
}

/// sys_win_get_size - Get window content size. arg2=buf_ptr: [w:u32, h:u32]
pub fn sys_win_get_size(window_id: u32, buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
    match crate::ui::desktop::with_desktop(|desktop| {
        desktop.window_content(window_id).map(|w| (w.width, w.height))
    }) {
        Some(Some((w, h))) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = w;
                *buf.add(1) = h;
            }
            0
        }
        _ => u32::MAX,
    }
}

/// sys_win_blit - Blit ARGB pixel data (u32 per pixel, 0xAARRGGBB) to window content surface.
/// params_ptr: [x:i16, y:i16, w:u16, h:u16, data_ptr:u32] = 12 bytes
pub fn sys_win_blit(window_id: u32, params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let params = unsafe { core::slice::from_raw_parts(params_ptr as *const u8, 12) };
    let x = i16::from_le_bytes([params[0], params[1]]) as i32;
    let y = i16::from_le_bytes([params[2], params[3]]) as i32;
    let w = u16::from_le_bytes([params[4], params[5]]) as u32;
    let h = u16::from_le_bytes([params[6], params[7]]) as u32;
    let data_ptr = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);

    if data_ptr == 0 || w == 0 || h == 0 { return u32::MAX; }
    let pixel_count = (w * h) as usize;
    let src = unsafe { core::slice::from_raw_parts(data_ptr as *const u32, pixel_count) };

    crate::ui::desktop::with_desktop(|desktop| {
        if let Some(win) = desktop.window_content(window_id) {
            let surface = &mut win.content;
            for row in 0..h as i32 {
                let sy = y + row;
                if sy < 0 || sy >= surface.height as i32 { continue; }
                for col in 0..w as i32 {
                    let sx = x + col;
                    if sx < 0 || sx >= surface.width as i32 { continue; }
                    let pixel = src[(row as u32 * w + col as u32) as usize];
                    let a = (pixel >> 24) & 0xFF;
                    if a == 0 { continue; }
                    let dst_off = (sy as u32 * surface.width + sx as u32) as usize;
                    if a >= 255 {
                        surface.pixels[dst_off] = pixel;
                    } else {
                        let r = (pixel >> 16) & 0xFF;
                        let g = (pixel >> 8) & 0xFF;
                        let b = pixel & 0xFF;
                        let dst = surface.pixels[dst_off];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - a;
                        let or = (r * a + dr * inv) / 255;
                        let og = (g * a + dg * inv) / 255;
                        let ob = (b * a + db * inv) / 255;
                        surface.pixels[dst_off] = (0xFF << 24) | (or << 16) | (og << 8) | ob;
                    }
                }
            }
            win.mark_dirty();
        }
    });
    0
}

/// sys_win_list - List open windows. buf_ptr: array of 64-byte entries.
/// Each entry: [id:u32, title_len:u32, title:56bytes]
/// Returns number of windows.
pub fn sys_win_list(buf_ptr: u32, max_entries: u32) -> u32 {
    match crate::ui::desktop::with_desktop(|desktop| {
        let count = desktop.windows.len();
        if buf_ptr != 0 && max_entries > 0 {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, (max_entries as usize) * 64) };
            for (i, w) in desktop.windows.iter().enumerate().take(max_entries as usize) {
                let off = i * 64;
                // id
                buf[off..off + 4].copy_from_slice(&w.id.to_le_bytes());
                // title_len
                let title_bytes = w.title.as_bytes();
                let tlen = title_bytes.len().min(56);
                buf[off + 4..off + 8].copy_from_slice(&(tlen as u32).to_le_bytes());
                // title
                buf[off + 8..off + 8 + tlen].copy_from_slice(&title_bytes[..tlen]);
                // zero-fill remainder
                for b in &mut buf[off + 8 + tlen..off + 64] { *b = 0; }
            }
        }
        count as u32
    }) {
        Some(c) => c,
        None => 0,
    }
}

/// sys_win_focus - Focus/raise a window by ID.
pub fn sys_win_focus(window_id: u32) -> u32 {
    match crate::ui::desktop::with_desktop(|desktop| {
        desktop.focus_window_by_id(window_id)
    }) {
        Some(_) => 0,
        None => u32::MAX,
    }
}

/// sys_screen_size - Get screen dimensions. buf_ptr: [width:u32, height:u32]
pub fn sys_screen_size(buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
    match crate::ui::desktop::with_desktop(|desktop| {
        (desktop.screen_width, desktop.screen_height)
    }) {
        Some((w, h)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = w;
                *buf.add(1) = h;
            }
            0
        }
        None => u32::MAX,
    }
}

// =========================================================================
// Device management (existing)
// =========================================================================

pub fn sys_devlist(buf_ptr: u32, buf_size: u32) -> u32 {
    let devices = crate::drivers::hal::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 32usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, (path, _, _)) in devices.iter().enumerate().take(max_entries.min(count)) {
            let offset = i * entry_size;
            let path_bytes = path.as_bytes();
            let copy_len = path_bytes.len().min(entry_size - 1);
            buf[offset..offset + copy_len].copy_from_slice(&path_bytes[..copy_len]);
            buf[offset + copy_len] = 0;
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
    if buf_ptr == 0 || len == 0 { return 0; }
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
    if buf_ptr == 0 || len == 0 { return 0; }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    crate::ipc::pipe::write(pipe_id, buf)
}

/// sys_pipe_open - Open an existing pipe by name. Returns pipe_id or 0 if not found.
pub fn sys_pipe_open(name_ptr: u32) -> u32 {
    if name_ptr == 0 { return 0; }
    let name = unsafe { read_user_str(name_ptr) };
    crate::ipc::pipe::open(name)
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
    match crate::task::dll::get_dll_base(path) {
        Some(base) => base,
        None => 0,
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
        if buf_ptr != 0 {
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
        if buf_ptr != 0 {
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
