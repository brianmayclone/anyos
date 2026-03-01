//! System information and miscellaneous syscall handlers.
//!
//! Covers time/uptime, kernel log (dmesg), system info queries,
//! environment variables, keyboard layout, random numbers,
//! hostname, crash info, and power management (shutdown).

use super::helpers::{is_valid_user_ptr, read_user_str};

// =========================================================================
// System Information (SYS_TIME, SYS_UPTIME, SYS_SYSINFO)
// =========================================================================

/// sys_time - Get current date/time.
/// arg1=buf_ptr: output [year_lo:u8, year_hi:u8, month:u8, day:u8, hour:u8, min:u8, sec:u8, pad:u8]
pub fn sys_time(buf_ptr: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    let (year, month, day, hour, min, sec) = crate::drivers::rtc::read_datetime();
    #[cfg(target_arch = "aarch64")]
    let (year, month, day, hour, min, sec): (u16, u8, u8, u8, u8, u8) = (1970, 1, 1, 0, 0, 0);
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

/// sys_uptime - Get system uptime in timer ticks (see `hal::timer_frequency_hz`).
pub fn sys_uptime() -> u32 {
    crate::arch::hal::timer_current_ticks()
}

/// sys_tick_hz - Get the timer tick rate in Hz.
pub fn sys_tick_hz() -> u32 {
    crate::arch::hal::timer_frequency_hz() as u32
}

/// sys_uptime_ms - Get uptime in milliseconds.
pub fn sys_uptime_ms() -> u32 {
    #[cfg(target_arch = "x86_64")]
    { crate::arch::x86::pit::real_ms_since_boot() as u32 }
    #[cfg(target_arch = "aarch64")]
    { crate::arch::hal::timer_current_ticks() } // Already in ms
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
            // Thread list: 60 bytes each
            // [tid:u32, prio:u8, state:u8, arch:u8, pad:u8, name:24bytes,
            //  user_pages:u32, cpu_ticks:u32, io_read_bytes:u64, io_write_bytes:u64,
            //  uid:u16, pad:u16]
            let threads = crate::task::scheduler::list_threads();
            if buf_ptr != 0 && buf_size > 0 {
                let entry_size = 60usize;
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
                    // uid at offset 56, pad at 58
                    buf[off + 56..off + 58].copy_from_slice(&t.uid.to_le_bytes());
                    buf[off + 58] = 0;
                    buf[off + 59] = 0;
                }
            }
            threads.len() as u32
        }
        2 => crate::arch::hal::cpu_count() as u32,
        3 => {
            // CPU load (extended):
            //   [0] total_sched_ticks (u32)
            //   [1] total_idle_ticks  (u32)
            //   [2] num_cpus          (u32)
            //   [3] reserved          (u32)
            //   [4..4+num_cpus*2] per_cpu_total[i], per_cpu_idle[i] pairs
            // Minimum 16 bytes for header, +8 per CPU
            let num_cpus = crate::arch::hal::cpu_count();
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
            // Hardware info: up to 108-byte struct (backwards-compatible)
            //   [0..48]    CPU brand string (null-terminated)
            //   [48..64]   CPU vendor string (null-terminated)
            //   [64..68]   TSC frequency in MHz (u32 LE)
            //   [68..72]   CPU count (u32 LE)
            //   [72..76]   Boot mode: 0=BIOS, 1=UEFI (u32 LE)
            //   [76..80]   Total physical memory in MiB (u32 LE)
            //   [80..84]   Free physical memory in MiB (u32 LE)
            //   [84..88]   Framebuffer width (u32 LE)
            //   [88..92]   Framebuffer height (u32 LE)
            //   [92..96]   Framebuffer BPP (u32 LE)
            //   [96..100]  Current CPU frequency in MHz (u32 LE)
            //   [100..104] Max CPU frequency in MHz (u32 LE)
            //   [104..108] Power features: bit0=HWP, bit1=Turbo, bit2=APERF (u32 LE)
            if buf_ptr == 0 || buf_size < 96 { return u32::MAX; }
            let actual_size = if buf_size >= 108 { 108usize } else { 96usize };
            if !is_valid_user_ptr(buf_ptr as u64, actual_size as u64) { return u32::MAX; }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, actual_size) };
            buf.fill(0);

            // CPU brand (48 bytes) and vendor (16 bytes)
            #[cfg(target_arch = "x86_64")]
            {
                let brand = crate::arch::x86::cpuid::brand();
                let vendor = crate::arch::x86::cpuid::vendor();
                buf[0..48].copy_from_slice(brand);
                buf[48..64].copy_from_slice(vendor);
                // TSC MHz
                let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
                buf[64..68].copy_from_slice(&tsc_mhz.to_le_bytes());
            }
            #[cfg(target_arch = "aarch64")]
            {
                let brand = b"AArch64 Processor\0";
                buf[0..brand.len().min(48)].copy_from_slice(&brand[..brand.len().min(48)]);
                let vendor = b"ARM\0";
                buf[48..48 + vendor.len().min(16)].copy_from_slice(&vendor[..vendor.len().min(16)]);
            }

            // CPU count
            let ncpu = crate::arch::hal::cpu_count() as u32;
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

            // Extended fields (108-byte callers only)
            if actual_size >= 108 {
                #[cfg(target_arch = "x86_64")]
                {
                    let cur_freq = crate::arch::x86::power::current_frequency_mhz();
                    let max_freq = crate::arch::x86::power::max_frequency_mhz();
                    let features = crate::arch::x86::power::features_bitfield();
                    buf[96..100].copy_from_slice(&cur_freq.to_le_bytes());
                    buf[100..104].copy_from_slice(&max_freq.to_le_bytes());
                    buf[104..108].copy_from_slice(&features.to_le_bytes());
                }
                // ARM64: fields left as 0 (filled above)
            }

            actual_size as u32
        }
        _ => u32::MAX,
    }
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

// =========================================================================
// Keyboard layout syscalls
// =========================================================================

/// SYS_KBD_GET_LAYOUT (200): Returns the currently active keyboard layout ID.
pub fn sys_kbd_get_layout() -> u32 {
    #[cfg(target_arch = "x86_64")]
    { crate::drivers::input::layout::get_layout() as u32 }
    #[cfg(target_arch = "aarch64")]
    { 0 } // ARM64: TODO — keyboard layout
}

/// SYS_KBD_SET_LAYOUT (201): Set the active keyboard layout by ID.
/// Returns 0 on success, u32::MAX if the layout ID is invalid.
pub fn sys_kbd_set_layout(layout_id: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        match crate::drivers::input::layout::layout_id_from_u32(layout_id) {
            Some(id) => {
                crate::drivers::input::layout::set_layout(id);
                crate::serial_println!("Keyboard layout changed to {:?}", id);
                0
            }
            None => u32::MAX,
        }
    }
    #[cfg(target_arch = "aarch64")]
    { let _ = layout_id; u32::MAX }
}

/// SYS_RANDOM (210): Fill a user buffer with random bytes.
/// arg1 = buf_ptr, arg2 = len (max 256 bytes per call).
/// Uses RDRAND if available, falls back to TSC-based PRNG.
/// Returns number of bytes written.
pub fn sys_random(buf_ptr: u32, len: u32) -> u32 {
    let len = (len as usize).min(256);
    if len == 0 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return 0;
    }

    let dst = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    #[cfg(target_arch = "x86_64")]
    let has_rdrand = crate::arch::x86::cpuid::features().rdrand;
    #[cfg(target_arch = "aarch64")]
    let has_rdrand = true; // ARM64: try RNDR (falls back to counter if fails)

    let mut filled = 0usize;
    if has_rdrand {
        // Use RDRAND: generates 64 bits of hardware random per call
        while filled + 8 <= len {
            if let Some(val) = rdrand64() {
                dst[filled..filled + 8].copy_from_slice(&val.to_ne_bytes());
                filled += 8;
            } else {
                break; // RDRAND failed, fall through to TSC
            }
        }
        // Handle remaining bytes
        if filled < len {
            if let Some(val) = rdrand64() {
                let bytes = val.to_ne_bytes();
                let remaining = len - filled;
                dst[filled..filled + remaining].copy_from_slice(&bytes[..remaining]);
                filled = len;
            }
        }
    }

    // Fallback: TSC-based xorshift64 for any unfilled bytes
    if filled < len {
        let mut state = rdtsc();
        // Mix in some additional entropy
        state ^= buf_ptr as u64;
        state ^= len as u64;
        while filled < len {
            state = xorshift64(state);
            let bytes = state.to_ne_bytes();
            let chunk = (len - filled).min(8);
            dst[filled..filled + chunk].copy_from_slice(&bytes[..chunk]);
            filled += chunk;
        }
    }

    filled as u32
}

/// Try to read 64 bits from hardware RNG.
#[inline]
fn rdrand64() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        let val: u64;
        let ok: u8;
        unsafe {
            core::arch::asm!(
                "rdrand {val}",
                "setc {ok}",
                val = out(reg) val,
                ok = out(reg_byte) ok,
            );
        }
        if ok != 0 { Some(val) } else { None }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let val: u64;
        unsafe {
            core::arch::asm!("mrs {}, s3_3_c2_c4_0", out(reg) val, options(nomem, nostack));
        }
        if val != 0 { Some(val) } else { None }
    }
}

/// Read a hardware monotonic counter.
#[inline]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let lo: u32;
        let hi: u32;
        unsafe {
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi);
        }
        ((hi as u64) << 32) | lo as u64
    }
    #[cfg(target_arch = "aarch64")]
    {
        let cnt: u64;
        unsafe {
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nomem, nostack));
        }
        cnt
    }
}

/// xorshift64 PRNG step.
#[inline]
fn xorshift64(mut x: u64) -> u64 {
    if x == 0 { x = 0x123456789ABCDEF0; }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// SYS_KBD_LIST_LAYOUTS (202): Write layout info entries to a user buffer.
/// arg1 = buf_ptr (array of LayoutInfo), arg2 = max_entries.
/// Returns number of entries written.
pub fn sys_kbd_list_layouts(buf_ptr: u32, max_entries: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        use crate::drivers::input::layout::{LAYOUT_INFOS, LAYOUT_COUNT, LayoutInfo};

        let count = (max_entries as usize).min(LAYOUT_COUNT);
        let byte_size = count * core::mem::size_of::<LayoutInfo>();

        if buf_ptr == 0 || byte_size == 0 || !is_valid_user_ptr(buf_ptr as u64, byte_size as u64) {
            return 0;
        }

        let dst = unsafe {
            core::slice::from_raw_parts_mut(buf_ptr as *mut LayoutInfo, count)
        };
        for i in 0..count {
            dst[i] = LAYOUT_INFOS[i];
        }
        count as u32
    }
    #[cfg(target_arch = "aarch64")]
    { let _ = (buf_ptr, max_entries); 0 }
}

// =========================================================================
// Crash Info (SYS_GET_CRASH_INFO)
// =========================================================================

/// SYS_GET_CRASH_INFO (260): Retrieve crash report for a terminated thread.
/// arg1 = tid, arg2 = buf_ptr, arg3 = buf_size.
/// Copies the raw CrashReport struct into the user buffer.
/// Returns bytes written, or 0 if no crash report exists for that TID.
pub fn sys_get_crash_info(tid: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    use crate::task::crash_info;

    if buf_ptr == 0 || buf_size == 0 {
        return 0;
    }

    let needed = crash_info::CRASH_REPORT_SIZE;
    if (buf_size as usize) < needed {
        return 0;
    }

    if !is_valid_user_ptr(buf_ptr as u64, needed as u64) {
        return 0;
    }

    match crash_info::take_crash(tid) {
        Some(report) => {
            let src = &report as *const crash_info::CrashReport as *const u8;
            let dst = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, needed) };
            unsafe {
                core::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), needed);
            }
            needed as u32
        }
        None => 0,
    }
}

// ── Hostname ──────────────────────────────────────────

static HOSTNAME: crate::sync::mutex::Mutex<[u8; 64]> = {
    let mut buf = [0u8; 64];
    buf[0] = b'a'; buf[1] = b'n'; buf[2] = b'y';
    buf[3] = b'O'; buf[4] = b'S';
    crate::sync::mutex::Mutex::new(buf)
};
static HOSTNAME_LEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(5);

/// SYS_GET_HOSTNAME - Copy current hostname into user buffer.
///   arg1: buf_ptr, arg2: buf_len
/// Returns bytes written, or u32::MAX on error.
pub fn sys_get_hostname(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return u32::MAX;
    }
    let len = HOSTNAME_LEN.load(core::sync::atomic::Ordering::Relaxed);
    let copy_len = len.min(buf_len);
    let host = HOSTNAME.lock();
    let dst = buf_ptr as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(host.as_ptr(), dst, copy_len as usize);
    }
    copy_len
}

/// SYS_SET_HOSTNAME - Set the system hostname.
///   arg1: name_ptr, arg2: name_len
/// Returns 0 on success, u32::MAX on error.
pub fn sys_set_hostname(name_ptr: u32, name_len: u32) -> u32 {
    if name_ptr == 0 || name_len == 0 || name_len > 63 {
        return u32::MAX;
    }
    let src = name_ptr as *const u8;
    let mut host = HOSTNAME.lock();
    unsafe {
        core::ptr::copy_nonoverlapping(src, host.as_mut_ptr(), name_len as usize);
    }
    host[name_len as usize] = 0;
    HOSTNAME_LEN.store(name_len, core::sync::atomic::Ordering::Relaxed);
    0
}

// ── Power management ────────────────────────────────────────────────────────

/// Shut down or reboot the system.
///
/// `mode`: 0 = power off, 1 = reboot.
///
/// The compositor is expected to have already drawn a shutdown screen and
/// killed user processes before invoking this syscall. The kernel's job is:
/// 1. Kill any remaining user threads (safety net).
/// 2. Halt all other CPUs via IPI.
/// 3. Power off (ACPI) or reboot (keyboard controller reset).
///
/// This function does not return.
pub fn sys_shutdown(mode: u32) -> u32 {
    let action = if mode == 1 { "reboot" } else { "shutdown" };
    crate::serial_println!("kernel: {} requested — beginning shutdown sequence...", action);

    // ── Phase 1: Kill any remaining user threads (safety net) ──
    let my_tid = crate::task::scheduler::current_tid();
    let tids = crate::task::scheduler::all_live_tids();
    let mut killed = 0u32;
    for &tid in &tids {
        if tid == my_tid { continue; }
        if crate::task::scheduler::kill_thread(tid) == 0 {
            killed += 1;
        }
    }
    if killed > 0 {
        crate::serial_println!("kernel: terminated {} remaining threads", killed);
    }

    // ── Phase 2: Halt all other CPUs ──
    crate::serial_println!("kernel: halting other CPUs...");
    crate::arch::hal::halt_other_cpus();
    crate::arch::hal::disable_interrupts();

    // ── Phase 3: Power off or reboot ──
    #[cfg(target_arch = "x86_64")]
    {
        if mode == 1 {
            crate::serial_println!("kernel: rebooting via keyboard controller...");
            unsafe {
                let mut timeout = 100_000u32;
                while crate::arch::x86::port::inb(0x64) & 0x02 != 0 && timeout > 0 {
                    timeout -= 1;
                }
                crate::arch::x86::port::outb(0x64, 0xFE);
            }
        } else {
            crate::serial_println!("kernel: powering off via ACPI...");
            unsafe { crate::arch::x86::port::outw(0x604, 0x2000); }
            unsafe { crate::arch::x86::port::outw(0xB004, 0x2000); }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if mode == 1 {
            crate::serial_println!("kernel: rebooting via PSCI...");
            crate::arch::arm64::power::reset();
        } else {
            crate::serial_println!("kernel: powering off via PSCI...");
            crate::arch::arm64::power::shutdown();
        }
    }

    // Fallback: halt indefinitely if above methods didn't work
    crate::serial_println!("kernel: halt (shutdown method did not take effect)");
    loop {
        crate::arch::hal::halt();
    }
}
