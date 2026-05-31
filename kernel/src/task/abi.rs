//! User process ABI personality.
//!
//! The syscall entry code is shared, but processes can opt into a different
//! userspace ABI. Native anyOS programs keep the existing syscall numbering and
//! register convention; lxe processes use the Linux x86_64 syscall ABI; wxe
//! processes use the Windows x86_64 NT syscall ABI profile exposed by WXE's
//! own `ntdll.dll`.

pub const LINUX_MAX_USER_PRIORITY: u8 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiPersonality {
    AnyOs,
    LinuxX86_64,
    WindowsX86_64,
}

impl AbiPersonality {
    pub const fn event_tag(self) -> u32 {
        match self {
            AbiPersonality::AnyOs => 0,
            AbiPersonality::LinuxX86_64 => 1,
            AbiPersonality::WindowsX86_64 => 2,
        }
    }

    /// Inverse of [`event_tag`]; any unknown tag decodes to `AnyOs` (the safe
    /// default — native dispatch). Used to read back the lock-free per-CPU ABI
    /// cache maintained by the scheduler.
    pub const fn from_event_tag(tag: u32) -> Self {
        match tag {
            1 => AbiPersonality::LinuxX86_64,
            2 => AbiPersonality::WindowsX86_64,
            _ => AbiPersonality::AnyOs,
        }
    }
}
