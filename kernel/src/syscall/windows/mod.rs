//! wxe Windows x86_64 NT syscall dispatch.
//!
//! This is the first ABI skeleton for WXE. The actual PE loader and WXE
//! `ntdll.dll` will own the service-number profile; until services are wired,
//! every call returns `STATUS_NOT_IMPLEMENTED` using Windows NTSTATUS semantics.

use super::SyscallRegs;

pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax as u32;
    let a1 = regs.r10;
    let a2 = regs.rdx;
    let a3 = regs.r8;
    let a4 = regs.r9;

    crate::serial_verbose_println!(
        "wxe nt: unsupported service nr={:#x} rip={:#x} rsp={:#x} args={:#x},{:#x},{:#x},{:#x}",
        nr,
        regs.rip,
        regs.rsp,
        a1,
        a2,
        a3,
        a4
    );

    STATUS_NOT_IMPLEMENTED as u64
}
