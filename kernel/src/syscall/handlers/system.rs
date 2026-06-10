//! System information and miscellaneous syscall handlers.
//!
//! Covers time/uptime, kernel log (dmesg), system info queries,
//! environment variables, keyboard layout, random numbers,
//! hostname, crash info, and power management (shutdown).

use super::helpers::{
    copy_to_user_bytes, copy_user_bytes, read_user_str_safe, resolve_path,
};

// =========================================================================
// System Information (SYS_TIME, SYS_UPTIME, SYS_SYSINFO)
// =========================================================================

/// sys_time - Get current date/time.
/// arg1=buf_ptr: output [year_lo:u8, year_hi:u8, month:u8, day:u8, hour:u8, min:u8, sec:u8, pad:u8]
pub fn sys_time(buf_ptr: u64) -> u32 {
    #[cfg(target_arch = "x86_64")]
    let (year, month, day, hour, min, sec) = crate::drivers::rtc::read_datetime();
    #[cfg(target_arch = "aarch64")]
    let (year, month, day, hour, min, sec): (u16, u8, u8, u8, u8, u8) = (1970, 1, 1, 0, 0, 0);
    // NULL buffer keeps the historical no-op-and-succeed behavior; a non-NULL
    // buffer is written through a mapping-validated copy so a kernel-space or
    // unmapped pointer returns an error instead of faulting / corrupting the
    // kernel.
    if buf_ptr != 0 {
        let yb = (year as u16).to_le_bytes();
        let bytes = [
            yb[0],
            yb[1],
            month as u8,
            day as u8,
            hour as u8,
            min as u8,
            sec as u8,
            0,
        ];
        if !copy_to_user_bytes(buf_ptr, &bytes, bytes.len()) {
            return u32::MAX;
        }
    }
    0
}

/// sys_set_time - Set system date/time via RTC.
/// arg1=buf_ptr: input [year_lo:u8, year_hi:u8, month:u8, day:u8, hour:u8, min:u8, sec:u8, pad:u8]
/// Returns 0 on success, u32::MAX on error.
pub fn sys_set_time(buf_ptr: u64) -> u32 {
    // Read the 8-byte time record through a mapping-validated copy so a NULL,
    // kernel-space, or unmapped pointer returns an error instead of faulting or
    // reading kernel memory.
    let buf = match copy_user_bytes(buf_ptr, 8, 8) {
        Some(b) => b,
        None => return u32::MAX,
    };
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    let sec = buf[6];
    // Basic validation.
    if month == 0
        || month > 12
        || day == 0
        || day > 31
        || hour > 23
        || min > 59
        || sec > 59
        || year < 2000
        || year > 2099
    {
        return u32::MAX;
    }
    #[cfg(target_arch = "x86_64")]
    crate::drivers::rtc::set_time(year, month, day, hour, min, sec);
    crate::serial_println!(
        "RTC set: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year,
        month,
        day,
        hour,
        min,
        sec
    );
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
    {
        crate::arch::x86::pit::real_ms_since_boot() as u32
    }
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::hal::timer_current_ticks()
    } // Already in ms
}

/// sys_dmesg - Read kernel log ring buffer.
/// arg1=buf_ptr, arg2=buf_size. Returns bytes written.
pub fn sys_dmesg(buf_ptr: u64, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 {
        return 0;
    }
    // The kernel log ring buffer is 32 KiB, so cap the temporary allocation to
    // that even though buf_size is fully user-controlled. Fill a bounded kernel
    // buffer, then copy out through the mapping-validated helper so an unmapped
    // user page returns 0 instead of faulting the kernel.
    let n = (buf_size as usize).min(32 * 1024);
    let mut tmp = alloc::vec![0u8; n];
    let written = crate::drivers::serial::read_log(&mut tmp).min(n);
    if written == 0 {
        return 0;
    }
    if !copy_to_user_bytes(buf_ptr, &tmp[..written], n) {
        return 0;
    }
    written as u32
}

/// sys_sysinfo - Get system information.
/// arg1=cmd: 0=memory, 1=threads, 2=cpus, 3=cpu_load, 4=hardware,
///           5=cpu_power, 6=cpu_frequency
/// arg2=buf_ptr, arg3=buf_size
pub fn sys_sysinfo(cmd: u32, buf_ptr: u64, buf_size: u32) -> u32 {
    match cmd {
        0 => {
            // Memory: u32 words [total_frames, free_frames, heap_used,
            // heap_total, swap_total_pages, swap_free_pages, swap_areas].
            if buf_ptr != 0 && buf_size >= 8 {
                // Build the fixed 28-byte layout locally then copy only the
                // byte count the caller asked for (matching the historic
                // buf_size thresholds) through the mapping-validated helper.
                let mut out = [0u8; 28];
                out[0..4]
                    .copy_from_slice(&(crate::memory::physical::total_frames() as u32).to_le_bytes());
                out[4..8]
                    .copy_from_slice(&(crate::memory::physical::free_frames() as u32).to_le_bytes());
                if buf_size >= 16 {
                    let (heap_used, heap_total) = crate::memory::heap::heap_stats();
                    out[8..12].copy_from_slice(&(heap_used as u32).to_le_bytes());
                    out[12..16].copy_from_slice(&(heap_total as u32).to_le_bytes());
                }
                if buf_size >= 24 {
                    let swap = crate::memory::swap::stats();
                    out[16..20]
                        .copy_from_slice(&(swap.total_pages.min(u32::MAX as u64) as u32).to_le_bytes());
                    out[20..24]
                        .copy_from_slice(&(swap.free_pages.min(u32::MAX as u64) as u32).to_le_bytes());
                    if buf_size >= 28 {
                        out[24..28].copy_from_slice(&swap.areas.to_le_bytes());
                    }
                }
                let n = (buf_size as usize).min(28);
                copy_to_user_bytes(buf_ptr, &out[..n], 28);
            }
            0
        }
        1 => {
            // Thread list: 80 bytes each
            // [tid:u32, prio:u8, state:u8, arch:u8, flags:u8, name:24bytes,
            //  user_pages:u32, cpu_ticks:u32, io_read_bytes:u64, io_write_bytes:u64,
            //  uid:u16, pad:u16, parent_tid:u32,
            //  net_tx_bytes:u64, net_rx_bytes:u64]
            // flags byte (offset 7): bit 0 = pd_shared (child thread of same process)
            let threads = crate::task::scheduler::list_threads();
            if buf_ptr != 0 && buf_size > 0 {
                let entry_size = 80usize;
                let max = (buf_size as usize) / entry_size;
                let count = threads.len().min(max);
                // Serialize the records into a kernel-owned buffer first, then
                // copy out via the mapping-validated helper so a bad user
                // pointer cannot fault the kernel.
                let mut out = alloc::vec![0u8; count * entry_size];
                for (i, t) in threads.iter().enumerate().take(count) {
                    let off = i * entry_size;
                    out[off..off + 4].copy_from_slice(&t.tid.to_le_bytes());
                    out[off + 4] = t.priority;
                    out[off + 5] = match t.state {
                        "ready" => 0,
                        "running" => 1,
                        "blocked" => 2,
                        "dead" => 3,
                        "stopped" => 4,
                        _ => 255,
                    };
                    out[off + 6] = t.arch_mode; // reserved (always 0 = 64-bit since 32-bit user removed)
                    out[off + 7] = if t.pd_shared { 1 } else { 0 };
                    let name_bytes = t.name.as_bytes();
                    let n = name_bytes.len().min(23);
                    out[off + 8..off + 8 + n].copy_from_slice(&name_bytes[..n]);
                    out[off + 8 + n] = 0;
                    // user_pages at offset 32
                    out[off + 32..off + 36].copy_from_slice(&t.user_pages.to_le_bytes());
                    // cpu_ticks at offset 36
                    out[off + 36..off + 40].copy_from_slice(&t.cpu_ticks.to_le_bytes());
                    // io_read_bytes at offset 40, io_write_bytes at offset 48
                    out[off + 40..off + 48].copy_from_slice(&t.io_read_bytes.to_le_bytes());
                    out[off + 48..off + 56].copy_from_slice(&t.io_write_bytes.to_le_bytes());
                    // uid at offset 56, pad at 58
                    out[off + 56..off + 58].copy_from_slice(&t.uid.to_le_bytes());
                    out[off + 58] = 0;
                    out[off + 59] = 0;
                    // parent_tid at offset 60
                    out[off + 60..off + 64].copy_from_slice(&t.parent_tid.to_le_bytes());
                    // net_tx_bytes at offset 64, net_rx_bytes at offset 72
                    out[off + 64..off + 72].copy_from_slice(&t.net_tx_bytes.to_le_bytes());
                    out[off + 72..off + 80].copy_from_slice(&t.net_rx_bytes.to_le_bytes());
                }
                if !out.is_empty() {
                    copy_to_user_bytes(buf_ptr, &out, out.len());
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
                // Build the contiguous prefix (16-byte header + per-CPU pairs
                // that fit) into a kernel buffer, then copy out via the
                // mapping-validated helper. `words` is bounded by the actual
                // CPU count so a huge user buf_size cannot force an oversized
                // allocation.
                let mut words: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
                words.push(crate::task::scheduler::total_sched_ticks());
                words.push(crate::task::scheduler::idle_sched_ticks());
                words.push(num_cpus as u32);
                words.push(0);
                for i in 0..num_cpus {
                    let off = 4 + i * 2;
                    if (off + 2) * 4 <= buf_size as usize {
                        words.push(crate::task::scheduler::per_cpu_total_ticks(i));
                        words.push(crate::task::scheduler::per_cpu_idle_ticks(i));
                    }
                }
                let mut out = alloc::vec![0u8; words.len() * 4];
                for (i, w) in words.iter().enumerate() {
                    out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
                }
                copy_to_user_bytes(buf_ptr, &out, out.len());
            }
            0
        }
        4 => {
            // Hardware info: up to 116-byte struct (backwards-compatible)
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
            //   [104..108] Power features: bit0=HWP, bit1=Turbo, bit2=APERF,
            //              bit3=hypervisor, bit4=active frequency control (u32 LE)
            //   [108..112] Active CPU power profile: 0=saver,1=balanced,2=performance
            //   [112..116] CPU power driver: 0=none,1=Intel HWP,2=Intel legacy,
            //              3=AMD P-state,4=KVM/host best-effort
            if buf_ptr == 0 || buf_size < 96 {
                return u32::MAX;
            }
            let actual_size = if buf_size >= 116 {
                116usize
            } else if buf_size >= 108 {
                108usize
            } else {
                96usize
            };
            // Build the fixed hardware struct in a local buffer, then copy out
            // through the mapping-validated helper (which also validates the
            // destination mapping, so the range-only is_valid_user_ptr gate is
            // no longer needed).
            let mut out = [0u8; 116];
            let buf = &mut out[..actual_size];

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

            // Extended fields (108/116-byte callers only)
            if actual_size >= 108 {
                #[cfg(target_arch = "x86_64")]
                {
                    let cur_freq = crate::arch::x86::power::average_frequency_mhz();
                    let max_freq = crate::arch::x86::power::max_frequency_mhz();
                    let features = crate::arch::x86::power::features_bitfield();
                    buf[96..100].copy_from_slice(&cur_freq.to_le_bytes());
                    buf[100..104].copy_from_slice(&max_freq.to_le_bytes());
                    buf[104..108].copy_from_slice(&features.to_le_bytes());
                }
                // ARM64: fields left as 0 (filled above)
            }
            if actual_size >= 116 {
                let profile = crate::arch::hal::cpu_power_profile();
                let driver = crate::arch::hal::cpu_power_driver_kind();
                buf[108..112].copy_from_slice(&profile.to_le_bytes());
                buf[112..116].copy_from_slice(&driver.to_le_bytes());
            }

            if !copy_to_user_bytes(buf_ptr, &out[..actual_size], actual_size) {
                return u32::MAX;
            }
            actual_size as u32
        }
        5 => {
            // CPU power profile.
            // Query with a >=20-byte buffer:
            //   [0] profile, [1] driver, [2] features, [3] current MHz, [4] max MHz.
            // Set with buf_size=0 and buf_ptr=<profile id>.
            if buf_size == 0 {
                return if crate::arch::hal::set_cpu_power_profile(buf_ptr as u32) {
                    0
                } else {
                    u32::MAX
                };
            }
            if buf_ptr == 0 || buf_size < 20 {
                return u32::MAX;
            }
            // Build the fixed 20-byte (5 u32) struct locally, then copy out via
            // the mapping-validated helper (replaces the range-only gate).
            let mut out = [0u8; 20];
            out[0..4].copy_from_slice(&crate::arch::hal::cpu_power_profile().to_le_bytes());
            out[4..8].copy_from_slice(&crate::arch::hal::cpu_power_driver_kind().to_le_bytes());
            #[cfg(target_arch = "x86_64")]
            {
                out[8..12].copy_from_slice(&crate::arch::x86::power::features_bitfield().to_le_bytes());
                out[12..16]
                    .copy_from_slice(&crate::arch::x86::power::average_frequency_mhz().to_le_bytes());
                out[16..20]
                    .copy_from_slice(&crate::arch::x86::power::max_frequency_mhz().to_le_bytes());
            }
            // aarch64: words 2..5 remain zero.
            if !copy_to_user_bytes(buf_ptr, &out, 20) {
                return u32::MAX;
            }
            20
        }
        6 => {
            // CPU frequency snapshot:
            //   [0] num_cpus
            //   [1] average current MHz across sampled CPUs
            //   [2] total current MHz across sampled CPUs
            //   [3] max CPU MHz
            //   [4] power driver
            //   [5] active power profile
            //   [6] power feature bitfield
            //   [7] reserved
            //   [8..] per-core current MHz, one u32 per CPU
            let num_cpus = crate::arch::hal::cpu_count().min(64);
            let required = 32usize.saturating_add(num_cpus.saturating_mul(4));
            if buf_ptr == 0 || buf_size < 32 {
                return u32::MAX;
            }
            // Bound the word count to what we actually fill (8 header words plus
            // one per CPU) so a huge user buf_size cannot force an oversized
            // kernel allocation, then copy out via the mapping-validated helper.
            let words = ((buf_size as usize) / 4).min(8 + num_cpus);
            let mut buf = alloc::vec![0u32; words];
            buf[0] = num_cpus as u32;
            if words > 4 {
                buf[4] = crate::arch::hal::cpu_power_driver_kind();
            }
            if words > 5 {
                buf[5] = crate::arch::hal::cpu_power_profile();
            }
            #[cfg(target_arch = "x86_64")]
            {
                crate::arch::x86::power::sample_current_cpu_frequency_mhz();
                if words > 1 {
                    buf[1] = crate::arch::x86::power::average_frequency_mhz();
                }
                if words > 2 {
                    buf[2] = crate::arch::x86::power::total_frequency_mhz();
                }
                if words > 3 {
                    buf[3] = crate::arch::x86::power::max_frequency_mhz();
                }
                if words > 6 {
                    buf[6] = crate::arch::x86::power::features_bitfield();
                }
                let limit = num_cpus.min(words.saturating_sub(8));
                for cpu in 0..limit {
                    buf[8 + cpu] = crate::arch::x86::power::per_cpu_frequency_mhz(cpu);
                }
            }
            // aarch64: words 1/2/3/6 remain zero (matching the original).
            let mut out = alloc::vec![0u8; words * 4];
            for (i, w) in buf.iter().enumerate() {
                out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
            }
            if !out.is_empty() {
                copy_to_user_bytes(buf_ptr, &out, out.len());
            }
            required.min(buf_size as usize) as u32
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
pub fn sys_setenv(key_ptr: u64, val_ptr: u64) -> u32 {
    // Use the mapping-validated string reader so a bad key/value pointer
    // returns an error instead of faulting on the scan-to-NUL.
    let key = match read_user_str_safe(key_ptr) {
        Some(k) if !k.is_empty() => k,
        _ => return u32::MAX,
    };

    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return u32::MAX,
    };

    if val_ptr == 0 {
        crate::task::env::unset(pd, key);
    } else {
        let val = match read_user_str_safe(val_ptr) {
            Some(v) => v,
            None => return u32::MAX,
        };
        crate::task::env::set(pd, key, val);
    }
    0
}

/// sys_getenv - Get an environment variable.
/// arg1 = key_ptr (null-terminated), arg2 = val_buf_ptr, arg3 = val_buf_size.
/// Returns length of value (bytes written, excluding null terminator), or u32::MAX if not found.
pub fn sys_getenv(key_ptr: u64, val_buf_ptr: u64, val_buf_size: u32) -> u32 {
    // Mapping-validated key read.
    let key = match read_user_str_safe(key_ptr) {
        Some(k) if !k.is_empty() => k,
        _ => return u32::MAX,
    };

    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return u32::MAX,
    };

    match crate::task::env::get(pd, key) {
        Some(val) => {
            let val_bytes = val.as_bytes();
            let copy_len = val_bytes.len().min(val_buf_size as usize);
            if val_buf_ptr != 0 && val_buf_size > 0 {
                // Build the exact bytes to write (value + optional NUL when
                // there is room) and copy out through the mapping-validated
                // helper, replacing the raw slice + range-only gate.
                let total = if copy_len < val_buf_size as usize {
                    copy_len + 1
                } else {
                    copy_len
                };
                if total > 0 {
                    let mut out = alloc::vec![0u8; total];
                    out[..copy_len].copy_from_slice(&val_bytes[..copy_len]);
                    // out[copy_len] (the terminator slot, if present) is already 0.
                    copy_to_user_bytes(val_buf_ptr, &out, val_buf_size as usize);
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
pub fn sys_listenv(buf_ptr: u64, buf_size: u32) -> u32 {
    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd.as_u64(),
        None => return 0,
    };

    if buf_ptr == 0 || buf_size == 0 {
        // Just return the needed size
        let mut dummy = [0u8; 0];
        return crate::task::env::list(pd, &mut dummy) as u32;
    }

    // Fill a bounded kernel buffer (env::list writes only the entries that fit
    // and returns the total needed size), then copy out through the
    // mapping-validated helper. The cap keeps a huge user buf_size from forcing
    // an unbounded allocation; env::list still reports the true needed size.
    const MAX_ENV_BYTES: usize = 64 * 1024;
    let n = (buf_size as usize).min(MAX_ENV_BYTES);
    let mut tmp = alloc::vec![0u8; n];
    let needed = crate::task::env::list(pd, &mut tmp);
    let written = needed.min(n);
    if written > 0 {
        copy_to_user_bytes(buf_ptr, &tmp[..written], n);
    }
    needed as u32
}

// =========================================================================
// Keyboard layout syscalls
// =========================================================================

/// SYS_KBD_GET_LAYOUT (200): Returns the currently active keyboard layout ID.
pub fn sys_kbd_get_layout() -> u32 {
    crate::drivers::layout::get_layout() as u32
}

/// SYS_KBD_SET_LAYOUT (201): Set the active keyboard layout by ID.
/// Returns 0 on success, u32::MAX if the layout ID is invalid.
pub fn sys_kbd_set_layout(layout_id: u32) -> u32 {
    match crate::drivers::layout::layout_id_from_u32(layout_id) {
        Some(id) => {
            crate::drivers::layout::set_layout(id);
            crate::serial_println!("Keyboard layout changed to {:?}", id);
            0
        }
        None => u32::MAX,
    }
}

/// SYS_RANDOM (210): Fill a user buffer with random bytes.
/// arg1 = buf_ptr, arg2 = len (max 256 bytes per call).
/// Uses the kernel crypto entropy pipeline.
/// Returns number of bytes written.
pub fn sys_random(buf_ptr: u64, len: u32) -> u32 {
    let len = (len as usize).min(256);
    if len == 0 {
        return 0;
    }

    // Generate into a fixed local buffer (len already capped to 256), then copy
    // out through the mapping-validated helper instead of writing a raw user
    // slice.
    let mut tmp = [0u8; 256];
    crate::crypto::random::fill_random(&mut tmp[..len]);
    if !copy_to_user_bytes(buf_ptr, &tmp[..len], len) {
        return 0;
    }
    len as u32
}

/// SYS_KBD_LIST_LAYOUTS (202): Write layout info entries to a user buffer.
/// arg1 = buf_ptr (array of LayoutInfo), arg2 = max_entries.
/// Returns number of entries written.
pub fn sys_kbd_list_layouts(buf_ptr: u64, max_entries: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        use crate::drivers::layout::{LayoutInfo, LAYOUT_COUNT, LAYOUT_INFOS};

        let count = (max_entries as usize).min(LAYOUT_COUNT);
        let byte_size = count * core::mem::size_of::<LayoutInfo>();

        if buf_ptr == 0 || byte_size == 0 {
            return 0;
        }

        // LayoutInfo is repr(C) POD, so the first `count` entries of the static
        // LAYOUT_INFOS form a contiguous kernel-internal byte slice we can copy
        // out through the mapping-validated helper.
        let src = unsafe {
            core::slice::from_raw_parts(LAYOUT_INFOS.as_ptr() as *const u8, byte_size)
        };
        if !copy_to_user_bytes(buf_ptr, src, byte_size) {
            return 0;
        }
        count as u32
    }
    #[cfg(target_arch = "aarch64")]
    {
        let _ = (buf_ptr, max_entries);
        0
    }
}

// =========================================================================
// Crash Info (SYS_GET_CRASH_INFO)
// =========================================================================

/// SYS_GET_CRASH_INFO (260): Retrieve crash report for a terminated thread.
/// arg1 = tid, arg2 = buf_ptr, arg3 = buf_size.
/// Copies the raw CrashReport struct into the user buffer.
/// Returns bytes written, or 0 if no crash report exists for that TID.
pub fn sys_get_crash_info(tid: u32, buf_ptr: u64, buf_size: u32) -> u32 {
    use crate::task::crash_info;

    if buf_ptr == 0 || buf_size == 0 {
        return 0;
    }

    let needed = crash_info::CRASH_REPORT_SIZE;
    if (buf_size as usize) < needed {
        return 0;
    }

    match crash_info::take_crash(tid) {
        Some(report) => {
            // `report` is a kernel-local struct; expose its bytes as a
            // kernel-internal slice and copy out through the mapping-validated
            // helper (replaces the range-only is_valid_user_ptr gate).
            let src = unsafe {
                core::slice::from_raw_parts(
                    &report as *const crash_info::CrashReport as *const u8,
                    needed,
                )
            };
            if !copy_to_user_bytes(buf_ptr, src, needed) {
                return 0;
            }
            needed as u32
        }
        None => 0,
    }
}

// ── Swap control ─────────────────────────────────────

/// SYS_SWAPON - Enable a regular file as swap backing store.
///   arg1: path_ptr, arg2: flags
/// Returns 0 on success, or u32::MAX on error.
pub fn sys_swapon(path_ptr: u64, flags: u32) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(path) if !path.is_empty() => path,
        _ => return u32::MAX,
    };
    let resolved = resolve_path(path);
    match crate::memory::swap::swapon_path(&resolved, flags) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

/// SYS_SWAPOFF - Disable a swap backing file if no slots are in use.
///   arg1: path_ptr
/// Returns 0 on success, or u32::MAX on error.
pub fn sys_swapoff(path_ptr: u64) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(path) if !path.is_empty() => path,
        _ => return u32::MAX,
    };
    let resolved = resolve_path(path);
    match crate::memory::swap::swapoff_path(&resolved) {
        Ok(()) => 0,
        Err(_) => u32::MAX,
    }
}

// ── Hostname ──────────────────────────────────────────

static HOSTNAME: crate::sync::mutex::Mutex<[u8; 64]> = {
    let mut buf = [0u8; 64];
    buf[0] = b'a';
    buf[1] = b'n';
    buf[2] = b'y';
    buf[3] = b'O';
    buf[4] = b'S';
    crate::sync::mutex::Mutex::new(buf)
};
static HOSTNAME_LEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(5);

/// SYS_GET_HOSTNAME - Copy current hostname into user buffer.
///   arg1: buf_ptr, arg2: buf_len
/// Returns bytes written, or u32::MAX on error.
pub fn sys_get_hostname(buf_ptr: u64, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return u32::MAX;
    }
    let len = HOSTNAME_LEN.load(core::sync::atomic::Ordering::Relaxed);
    let copy_len = len.min(buf_len);
    if copy_len == 0 {
        return 0;
    }
    // The HOSTNAME bytes are kernel-internal; copy them out through the
    // mapping-validated helper instead of writing the raw user pointer.
    let host = HOSTNAME.lock();
    if !copy_to_user_bytes(buf_ptr, &host[..copy_len as usize], copy_len as usize) {
        return u32::MAX;
    }
    copy_len
}

/// SYS_SET_HOSTNAME - Set the system hostname.
///   arg1: name_ptr, arg2: name_len
/// Returns 0 on success, u32::MAX on error.
pub fn sys_set_hostname(name_ptr: u64, name_len: u32) -> u32 {
    if name_ptr == 0 || name_len == 0 || name_len > 63 {
        return u32::MAX;
    }
    // Read the name through the mapping-validated helper first, then store it
    // under the lock, so a bad user pointer returns an error instead of
    // faulting the kernel.
    let bytes = match copy_user_bytes(name_ptr, name_len as usize, 63) {
        Some(b) => b,
        None => return u32::MAX,
    };
    let mut host = HOSTNAME.lock();
    host[..bytes.len()].copy_from_slice(&bytes);
    host[bytes.len()] = 0;
    HOSTNAME_LEN.store(bytes.len() as u32, core::sync::atomic::Ordering::Relaxed);
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
/// SYS_SYNC — Flush all dirty filesystem metadata and storage write caches.
pub fn sys_sync() -> u32 {
    crate::fs::vfs::sync_all();
    0
}

/// SYS_FSYNC — Flush deferred metadata for a specific open file to disk.
/// arg1 = local file descriptor
pub fn sys_fsync(fd: u32) -> u32 {
    sync_fd(fd, true)
}

pub fn sys_fdatasync(fd: u32) -> u32 {
    sync_fd(fd, false)
}

fn sync_fd(fd: u32, flush_hardware: bool) -> u32 {
    use crate::fs::fd_table::FdKind;

    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => {
            match entry.kind {
                FdKind::File { global_id } => {
                    let result = if flush_hardware {
                        crate::fs::vfs::fsync(global_id)
                    } else {
                        crate::fs::vfs::fdatasync(global_id)
                    };
                    match result {
                        Ok(()) => 0,
                        Err(_) => u32::MAX, // EIO
                    }
                }
                _ => 0, // Pipes/TTY: nothing to flush, succeed silently
            }
        }
        None => u32::MAX, // EBADF
    }
}

/// 3. Power off (ACPI) or reboot (keyboard controller reset).
///
/// Full shutdown sequence with proper device teardown:
///   Phase 1: Sync filesystems (while all processes still have valid handles)
///   Phase 2: Halt all other CPUs (prevents cascade-kills / scheduler interference)
///   Phase 3: Show shutdown/reboot screen (direct framebuffer write)
///   Phase 4: Kill all remaining user threads
///   Phase 5: Final filesystem sync (flush kernel-side caches)
///   Phase 6: Power off or reboot (with robust fallback chain)
///
/// This function does not return.
pub fn sys_shutdown(mode: u32) -> u32 {
    let is_reboot = mode == 1;
    let action = if is_reboot { "reboot" } else { "shutdown" };
    crate::serial_println!(
        "kernel: {} requested — beginning shutdown sequence...",
        action
    );

    // ── Phase 1: First filesystem sync ──
    // Sync while processes are still alive so their pending writes are flushed.
    crate::serial_println!("kernel: syncing filesystems...");
    crate::fs::vfs::sync_all();
    crate::serial_println!("kernel: filesystems synced");

    // ── Phase 2: Halt all other CPUs ──
    // Do this BEFORE killing threads so no other CPU can schedule, cascade-kill,
    // or interfere with our shutdown sequence. After this we are single-threaded.
    crate::serial_println!("kernel: halting other CPUs...");
    crate::arch::hal::halt_other_cpus();
    crate::arch::hal::disable_interrupts();

    // ── Phase 3: Show shutdown/reboot screen ──
    // Write directly to framebuffer. Other CPUs are halted, compositor is frozen,
    // so this is safe and the screen will be visible immediately.
    crate::serial_println!("kernel: displaying {} screen...", action);
    crate::drivers::shutdown_screen::show(is_reboot);

    // ── Phase 4: Kill all remaining user threads ──
    // With other CPUs halted and interrupts off, kill_thread cannot cascade-kill
    // our own thread via the scheduler. We skip our own TID and idle threads
    // (all_live_tids already filters idle threads).
    let my_tid = crate::task::scheduler::current_tid();
    let tids = crate::task::scheduler::all_live_tids();
    let mut killed = 0u32;
    for &tid in &tids {
        if tid == my_tid {
            continue;
        }
        if crate::task::scheduler::kill_thread(tid) == 0 {
            killed += 1;
        }
    }
    if killed > 0 {
        crate::serial_println!("kernel: terminated {} threads", killed);
    }

    // ── Phase 5: Final filesystem sync ──
    // Flush any remaining kernel-side block cache and storage write caches.
    crate::serial_println!("kernel: final filesystem sync...");
    crate::fs::vfs::sync_all();
    crate::serial_println!("kernel: done");

    // ── Phase 6: Power off or reboot ──
    #[cfg(target_arch = "x86_64")]
    {
        if is_reboot {
            x86_reboot_sequence();
        } else {
            crate::serial_println!("kernel: powering off via ACPI PM...");
            crate::arch::x86::acpi_pm::shutdown();
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if is_reboot {
            crate::serial_println!("kernel: rebooting via PSCI...");
            crate::arch::arm64::power::reset();
        } else {
            crate::serial_println!("kernel: powering off via PSCI...");
            crate::arch::arm64::power::shutdown();
        }
    }

    // Fallback: halt indefinitely if above methods didn't work
    #[allow(unreachable_code)]
    {
        crate::serial_println!("kernel: halt (all shutdown/reboot methods exhausted)");
        loop {
            crate::arch::hal::halt();
        }
    }
}

/// x86_64 reboot fallback chain — tries every known method in order:
///   1. 8042 keyboard controller reset (0xFE to port 0x64)
///   2. ACPI RESET_REG (FADT offset 128+, ACPI 2.0+)
///   3. PCI CF9 reset (port 0xCF9 — works on most Intel/AMD chipsets)
///   4. Fast A20/system reset via port 0x92
///   5. Triple fault (load empty IDT, trigger #UD → CPU reset)
#[cfg(target_arch = "x86_64")]
fn x86_reboot_sequence() -> ! {
    // Spin helper: brief delay to let the hardware react
    fn spin_brief() {
        for _ in 0..1_000_000u32 {
            core::hint::spin_loop();
        }
    }

    // ── Method 1: 8042 keyboard controller reset ──
    crate::serial_println!("kernel: reboot method 1/5 — 8042 keyboard controller (port 0x64)...");
    unsafe {
        let mut timeout = 100_000u32;
        while crate::arch::x86::port::inb(0x64) & 0x02 != 0 && timeout > 0 {
            timeout -= 1;
        }
        crate::arch::x86::port::outb(0x64, 0xFE);
    }
    spin_brief();

    // ── Method 2: ACPI Reset Register ──
    crate::serial_println!("kernel: reboot method 2/5 — ACPI RESET_REG...");
    if crate::arch::x86::acpi_pm::acpi_reboot() {
        spin_brief();
    }

    // ── Method 3: PCI CF9 reset ──
    // The CF9 register is on the Intel/AMD LPC or PCH. Writing 0x06 triggers
    // a hard reset (full platform reset), 0x0E triggers a warm reset.
    crate::serial_println!("kernel: reboot method 3/5 — PCI CF9 reset...");
    unsafe {
        // First clear, then write reset type
        crate::arch::x86::port::outb(0xCF9, 0x02); // set bit 1 (system reset)
        crate::arch::x86::port::outb(0xCF9, 0x06); // set bit 2 (full reset) + bit 1
    }
    spin_brief();

    // ── Method 4: Fast reset via port 0x92 ──
    crate::serial_println!("kernel: reboot method 4/5 — port 0x92 fast reset...");
    unsafe {
        let mut value = crate::arch::x86::port::inb(0x92);
        value |= 0x01;
        value &= !0x02;
        crate::arch::x86::port::outb(0x92, value);
    }
    spin_brief();

    // ── Method 5: Triple fault ──
    // Load an empty IDT and trigger an undefined instruction. With no #UD handler,
    // the CPU double-faults; with no #DF handler, it triple-faults and resets.
    crate::serial_println!("kernel: reboot method 5/5 — triple fault...");
    unsafe {
        // IDTR with limit=0, base=0 → no valid IDT entries
        let null_idtr: [u8; 10] = [0; 10];
        core::arch::asm!(
            "lidt [{}]",
            "ud2",
            in(reg) null_idtr.as_ptr(),
            options(noreturn)
        );
    }
}

/// SYS_SET_SERIAL_VERBOSE (283): Enable or disable verbose serial output.
/// arg1: 0 = disable (default, kernel-only output), 1 = enable (all driver/subsystem messages).
/// Returns 0 on success.
pub fn sys_set_serial_verbose(enable: u32) -> u32 {
    let enabled = enable != 0;
    crate::drivers::serial::set_verbose(enabled);
    crate::serial_println!(
        "kernel: serial verbose mode {}",
        if enabled { "enabled" } else { "disabled" }
    );
    0
}

// =========================================================================
// Text-mode console I/O (SYS_CON_WRITE, SYS_CON_READ)
// Used exclusively by textmode_console in nogui boot mode.
// =========================================================================

/// SYS_CON_WRITE (290): Write a UTF-8 string to the kernel framebuffer console.
/// arg1 = buf_ptr (user pointer), arg2 = len (bytes).
/// Returns number of bytes written, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_write(buf_ptr: u64, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    if len > 65536 {
        return u32::MAX;
    }
    // Copy the user bytes in through the mapping-validated helper, then operate
    // on the kernel-owned data instead of a raw user slice.
    let data = match copy_user_bytes(buf_ptr, len as usize, 65536) {
        Some(d) => d,
        None => return u32::MAX,
    };
    if let Ok(s) = core::str::from_utf8(&data) {
        crate::drivers::textcon::write_str(s);
        len
    } else {
        // Write byte-by-byte, skipping non-ASCII
        for &b in &data {
            if b < 128 {
                crate::drivers::textcon::write_char(b);
            }
        }
        len
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_write(_buf_ptr: u64, _len: u32) -> u32 {
    u32::MAX
}

/// SYS_CON_READ (291): Read a line from the keyboard with echo to the console.
/// arg1 = buf_ptr (user buffer), arg2 = buf_len.
/// arg2 high bit (0x80000000): if set, suppress echo (password mode).
/// Blocks until Enter is pressed.
/// Returns number of bytes read (not including null terminator), or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_read(buf_ptr: u64, buf_len: u32) -> u32 {
    let echo = (buf_len & 0x8000_0000) == 0;
    let max_len = (buf_len & 0x7FFF_FFFF) as usize;
    if buf_ptr == 0 || max_len == 0 {
        return 0;
    }
    if max_len > 4096 {
        return u32::MAX;
    }
    // Read the line into a bounded local buffer (max_len already <= 4096), then
    // copy out through the mapping-validated helper instead of a raw user slice.
    let mut tmp = [0u8; 4096];
    let n = crate::drivers::textcon::read_line(&mut tmp[..max_len], echo).min(max_len);
    if n == 0 {
        return 0;
    }
    if !copy_to_user_bytes(buf_ptr, &tmp[..n], max_len) {
        return u32::MAX;
    }
    n as u32
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_read(_buf_ptr: u64, _buf_len: u32) -> u32 {
    u32::MAX
}

/// SYS_CON_POLL_KEY (292): Non-blocking keyboard poll for the text console.
/// Returns the Unicode codepoint of the next pressed key, or 0 if no key pending.
/// Ctrl modifier: bit 29 set. Special values: 0x03=Ctrl+C, 0x04=Ctrl+D.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_poll_key() -> u32 {
    use crate::drivers::input::keyboard::Key;
    loop {
        match crate::drivers::input::keyboard::read_event() {
            None => return 0,
            Some(evt) if !evt.pressed => continue, // skip key-up events
            Some(evt) => {
                let ctrl = evt.modifiers.ctrl;
                let code: u32 = match evt.key {
                    Key::Char(c) => {
                        let cp = c as u32;
                        if ctrl && cp >= 64 && cp < 96 {
                            // Ctrl+letter: subtract 64 → control codes
                            cp - 64
                        } else if ctrl && cp >= 96 && cp < 128 {
                            cp - 96
                        } else {
                            cp
                        }
                    }
                    Key::Enter => b'\n' as u32,
                    Key::Backspace => b'\x08' as u32,
                    Key::Tab => b'\t' as u32,
                    Key::Escape => 0x1B,
                    Key::Up => {
                        if evt.modifiers.shift {
                            // Shift+Up: scroll viewport back — handled entirely in kernel
                            crate::drivers::textcon::scroll_viewport(1);
                            continue; // consume key, return nothing to userspace
                        } else {
                            0x10_0041
                        }
                    }
                    Key::Down => {
                        if evt.modifiers.shift {
                            // Shift+Down: scroll viewport forward — handled entirely in kernel
                            crate::drivers::textcon::scroll_viewport(-1);
                            continue; // consume key, return nothing to userspace
                        } else {
                            0x10_0042
                        }
                    }
                    Key::Left => {
                        if evt.modifiers.shift {
                            0x1400_0044
                        } else {
                            0x10_0044
                        }
                    }
                    Key::Right => {
                        if evt.modifiers.shift {
                            0x1400_0043
                        } else {
                            0x10_0043
                        }
                    }
                    Key::Space => b' ' as u32,
                    // Navigation keys (encoded as 0x20_00XX)
                    Key::Home => 0x20_0048,
                    Key::End => 0x20_004B,
                    Key::PageUp => 0x20_0049,
                    Key::PageDown => 0x20_0051,
                    Key::Delete => 0x20_0053,
                    // Function keys (encoded as 0x30_000N where N=1..12)
                    Key::F1 => 0x30_0001,
                    Key::F2 => 0x30_0002,
                    Key::F3 => 0x30_0003,
                    Key::F4 => 0x30_0004,
                    Key::F5 => 0x30_0005,
                    Key::F6 => 0x30_0006,
                    Key::F7 => 0x30_0007,
                    Key::F8 => 0x30_0008,
                    Key::F9 => 0x30_0009,
                    Key::F10 => 0x30_000A,
                    Key::F11 => 0x30_000B,
                    Key::F12 => 0x30_000C,
                    // Modifier-only keys: skip and poll again
                    Key::LeftShift
                    | Key::RightShift
                    | Key::LeftCtrl
                    | Key::RightCtrl
                    | Key::LeftAlt
                    | Key::RightAlt
                    | Key::CapsLock => continue,
                    _ => 0,
                };
                if ctrl && code < 32 {
                    return code; // raw control code (0x03 = Ctrl+C etc.)
                }
                let result = if ctrl { code | 0x2000_0000 } else { code };
                return result;
            }
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_poll_key() -> u32 {
    0
}

/// SYS_CON_GET_SIZE (293): Return console dimensions as cols<<16 | rows.
/// Both values are derived from the current framebuffer resolution and font size.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_get_size() -> u32 {
    crate::drivers::textcon::get_size_packed()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_get_size() -> u32 {
    0
}

/// SYS_CON_SET_MODE (294): Set console mode flags.
/// arg1 = new flags bitmask:
///   bit 0 (0x01): 1 = hide cursor, 0 = show cursor
///   bit 1 (0x02): 1 = disable auto-scroll, 0 = enable auto-scroll
/// Returns previous flags value.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_set_mode(flags: u32) -> u32 {
    crate::drivers::textcon::set_mode(flags)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_set_mode(_flags: u32) -> u32 {
    0
}

/// SYS_CON_RESIZE (295): Resize the text console to a specific number of columns/rows.
/// arg1 = (cols << 16) | rows.  Recomputes cell size and repaints the screen.
/// Returns new packed size on success, 0 if not initialised.
#[cfg(target_arch = "x86_64")]
pub fn sys_con_resize(packed: u32) -> u32 {
    let cols = packed >> 16;
    let rows = packed & 0xFFFF;
    crate::drivers::textcon::resize(cols, rows)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn sys_con_resize(_packed: u32) -> u32 {
    0
}
