/// PAT (Page Attribute Table) MSR programming for write-combining support.
///
/// Reprograms the PAT MSR to place Write-Combining (WC) in PAT entry 1,
/// enabling WC for framebuffer pages by setting PWT=1 in their PTEs.
///
/// Default PAT layout:
///   PAT0=WB(06), PAT1=WT(04), PAT2=UC-(07), PAT3=UC(00)
///   PAT4=WB(06), PAT5=WT(04), PAT6=UC-(07), PAT7=UC(00)
///
/// Reprogrammed layout:
///   PAT0=WB(06), PAT1=WC(01), PAT2=UC-(07), PAT3=UC(00)
///   PAT4=WB(06), PAT5=WT(04), PAT6=UC-(07), PAT7=UC(00)
///
/// PTE bit encoding to select PAT1: PWT=1, PCD=0, PAT=0 → bit 3 in PTE.

const PAT_MSR: u32 = 0x277;

/// PAT MSR value with PAT1 = Write-Combining (0x01).
const PAT_VALUE: u64 = 0x00070406_00070106;

/// Program the PAT MSR on the current CPU.
///
/// Must be called on BSP before `virtual_mem::init()` maps the framebuffer,
/// and on each AP during startup so all CPUs agree on memory types.
pub fn init() {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") PAT_MSR,
            in("eax") (PAT_VALUE & 0xFFFF_FFFF) as u32,
            in("edx") ((PAT_VALUE >> 32) & 0xFFFF_FFFF) as u32,
            options(nomem, nostack, preserves_flags),
        );
    }
    crate::serial_println!("[OK] PAT programmed (PAT1=WC)");
}
