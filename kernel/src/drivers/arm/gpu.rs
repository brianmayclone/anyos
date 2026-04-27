//! VirtIO GPU 2D driver over MMIO transport for ARM64.
//!
//! Self-contained driver using VirtIO MMIO transport — no dependency on
//! `drivers::gpu` or `drivers::virtio`. Uses two virtqueues (controlq + cursorq)
//! and the standard VirtIO GPU 2D command set.
//!
//! After initialization, registers the framebuffer via `drivers::framebuffer::update()`.

use core::ptr;

use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;

use super::virtqueue::{VirtQueue, DEFAULT_QUEUE_SIZE, VRING_DESC_F_WRITE};
use super::VirtioMmioDevice;

// ---------------------------------------------------------------------------
// VirtIO GPU Command Types
// ---------------------------------------------------------------------------

const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x0102;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
const VIRTIO_GPU_CMD_UPDATE_CURSOR: u32 = 0x0300;
const VIRTIO_GPU_CMD_MOVE_CURSOR: u32 = 0x0301;

const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;

const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32 = 1;
const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;

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
        GpuCtrlHdr {
            type_,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            ring_idx: 0,
            padding: [0; 3],
        }
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
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    scanout_id: u32,
    resource_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TransferToHost2d {
    hdr: GpuCtrlHdr,
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    offset: u64,
    resource_id: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceFlush {
    hdr: GpuCtrlHdr,
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
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
struct ResourceDetachBacking {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    padding: u32,
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
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    enabled: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RespDisplayInfo {
    hdr: GpuCtrlHdr,
    pmodes: [DisplayOne; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CursorPos {
    scanout_id: u32,
    x: u32,
    y: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UpdateCursor {
    hdr: GpuCtrlHdr,
    pos: CursorPos,
    resource_id: u32,
    hot_x: u32,
    hot_y: u32,
    padding: u32,
}

// ---------------------------------------------------------------------------
// GPU State
// ---------------------------------------------------------------------------

struct VirtioGpu {
    base: usize,
    controlq: VirtQueue,
    cursorq: VirtQueue,
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
    cursor_resource_id: u32,
    next_resource_id: u32,
    cursor_buf_phys: u64,
    cursor_hot_x: u32,
    cursor_hot_y: u32,
    cursor_x: u32,
    cursor_y: u32,
}

static GPU_DEVICE: Spinlock<Option<VirtioGpu>> = Spinlock::new(None);

#[inline]
fn dcache_line_size() -> usize {
    let ctr: u64;
    unsafe {
        core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags));
    }
    let log2_words = ((ctr >> 16) & 0xF) as usize;
    4usize << log2_words
}

fn clean_dcache_range(virt: usize, len: usize) {
    if len == 0 {
        return;
    }
    let line = dcache_line_size().max(16);
    let start = virt & !(line - 1);
    let end = (virt + len + line - 1) & !(line - 1);

    unsafe {
        let mut addr = start;
        while addr < end {
            core::arch::asm!("dc cvac, {}", in(reg) addr, options(nostack, preserves_flags));
            addr += line;
        }
        core::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }
}

#[inline]
fn fb_offset_bytes(gpu: &VirtioGpu, x: u32, y: u32) -> u64 {
    ((y as u64 * gpu.width as u64) + x as u64) * 4
}

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
        crate::serial_verbose_println!("  virtio-gpu: feature negotiation failed");
        return;
    }

    // Allocate controlq (queue 0)
    let controlq = match VirtQueue::new(0, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  virtio-gpu: failed to allocate controlq");
            return;
        }
    };

    let (desc_phys, avail_phys, used_phys) = controlq.phys_addrs();
    if !dev.setup_queue_raw(0, DEFAULT_QUEUE_SIZE, desc_phys, avail_phys, used_phys) {
        crate::serial_verbose_println!("  virtio-gpu: failed to setup controlq");
        return;
    }

    let cursorq = match VirtQueue::new(1, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  virtio-gpu: failed to allocate cursorq");
            return;
        }
    };

    let (desc_phys, avail_phys, used_phys) = cursorq.phys_addrs();
    if !dev.setup_queue_raw(1, DEFAULT_QUEUE_SIZE, desc_phys, avail_phys, used_phys) {
        crate::serial_verbose_println!("  virtio-gpu: failed to setup cursorq");
        return;
    }

    // Allocate command buffer (one 4K page for commands + responses)
    let cmd_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => {
            crate::serial_verbose_println!("  virtio-gpu: failed to allocate command buffer");
            return;
        }
    };
    let cmd_phys = cmd_frame.0;
    let cmd_virt = phys_to_virt(cmd_phys);
    unsafe {
        ptr::write_bytes(cmd_virt as *mut u8, 0, FRAME_SIZE);
    }

    let cursor_frame = match physical::alloc_contiguous(4) {
        Some(f) => f,
        None => {
            crate::serial_verbose_println!("  virtio-gpu: failed to allocate cursor buffer");
            return;
        }
    };
    let cursor_buf_phys = cursor_frame.0;
    unsafe {
        ptr::write_bytes(phys_to_virt(cursor_buf_phys) as *mut u8, 0, 4 * FRAME_SIZE);
    }

    dev.driver_ok();

    let mut gpu = VirtioGpu {
        base: dev.base(),
        controlq,
        cursorq,
        cmd_phys,
        cmd_virt,
        fb_virt: 0,
        fb_phys: 0,
        width: 0,
        height: 0,
        resource_id: 1,
        cursor_resource_id: 0,
        next_resource_id: 2,
        cursor_buf_phys,
        cursor_hot_x: 0,
        cursor_hot_y: 0,
        cursor_x: 0,
        cursor_y: 0,
    };

    // Get display info
    let (width, height) = get_display_info(&mut gpu, dev);
    if width == 0 || height == 0 {
        crate::serial_verbose_println!("  virtio-gpu: no display detected, using 1024x768");
        gpu.width = 1024;
        gpu.height = 768;
    } else {
        gpu.width = width;
        gpu.height = height;
        crate::serial_verbose_println!("  virtio-gpu: display {}x{}", width, height);
    }

    // Setup framebuffer
    if !setup_framebuffer(&mut gpu, dev) {
        crate::serial_verbose_println!("  virtio-gpu: framebuffer setup failed");
        return;
    }

    crate::serial_verbose_println!(
        "  virtio-gpu: framebuffer at virt={:#x}, {}x{}",
        gpu.fb_virt,
        gpu.width,
        gpu.height
    );

    // Register framebuffer with the global framebuffer module
    let pitch = gpu.width * 4;
    crate::drivers::framebuffer::update(gpu.fb_virt as u64, pitch, gpu.width, gpu.height, 32);

    *GPU_DEVICE.lock() = Some(gpu);
}

// ---------------------------------------------------------------------------
// GPU Commands
// ---------------------------------------------------------------------------

/// Send a command using stored device base (no VirtioMmioDevice reference needed).
/// Used by the public `flush()` API after initialization.
fn send_cmd_raw(gpu: &mut VirtioGpu, cmd_size: usize, resp_size: usize) -> bool {
    let cmd_phys = gpu.cmd_phys;
    let resp_phys = gpu.cmd_phys + 2048;

    unsafe {
        ptr::write_bytes((gpu.cmd_virt + 2048) as *mut u8, 0, resp_size.max(64));
    }

    let chain = [
        (cmd_phys, cmd_size as u32, 0u16),
        (resp_phys, resp_size as u32, VRING_DESC_F_WRITE),
    ];

    if gpu.controlq.push_chain(&chain).is_none() {
        return false;
    }

    // Notify via MMIO (DSB + write to QueueNotify register)
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        ptr::write_volatile((gpu.base + 0x050) as *mut u32, 0);
    }

    let mut timeout = 5_000_000u32;
    while !gpu.controlq.has_used() {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            return false;
        }
    }

    gpu.controlq.pop_used();
    true
}

fn send_cursor_cmd_raw(gpu: &mut VirtioGpu, cmd_size: usize, resp_size: usize) -> bool {
    let cmd_phys = gpu.cmd_phys + 1024;
    let cmd_virt = gpu.cmd_virt + 1024;
    let resp_phys = gpu.cmd_phys + 3072;
    let resp_virt = gpu.cmd_virt + 3072;

    unsafe {
        ptr::write_bytes(resp_virt as *mut u8, 0, resp_size.max(64));
    }

    let chain = [
        (cmd_phys, cmd_size as u32, 0u16),
        (resp_phys, resp_size as u32, VRING_DESC_F_WRITE),
    ];

    if gpu.cursorq.push_chain(&chain).is_none() {
        return false;
    }

    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        ptr::write_volatile((gpu.base + 0x050) as *mut u32, 1);
    }

    let mut timeout = 5_000_000u32;
    while !gpu.cursorq.has_used() {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            return false;
        }
    }

    gpu.cursorq.pop_used();
    true
}

/// Send a command and wait for response using the controlq.
fn send_cmd(
    gpu: &mut VirtioGpu,
    dev: &VirtioMmioDevice,
    cmd_size: usize,
    resp_size: usize,
) -> bool {
    let cmd_phys = gpu.cmd_phys;
    let resp_phys = gpu.cmd_phys + 2048; // Response in second half of page

    // Zero response area
    unsafe {
        ptr::write_bytes((gpu.cmd_virt + 2048) as *mut u8, 0, resp_size.max(64));
    }

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
            crate::serial_verbose_println!("  virtio-gpu: command timeout");
            return false;
        }
    }

    gpu.controlq.pop_used();
    true
}

/// GET_DISPLAY_INFO — returns (width, height) of scanout 0.
fn get_display_info(gpu: &mut VirtioGpu, dev: &VirtioMmioDevice) -> (u32, u32) {
    let hdr = GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
    unsafe {
        ptr::write(gpu.cmd_virt as *mut GpuCtrlHdr, hdr);
    }

    if !send_cmd(
        gpu,
        dev,
        core::mem::size_of::<GpuCtrlHdr>(),
        core::mem::size_of::<RespDisplayInfo>(),
    ) {
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
    unsafe {
        ptr::write_bytes(gpu.fb_virt as *mut u8, 0, fb_pages * FRAME_SIZE);
    }

    let rid = gpu.resource_id;

    // 1. RESOURCE_CREATE_2D
    let cmd = ResourceCreate2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
        resource_id: rid,
        format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
        width: w,
        height: h,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceCreate2d, cmd);
    }
    if !send_cmd(
        gpu,
        dev,
        core::mem::size_of::<ResourceCreate2d>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    ) {
        return false;
    }
    let resp_type = unsafe { (*((gpu.cmd_virt + 2048) as *const GpuCtrlHdr)).type_ };
    if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
        crate::serial_verbose_println!("  virtio-gpu: RESOURCE_CREATE_2D failed: {:#x}", resp_type);
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
        ptr::write(
            (gpu.cmd_virt + core::mem::size_of::<ResourceAttachBacking>()) as *mut MemEntry,
            entry,
        );
    }
    let cmd_size = core::mem::size_of::<ResourceAttachBacking>() + core::mem::size_of::<MemEntry>();
    if !send_cmd(gpu, dev, cmd_size, core::mem::size_of::<GpuCtrlHdr>()) {
        return false;
    }

    // 3. SET_SCANOUT
    let scanout = SetScanout {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_SET_SCANOUT),
        r_x: 0,
        r_y: 0,
        r_width: w,
        r_height: h,
        scanout_id: 0,
        resource_id: rid,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut SetScanout, scanout);
    }
    if !send_cmd(
        gpu,
        dev,
        core::mem::size_of::<SetScanout>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    ) {
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
        r_x: x,
        r_y: y,
        r_width: w,
        r_height: h,
        offset: fb_offset_bytes(gpu, x, y),
        resource_id: rid,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut TransferToHost2d, transfer);
    }
    send_cmd(
        gpu,
        dev,
        core::mem::size_of::<TransferToHost2d>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );

    // RESOURCE_FLUSH
    let flush = ResourceFlush {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
        r_x: x,
        r_y: y,
        r_width: w,
        r_height: h,
        resource_id: rid,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceFlush, flush);
    }
    send_cmd(
        gpu,
        dev,
        core::mem::size_of::<ResourceFlush>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Flush a rectangular region of the framebuffer to the display.
pub fn flush(x: u32, y: u32, w: u32, h: u32) {
    let mut guard = GPU_DEVICE.lock();
    let gpu = match guard.as_mut() {
        Some(g) => g,
        None => return,
    };
    if w == 0 || h == 0 {
        return;
    }
    let rid = gpu.resource_id;

    let pitch = gpu.width as usize * 4;
    let x_bytes = x as usize * 4;
    let rows = h as usize;
    let row_bytes = w as usize * 4;
    for row in 0..rows {
        let row_off = (y as usize + row) * pitch + x_bytes;
        clean_dcache_range(gpu.fb_virt + row_off, row_bytes);
    }

    // TRANSFER_TO_HOST_2D
    let transfer = TransferToHost2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
        r_x: x,
        r_y: y,
        r_width: w,
        r_height: h,
        offset: fb_offset_bytes(gpu, x, y),
        resource_id: rid,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut TransferToHost2d, transfer);
    }
    send_cmd_raw(
        gpu,
        core::mem::size_of::<TransferToHost2d>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );

    // RESOURCE_FLUSH
    let flush_cmd = ResourceFlush {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
        r_x: x,
        r_y: y,
        r_width: w,
        r_height: h,
        resource_id: rid,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceFlush, flush_cmd);
    }
    send_cmd_raw(
        gpu,
        core::mem::size_of::<ResourceFlush>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );
}

/// Get the framebuffer virtual address and dimensions.
pub fn framebuffer_info() -> Option<(usize, u32, u32)> {
    let guard = GPU_DEVICE.lock();
    guard.as_ref().map(|g| (g.fb_virt, g.width, g.height))
}

/// Get the framebuffer mapping details needed by userspace.
pub fn framebuffer_mapping_info() -> Option<(u64, usize, u32, u32, u32)> {
    let guard = GPU_DEVICE.lock();
    guard
        .as_ref()
        .map(|g| (g.fb_phys, g.fb_virt, g.width, g.height, g.width * 4))
}

/// Check if VirtIO GPU is available.
pub fn is_available() -> bool {
    GPU_DEVICE.lock().is_some()
}

pub fn has_hw_cursor() -> bool {
    GPU_DEVICE.lock().is_some()
}

pub fn move_cursor(x: u32, y: u32) {
    let mut guard = GPU_DEVICE.lock();
    let gpu = match guard.as_mut() {
        Some(g) => g,
        None => return,
    };

    gpu.cursor_x = x;
    gpu.cursor_y = y;
    let cmd = UpdateCursor {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_MOVE_CURSOR),
        pos: CursorPos {
            scanout_id: 0,
            x,
            y,
            padding: 0,
        },
        resource_id: gpu.cursor_resource_id,
        hot_x: gpu.cursor_hot_x,
        hot_y: gpu.cursor_hot_y,
        padding: 0,
    };
    unsafe {
        ptr::write((gpu.cmd_virt + 1024) as *mut UpdateCursor, cmd);
    }
    let _ = send_cursor_cmd_raw(
        gpu,
        core::mem::size_of::<UpdateCursor>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );
}

pub fn show_cursor(visible: bool) {
    let mut guard = GPU_DEVICE.lock();
    let gpu = match guard.as_mut() {
        Some(g) => g,
        None => return,
    };

    let cmd = UpdateCursor {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
        pos: CursorPos {
            scanout_id: 0,
            x: gpu.cursor_x,
            y: gpu.cursor_y,
            padding: 0,
        },
        resource_id: if visible { gpu.cursor_resource_id } else { 0 },
        hot_x: gpu.cursor_hot_x,
        hot_y: gpu.cursor_hot_y,
        padding: 0,
    };
    unsafe {
        ptr::write((gpu.cmd_virt + 1024) as *mut UpdateCursor, cmd);
    }
    let _ = send_cursor_cmd_raw(
        gpu,
        core::mem::size_of::<UpdateCursor>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );
}

pub fn define_cursor(w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
    let mut guard = GPU_DEVICE.lock();
    let gpu = match guard.as_mut() {
        Some(g) => g,
        None => return,
    };

    const CURSOR_W: u32 = 64;
    const CURSOR_H: u32 = 64;
    const CURSOR_PAGES: usize = 4;

    if gpu.cursor_resource_id != 0 {
        let detach = ResourceDetachBacking {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
            resource_id: gpu.cursor_resource_id,
            padding: 0,
        };
        unsafe {
            ptr::write(gpu.cmd_virt as *mut ResourceDetachBacking, detach);
        }
        let _ = send_cmd_raw(
            gpu,
            core::mem::size_of::<ResourceDetachBacking>(),
            core::mem::size_of::<GpuCtrlHdr>(),
        );

        let unref = GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_UNREF);
        unsafe {
            ptr::write(gpu.cmd_virt as *mut GpuCtrlHdr, unref);
            ptr::write(
                (gpu.cmd_virt + core::mem::size_of::<GpuCtrlHdr>()) as *mut u32,
                gpu.cursor_resource_id,
            );
            ptr::write(
                (gpu.cmd_virt + core::mem::size_of::<GpuCtrlHdr>() + 4) as *mut u32,
                0u32,
            );
        }
        let _ = send_cmd_raw(
            gpu,
            core::mem::size_of::<GpuCtrlHdr>() + 8,
            core::mem::size_of::<GpuCtrlHdr>(),
        );
        gpu.cursor_resource_id = 0;
    }

    let cursor_res = gpu.next_resource_id;
    gpu.next_resource_id += 1;

    let create = ResourceCreate2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
        resource_id: cursor_res,
        format: VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
        width: CURSOR_W,
        height: CURSOR_H,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceCreate2d, create);
    }
    if !send_cmd_raw(
        gpu,
        core::mem::size_of::<ResourceCreate2d>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    ) {
        return;
    }

    let cursor_virt = phys_to_virt(gpu.cursor_buf_phys);
    unsafe {
        ptr::write_bytes(cursor_virt as *mut u8, 0, CURSOR_PAGES * FRAME_SIZE);
    }
    unsafe {
        let dst = cursor_virt as *mut u32;
        for row in 0..(h.min(CURSOR_H) as usize) {
            for col in 0..(w.min(CURSOR_W) as usize) {
                let src_idx = row * w as usize + col;
                let dst_idx = row * CURSOR_W as usize + col;
                let pixel = pixels.get(src_idx).copied().unwrap_or(0);
                ptr::write(dst.add(dst_idx), pixel);
            }
        }
    }

    let attach = ResourceAttachBacking {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
        resource_id: cursor_res,
        nr_entries: 1,
    };
    let entry = MemEntry {
        addr: gpu.cursor_buf_phys,
        length: (CURSOR_PAGES * FRAME_SIZE) as u32,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut ResourceAttachBacking, attach);
        ptr::write(
            (gpu.cmd_virt + core::mem::size_of::<ResourceAttachBacking>()) as *mut MemEntry,
            entry,
        );
    }
    if !send_cmd_raw(
        gpu,
        core::mem::size_of::<ResourceAttachBacking>() + core::mem::size_of::<MemEntry>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    ) {
        return;
    }

    let transfer = TransferToHost2d {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
        r_x: 0,
        r_y: 0,
        r_width: CURSOR_W,
        r_height: CURSOR_H,
        offset: 0,
        resource_id: cursor_res,
        padding: 0,
    };
    unsafe {
        ptr::write(gpu.cmd_virt as *mut TransferToHost2d, transfer);
    }
    let _ = send_cmd_raw(
        gpu,
        core::mem::size_of::<TransferToHost2d>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );

    gpu.cursor_resource_id = cursor_res;
    gpu.cursor_hot_x = hotx;
    gpu.cursor_hot_y = hoty;

    let update = UpdateCursor {
        hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
        pos: CursorPos {
            scanout_id: 0,
            x: gpu.cursor_x,
            y: gpu.cursor_y,
            padding: 0,
        },
        resource_id: cursor_res,
        hot_x: hotx,
        hot_y: hoty,
        padding: 0,
    };
    unsafe {
        ptr::write((gpu.cmd_virt + 1024) as *mut UpdateCursor, update);
    }
    let _ = send_cursor_cmd_raw(
        gpu,
        core::mem::size_of::<UpdateCursor>(),
        core::mem::size_of::<GpuCtrlHdr>(),
    );
}
