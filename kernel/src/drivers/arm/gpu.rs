//! VirtIO GPU 2D driver over MMIO transport for ARM64.
//!
//! Self-contained driver using VirtIO MMIO transport — no dependency on
//! `drivers::gpu` or `drivers::virtio`. Uses two virtqueues (controlq + cursorq)
//! and the standard VirtIO GPU 2D command set.
//!
//! After initialization, registers the framebuffer via `drivers::framebuffer::update()`.

use core::ptr;
use core::sync::atomic::{fence, Ordering};

use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;

use super::VirtioMmioDevice;
use super::virtqueue::{VirtQueue, VRING_DESC_F_WRITE, DEFAULT_QUEUE_SIZE};

// ---------------------------------------------------------------------------
// VirtIO GPU Command Types
// ---------------------------------------------------------------------------

const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32     = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32   = 0x0101;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32       = 0x0102;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32          = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32       = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32  = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

const VIRTIO_GPU_RESP_OK_NODATA: u32           = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32     = 0x1101;

const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32    = 2;

// ---------------------------------------------------------------------------
// Command Structures (repr(C), matches VirtIO GPU spec)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCtrlHdr {
    type_: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    ring_idx: u8,
    padding: [u8; 3],
}

impl GpuCtrlHdr {
    fn new(type_: u32) -> Self {
        GpuCtrlHdr { type_, flags: 0, fence_id: 0, ctx_id: 0, ring_idx: 0, padding: [0; 3] }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceCreate2d {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SetScanout {
    hdr: GpuCtrlHdr,
    r_x: u32, r_y: u32, r_width: u32, r_height: u32,
    scanout_id: u32,
    resource_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TransferToHost2d {
    hdr: GpuCtrlHdr,
    r_x: u32, r_y: u32, r_width: u32, r_height: u32,
    offset: u64,
    resource_id: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceFlush {
    hdr: GpuCtrlHdr,
    r_x: u32, r_y: u32, r_width: u32, r_height: u32,
    resource_id: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceAttachBacking {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MemEntry {
    addr: u64,
    length: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DisplayOne {
    r_x: u32, r_y: u32, r_width: u32, r_height: u32,
    enabled: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RespDisplayInfo {
    hdr: GpuCtrlHdr,
    pmodes: [DisplayOne; 16],
}

// ---------------------------------------------------------------------------
// GPU State
// ---------------------------------------------------------------------------

struct VirtioGpu {
    base: usize,
    controlq: VirtQueue,
    /// Physical address of the command/response buffer (one page).
    cmd_phys: u64,
    cmd_virt: usize,
    /// Framebuffer virtual address (kernel).
    fb_virt: usize,
    /// Framebuffer physical base.
    fb_phys: u64,
    /// Display dimensions.
    width: u32,
    height: u32,
    resource_id: u32,
}

static GPU_DEVICE: Spinlock<Option<VirtioGpu>> = Spinlock::new(None);

/// Convert RAM physical to kernel virtual.
#[inline]
fn phys_to_virt(phys: u64) -> usize {
    (phys + 0xFFFF_0000_4000_0000) as usize
}

#[inline]
fn virt_to_phys(virt: usize) -> u64 {
    (virt as u64).wrapping_sub(0xFFFF_0000_4000_0000)
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the VirtIO GPU device.
pub fn init(dev: &VirtioMmioDevice) {
    // Feature negotiation (no special features needed)
    if dev.init_device(0).is_none() {
        crate::serial_println!("  virtio-gpu: feature negotiation failed");
        return;
    }

    // Allocate controlq (queue 0)
    let controlq = match VirtQueue::new(0, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_println!("  virtio-gpu: failed to allocate controlq");
            return;
        }
    };

    let (desc_phys, avail_phys, used_phys) = controlq.phys_addrs();
    if !dev.setup_queue_raw(0, DEFAULT_QUEUE_SIZE, desc_phys, avail_phys, used_phys) {
        crate::serial_println!("  virtio-gpu: failed to setup controlq");
        return;
    }

    // Allocate command buffer (one 4K page for commands + responses)
    let cmd_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => {
            crate::serial_println!("  virtio-gpu: failed to allocate command buffer");
            return;
        }
    };
    let cmd_phys = cmd_frame.0;
    let cmd_virt = phys_to_virt(cmd_phys);
    unsafe { ptr::write_bytes(cmd_virt as *mut u8, 0, FRAME_SIZE); }

    dev.driver_ok();

    let mut gpu = VirtioGpu {
        base: dev.base(),
        controlq,
        cmd_phys,
        cmd_virt,
        fb_virt: 0,
        fb_phys: 0,
        width: 0,
        height: 0,
        resource_id: 1,
    };

    // Get display info
    let (width, height) = get_display_info(&mut gpu, dev);
    if width == 0 || height == 0 {
        crate::serial_println!("  virtio-gpu: no display detected, using 1024x768");
        gpu.width = 1024;
        gpu.height = 768;
    } else {
        gpu.width = width;
        gpu.height = height;
        crate::serial_println!("  virtio-gpu: display {}x{}", width, height);
    }

    // Setup framebuffer
    if !setup_framebuffer(&mut gpu, dev) {
        crate::serial_println!("  virtio-gpu: framebuffer setup failed");
        return;
    }

    crate::serial_println!("  virtio-gpu: framebuffer at virt={:#x}, {}x{}",
        gpu.fb_virt, gpu.width, gpu.height);

    // Register framebuffer with the global framebuffer module
    let pitch = gpu.width * 4;
    crate::drivers::framebuffer::update(gpu.fb_phys as u32, pitch, gpu.width, gpu.height, 32);

    *GPU_DEVICE.lock() = Some(gpu);
}

// ---------------------------------------------------------------------------
// GPU Commands
// ---------------------------------------------------------------------------

/// Send a command and wait for response using the controlq.
fn send_cmd(gpu: &mut VirtioGpu, dev: &VirtioMmioDevice, cmd_size: usize, resp_size: usize) -> bool {
    let cmd_phys = gpu.cmd_phys;
    let resp_phys = gpu.cmd_phys + 2048; // Response in second half of page

    // Zero response area
    unsafe { ptr::write_bytes((gpu.cmd_virt + 2048) as *mut u8, 0, resp_size.max(64)); }

    // Push 2-descriptor chain: command (device-readable) → response (device-writable)
    let chain = [
        (cmd_phys, cmd_size as u32, 0u16),
        (resp_phys, resp_size as u32, VRING_DESC_F_WRITE),
    ];

    if gpu.controlq.push_chain(&chain).is_none() {
        return false;
    }

    // Notify device
    dev.notify_queue(0);

    // Poll for completion
    let mut timeout = 5_000_000u32;
    while !gpu.controlq.has_used() {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            crate::serial_println!("  virtio-gpu: command timeout");
            return false;
        }
    }

    gpu.controlq.pop_used();
    true
}

/// GET_DISPLAY_INFO — returns (width, height) of scanout 0.
fn get_display_info(gpu: &mut VirtioGpu, dev: &VirtioMmioDevice) -> (u32, u32) {
    let hdr = GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
    unsafe { ptr::write(gpu.cmd_virt as *mut GpuCtrlHdr, hdr); }

    if !send_cmd(gpu, dev, core::mem::size_of::<GpuCtrlHdr>(),
                 core::mem::size_of::<RespDisplayInfo>()) {
        return (0, 0);
    }

    let resp = unsafe { ptr::read((gpu.cmd_virt + 2048) as *const RespDisplayInfo) };
    if resp.hdr.type_ != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
        return (0, 0);
    }

    let d = &resp.pmodes[0];
    if d.enabled != 0 && d.r_width > 0 && d.r_height > 0 {
        (d.r_width, d.r_height)
    } else {
        (0, 0)
    }
}

/// Set up the GPU framebuffer: create resource, attach backing, set scanout.
fn setup_framebuffer(gpu: &mut VirtioGpu, dev: &VirtioMmioDevice) -> bool {
    let w = gpu.width;
    let h = gpu.height;
    let fb_bytes = (w * h * 4) as usize;
    let fb_pages = (fb_bytes + FRAME_SIZE - 1) / FRAME_SIZE;

    // Allocate framebuffer pages
    let fb_frame = match physical::alloc_contiguous(fb_pages) {
        Some(f) => f,
        None => return false,
    };
    gpu.fb_phys = fb_frame.0;
    gpu.fb_virt = phys_to_virt(gpu.fb_phys);

    // Zero framebuffer
    unsafe { ptr::write_bytes(gpu.fb_virt as *mut u8, 0, fb_pages * FRAME_SIZE); }

    let rid = gpu.resource_id;

    // 1. RESOURCE_CREATE_2D
    let cmd = ResourceCreate2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
        resource_id: rid,
        format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
        width: w,
        height: h,
    };
    unsafe { ptr::write(gpu.cmd_virt as *mut ResourceCreate2d, cmd); }
    if !send_cmd(gpu, dev, core::mem::size_of::<ResourceCreate2d>(),
                 core::mem::size_of::<GpuCtrlHdr>()) {
        return false;
    }
    let resp_type = unsafe { (*(( gpu.cmd_virt + 2048) as *const GpuCtrlHdr)).type_ };
    if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
        crate::serial_println!("  virtio-gpu: RESOURCE_CREATE_2D failed: {:#x}", resp_type);
        return false;
    }

    // 2. RESOURCE_ATTACH_BACKING (header + 1 mem entry)
    let attach_hdr = ResourceAttachBacking {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
        resource_id: rid,
        nr_entries: 1,
    };
    let entry = MemEntry {
        addr: gpu.fb_phys,
        length: fb_bytes as u32,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceAttachBacking, attach_hdr);
        ptr::write((gpu.cmd_virt + core::mem::size_of::<ResourceAttachBacking>()) as *mut MemEntry, entry);
    }
    let cmd_size = core::mem::size_of::<ResourceAttachBacking>() + core::mem::size_of::<MemEntry>();
    if !send_cmd(gpu, dev, cmd_size, core::mem::size_of::<GpuCtrlHdr>()) {
        return false;
    }

    // 3. SET_SCANOUT
    let scanout = SetScanout {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_SET_SCANOUT),
        r_x: 0, r_y: 0, r_width: w, r_height: h,
        scanout_id: 0,
        resource_id: rid,
    };
    unsafe { ptr::write(gpu.cmd_virt as *mut SetScanout, scanout); }
    if !send_cmd(gpu, dev, core::mem::size_of::<SetScanout>(),
                 core::mem::size_of::<GpuCtrlHdr>()) {
        return false;
    }

    // 4. Initial TRANSFER_TO_HOST_2D + RESOURCE_FLUSH
    flush_region(gpu, dev, 0, 0, w, h);

    true
}

/// Transfer a region and flush it to the display.
fn flush_region(gpu: &mut VirtioGpu, dev: &VirtioMmioDevice, x: u32, y: u32, w: u32, h: u32) {
    let rid = gpu.resource_id;

    // TRANSFER_TO_HOST_2D
    let transfer = TransferToHost2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
        r_x: x, r_y: y, r_width: w, r_height: h,
        offset: 0,
        resource_id: rid,
        padding: 0,
    };
    unsafe { ptr::write(gpu.cmd_virt as *mut TransferToHost2d, transfer); }
    send_cmd(gpu, dev, core::mem::size_of::<TransferToHost2d>(),
             core::mem::size_of::<GpuCtrlHdr>());

    // RESOURCE_FLUSH
    let flush = ResourceFlush {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
        r_x: x, r_y: y, r_width: w, r_height: h,
        resource_id: rid,
        padding: 0,
    };
    unsafe { ptr::write(gpu.cmd_virt as *mut ResourceFlush, flush); }
    send_cmd(gpu, dev, core::mem::size_of::<ResourceFlush>(),
             core::mem::size_of::<GpuCtrlHdr>());
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Flush a rectangular region of the framebuffer to the display.
pub fn flush(x: u32, y: u32, w: u32, h: u32) {
    // This needs access to the device to send commands.
    // For now, store enough state to do a flush later.
    // TODO: implement runtime flush via stored device reference
    let _ = (x, y, w, h);
}

/// Get the framebuffer virtual address and dimensions.
pub fn framebuffer_info() -> Option<(usize, u32, u32)> {
    let guard = GPU_DEVICE.lock();
    guard.as_ref().map(|g| (g.fb_virt, g.width, g.height))
}

/// Check if VirtIO GPU is available.
pub fn is_available() -> bool {
    GPU_DEVICE.lock().is_some()
}
