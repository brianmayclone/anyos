//! VirtualBox VMMDev (Virtual Machine Monitor Device) driver.
//!
//! PCI device `0x80EE:0xCAFE` — provides guest ↔ host integration:
//! - Absolute mouse coordinates (replaces PS/2 relative when mouse integration active)
//! - Guest capability reporting
//! - Event notifications from host
//!
//! Protocol: allocate a request struct in physical memory, write its physical
//! address to the I/O port (BAR0 offset 0). The host processes the request
//! in-place and sets the `rc` field in the header.

use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

// ── PCI identification ──────────────────────────────

const VMMDEV_VENDOR: u16 = 0x80EE;
const VMMDEV_DEVICE: u16 = 0xCAFE;

// ── Request header version ──────────────────────────

const VMMDEV_REQUEST_HEADER_VERSION: u32 = 0x10001;

// ── Request type codes ──────────────────────────────

const VMMDEVREQ_GET_MOUSE_STATUS: u32     = 1;
const VMMDEVREQ_SET_MOUSE_STATUS: u32     = 2;
const VMMDEVREQ_GET_HOST_VERSION: u32     = 4;
const VMMDEVREQ_ACKNOWLEDGE_EVENTS: u32   = 41;
const VMMDEVREQ_CTL_GUEST_FILTER_MASK: u32 = 42;
const VMMDEVREQ_REPORT_GUEST_INFO: u32    = 50;

// ── Mouse feature flags ─────────────────────────────

const VMMDEV_MOUSE_GUEST_CAN_ABSOLUTE: u32     = 0x01;
const VMMDEV_MOUSE_HOST_WANTS_ABSOLUTE: u32     = 0x04;
const VMMDEV_MOUSE_GUEST_NEEDS_HOST_CURSOR: u32 = 0x10;
const VMMDEV_MOUSE_NEW_PROTOCOL: u32            = 0x20;

// ── Event flags ─────────────────────────────────────

const VMMDEV_EVENT_MOUSE_CAPABILITIES_CHANGED: u32 = 1 << 0;

// ── OS type for ReportGuestInfo ─────────────────────

const VBOXOSTYPE_UNKNOWN: u32 = 0;

// ── MMIO virtual address ────────────────────────────

const VMMDEV_MMIO_VIRT: u64 = 0xFFFF_FFFF_D012_0000;

// ── Request structures (all #[repr(C)]) ─────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevRequestHeader {
    size: u32,
    version: u32,
    request_type: u32,
    rc: i32,
    reserved1: u32,
    requestor: u32,
}

impl VMMDevRequestHeader {
    fn new(request_type: u32, total_size: u32) -> Self {
        Self {
            size: total_size,
            version: VMMDEV_REQUEST_HEADER_VERSION,
            request_type,
            rc: -1, // VERR_GENERAL_FAILURE — will be overwritten by host
            reserved1: 0,
            requestor: 0,
        }
    }
}

/// VMMDevReq_GetMouseStatus / VMMDevReq_SetMouseStatus
#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevReqMouseStatus {
    header: VMMDevRequestHeader,
    mouse_features: u32,
    pointer_x: i32,
    pointer_y: i32,
}

/// VMMDevReq_GetHostVersion
#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevReqHostVersion {
    header: VMMDevRequestHeader,
    major: u16,
    minor: u16,
    build: u32,
    revision: u32,
    features: u32,
}

/// VMMDevReq_AcknowledgeEvents
#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevReqAckEvents {
    header: VMMDevRequestHeader,
    events: u32,
}

/// VMMDevReq_CtlGuestFilterMask
#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevReqGuestFilterMask {
    header: VMMDevRequestHeader,
    or_mask: u32,
    not_mask: u32,
}

/// VMMDevReq_ReportGuestInfo
#[repr(C)]
#[derive(Clone, Copy)]
struct VMMDevReqGuestInfo {
    header: VMMDevRequestHeader,
    interface_version: u32,
    os_type: u32,
}

// ── Global state ────────────────────────────────────

static AVAILABLE: AtomicBool = AtomicBool::new(false);
static SCREEN_WIDTH: AtomicU16 = AtomicU16::new(1920);
static SCREEN_HEIGHT: AtomicU16 = AtomicU16::new(1080);

/// I/O port base (BAR0) for request submission.
static mut IO_PORT: u16 = 0;

/// Physical address of the DMA request page (identity-mapped).
/// Must be < 4 GiB for VMMDev compatibility.
static mut REQ_PAGE_PHYS: u64 = 0;

/// Virtual address of the DMA request page (identity-mapped = phys).
static mut REQ_PAGE_VIRT: u64 = 0;

// ── Public API ──────────────────────────────────────

/// Check if VMMDev is initialized and available.
#[inline]
pub fn is_available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

/// Update screen dimensions (called by compositor/GPU driver on mode set).
pub fn set_screen_size(width: u16, height: u16) {
    SCREEN_WIDTH.store(width, Ordering::Relaxed);
    SCREEN_HEIGHT.store(height, Ordering::Relaxed);
}

/// Poll VMMDev for current absolute mouse position.
/// Returns `Some((pixel_x, pixel_y, buttons))` if new position available,
/// `None` if VMMDev not available or no change.
pub fn poll_mouse() -> Option<(i32, i32, u32)> {
    if !is_available() {
        return None;
    }

    let req = VMMDevReqMouseStatus {
        header: VMMDevRequestHeader::new(
            VMMDEVREQ_GET_MOUSE_STATUS,
            core::mem::size_of::<VMMDevReqMouseStatus>() as u32,
        ),
        mouse_features: 0,
        pointer_x: 0,
        pointer_y: 0,
    };

    let resp: VMMDevReqMouseStatus = unsafe { submit_request(&req) };

    if resp.header.rc < 0 {
        return None;
    }

    // Host doesn't want absolute? No position data.
    if resp.mouse_features & VMMDEV_MOUSE_HOST_WANTS_ABSOLUTE == 0 {
        return None;
    }

    // Scale from 0..0xFFFF to screen pixels
    let sw = SCREEN_WIDTH.load(Ordering::Relaxed) as i32;
    let sh = SCREEN_HEIGHT.load(Ordering::Relaxed) as i32;
    let px = (resp.pointer_x as i64 * sw as i64 / 0xFFFF) as i32;
    let py = (resp.pointer_y as i64 * sh as i64 / 0xFFFF) as i32;

    // Extract button state from mouse_features (bits 8-10 in some VBox versions)
    // VMMDev doesn't provide buttons in GetMouseStatus — buttons come via PS/2.
    // Return 0 for buttons; the PS/2 button state is tracked separately.
    Some((px, py, 0))
}

// ── Init ────────────────────────────────────────────

/// Initialize VMMDev from PCI probe. Called by HAL.
pub fn init_and_register(pci: &PciDevice) {
    if pci.vendor_id != VMMDEV_VENDOR || pci.device_id != VMMDEV_DEVICE {
        return;
    }

    // BAR0 = I/O port base
    let bar0 = pci.bars[0];
    if bar0 & 1 == 0 {
        crate::serial_println!("  VMMDev: BAR0 is not I/O port — aborting");
        return;
    }
    let io_base = (bar0 & 0xFFFC) as u16;
    crate::serial_println!("  VMMDev: I/O port base = {:#06x}", io_base);

    // BAR1 = MMIO (shared memory area)
    let bar1 = pci.bars[1];
    if bar1 & 1 != 0 {
        crate::serial_println!("  VMMDev: BAR1 is I/O port (expected MMIO) — aborting");
        return;
    }
    let mmio_phys = (bar1 & 0xFFFFF000) as u64;
    crate::serial_println!("  VMMDev: MMIO phys = {:#010x}", mmio_phys);

    // Enable PCI bus mastering + I/O + memory
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map BAR1 MMIO (1 page)
    crate::memory::virtual_mem::map_page(
        VirtAddr::new(VMMDEV_MMIO_VIRT),
        PhysAddr::new(mmio_phys),
        0x03, // Present + Writable
    );

    // Allocate a DMA page for requests (must be < 4 GiB, identity-mapped)
    let req_phys = match crate::memory::physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  VMMDev: failed to allocate DMA page");
            return;
        }
    };
    // Identity-map the request page (virt = phys) for DMA
    crate::memory::virtual_mem::map_page(
        VirtAddr::new(req_phys),
        PhysAddr::new(req_phys),
        0x03,
    );

    unsafe {
        IO_PORT = io_base;
        REQ_PAGE_PHYS = req_phys;
        REQ_PAGE_VIRT = req_phys; // identity-mapped
    }

    // Zero the request page
    unsafe {
        core::ptr::write_bytes(req_phys as *mut u8, 0, 4096);
    }

    // Step 1: Report guest info
    let guest_info = VMMDevReqGuestInfo {
        header: VMMDevRequestHeader::new(
            VMMDEVREQ_REPORT_GUEST_INFO,
            core::mem::size_of::<VMMDevReqGuestInfo>() as u32,
        ),
        interface_version: 0x00010004, // VMMDev interface version 1.04
        os_type: VBOXOSTYPE_UNKNOWN,
    };
    let resp: VMMDevReqGuestInfo = unsafe { submit_request(&guest_info) };
    if resp.header.rc < 0 {
        crate::serial_println!("  VMMDev: ReportGuestInfo failed (rc={})", resp.header.rc);
    } else {
        crate::serial_println!("  VMMDev: ReportGuestInfo OK");
    }

    // Step 2: Get host version
    let ver_req = VMMDevReqHostVersion {
        header: VMMDevRequestHeader::new(
            VMMDEVREQ_GET_HOST_VERSION,
            core::mem::size_of::<VMMDevReqHostVersion>() as u32,
        ),
        major: 0,
        minor: 0,
        build: 0,
        revision: 0,
        features: 0,
    };
    let ver_resp: VMMDevReqHostVersion = unsafe { submit_request(&ver_req) };
    if ver_resp.header.rc >= 0 {
        crate::serial_println!(
            "  VMMDev: Host version {}.{}.{} (rev {})",
            ver_resp.major, ver_resp.minor, ver_resp.build, ver_resp.revision
        );
    }

    // Step 3: Enable absolute mouse
    let mouse_req = VMMDevReqMouseStatus {
        header: VMMDevRequestHeader::new(
            VMMDEVREQ_SET_MOUSE_STATUS,
            core::mem::size_of::<VMMDevReqMouseStatus>() as u32,
        ),
        mouse_features: VMMDEV_MOUSE_GUEST_CAN_ABSOLUTE | VMMDEV_MOUSE_NEW_PROTOCOL,
        pointer_x: 0,
        pointer_y: 0,
    };
    let mouse_resp: VMMDevReqMouseStatus = unsafe { submit_request(&mouse_req) };
    if mouse_resp.header.rc < 0 {
        crate::serial_println!("  VMMDev: SetMouseStatus failed (rc={})", mouse_resp.header.rc);
    } else {
        crate::serial_println!("  VMMDev: Absolute mouse enabled");
    }

    // Step 4: Set event filter (enable mouse capability change events)
    let filter = VMMDevReqGuestFilterMask {
        header: VMMDevRequestHeader::new(
            VMMDEVREQ_CTL_GUEST_FILTER_MASK,
            core::mem::size_of::<VMMDevReqGuestFilterMask>() as u32,
        ),
        or_mask: VMMDEV_EVENT_MOUSE_CAPABILITIES_CHANGED,
        not_mask: 0,
    };
    let _: VMMDevReqGuestFilterMask = unsafe { submit_request(&filter) };

    AVAILABLE.store(true, Ordering::Release);
    crate::serial_println!("[OK] VMMDev initialized (abs mouse, event filter)");
}

// ── Low-level request submission ────────────────────

// ── HAL integration ─────────────────────────────────────────────────────────

use alloc::boxed::Box;
use crate::drivers::hal::{Driver, DriverType, DriverError};

struct VMMDevHalDriver;

impl Driver for VMMDevHalDriver {
    fn name(&self) -> &str { "VMMDev Guest Integration" }
    fn driver_type(&self) -> DriverType { DriverType::Bus }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Create a HAL Driver wrapper for VMMDev.
pub fn create_hal_driver() -> Option<Box<dyn Driver>> {
    Some(Box::new(VMMDevHalDriver))
}

/// Probe: initialize VMMDev and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    init_and_register(pci);
    create_hal_driver()
}

// ── Low-level request submission ────────────────────

/// Submit a VMMDev request. Copies `req` to the DMA page, writes phys addr
/// to the I/O port, and returns the response (host modifies the struct in-place).
///
/// # Safety
/// Caller must ensure `T` is a valid VMMDev request struct with a `VMMDevRequestHeader`
/// as its first field, and `size_of::<T>()` fits in one page (4096 bytes).
unsafe fn submit_request<T: Copy>(req: &T) -> T {
    let size = core::mem::size_of::<T>();
    debug_assert!(size <= 4096);

    let virt = REQ_PAGE_VIRT as *mut u8;
    let phys = REQ_PAGE_PHYS;
    let port = IO_PORT;

    // Copy request to DMA page
    core::ptr::copy_nonoverlapping(req as *const T as *const u8, virt, size);

    // Submit: write physical address of request to I/O port
    crate::arch::x86::port::outl(port, phys as u32);

    // Read response back from DMA page
    core::ptr::read_volatile(virt as *const T)
}
