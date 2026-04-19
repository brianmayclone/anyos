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
use crate::arch::x86::port::{inl, inw, outw};

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
/// Covers ACPI 1.0 + 2.0 fields needed for P-states, C-states, sleep, and reset.
#[derive(Debug, Clone, Copy)]
pub struct Fadt {
    /// Legacy 32-bit DSDT physical address.
    pub dsdt: u32,
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
    /// FADT table length (from SDT header). Used to check ACPI 2.0+ availability.
    pub table_length: u32,
    /// ACPI 2.0+ RESET_REG: address space ID (0 = system memory, 1 = system I/O).
    pub reset_reg_addr_space: u8,
    /// ACPI 2.0+ RESET_REG: register address (port or MMIO address).
    pub reset_reg_address: u64,
    /// ACPI 2.0+ RESET_VALUE: value to write to trigger platform reset.
    pub reset_value: u8,
    /// ACPI 2.0+ X_DSDT physical address (0 if unavailable).
    pub x_dsdt: u64,
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

    // Read table length from the SDT header (offset 4) for ACPI 2.0+ field bounds check.
    let table_length   = ru32!(4);

    let dsdt           = ru32!(40);
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

    // ACPI 2.0+ layout:
    //   [116..128) RESET_REG GAS
    //   [128]      RESET_VALUE
    //   [140..148) X_DSDT
    let (reset_reg_addr_space, reset_reg_address, reset_value, x_dsdt) = if table_length >= 148 {
        macro_rules! ru64 {
            ($off:expr) => { unsafe { (b.add($off) as *const u64).read_unaligned() } }
        }
        let addr_space = ru8!(116);
        let address    = ru64!(120);
        let value      = ru8!(128);
        let x_dsdt     = ru64!(140);
        crate::serial_verbose_println!(
            "  ACPI PM: RESET_REG addr_space={} address={:#010x} value={:#04x}",
            addr_space, address, value
        );
        (addr_space, address, value, x_dsdt)
    } else {
        (0, 0u64, 0u8, 0u64)
    };

    Some(Fadt {
        dsdt,
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
        table_length,
        reset_reg_addr_space,
        reset_reg_address,
        reset_value,
        x_dsdt,
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

pub fn request_sleep_state(state: SleepState) {
    let fadt = match get_fadt() {
        Some(f) => f,
        None => {
            crate::serial_verbose_println!("  ACPI PM: request_sleep_state — no FADT");
            return;
        }
    };

    if state == SleepState::S0 {
        return;
    }

    enable_acpi_mode(&fadt);

    let (slp_typ_a, slp_typ_b) = match sleep_types_for_state(&fadt, state) {
        Some(v) => v,
        None => return,
    };

    crate::serial_verbose_println!(
        "  ACPI PM: requesting sleep state {:?} — SLP_TYPa={} SLP_TYPb={}",
        state, slp_typ_a, slp_typ_b
    );

    let port_a = fadt.pm1a_cnt_blk as u16;
    let current_a = unsafe { inw(port_a) };
    let value_a = (current_a & !((0x7 << 10) | (1 << 13))) | (slp_typ_a << 10);
    unsafe {
        outw(port_a, value_a);
        outw(port_a, value_a | (1 << 13));
    }

    // Optional: write to PM1b_CNT if it exists
    if fadt.pm1b_cnt_blk != 0 {
        let port_b = fadt.pm1b_cnt_blk as u16;
        let current_b = unsafe { inw(port_b) };
        let value_b = (current_b & !((0x7 << 10) | (1 << 13))) | (slp_typ_b << 10);
        unsafe {
            outw(port_b, value_b);
            outw(port_b, value_b | (1 << 13));
        }
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

fn parse_pkg_length(bytes: &[u8], offset: usize) -> Option<(usize, usize)> {
    let lead = *bytes.get(offset)?;
    let follow_count = (lead >> 6) as usize;
    if follow_count > 3 || offset + follow_count >= bytes.len() {
        return None;
    }

    let mut value = (lead & 0x0F) as usize;
    for i in 0..follow_count {
        let b = *bytes.get(offset + 1 + i)? as usize;
        value |= b << (4 + i * 8);
    }
    Some((value, 1 + follow_count))
}

fn parse_aml_integer(bytes: &[u8], offset: usize) -> Option<(u16, usize)> {
    let op = *bytes.get(offset)?;
    match op {
        0x00 => Some((0, 1)),
        0x01 => Some((1, 1)),
        0x0A => Some((*bytes.get(offset + 1)? as u16, 2)),
        0x0B => {
            let lo = *bytes.get(offset + 1)? as u16;
            let hi = *bytes.get(offset + 2)? as u16;
            Some((lo | (hi << 8), 3))
        }
        0x0C => {
            let b0 = *bytes.get(offset + 1)? as u32;
            let b1 = *bytes.get(offset + 2)? as u32;
            let b2 = *bytes.get(offset + 3)? as u32;
            let b3 = *bytes.get(offset + 4)? as u32;
            Some((((b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) & 0xFFFF) as u16, 5))
        }
        0x0E => {
            let mut value = 0u64;
            for i in 0..8 {
                value |= (*bytes.get(offset + 1 + i)? as u64) << (i * 8);
            }
            Some(((value & 0xFFFF) as u16, 9))
        }
        _ if op <= 0x3F => Some((op as u16, 1)),
        _ => None,
    }
}

fn parse_s5_sleep_types(dsdt_phys: u64) -> Option<(u16, u16)> {
    if dsdt_phys == 0 {
        return None;
    }

    let header_virt = pm_map(dsdt_phys as u32, 0x1000);
    let header = header_virt as *const AcpiSdtHeader;
    let sig = unsafe { core::ptr::addr_of!((*header).signature).read_unaligned() };
    let length = unsafe { core::ptr::addr_of!((*header).length).read_unaligned() } as usize;
    pm_unmap(1);

    if &sig != b"DSDT" || length < core::mem::size_of::<AcpiSdtHeader>() {
        return None;
    }

    let table_virt = pm_map(dsdt_phys as u32, length as u32);
    let table = unsafe { core::slice::from_raw_parts(table_virt as *const u8, length) };
    let body = &table[core::mem::size_of::<AcpiSdtHeader>()..];

    let mut result = None;
    for i in 0..body.len().saturating_sub(4) {
        if &body[i..i + 4] != b"_S5_" {
            continue;
        }

        let has_nameop = (i >= 1 && body[i - 1] == 0x08)
            || (i >= 2 && body[i - 2] == 0x08 && body[i - 1] == b'\\');
        if !has_nameop {
            continue;
        }

        let mut off = i + 4;
        if body.get(off) != Some(&0x12) {
            continue;
        }
        off += 1;

        let (_pkg_len, pkg_len_bytes) = match parse_pkg_length(body, off) {
            Some(v) => v,
            None => continue,
        };
        off += pkg_len_bytes;

        if body.get(off).is_none() {
            continue;
        }
        off += 1;

        let (slp_typ_a, used_a) = match parse_aml_integer(body, off) {
            Some(v) => v,
            None => continue,
        };
        off += used_a;
        let (slp_typ_b, _used_b) = match parse_aml_integer(body, off) {
            Some(v) => v,
            None => continue,
        };

        result = Some((slp_typ_a & 0x7, slp_typ_b & 0x7));
        break;
    }

    let pages = (length + 0xFFF) / 0x1000;
    pm_unmap(pages);
    result
}

fn sleep_types_for_state(fadt: &Fadt, state: SleepState) -> Option<(u16, u16)> {
    match state {
        SleepState::S0 => None,
        SleepState::S3 => Some((0b001, 0b001)),
        SleepState::S4 => Some((0b010, 0b010)),
        SleepState::S5 => {
            let dsdt_phys = if fadt.x_dsdt != 0 { fadt.x_dsdt } else { fadt.dsdt as u64 };
            parse_s5_sleep_types(dsdt_phys).or(Some((0b101, 0b101)))
        }
    }
}

fn enable_acpi_mode(fadt: &Fadt) {
    if fadt.pm1a_cnt_blk == 0 {
        return;
    }

    let port = fadt.pm1a_cnt_blk as u16;
    if unsafe { inw(port) } & 1 != 0 {
        return;
    }

    if fadt.smi_cmd == 0 || fadt.acpi_enable == 0 {
        return;
    }

    crate::serial_verbose_println!(
        "  ACPI PM: enabling ACPI mode via SMI_CMD {:#010x} value {:#04x}",
        fadt.smi_cmd,
        fadt.acpi_enable
    );
    unsafe { crate::arch::x86::port::outb(fadt.smi_cmd as u16, fadt.acpi_enable); }

    for _ in 0..1_000_000u32 {
        if unsafe { inw(port) } & 1 != 0 {
            crate::serial_verbose_println!("  ACPI PM: ACPI mode enabled");
            return;
        }
        core::hint::spin_loop();
    }

    crate::serial_verbose_println!("  ACPI PM: ACPI mode did not report SCI_EN");
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

    crate::serial_verbose_println!("  ACPI PM: trying port 0x4004 (VirtualBox)");
    unsafe { outw(0x4004, 0x3400); }

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
    let _ = fadt;
    request_sleep_state(SleepState::S5);
}

/// Attempt a platform reboot via the ACPI RESET_REG (FADT offset 128+).
///
/// Returns `true` if ACPI reset register was found and written (caller should
/// spin briefly and fall through if it didn't take effect).
/// Returns `false` if no usable ACPI reset register is available.
pub fn acpi_reboot() -> bool {
    let fadt = match get_fadt() {
        Some(f) => f,
        None => return false,
    };
    // ACPI 2.0+ RESET_REG must have a non-zero address
    if fadt.reset_reg_address == 0 || fadt.reset_value == 0 {
        return false;
    }
    match fadt.reset_reg_addr_space {
        1 => {
            // System I/O space
            let port = fadt.reset_reg_address as u16;
            crate::serial_println!("kernel: ACPI reboot via I/O port {:#06x} value {:#04x}",
                port, fadt.reset_value);
            unsafe { crate::arch::x86::port::outb(port, fadt.reset_value); }
            true
        }
        0 => {
            // System Memory space
            let addr = fadt.reset_reg_address;
            crate::serial_println!("kernel: ACPI reboot via MMIO {:#010x} value {:#04x}",
                addr, fadt.reset_value);
            unsafe { (addr as *mut u8).write_volatile(fadt.reset_value); }
            true
        }
        _ => false,
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
