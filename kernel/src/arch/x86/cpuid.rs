//! CPUID feature detection for x86-64 processors.
//!
//! Queries CPU feature flags at boot time and stores the results in a global
//! `CpuFeatures` struct. Used to verify mandatory features (FPU, SSE, SSE2,
//! FXSR) and to log available optional features.

use core::sync::atomic::{AtomicBool, Ordering};

static DETECTED: AtomicBool = AtomicBool::new(false);
static mut FEATURES: CpuFeatures = CpuFeatures::empty();

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
            nx: false,
            syscall: false,
            avx2: false,
            erms: false,
            fsgsbase: false,
            bmi1: false,
            bmi2: false,
            rdpid: false,
        }
    }
}

/// Execute CPUID instruction.
#[inline]
fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
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

    // Extended leaf 0x80000001: NX, SYSCALL
    let max_ext = cpuid(0x80000000, 0).0;
    if max_ext >= 0x80000001 {
        let (_eax, _ebx, _ecx, edx) = cpuid(0x80000001, 0);
        f.nx = edx & (1 << 20) != 0;
        f.syscall = edx & (1 << 11) != 0;
    }

    // Leaf 7 subleaf 0: structured extended features
    let max_leaf = cpuid(0, 0).0;
    if max_leaf >= 7 {
        let (_eax, ebx, ecx, _edx) = cpuid(7, 0);
        f.fsgsbase = ebx & (1 << 0) != 0;
        f.bmi1 = ebx & (1 << 3) != 0;
        f.avx2 = ebx & (1 << 5) != 0;
        f.bmi2 = ebx & (1 << 8) != 0;
        f.erms = ebx & (1 << 9) != 0;
        f.rdpid = ecx & (1 << 22) != 0;
    }

    // Store and mark detected
    unsafe { FEATURES = f; }
    DETECTED.store(true, Ordering::Release);

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
        "  NX={} SYSCALL={} PCID={} RDRAND={}",
        f.nx, f.syscall, f.pcid, f.rdrand
    );
    crate::serial_println!(
        "  ERMS={} FSGSBASE={} BMI1={} BMI2={}",
        f.erms, f.fsgsbase, f.bmi1, f.bmi2
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
