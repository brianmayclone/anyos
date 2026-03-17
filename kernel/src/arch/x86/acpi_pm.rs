//! ACPI Power Management support.
//!
//! Parses the Fixed ACPI Description Table (FADT / "FACP") from the RSDT to
//! obtain PM1 event/control block addresses and SMI command port.
//!
//! Provides:
//! - C-state entry helpers (C1 via HLT, C2 via P_LVL2 port read).
//! - Sleep state request (SLP_TYP + SLP_EN written to PM1a_CNT).
//! - P-state frequency ratio read/write via IA32_PERF_CTL / IA32_PERF_STATUS
//!   (Intel) or MSR_AMD_PERF_CTL / MSR_AMD_PERF_STATUS (AMD).
//! - `shutdown()` for S5 power-off with QEMU/Bochs fallback.
//!
//! The ACPI tables are mapped using a private virtual window at
//! `0xFFFF_FFFF_D030_0000` (64 pages = 256 KiB) separate from the main ACPI
//! window used by `crate::arch::x86::acpi` to avoid interference.

use crate::sync::spinlock::Spinlock;
use crate::arch::x86::port::{inl, outl, inw, outw};

// ── Private Virtual Window ────────────────────────────────────────────────────

/// Virtual base address for ACPI PM table mapping (256 KiB = 64 pages).
/// Distinct from the ACPI MADT window at 0xFFFF_FFFF_D020_0000.
const ACPI_PM_MAP_BASE: u64 = 0xFFFF_FFFF_D030_0000;
const ACPI_PM_MAP_PAGES: usize = 64;

/// Map a physical address into our private ACPI PM virtual window.
/// Returns the virtual address corresponding to `phys_addr`.
fn pm_map(phys_addr: u32, size: u32) -> u64 {
    use crate::memory::address::{PhysAddr, VirtAddr};
    use crate::memory::virtual_mem;

    let phys = phys_addr as u64;
    let page_start = phys & !0xFFF;
    let page_end = (phys + size as u64 + 0xFFF) & !0xFFF;
    let num_pages = ((page_end - page_start) / 0x1000) as usize;

    for i in 0..num_pages.min(ACPI_PM_MAP_PAGES) {
        let virt = ACPI_PM_MAP_BASE + (i as u64) * 0x1000;
        let p = page_start + (i as u64) * 0x1000;
        // Safety: we are mapping physical ACPI firmware tables as read-only.
        virtual_mem::map_page(
            VirtAddr::new(virt),
            PhysAddr::new(p),
            0x01, // PAGE_PRESENT | read-only
        );
    }

    ACPI_PM_MAP_BASE + (phys - page_start)
}

/// Unmap pages from the ACPI PM virtual window.
fn pm_unmap(num_pages: usize) {
    use crate::memory::address::VirtAddr;
    use crate::memory::virtual_mem;

    for i in 0..num_pages.min(ACPI_PM_MAP_PAGES) {
        let virt = ACPI_PM_MAP_BASE + (i as u64) * 0x1000;
        virtual_mem::unmap_page(VirtAddr::new(virt));
    }
}

// ── ACPI Table Structures ─────────────────────────────────────────────────────

/// RSDP — Root System Description Pointer.
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

/// Generic ACPI SDT header (36 bytes).
#[repr(C, packed)]
struct AcpiSdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// FADT fields extracted from the ACPI table.
/// Covers ACPI 1.0 + 2.0 fields needed for P-states, C-states, and sleep.
#[derive(Debug, Clone, Copy)]
pub struct Fadt {
    /// SCI interrupt vector (x86 IRQ line).
    pub sci_interrupt: u16,
    /// SMI command port (write `acpi_enable` here to take ACPI ownership).
    pub smi_cmd: u32,
    /// Value to write to `smi_cmd` to enable ACPI mode.
    pub acpi_enable: u8,
    /// Value to write to `smi_cmd` to disable ACPI mode.
    pub acpi_disable: u8,
    /// PM1a Event Block port address.
    pub pm1a_evt_blk: u32,
    /// PM1b Event Block port address (0 if not present).
    pub pm1b_evt_blk: u32,
    /// PM1a Control Block port address.
    pub pm1a_cnt_blk: u32,
    /// PM1b Control Block port address (0 if not present).
    pub pm1b_cnt_blk: u32,
    /// PM2 Control Block port address (0 if not present).
    pub pm2_cnt_blk: u32,
    /// Power Management Timer port address.
    pub pm_tmr_blk: u32,
    /// GPE0 Block port address.
    pub gpe0_blk: u32,
    /// GPE1 Block port address.
    pub gpe1_blk: u32,
    /// PM1 event block register length in bytes.
    pub pm1_evt_len: u8,
    /// PM1 control block register length in bytes.
    pub pm1_cnt_len: u8,
    /// PM timer register length in bytes.
    pub pm_tmr_len: u8,
    /// Worst-case C2 latency in microseconds.
    pub p_lvl2_lat: u16,
    /// Worst-case C3 latency in microseconds.
    pub p_lvl3_lat: u16,
    /// Duty cycle offset (throttling control).
    pub duty_offset: u8,
    /// Duty cycle width bits.
    pub duty_width: u8,
    /// IA-PC Boot Architecture flags.
    pub iapc_boot_arch: u16,
    /// FADT feature flags.
    pub flags: u32,
}

// ── Global State ─────────────────────────────────────────────────────────────

/// Cached FADT, populated by `init()`.
static FADT: Spinlock<Option<Fadt>> = Spinlock::new(None);

// ── Initialization ────────────────────────────────────────────────────────────

/// Parse the RSDP → RSDT → FADT and cache the FADT fields.
///
/// `rsdp_hint`: physical address of the RSDP as provided by the bootloader
/// (UEFI/BIOS).  If zero, we scan the BIOS ROM area.
pub fn init(rsdp_hint: u32) {
    match parse_fadt(rsdp_hint) {
        Some(fadt) => {
            crate::serial_verbose_println!(
                "[OK] ACPI PM: FADT parsed — PM1a_CNT={:#010x} PM_TMR={:#010x} SCI_INT={}",
                fadt.pm1a_cnt_blk, fadt.pm_tmr_blk, fadt.sci_interrupt
            );
            *FADT.lock() = Some(fadt);
        }
        None => {
            crate::serial_verbose_println!("  ACPI PM: FADT not found or parse failed");
        }
    }
}

/// Get a copy of the cached FADT.
pub fn get_fadt() -> Option<Fadt> {
    *FADT.lock()
}

// ── FADT Parser ───────────────────────────────────────────────────────────────

fn parse_fadt(rsdp_hint: u32) -> Option<Fadt> {
    let rsdp_virt = find_rsdp_virt(rsdp_hint)?;

    let rsdt_phys = unsafe {
        core::ptr::addr_of!((*(rsdp_virt as *const Rsdp)).rsdt_address).read_unaligned()
    };
    crate::serial_verbose_println!("  ACPI PM: RSDP found, RSDT at {:#010x}", rsdt_phys);
    pm_unmap(1);

    // Map RSDT
    let rsdt_virt = pm_map(rsdt_phys, 0x4000);
    let rsdt = rsdt_virt as *const AcpiSdtHeader;
    let sig = unsafe { core::ptr::addr_of!((*rsdt).signature).read_unaligned() };
    if &sig != b"RSDT" {
        crate::serial_verbose_println!("  ACPI PM: RSDT signature mismatch");
        pm_unmap(16);
        return None;
    }

    let rsdt_len = unsafe { core::ptr::addr_of!((*rsdt).length).read_unaligned() };
    let header_sz = core::mem::size_of::<AcpiSdtHeader>() as u32;
    let num_entries = ((rsdt_len.saturating_sub(header_sz)) / 4) as usize;

    // Pre-read all RSDT entry addresses before remapping
    let entries_base = (rsdt_virt + header_sz as u64) as *const u32;
    let mut table_addrs = [0u32; 32];
    let count = num_entries.min(table_addrs.len());
    for i in 0..count {
        table_addrs[i] = unsafe { entries_base.add(i).read_unaligned() };
    }
    pm_unmap(16);

    for i in 0..count {
        let table_phys = table_addrs[i];
        if table_phys == 0 { continue; }

        let tbl_virt = pm_map(table_phys, 0x1000);
        let tbl = tbl_virt as *const AcpiSdtHeader;
        let tbl_sig = unsafe { core::ptr::addr_of!((*tbl).signature).read_unaligned() };
        pm_unmap(1);

        if &tbl_sig == b"FACP" {
            // Found the FADT; remap with enough pages for the full table
            let full_virt = pm_map(table_phys, 0x400);
            let fadt = parse_fadt_table(full_virt);
            pm_unmap(4);
            return fadt;
        }
    }

    crate::serial_verbose_println!("  ACPI PM: FADT (FACP) table not found in RSDT");
    None
}

/// Locate the RSDP and return its virtual address in our PM mapping window.
fn find_rsdp_virt(hint: u32) -> Option<u64> {
    const RSDP_SIG: [u8; 8] = *b"RSD PTR ";

    if hint != 0 {
        let virt = pm_map(hint, core::mem::size_of::<Rsdp>() as u32);
        let sig = unsafe { core::ptr::addr_of!((*(virt as *const Rsdp)).signature).read_unaligned() };
        if sig == RSDP_SIG && validate_rsdp(virt as *const Rsdp) {
            return Some(virt);
        }
        pm_unmap(1);
        crate::serial_verbose_println!("  ACPI PM: bootloader RSDP hint invalid, scanning");
    }

    // Scan BIOS ROM 0xE0000–0xFFFFF (identity-mapped in the kernel)
    let mut addr = 0x000E_0000usize;
    while addr < 0x0010_0000 {
        let ptr = addr as *const [u8; 8];
        let sig = unsafe { ptr.read() };
        if sig == RSDP_SIG {
            let rsdp = addr as *const Rsdp;
            if validate_rsdp(rsdp) {
                return Some(addr as u64);
            }
        }
        addr += 16;
    }

    None
}

fn validate_rsdp(rsdp: *const Rsdp) -> bool {
    let bytes = rsdp as *const u8;
    let mut sum: u8 = 0;
    for i in 0..20 {
        sum = sum.wrapping_add(unsafe { bytes.add(i).read() });
    }
    sum == 0
}

/// Parse a mapped FADT virtual address and extract the relevant fields.
fn parse_fadt_table(virt: u64) -> Option<Fadt> {
    // FADT layout (ACPI spec, all offsets from start of table header):
    //   [0..36]   AcpiSdtHeader
    //   [36..40]  FIRMWARE_CTRL (physical address of FACS)
    //   [40..44]  DSDT physical address
    //   [44]      Reserved (ACPI 1.0: INT_MODEL)
    //   [45]      PREFERRED_PM_PROFILE
    //   [46..48]  SCI_INT
    //   [48..52]  SMI_CMD
    //   [52]      ACPI_ENABLE
    //   [53]      ACPI_DISABLE
    //   [54]      S4BIOS_REQ
    //   [55]      PSTATE_CNT
    //   [56..60]  PM1a_EVT_BLK
    //   [60..64]  PM1b_EVT_BLK
    //   [64..68]  PM1a_CNT_BLK
    //   [68..72]  PM1b_CNT_BLK
    //   [72..76]  PM2_CNT_BLK
    //   [76..80]  PM_TMR_BLK
    //   [80..84]  GPE0_BLK
    //   [84..88]  GPE1_BLK
    //   [88]      PM1_EVT_LEN
    //   [89]      PM1_CNT_LEN
    //   [90]      PM2_CNT_LEN
    //   [91]      PM_TMR_LEN
    //   [92]      GPE0_BLK_LEN
    //   [93]      GPE1_BLK_LEN
    //   [94]      GPE1_BASE
    //   [95]      CST_CNT
    //   [96..98]  P_LVL2_LAT
    //   [98..100] P_LVL3_LAT
    //   [100..102]FLUSH_SIZE
    //   [102..104]FLUSH_STRIDE
    //   [104]     DUTY_OFFSET
    //   [105]     DUTY_WIDTH
    //   [106]     DAY_ALRM
    //   [107]     MON_ALRM
    //   [108]     CENTURY
    //   [109..111]IAPC_BOOT_ARCH
    //   [111]     Reserved
    //   [112..116]FLAGS

    let b = virt as *const u8;

    // Helper: read a u32 at byte offset from table start
    macro_rules! ru32 {
        ($off:expr) => { unsafe { (b.add($off) as *const u32).read_unaligned() } }
    }
    macro_rules! ru16 {
        ($off:expr) => { unsafe { (b.add($off) as *const u16).read_unaligned() } }
    }
    macro_rules! ru8 {
        ($off:expr) => { unsafe { b.add($off).read() } }
    }

    let sci_interrupt  = ru16!(46);
    let smi_cmd        = ru32!(48);
    let acpi_enable    = ru8!(52);
    let acpi_disable   = ru8!(53);
    let pm1a_evt_blk   = ru32!(56);
    let pm1b_evt_blk   = ru32!(60);
    let pm1a_cnt_blk   = ru32!(64);
    let pm1b_cnt_blk   = ru32!(68);
    let pm2_cnt_blk    = ru32!(72);
    let pm_tmr_blk     = ru32!(76);
    let gpe0_blk       = ru32!(80);
    let gpe1_blk       = ru32!(84);
    let pm1_evt_len    = ru8!(88);
    let pm1_cnt_len    = ru8!(89);
    let pm_tmr_len     = ru8!(91);
    let p_lvl2_lat     = ru16!(96);
    let p_lvl3_lat     = ru16!(98);
    let duty_offset    = ru8!(104);
    let duty_width     = ru8!(105);
    let iapc_boot_arch = ru16!(109);
    let flags          = ru32!(112);

    Some(Fadt {
        sci_interrupt,
        smi_cmd,
        acpi_enable,
        acpi_disable,
        pm1a_evt_blk,
        pm1b_evt_blk,
        pm1a_cnt_blk,
        pm1b_cnt_blk,
        pm2_cnt_blk,
        pm_tmr_blk,
        gpe0_blk,
        gpe1_blk,
        pm1_evt_len,
        pm1_cnt_len,
        pm_tmr_len,
        p_lvl2_lat,
        p_lvl3_lat,
        duty_offset,
        duty_width,
        iapc_boot_arch,
        flags,
    })
}

// ── C-State Helpers ───────────────────────────────────────────────────────────

/// Enter CPU C1 idle state by executing HLT.
/// The CPU resumes on any enabled interrupt.
#[inline]
pub fn enter_c1() {
    // Safety: HLT is safe here; the kernel enables interrupts before calling
    // the scheduler idle path that uses this function.
    unsafe {
        core::arch::asm!("hlt", options(nostack, preserves_flags, nomem));
    }
}

/// Enter CPU C2 idle state by reading the P_LVL2 port from the FADT.
/// The hardware transitions to C2 upon the read; the CPU resumes on an
/// interrupt or SMI.
pub fn enter_c2(fadt: &Fadt) {
    if fadt.p_lvl2_lat > 100 {
        // Latency too high; fall back to C1 to avoid hurting responsiveness.
        enter_c1();
        return;
    }
    let lvl2_port = (fadt.pm_tmr_blk + 8) as u16;
    // Safety: reading from a PM I/O port causes a C2 state entry on ICH hardware.
    // Safety: reading the P_LVL2 I/O port triggers a C2 state entry.
    let _dummy = unsafe { inl(lvl2_port) };
    let _ = _dummy;
}

/// Request a system sleep state by writing SLP_TYP + SLP_EN to PM1a_CNT.
///
/// The SLP_TYP values used here are per the ACPI spec defaults:
/// - S0 = no-op (normal running)
/// - S3 = suspend to RAM (SLP_TYP = 0b001)
/// - S4 = suspend to disk (SLP_TYP = 0b010)
/// - S5 = power off (SLP_TYP = 0b101)
///
/// Note: In a real system the _S3/_S4/_S5 AML objects in the DSDT define the
/// actual SLP_TYP values; we use the ACPI spec defaults as a reasonable
/// approximation without AML evaluation.
pub fn request_sleep_state(state: SleepState) {
    let fadt = match get_fadt() {
        Some(f) => f,
        None => {
            crate::serial_verbose_println!("  ACPI PM: request_sleep_state — no FADT");
            return;
        }
    };

    let slp_typ: u16 = match state {
        SleepState::S0 => return, // Already in S0
        SleepState::S3 => 0b001,
        SleepState::S4 => 0b010,
        SleepState::S5 => 0b101,
    };

    // PM1 Control register layout:
    //   bits [12:10] = SLP_TYP
    //   bit  [13]    = SLP_EN (write-only; triggers state change)
    let value: u16 = (slp_typ << 10) | (1 << 13); // SLP_TYP + SLP_EN

    crate::serial_verbose_println!(
        "  ACPI PM: requesting sleep state {:?} — writing {:#06x} to PM1a_CNT {:#010x}",
        state, value, fadt.pm1a_cnt_blk
    );

    let port_a = fadt.pm1a_cnt_blk as u16;
    // Safety: writing to the PM1 control port causes a hardware power state
    // transition on ACPI-capable systems. This is the correct way to power off.
    unsafe { outw(port_a, value); }

    // Optional: write to PM1b_CNT if it exists
    if fadt.pm1b_cnt_blk != 0 {
        let port_b = fadt.pm1b_cnt_blk as u16;
        unsafe { outw(port_b, value); }
    }
}

/// Desired sleep / power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepState {
    /// S0 — Normal running (no action).
    S0,
    /// S3 — Suspend to RAM.
    S3,
    /// S4 — Suspend to disk (hibernate).
    S4,
    /// S5 — Soft power off.
    S5,
}

// ── P-State (Frequency Scaling) ───────────────────────────────────────────────

// Intel SpeedStep MSRs
const IA32_PERF_CTL:    u32 = 0x199;
const IA32_PERF_STATUS: u32 = 0x198;

// AMD PowerNow MSRs
const MSR_AMD_PERF_CTL:    u32 = 0xC001_0062;
const MSR_AMD_PERF_STATUS: u32 = 0xC001_0063;

/// Set the CPU frequency ratio.
///
/// - Intel: writes `ratio` to bits [15:8] of `IA32_PERF_CTL`.
/// - AMD:   writes `ratio & 0x07` to bits [2:0] of `MSR_AMD_PERF_CTL`
///           (P-state index 0–7).
pub fn set_perf_level(ratio: u8) {
    use crate::drivers::thermal::cpu_vendor;
    use crate::drivers::thermal::CpuVendor;

    match cpu_vendor() {
        CpuVendor::Intel => {
            let val = (ratio as u64) << 8;
            // Safety: writing IA32_PERF_CTL adjusts CPU frequency; valid on Intel CPUs
            // with SpeedStep (CPUID 6 EAX bit 5). The power driver already checked this.
            unsafe { wrmsr(IA32_PERF_CTL, val); }
        }
        CpuVendor::Amd => {
            let pstate_idx = (ratio & 0x07) as u64;
            unsafe { wrmsr(MSR_AMD_PERF_CTL, pstate_idx); }
        }
        CpuVendor::Unknown => {
            crate::serial_verbose_println!("  ACPI PM: set_perf_level — unknown CPU vendor");
        }
    }
}

/// Read the current CPU frequency ratio.
///
/// - Intel: reads bits [15:8] of `IA32_PERF_STATUS`.
/// - AMD:   reads bits [2:0] of `MSR_AMD_PERF_STATUS` (current P-state index).
///
/// Returns the raw ratio/index byte.
pub fn get_perf_level() -> u8 {
    use crate::drivers::thermal::cpu_vendor;
    use crate::drivers::thermal::CpuVendor;

    match cpu_vendor() {
        CpuVendor::Intel => {
            let val = unsafe { rdmsr(IA32_PERF_STATUS) };
            ((val >> 8) & 0xFF) as u8
        }
        CpuVendor::Amd => {
            let val = unsafe { rdmsr(MSR_AMD_PERF_STATUS) };
            (val & 0x07) as u8
        }
        CpuVendor::Unknown => 0,
    }
}

/// Approximate CPU frequency from a ratio value (Intel-style: ratio × 100 MHz).
///
/// For AMD this is less meaningful (the ratio is a P-state index, not a
/// multiplier), but we return `ratio * 100` as a consistent approximation.
pub fn get_perf_ratio_mhz(ratio: u8) -> u32 {
    (ratio as u32) * 100
}

// ── Shutdown / Power-Off ──────────────────────────────────────────────────────

/// ACPI power-off: write S5 sleep state to PM1a_CNT, with emulator fallbacks.
///
/// Fallback sequence if FADT is unavailable:
/// 1. QEMU "isa-debug-exit" or ACPI shutdown port 0x604.
/// 2. Bochs ACPI shutdown port 0xB004.
/// 3. Spin-loop halt.
pub fn shutdown() {
    crate::serial_verbose_println!("  ACPI PM: shutdown() initiated");

    if get_fadt().is_some() {
        // Use ACPI S5 via the FADT
        request_sleep_state(SleepState::S5);

        // Allow a brief moment for the hardware to process the write before falling through
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
        crate::serial_verbose_println!("  ACPI PM: S5 write did not take effect, trying fallback ports");
    }

    // Fallback: QEMU ACPI shutdown port (also works on many virtio/KVM setups)
    crate::serial_verbose_println!("  ACPI PM: trying port 0x604 (QEMU)");
    unsafe { outw(0x604, 0x2000); }

    // Fallback: Bochs ACPI shutdown
    crate::serial_verbose_println!("  ACPI PM: trying port 0xB004 (Bochs)");
    unsafe { outw(0xB004, 0x2000); }

    // Nothing worked; halt indefinitely
    crate::serial_verbose_println!("  ACPI PM: shutdown fallback exhausted, halting");
    loop {
        crate::arch::hal::halt();
    }
}

/// Power off via ACPI given an explicit FADT reference (used by callers that
/// already have a validated FADT without re-locking FADT global).
pub fn acpi_poweroff(fadt: &Fadt) {
    let slp_typ: u16 = 0b101; // S5 SLP_TYP (ACPI default)
    let value: u16 = (slp_typ << 10) | (1 << 13);

    unsafe {
        outw(fadt.pm1a_cnt_blk as u16, value);
        if fadt.pm1b_cnt_blk != 0 {
            outw(fadt.pm1b_cnt_blk as u16, value);
        }
    }
}

// ── MSR Helpers ───────────────────────────────────────────────────────────────

/// Read a Model Specific Register.
///
/// # Safety
/// RDMSR with an unsupported MSR causes a #GP fault. Callers must guard with
/// CPUID capability checks before invoking.
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, preserves_flags),
    );
    ((hi as u64) << 32) | (lo as u64)
}

/// Write a Model Specific Register.
///
/// # Safety
/// WRMSR with an unsupported MSR causes a #GP fault. Writing incorrect values
/// can affect CPU frequency, voltage, and power state.
#[inline]
unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, preserves_flags),
    );
}
