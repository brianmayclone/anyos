//! CPUID feature detection for x86-64 processors.
//!
//! Queries CPU feature flags at boot time and stores the results in a global
//! `CpuFeatures` struct. Used to verify mandatory features (FPU, SSE, SSE2,
//! FXSR) and to log available optional features.

use core::sync::atomic::{AtomicBool, Ordering};

static DETECTED: AtomicBool = AtomicBool::new(false);
/// Set once at boot after detect(). Read by the idle loop for MONITOR/MWAIT.
pub static HAS_MWAIT: AtomicBool = AtomicBool::new(false);
static mut FEATURES: CpuFeatures = CpuFeatures::empty();
/// 12-byte vendor string from CPUID leaf 0 (e.g. "GenuineIntel"), null-padded to 16.
static mut CPU_VENDOR: [u8; 16] = [0; 16];
/// 48-byte brand string from CPUID leaves 0x80000002-4 (e.g. "QEMU Virtual CPU...").
static mut CPU_BRAND: [u8; 48] = [0; 48];

/// CPU feature flags detected via CPUID.
#[derive(Debug, Clone, Copy)]
pub struct CpuFeatures {
    // Leaf 1 EDX
    pub fpu: bool,
    pub fxsr: bool,
    pub sse: bool,
    pub sse2: bool,
    // Leaf 1 ECX
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub aes_ni: bool,
    pub avx: bool,
    pub xsave: bool,
    pub rdrand: bool,
    pub pcid: bool,
    pub mwait: bool,
    // Extended leaf 0x80000001 EDX
    pub nx: bool,
    pub syscall: bool,
    // Leaf 7 EBX
    pub avx2: bool,
    pub erms: bool,
    pub fsgsbase: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    // Leaf 7 ECX
    pub rdpid: bool,
    // Leaf 7 EBX (supervisor-mode protection)
    pub smep: bool,
}

impl CpuFeatures {
    const fn empty() -> Self {
        CpuFeatures {
            fpu: false,
            fxsr: false,
            sse: false,
            sse2: false,
            sse3: false,
            ssse3: false,
            sse4_1: false,
            sse4_2: false,
            aes_ni: false,
            avx: false,
            xsave: false,
            rdrand: false,
            pcid: false,
            mwait: false,
            nx: false,
            syscall: false,
            avx2: false,
            erms: false,
            fsgsbase: false,
            bmi1: false,
            bmi2: false,
            rdpid: false,
            smep: false,
        }
    }
}

/// Execute CPUID instruction.
#[inline]
pub(crate) fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        // LLVM reserves RBX, so we must save/restore it manually
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            ebx_out = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    (eax, ebx, ecx, edx)
}

/// Detect CPU features via CPUID and store globally.
/// Panics if mandatory features (FPU, SSE, SSE2, FXSR) are missing.
pub fn detect() {
    let mut f = CpuFeatures::empty();

    // Leaf 0: vendor string (EBX-EDX-ECX = 12 bytes, e.g. "GenuineIntel")
    let (_max_leaf0, vbx, vcx, vdx) = cpuid(0, 0);
    unsafe {
        CPU_VENDOR[0..4].copy_from_slice(&vbx.to_le_bytes());
        CPU_VENDOR[4..8].copy_from_slice(&vdx.to_le_bytes());
        CPU_VENDOR[8..12].copy_from_slice(&vcx.to_le_bytes());
        // bytes 12..16 stay zero (null padding)
    }

    // Leaf 1: basic feature flags
    let (_eax, _ebx, ecx, edx) = cpuid(1, 0);
    f.fpu = edx & (1 << 0) != 0;
    f.fxsr = edx & (1 << 24) != 0;
    f.sse = edx & (1 << 25) != 0;
    f.sse2 = edx & (1 << 26) != 0;

    f.sse3 = ecx & (1 << 0) != 0;
    f.ssse3 = ecx & (1 << 9) != 0;
    f.sse4_1 = ecx & (1 << 19) != 0;
    f.sse4_2 = ecx & (1 << 20) != 0;
    f.aes_ni = ecx & (1 << 25) != 0;
    f.avx = ecx & (1 << 28) != 0;
    f.xsave = ecx & (1 << 26) != 0;
    f.rdrand = ecx & (1 << 30) != 0;
    f.pcid = ecx & (1 << 17) != 0;
    f.mwait = ecx & (1 << 3) != 0;

    // Extended leaf 0x80000001: NX, SYSCALL
    let max_ext = cpuid(0x80000000, 0).0;
    if max_ext >= 0x80000001 {
        let (_eax, _ebx, _ecx, edx) = cpuid(0x80000001, 0);
        f.nx = edx & (1 << 20) != 0;
        f.syscall = edx & (1 << 11) != 0;
    }

    // Extended leaves 0x80000002-4: brand string (3 × 16 = 48 bytes)
    if max_ext >= 0x80000004 {
        unsafe {
            for i in 0u32..3 {
                let (a, b, c, d) = cpuid(0x80000002 + i, 0);
                let off = (i as usize) * 16;
                CPU_BRAND[off..off + 4].copy_from_slice(&a.to_le_bytes());
                CPU_BRAND[off + 4..off + 8].copy_from_slice(&b.to_le_bytes());
                CPU_BRAND[off + 8..off + 12].copy_from_slice(&c.to_le_bytes());
                CPU_BRAND[off + 12..off + 16].copy_from_slice(&d.to_le_bytes());
            }
        }
    }

    // Leaf 7 subleaf 0: structured extended features
    let max_leaf = cpuid(0, 0).0;
    if max_leaf >= 7 {
        let (_eax, ebx, ecx, _edx) = cpuid(7, 0);
        f.fsgsbase = ebx & (1 << 0) != 0;
        f.bmi1 = ebx & (1 << 3) != 0;
        f.smep = ebx & (1 << 7) != 0;
        f.avx2 = ebx & (1 << 5) != 0;
        f.bmi2 = ebx & (1 << 8) != 0;
        f.erms = ebx & (1 << 9) != 0;
        f.rdpid = ecx & (1 << 22) != 0;
    }

    // Store and mark detected
    unsafe { FEATURES = f; }
    DETECTED.store(true, Ordering::Release);
    HAS_MWAIT.store(f.mwait, Ordering::Release);

    // Log vendor and brand
    let vendor_str = unsafe {
        let len = CPU_VENDOR.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8_unchecked(&CPU_VENDOR[..len])
    };
    let brand_str = unsafe {
        let len = CPU_BRAND.iter().position(|&b| b == 0).unwrap_or(48);
        core::str::from_utf8_unchecked(&CPU_BRAND[..len])
    };
    crate::serial_println!("  CPU: {} / {}", vendor_str, brand_str);

    // Log results
    crate::serial_println!("[OK] CPUID features detected:");
    crate::serial_println!("  FPU={} SSE={} SSE2={} FXSR={}", f.fpu, f.sse, f.sse2, f.fxsr);
    crate::serial_println!(
        "  SSE3={} SSSE3={} SSE4.1={} SSE4.2={}",
        f.sse3, f.ssse3, f.sse4_1, f.sse4_2
    );
    crate::serial_println!(
        "  AVX={} AVX2={} AES-NI={} XSAVE={}",
        f.avx, f.avx2, f.aes_ni, f.xsave
    );
    crate::serial_println!(
        "  NX={} SYSCALL={} PCID={} RDRAND={} MWAIT={}",
        f.nx, f.syscall, f.pcid, f.rdrand, f.mwait
    );
    crate::serial_println!(
        "  ERMS={} FSGSBASE={} BMI1={} BMI2={} SMEP={}",
        f.erms, f.fsgsbase, f.bmi1, f.bmi2, f.smep
    );

    // Assert mandatory features for x86_64
    assert!(f.fpu, "CPU lacks FPU support (mandatory for x86_64)");
    assert!(f.fxsr, "CPU lacks FXSR support (mandatory for x86_64)");
    assert!(f.sse, "CPU lacks SSE support (mandatory for x86_64)");
    assert!(f.sse2, "CPU lacks SSE2 support (mandatory for x86_64)");
}

/// Get the detected CPU features. Panics if `detect()` has not been called.
pub fn features() -> CpuFeatures {
    assert!(DETECTED.load(Ordering::Acquire), "CPUID not yet detected");
    unsafe { FEATURES }
}

/// 12-byte vendor string from CPUID leaf 0, null-padded to 16 bytes.
pub fn vendor() -> &'static [u8; 16] {
    unsafe { &CPU_VENDOR }
}

/// 48-byte brand string from CPUID leaves 0x80000002-4.
pub fn brand() -> &'static [u8; 48] {
    unsafe { &CPU_BRAND }
}

/// Try to get the TSC frequency from hypervisor CPUID leaves.
///
/// Hypervisors expose the TSC frequency via synthetic CPUID leaves:
/// - **VirtualBox**: leaf 0x40000010, EAX = TSC freq in kHz
/// - **VMware**: leaf 0x40000010, EAX = TSC freq in kHz
/// - **Hyper-V**: leaf 0x40000003, EAX = TSC freq in Hz (10 MHz units)
/// - **CPUID leaf 0x15**: standard Intel TSC/crystal ratio (QEMU, bare metal)
///
/// Returns `Some(hz)` if a valid frequency was found, `None` otherwise.
pub fn hypervisor_tsc_hz() -> Option<u64> {
    // Check if a hypervisor is present (CPUID.1:ECX bit 31)
    let (_, _, ecx1, _) = cpuid(1, 0);
    let hypervisor_present = ecx1 & (1 << 31) != 0;

    if hypervisor_present {
        // Read hypervisor vendor string from leaf 0x40000000
        let (max_hv_leaf, hbx, hcx, hdx) = cpuid(0x40000000, 0);
        let mut hv_vendor = [0u8; 12];
        hv_vendor[0..4].copy_from_slice(&hbx.to_le_bytes());
        hv_vendor[4..8].copy_from_slice(&hcx.to_le_bytes());
        hv_vendor[8..12].copy_from_slice(&hdx.to_le_bytes());

        let hv_str = core::str::from_utf8(&hv_vendor).unwrap_or("???");
        crate::serial_println!("  Hypervisor: \"{}\" (max leaf {:#x})", hv_str, max_hv_leaf);

        // VirtualBox: leaf 0x40000010 → EAX = TSC freq in kHz
        if &hv_vendor == b"VBoxVBoxVBox" && max_hv_leaf >= 0x40000010 {
            let (tsc_khz, apic_khz, _, _) = cpuid(0x40000010, 0);
            if tsc_khz > 0 {
                crate::serial_println!("  VBox CPUID: TSC={}kHz, APIC={}kHz", tsc_khz, apic_khz);
                return Some(tsc_khz as u64 * 1000);
            }
        }

        // VMware: leaf 0x40000010 → EAX = TSC freq in kHz
        if &hv_vendor == b"VMwareVMware" && max_hv_leaf >= 0x40000010 {
            let (tsc_khz, _, _, _) = cpuid(0x40000010, 0);
            if tsc_khz > 0 {
                crate::serial_println!("  VMware CPUID: TSC={}kHz", tsc_khz);
                return Some(tsc_khz as u64 * 1000);
            }
        }

        // Hyper-V / Microsoft Hv: leaf 0x40000003 → EAX = TSC freq in Hz
        if &hv_vendor == b"Microsoft Hv" && max_hv_leaf >= 0x40000003 {
            let (tsc_hz, _, _, _) = cpuid(0x40000003, 0);
            if tsc_hz > 0 {
                crate::serial_println!("  Hyper-V CPUID: TSC={}Hz", tsc_hz);
                return Some(tsc_hz as u64);
            }
        }

        // KVM: leaf 0x40000010 → EAX = kHz (KVM also supports this leaf)
        if &hv_vendor[0..4] == b"KVMK" && max_hv_leaf >= 0x40000010 {
            let (tsc_khz, _, _, _) = cpuid(0x40000010, 0);
            if tsc_khz > 0 {
                crate::serial_println!("  KVM CPUID: TSC={}kHz", tsc_khz);
                return Some(tsc_khz as u64 * 1000);
            }
        }
    }

    // Standard CPUID leaf 0x15: TSC / Core Crystal Clock ratio
    // Available on newer Intel CPUs and some QEMU configurations.
    let max_leaf = cpuid(0, 0).0;
    if max_leaf >= 0x15 {
        let (denom, numer, crystal_hz, _) = cpuid(0x15, 0);
        if denom > 0 && numer > 0 && crystal_hz > 0 {
            let tsc_hz = crystal_hz as u64 * numer as u64 / denom as u64;
            crate::serial_println!("  CPUID 0x15: crystal={}Hz, ratio={}/{}, TSC={}Hz",
                crystal_hz, numer, denom, tsc_hz);
            return Some(tsc_hz);
        }
    }

    None
}

/// Enable SMEP (Supervisor Mode Execution Prevention) if the CPU supports it.
///
/// Sets CR4 bit 20. After this, any attempt by ring-0 code to execute
/// instructions at a user-mode (non-canonical, < 0x8000_0000_0000) virtual
/// address raises a #PF, preventing privilege-escalation exploits that hijack
/// ring-0 control flow by redirecting it to user-space shellcode.
///
/// Safe to call from both BSP and AP initialization paths.
pub fn enable_smep() {
    if !features().smep {
        return;
    }
    unsafe {
        let cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem, preserves_flags));
        core::arch::asm!("mov cr4, {}", in(reg) cr4 | (1u64 << 20), options(nostack, nomem, preserves_flags));
    }
    crate::serial_println!("  SMEP enabled (CR4.SMEP=1)");
}
