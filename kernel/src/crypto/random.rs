//! Random byte helpers for cryptographic and security-sensitive kernel code.

/// Fill `buf` with the best entropy the kernel can provide.
pub fn fill_random(buf: &mut [u8]) {
    let mut filled = 0usize;

    #[cfg(target_arch = "x86_64")]
    {
        filled = crate::drivers::virtio::rng::fill_random(buf);
    }

    if filled < buf.len() && has_hw_rng() {
        while filled + 8 <= buf.len() {
            if let Some(val) = hw_random_u64() {
                buf[filled..filled + 8].copy_from_slice(&val.to_ne_bytes());
                filled += 8;
            } else {
                break;
            }
        }
        if filled < buf.len() {
            if let Some(val) = hw_random_u64() {
                let bytes = val.to_ne_bytes();
                let remaining = buf.len() - filled;
                buf[filled..filled + remaining].copy_from_slice(&bytes[..remaining]);
                filled = buf.len();
            }
        }
    }

    if filled < buf.len() {
        let mut state = counter_entropy();
        state ^= crate::arch::hal::timer_current_ticks() as u64;
        state ^= buf.len() as u64;
        while filled < buf.len() {
            state = xorshift64(state);
            let bytes = state.to_ne_bytes();
            let chunk = (buf.len() - filled).min(8);
            buf[filled..filled + chunk].copy_from_slice(&bytes[..chunk]);
            filled += chunk;
        }
    }
}

#[inline]
fn has_hw_rng() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::x86::cpuid::features().rdrand
    }
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::arm64::cpu_features::HAS_RNG.load(core::sync::atomic::Ordering::Relaxed)
    }
}

#[inline]
fn hw_random_u64() -> Option<u64> {
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

#[inline]
fn counter_entropy() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let lo: u32;
        let hi: u32;
        unsafe {
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
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

#[inline]
fn xorshift64(mut x: u64) -> u64 {
    if x == 0 {
        x = 0x1234_5678_9ABC_DEF0;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}
