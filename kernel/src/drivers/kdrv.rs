//! Loadable kernel driver system (.ddv bundles).
//!
//! Scans `/System/Drivers/{category}/` for `.ddv` bundles, parses their Info.conf
//! to match vendor/device IDs or class/subclass against **unbound** PCI devices,
//! and only loads matching KDRV binaries into kernel address space.
//!
//! ## Security
//! The `exec` binary must reside directly in the `.ddv` directory — subdirectory
//! traversal (`/`, `..`) in the exec field is rejected.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::drivers::hal::{self, Driver, DriverType, DriverError};
use crate::drivers::pci::PciDevice;

// ──────────────────────────────────────────────
// KDRV binary format
// ──────────────────────────────────────────────

/// Magic bytes at offset 0: "KDRV"
const KDRV_MAGIC: [u8; 4] = [b'K', b'D', b'R', b'V'];

/// Current KDRV format version.
const KDRV_VERSION: u32 = 1;

/// Current kernel ABI version that external drivers must match.
const KDRV_ABI_VERSION: u32 = 1;

/// KDRV header (first 4096 bytes of binary).
#[repr(C)]
struct KdrvHeader {
    magic: [u8; 4],       // "KDRV"
    version: u32,         // Format version
    abi_version: u32,     // Kernel ABI compatibility
    _reserved0: u32,
    exports_offset: u64,  // Byte offset from load base to DriverExports
    code_pages: u32,      // Number of read-only code pages
    data_pages: u32,      // Number of read-write data pages
    bss_pages: u32,       // Number of zeroed read-write BSS pages
    _reserved1: [u8; 4056], // Pad to 4096 bytes
}

/// Virtual address range for loaded external drivers: 0xFFFF_FFFF_B000_0000 — BFE0_0000
const KDRV_LOAD_BASE: u64 = 0xFFFF_FFFF_B000_0000;
const KDRV_LOAD_END: u64  = 0xFFFF_FFFF_BFE0_0000;
const PAGE_SIZE: u64 = 4096;

/// Next available virtual address for KDRV loading.
static NEXT_KDRV_VA: crate::sync::spinlock::Spinlock<u64> =
    crate::sync::spinlock::Spinlock::new(KDRV_LOAD_BASE);

// ──────────────────────────────────────────────
// Kernel ↔ Driver C ABI
// ──────────────────────────────────────────────

/// PCI device info passed to driver init (C ABI).
#[repr(C)]
pub struct PciDeviceC {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub _pad: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
    pub _pad2: [u8; 2],
    pub bars: [u32; 6],
}

impl From<&PciDevice> for PciDeviceC {
    fn from(dev: &PciDevice) -> Self {
        PciDeviceC {
            bus: dev.bus,
            device: dev.device,
            function: dev.function,
            _pad: 0,
            vendor_id: dev.vendor_id,
            device_id: dev.device_id,
            class_code: dev.class_code,
            subclass: dev.subclass,
            prog_if: dev.prog_if,
            revision_id: dev.revision_id,
            interrupt_line: dev.interrupt_line,
            interrupt_pin: dev.interrupt_pin,
            _pad2: [0; 2],
            bars: dev.bars,
        }
    }
}

/// Function table provided by the kernel to external drivers.
#[repr(C)]
pub struct KernelDriverApi {
    pub api_version: u32,
    // PCI config space
    pub pci_config_read32: extern "C" fn(bus: u8, dev: u8, func: u8, off: u8) -> u32,
    pub pci_config_write32: extern "C" fn(bus: u8, dev: u8, func: u8, off: u8, val: u32),
    pub pci_enable_bus_master: extern "C" fn(bus: u8, dev: u8, func: u8),
    // Physical memory
    pub alloc_frame: extern "C" fn() -> u64,
    pub free_frame: extern "C" fn(phys: u64),
    pub map_mmio: extern "C" fn(phys: u64, pages: u32) -> u64,
    // I/O ports
    pub inb: extern "C" fn(port: u16) -> u8,
    pub outb: extern "C" fn(port: u16, val: u8),
    pub inw: extern "C" fn(port: u16) -> u16,
    pub outw: extern "C" fn(port: u16, val: u16),
    pub inl: extern "C" fn(port: u16) -> u32,
    pub outl: extern "C" fn(port: u16, val: u32),
    // Logging + timing
    pub log: extern "C" fn(msg: *const u8, len: u32),
    pub delay_ms: extern "C" fn(ms: u32),
}

// ── API wrapper functions (extern "C" for driver calls) ──

extern "C" fn api_pci_config_read32(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    crate::drivers::pci::pci_config_read32(bus, dev, func, off)
}
extern "C" fn api_pci_config_write32(bus: u8, dev: u8, func: u8, off: u8, val: u32) {
    crate::drivers::pci::pci_config_write32(bus, dev, func, off, val);
}
extern "C" fn api_pci_enable_bus_master(bus: u8, dev: u8, func: u8) {
    let pci_dev = PciDevice {
        bus, device: dev, function: func,
        vendor_id: 0, device_id: 0,
        class_code: 0, subclass: 0, prog_if: 0, revision_id: 0,
        header_type: 0, interrupt_line: 0, interrupt_pin: 0,
        bars: [0; 6],
    };
    crate::drivers::pci::enable_bus_master(&pci_dev);
}
extern "C" fn api_alloc_frame() -> u64 {
    crate::memory::physical::alloc_frame().map(|f| f.as_u64()).unwrap_or(0)
}
extern "C" fn api_free_frame(phys: u64) {
    crate::memory::physical::free_frame(crate::memory::address::PhysAddr::new(phys));
}
extern "C" fn api_map_mmio(phys: u64, pages: u32) -> u64 {
    // Allocate MMIO virtual range from KDRV space
    // Note: In practice MMIO should map to dedicated MMIO VA range.
    // For external drivers we use the MMIO range starting at 0xFFFF_FFFF_D00A_0000
    static NEXT_MMIO: crate::sync::spinlock::Spinlock<u64> =
        crate::sync::spinlock::Spinlock::new(0xFFFF_FFFF_D00A_0000);

    let mut next = NEXT_MMIO.lock();
    let base = *next;
    let size = pages as u64 * PAGE_SIZE;
    *next += size;
    drop(next);

    use crate::memory::virtual_mem::map_page;
    use crate::memory::address::{VirtAddr, PhysAddr};
    const PG_PRESENT: u64 = 1;
    const PG_WRITABLE: u64 = 1 << 1;
    const PG_PWT: u64 = 1 << 3; // Write-through (uncacheable for MMIO)
    for i in 0..pages as u64 {
        let virt = VirtAddr::new(base + i * PAGE_SIZE);
        let p = PhysAddr::new(phys + i * PAGE_SIZE);
        map_page(virt, p, PG_PRESENT | PG_WRITABLE | PG_PWT);
    }
    base
}
extern "C" fn api_inb(port: u16) -> u8 {
    unsafe { crate::arch::x86::port::inb(port) }
}
extern "C" fn api_outb(port: u16, val: u8) {
    unsafe { crate::arch::x86::port::outb(port, val) }
}
extern "C" fn api_inw(port: u16) -> u16 {
    unsafe { crate::arch::x86::port::inw(port) }
}
extern "C" fn api_outw(port: u16, val: u16) {
    unsafe { crate::arch::x86::port::outw(port, val) }
}
extern "C" fn api_inl(port: u16) -> u32 {
    unsafe { crate::arch::x86::port::inl(port) }
}
extern "C" fn api_outl(port: u16, val: u32) {
    unsafe { crate::arch::x86::port::outl(port, val) }
}
extern "C" fn api_log(msg: *const u8, len: u32) {
    if msg.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(msg, len as usize) };
    if let Ok(s) = core::str::from_utf8(slice) {
        crate::serial_println!("  KDRV: {}", s);
    }
}
extern "C" fn api_delay_ms(ms: u32) {
    crate::arch::x86::pit::delay_ms(ms);
}

/// Static kernel API table passed to all external drivers.
static KERNEL_API: KernelDriverApi = KernelDriverApi {
    api_version: KDRV_ABI_VERSION,
    pci_config_read32: api_pci_config_read32,
    pci_config_write32: api_pci_config_write32,
    pci_enable_bus_master: api_pci_enable_bus_master,
    alloc_frame: api_alloc_frame,
    free_frame: api_free_frame,
    map_mmio: api_map_mmio,
    inb: api_inb,
    outb: api_outb,
    inw: api_inw,
    outw: api_outw,
    inl: api_inl,
    outl: api_outl,
    log: api_log,
    delay_ms: api_delay_ms,
};

// ──────────────────────────────────────────────
// Driver export struct (filled in by external driver)
// ──────────────────────────────────────────────

/// Export struct that each KDRV binary must provide.
/// Located at `exports_offset` bytes from the load base.
#[repr(C)]
pub struct DriverExports {
    pub name: *const u8,        // Null-terminated driver name
    pub driver_type: u32,       // DriverType discriminant
    pub init: extern "C" fn(api: *const KernelDriverApi, pci: *const PciDeviceC) -> i32,
    pub read: extern "C" fn(offset: u64, buf: *mut u8, len: u64) -> i64,
    pub write: extern "C" fn(offset: u64, buf: *const u8, len: u64) -> i64,
    pub ioctl: extern "C" fn(cmd: u32, arg: u32) -> i32,
    pub shutdown: extern "C" fn(),
}

// ──────────────────────────────────────────────
// Driver Info.conf matching
// ──────────────────────────────────────────────

/// Match rule parsed from a driver's Info.conf.
enum DriverMatchRule {
    /// Match specific vendor:device ID pair.
    VendorDevice { vendor: u16, device: u16 },
    /// Match PCI class:subclass.
    Class { class: u8, subclass: u8 },
}

/// Parsed driver bundle info.
struct DriverBundleInfo {
    exec: String,
    driver_type_str: String,
    match_rules: Vec<DriverMatchRule>,
}

/// Parse a driver's Info.conf from a .ddv bundle path.
fn parse_driver_info_conf(bundle_path: &str) -> Option<DriverBundleInfo> {
    let conf_path = alloc::format!("{}/Info.conf", bundle_path);
    let data = crate::fs::vfs::read_file_to_vec(&conf_path).ok()?;
    let text = core::str::from_utf8(&data).ok()?;

    let mut exec: Option<String> = None;
    let mut dtype = String::new();
    let mut matches = Vec::new();

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find('=') {
            let key = line[..idx].trim();
            let val = line[idx + 1..].trim();
            if val.is_empty() { continue; }

            match key {
                "exec" => exec = Some(String::from(val)),
                "type" => dtype = String::from(val),
                "match" => {
                    // Format: "VVVV:DDDD" (vendor:device in hex)
                    if let Some(colon) = val.find(':') {
                        let vendor = u16::from_str_radix(&val[..colon], 16).ok();
                        let device = u16::from_str_radix(&val[colon+1..], 16).ok();
                        if let (Some(v), Some(d)) = (vendor, device) {
                            matches.push(DriverMatchRule::VendorDevice { vendor: v, device: d });
                        }
                    }
                }
                "match_class" => {
                    // Format: "CC:SS" (class:subclass in hex)
                    if let Some(colon) = val.find(':') {
                        let class = u8::from_str_radix(&val[..colon], 16).ok();
                        let subclass = u8::from_str_radix(&val[colon+1..], 16).ok();
                        if let (Some(c), Some(s)) = (class, subclass) {
                            matches.push(DriverMatchRule::Class { class: c, subclass: s });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let exec_name = exec?;

    // Security: exec must be a plain filename — no path separators or ".."
    if exec_name.contains('/') || exec_name.contains("..") {
        crate::serial_println!(
            "  KDRV: SECURITY - rejected exec '{}' in {} (path traversal)",
            exec_name, bundle_path
        );
        return None;
    }

    if matches.is_empty() {
        return None; // No match rules → can't bind
    }

    Some(DriverBundleInfo {
        exec: exec_name,
        driver_type_str: dtype,
        match_rules: matches,
    })
}

/// Check if a PCI device matches any of the driver's match rules.
fn matches_pci_device(rules: &[DriverMatchRule], dev: &PciDevice) -> bool {
    rules.iter().any(|rule| match rule {
        DriverMatchRule::VendorDevice { vendor, device } => {
            dev.vendor_id == *vendor && dev.device_id == *device
        }
        DriverMatchRule::Class { class, subclass } => {
            dev.class_code == *class && dev.subclass == *subclass
        }
    })
}

/// Convert driver type string from Info.conf to DriverType enum.
fn parse_driver_type(s: &str) -> DriverType {
    match s {
        "gpu" | "display" => DriverType::Display,
        "storage" | "block" => DriverType::Block,
        "network" | "ethernet" => DriverType::Network,
        "input" => DriverType::Input,
        "audio" => DriverType::Audio,
        "bus" | "hostbus" => DriverType::Bus,
        "char" | "serial" => DriverType::Char,
        "output" => DriverType::Output,
        "sensor" => DriverType::Sensor,
        _ => DriverType::Unknown,
    }
}

// ──────────────────────────────────────────────
// KDRV Loader
// ──────────────────────────────────────────────

/// Load a KDRV binary into kernel address space.
/// Returns a reference to the DriverExports struct.
fn load_kdrv(bundle_path: &str, exec_name: &str) -> Option<&'static DriverExports> {
    let binary_path = alloc::format!("{}/{}", bundle_path, exec_name);
    let data = match crate::fs::vfs::read_file_to_vec(&binary_path) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("  KDRV: failed to read {}", binary_path);
            return None;
        }
    };

    if data.len() < 4096 {
        crate::serial_println!("  KDRV: {} too small ({}B, need >=4096)", binary_path, data.len());
        return None;
    }

    // Validate header
    let header = unsafe { &*(data.as_ptr() as *const KdrvHeader) };
    if header.magic != KDRV_MAGIC {
        crate::serial_println!("  KDRV: {} bad magic", binary_path);
        return None;
    }
    if header.version != KDRV_VERSION {
        crate::serial_println!("  KDRV: {} version mismatch (got {}, need {})",
            binary_path, header.version, KDRV_VERSION);
        return None;
    }
    if header.abi_version != KDRV_ABI_VERSION {
        crate::serial_println!("  KDRV: {} ABI mismatch (got {}, need {})",
            binary_path, header.abi_version, KDRV_ABI_VERSION);
        return None;
    }

    let code_pages = header.code_pages as u64;
    let data_pages = header.data_pages as u64;
    let bss_pages = header.bss_pages as u64;
    let total_pages = 1 + code_pages + data_pages + bss_pages; // 1 for header page
    let exports_offset = header.exports_offset;

    // Allocate virtual address range
    let mut next_va = NEXT_KDRV_VA.lock();
    let load_base = *next_va;
    let needed = total_pages * PAGE_SIZE;
    if load_base + needed > KDRV_LOAD_END {
        crate::serial_println!("  KDRV: out of virtual address space for {}", binary_path);
        return None;
    }
    *next_va = load_base + needed;
    drop(next_va);

    use crate::memory::virtual_mem::map_page;
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    const PG_PRESENT: u64 = 1;
    const PG_WRITABLE: u64 = 1 << 1;

    // Map header page (RO) — contains the header struct
    let header_frame = physical::alloc_frame()?;
    map_page(
        VirtAddr::new(load_base),
        header_frame,
        PG_PRESENT,
    );
    // Copy header data
    unsafe {
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            load_base as *mut u8,
            4096.min(data.len()),
        );
    }

    // Map code pages (RO)
    let code_start = load_base + PAGE_SIZE;
    let code_data_offset = 4096usize; // After header in file
    for i in 0..code_pages {
        let frame = physical::alloc_frame()?;
        map_page(
            VirtAddr::new(code_start + i * PAGE_SIZE),
            frame,
            PG_PRESENT, // Read-only (no WRITABLE)
        );
        let file_off = code_data_offset + (i as usize * PAGE_SIZE as usize);
        let copy_len = (PAGE_SIZE as usize).min(data.len().saturating_sub(file_off));
        if copy_len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(file_off),
                    (code_start + i * PAGE_SIZE) as *mut u8,
                    copy_len,
                );
            }
        }
    }

    // Map data pages (RW)
    let data_start = code_start + code_pages * PAGE_SIZE;
    let data_file_offset = code_data_offset + (code_pages as usize * PAGE_SIZE as usize);
    for i in 0..data_pages {
        let frame = physical::alloc_frame()?;
        map_page(
            VirtAddr::new(data_start + i * PAGE_SIZE),
            frame,
            PG_PRESENT | PG_WRITABLE,
        );
        let file_off = data_file_offset + (i as usize * PAGE_SIZE as usize);
        let copy_len = (PAGE_SIZE as usize).min(data.len().saturating_sub(file_off));
        if copy_len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(file_off),
                    (data_start + i * PAGE_SIZE) as *mut u8,
                    copy_len,
                );
            }
        }
    }

    // Map BSS pages (RW, zeroed)
    let bss_start = data_start + data_pages * PAGE_SIZE;
    for i in 0..bss_pages {
        let frame = physical::alloc_frame()?;
        map_page(
            VirtAddr::new(bss_start + i * PAGE_SIZE),
            frame,
            PG_PRESENT | PG_WRITABLE,
        );
        unsafe {
            core::ptr::write_bytes((bss_start + i * PAGE_SIZE) as *mut u8, 0, PAGE_SIZE as usize);
        }
    }

    // Return pointer to DriverExports
    let exports_addr = load_base + exports_offset;
    let exports = unsafe { &*(exports_addr as *const DriverExports) };

    crate::serial_println!(
        "  KDRV: loaded {} at {:#x} ({} code + {} data + {} bss pages)",
        binary_path, load_base, code_pages, data_pages, bss_pages
    );

    Some(exports)
}

// ──────────────────────────────────────────────
// External Driver HAL wrapper
// ──────────────────────────────────────────────

/// HAL Driver implementation that delegates to KDRV DriverExports function pointers.
struct ExternalDriver {
    name_str: String,
    dtype: DriverType,
    exports: &'static DriverExports,
}

unsafe impl Send for ExternalDriver {}

impl Driver for ExternalDriver {
    fn name(&self) -> &str {
        &self.name_str
    }

    fn driver_type(&self) -> DriverType {
        self.dtype
    }

    fn init(&mut self) -> Result<(), DriverError> {
        // init() was already called during probe — no-op here
        Ok(())
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let ret = (self.exports.read)(offset as u64, buf.as_mut_ptr(), buf.len() as u64);
        if ret < 0 {
            Err(DriverError::IoError)
        } else {
            Ok(ret as usize)
        }
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        let ret = (self.exports.write)(offset as u64, buf.as_ptr(), buf.len() as u64);
        if ret < 0 {
            Err(DriverError::IoError)
        } else {
            Ok(ret as usize)
        }
    }

    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        let ret = (self.exports.ioctl)(cmd, arg);
        if ret < 0 {
            Err(DriverError::IoError)
        } else {
            Ok(ret as u32)
        }
    }
}

// ──────────────────────────────────────────────
// Probe & Bind External Drivers
// ──────────────────────────────────────────────

/// Probe `/System/Drivers/` for .ddv bundles and load only those whose
/// match rules correspond to PCI devices that are NOT already bound by
/// the built-in HAL driver table.
///
/// Called from main.rs after filesystem init (Phase 7e) and after
/// built-in HAL probe (Phase 7d).
pub fn probe_external_drivers() {
    // Collect PCI devices already bound by built-in drivers
    let bound_devices = hal::bound_pci_devices();

    // All PCI devices on the system
    let all_pci = crate::drivers::pci::devices();

    // Filter to unbound devices (excluding bridges)
    let unbound: Vec<&PciDevice> = all_pci.iter()
        .filter(|d| d.class_code != 0x06)
        .filter(|d| !bound_devices.iter().any(|b|
            b.bus == d.bus && b.device == d.device && b.function == d.function
        ))
        .collect();

    if unbound.is_empty() {
        crate::serial_println!("  KDRV: all PCI devices already bound, skipping external probe");
        return;
    }

    crate::serial_println!(
        "  KDRV: {} unbound PCI device(s), scanning /System/Drivers/ ...",
        unbound.len()
    );

    let categories = ["gpu", "storage", "network", "input", "audio", "bus", "system"];

    let mut loaded = 0u32;

    for category in &categories {
        let dir_path = alloc::format!("/System/Drivers/{}", category);

        // Read directory listing via kernel VFS
        let entries = match crate::fs::vfs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in &entries {
            // Only process .ddv bundles (directories)
            if entry.file_type != crate::fs::file::FileType::Directory
                || !entry.name.ends_with(".ddv")
            {
                continue;
            }

            let name = &entry.name;
            let bundle_path = alloc::format!("{}/{}", dir_path, name);

            // Parse driver Info.conf
            let info = match parse_driver_info_conf(&bundle_path) {
                Some(i) => i,
                None => continue,
            };

            // Find matching unbound PCI device(s)
            for pci_dev in &unbound {
                if !matches_pci_device(&info.match_rules, pci_dev) {
                    continue;
                }

                crate::serial_println!(
                    "  KDRV: matched {} for PCI {:04x}:{:04x}",
                    name, pci_dev.vendor_id, pci_dev.device_id
                );

                // Load the KDRV binary
                let exports = match load_kdrv(&bundle_path, &info.exec) {
                    Some(e) => e,
                    None => continue,
                };

                // Call driver init
                let pci_c = PciDeviceC::from(*pci_dev);
                let ret = (exports.init)(&KERNEL_API as *const _, &pci_c as *const _);
                if ret != 0 {
                    crate::serial_println!(
                        "  KDRV: {} init() failed (returned {})",
                        name, ret
                    );
                    continue;
                }

                // Read driver name from exports
                let driver_name = if !exports.name.is_null() {
                    let mut len = 0usize;
                    unsafe {
                        while *exports.name.add(len) != 0 && len < 64 {
                            len += 1;
                        }
                        let slice = core::slice::from_raw_parts(exports.name, len);
                        core::str::from_utf8(slice)
                            .map(String::from)
                            .unwrap_or_else(|_| String::from(name))
                    }
                } else {
                    String::from(name)
                };

                let dtype = parse_driver_type(&info.driver_type_str);
                let dev_index = hal::count_by_type(dtype);
                let path = hal::make_device_path(dtype, dev_index);

                let ext_driver = ExternalDriver {
                    name_str: driver_name,
                    dtype,
                    exports,
                };

                hal::register_device(&path, Box::new(ext_driver), Some((*pci_dev).clone()));
                loaded += 1;
                break; // One driver per PCI device
            }
        }
    }

    crate::serial_println!("  KDRV: loaded {} external driver(s)", loaded);
}
