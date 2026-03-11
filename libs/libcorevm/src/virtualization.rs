//! Host hardware virtualization detection and backend abstraction.

/// Hardware virtualization backend selected for the host CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HardwareVirtualizationBackend {
    /// No supported hardware virtualization backend is available.
    Unavailable = 0,
    /// Intel VT-x / VMX is available.
    IntelVtx = 1,
    /// AMD-V / SVM is available.
    AmdV = 2,
}

/// Backend-agnostic description of the host's virtualization capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareVirtualization {
    backend: HardwareVirtualizationBackend,
}

impl HardwareVirtualization {
    /// Detect the best hardware virtualization backend supported by the host.
    pub fn detect() -> Self {
        Self {
            backend: detect_backend(),
        }
    }

    /// Return the selected backend.
    pub const fn backend(self) -> HardwareVirtualizationBackend {
        self.backend
    }

    /// Return whether any supported hardware virtualization backend is present.
    pub const fn is_available(self) -> bool {
        !matches!(self.backend, HardwareVirtualizationBackend::Unavailable)
    }

    /// Return a short user-facing name for the detected backend.
    pub const fn backend_name(self) -> &'static str {
        match self.backend {
            HardwareVirtualizationBackend::Unavailable => "unavailable",
            HardwareVirtualizationBackend::IntelVtx => "intel-vt-x",
            HardwareVirtualizationBackend::AmdV => "amd-v",
        }
    }
}

/// Detect host hardware virtualization support.
pub fn detect_hardware_virtualization() -> HardwareVirtualization {
    HardwareVirtualization::detect()
}

#[cfg(target_arch = "x86")]
fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let res = unsafe { core::arch::x86::__cpuid_count(leaf, subleaf) };
    (res.eax, res.ebx, res.ecx, res.edx)
}

#[cfg(target_arch = "x86_64")]
fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let res = unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) };
    (res.eax, res.ebx, res.ecx, res.edx)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_backend() -> HardwareVirtualizationBackend {
    const VENDOR_INTEL: [u8; 12] = *b"GenuineIntel";
    const VENDOR_AMD: [u8; 12] = *b"AuthenticAMD";

    let (max_basic_leaf, ebx0, ecx0, edx0) = cpuid(0, 0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&ebx0.to_le_bytes());
    vendor[4..8].copy_from_slice(&edx0.to_le_bytes());
    vendor[8..12].copy_from_slice(&ecx0.to_le_bytes());

    if vendor == VENDOR_INTEL && max_basic_leaf >= 1 {
        let (_, _, ecx1, _) = cpuid(1, 0);
        if (ecx1 & (1 << 5)) != 0 {
            return HardwareVirtualizationBackend::IntelVtx;
        }
    }

    let (max_extended_leaf, _, _, _) = cpuid(0x8000_0000, 0);
    if vendor == VENDOR_AMD && max_extended_leaf >= 0x8000_0001 {
        let (_, _, ecx_ext, _) = cpuid(0x8000_0001, 0);
        if (ecx_ext & (1 << 2)) != 0 {
            return HardwareVirtualizationBackend::AmdV;
        }
    }

    HardwareVirtualizationBackend::Unavailable
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn detect_backend() -> HardwareVirtualizationBackend {
    HardwareVirtualizationBackend::Unavailable
}
