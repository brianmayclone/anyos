//! User process ABI personality.
//!
//! The syscall entry code is shared, but processes can opt into a different
//! userspace ABI. Native anyOS programs keep the existing syscall numbering and
//! register convention; lxe processes use the Linux x86_64 syscall ABI.

pub const LINUX_MAX_USER_PRIORITY: u8 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiPersonality {
    AnyOs,
    LinuxX86_64,
}
