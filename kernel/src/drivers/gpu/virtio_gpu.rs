//! VirtIO GPU driver (2D mode).
//!
//! PCI device: vendor 0x1AF4, device 0x1050 (modern VirtIO GPU).
//! Uses the VirtIO transport layer with two queues: controlq (display commands)
//! and cursorq (cursor updates). Supports damage-based display updates via
//! TRANSFER_TO_HOST_2D + RESOURCE_FLUSH, and full-color ARGB hardware cursor.
//!
//! QEMU: `-vga virtio` (virtio-vga with VGA BIOS compat) or `-device virtio-gpu-pci`.

use super::GpuDriver;
use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::memory::physical;

// ──────────────────────────────────────────────
// VirtIO GPU Command Types
// ──────────────────────────────────────────────

const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32     = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32   = 0x0101;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32       = 0x0102;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32          = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32       = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32  = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;

const VIRTIO_GPU_CMD_UPDATE_CURSOR: u32        = 0x0300;
const VIRTIO_GPU_CMD_MOVE_CURSOR: u32          = 0x0301;

// ──────────────────────────────────────────────
// VirtIO GPU Response Types
// ──────────────────────────────────────────────

const VIRTIO_GPU_RESP_OK_NODATA: u32           = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32     = 0x1101;

// ──────────────────────────────────────────────
// VirtIO GPU Pixel Formats
// ──────────────────────────────────────────────

const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32    = 2;
const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32    = 1;

// ──────────────────────────────────────────────
// Command Structures (all repr(C), no padding)
// ──────────────────────────────────────────────

/// Common header for all VirtIO GPU commands and responses (24 bytes).
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

/// RESOURCE_CREATE_2D command (header + 4 fields).
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceCreate2d {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

/// RESOURCE_UNREF command.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceUnref {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    padding: u32,
}

/// SET_SCANOUT command.
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

/// TRANSFER_TO_HOST_2D command.
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

/// RESOURCE_FLUSH command.
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

/// RESOURCE_ATTACH_BACKING command header.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceAttachBacking {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
}

/// Memory entry for ATTACH_BACKING scatter-gather list.
#[repr(C)]
#[derive(Clone, Copy)]
struct MemEntry {
    addr: u64,
    length: u32,
    padding: u32,
}

/// RESOURCE_DETACH_BACKING command.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceDetachBacking {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    padding: u32,
}

/// Display info for one scanout (from GET_DISPLAY_INFO response).
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

/// GET_DISPLAY_INFO response (header + 16 scanouts).
#[repr(C)]
#[derive(Clone, Copy)]
struct RespDisplayInfo {
    hdr: GpuCtrlHdr,
    pmodes: [DisplayOne; 16],
}

/// UPDATE_CURSOR / MOVE_CURSOR command.
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

// ──────────────────────────────────────────────
// VirtIO GPU Driver State
// ──────────────────────────────────────────────

pub struct VirtioGpu {
    // VirtIO device handle
    device: VirtioDevice,

    // Queues
    controlq: VirtQueue,
    cursorq: VirtQueue,

    // Display state
    width: u32,
    height: u32,
    pitch: u32,
    fb_phys: u64,
    fb_pages: usize,

    // Resource tracking
    scanout_resource_id: u32,
    cursor_resource_id: u32,
    next_resource_id: u32,

    // Cursor hotspot and position (saved from define_cursor/move_cursor)
    cursor_hot_x: u32,
    cursor_hot_y: u32,
    cursor_x: u32,
    cursor_y: u32,

    // Pre-allocated DMA buffers for commands/responses (identity-mapped phys)
    cmd_buf: u64,   // 1 page (4096 bytes) for command payloads
    resp_buf: u64,  // 1 page (4096 bytes) for response payloads

    // Pre-allocated cursor backing store (64x64x4 = 16 KiB = 4 pages).
    // Allocated during init (under kernel CR3 with full identity mapping).
    // CRITICAL: user CR3 only identity-maps 64 MiB (PD[0..31]).
    // Runtime alloc_contiguous() may return frames above 64 MiB → page fault
    // when the kernel writes to them during a syscall under user CR3.
    cursor_buf_phys: u64,

    // Supported display modes (native first, then filtered COMMON_MODES)
    supported: Vec<(u32, u32)>,
}

// VirtioGpu is accessed under the GPU Spinlock
unsafe impl Send for VirtioGpu {}

impl VirtioGpu {
    // ── Command execution helpers ──

    /// Send a control command and wait for response.
    /// Returns the response type code.
    fn send_ctrl_cmd(&mut self, cmd: &[u8]) -> u32 {
        let cmd_len = cmd.len();
        if cmd_len > 4096 {
            crate::serial_println!("  VirtIO GPU: command too large ({} bytes)", cmd_len);
            return 0;
        }

        // Copy command to DMA buffer
        unsafe {
            core::ptr::copy_nonoverlapping(cmd.as_ptr(), self.cmd_buf as *mut u8, cmd_len);
        }

        // Zero response buffer header
        unsafe {
            core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24);
        }

        // Execute: cmd_buf (readable) → resp_buf (writable, enough for any response)
        let resp_len = 1024u32; // Large enough for display info response
        let notify_addr = self.device.notify_base;
        let notify_off_mul = self.device.notify_off_mul;
        let common_cfg = self.device.common_cfg;

        // Read queue notify offset for controlq (queue 0)
        virtio::mmio_write16(common_cfg + 0x16, 0); // select queue 0
        let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
        let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

        let result = self.controlq.execute_sync(
            &[(self.cmd_buf, cmd_len as u32)],
            &[(self.resp_buf, resp_len)],
            || { virtio::mmio_write16(notify_virt, 0); },
        );

        if result.is_none() {
            crate::serial_println!("  VirtIO GPU: command timeout (type={:#x})", {
                let hdr = unsafe { &*(cmd.as_ptr() as *const GpuCtrlHdr) };
                hdr.type_
            });
            return 0;
        }

        // Read ISR status to deassert any pending level-triggered PCI interrupt
        let _ = virtio::mmio_read8(self.device.isr_addr);

        // Read response type
        let resp_type = unsafe { core::ptr::read_volatile(self.resp_buf as *const u32) };
        resp_type
    }

    /// Send a cursor command via the cursor queue.
    fn send_cursor_cmd(&mut self, cmd: &[u8]) {
        let cmd_len = cmd.len();
        // Use second half of cmd_buf for cursor commands to avoid overlap
        let cursor_buf = self.cmd_buf + 2048;

        unsafe {
            core::ptr::copy_nonoverlapping(cmd.as_ptr(), cursor_buf as *mut u8, cmd_len);
        }

        // Zero response area
        let cursor_resp = self.resp_buf + 2048;
        unsafe {
            core::ptr::write_bytes(cursor_resp as *mut u8, 0, 24);
        }

        // Read queue notify offset for cursorq (queue 1)
        let common_cfg = self.device.common_cfg;
        let notify_addr = self.device.notify_base;
        let notify_off_mul = self.device.notify_off_mul;

        virtio::mmio_write16(common_cfg + 0x16, 1); // select queue 1
        let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
        let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

        self.cursorq.execute_sync(
            &[(cursor_buf, cmd_len as u32)],
            &[(cursor_resp, 24)],
            || { virtio::mmio_write16(notify_virt, 1); },
        );

        // Read ISR status to deassert any pending level-triggered PCI interrupt
        let _ = virtio::mmio_read8(self.device.isr_addr);
    }

    // ── GPU operations ──

    fn cmd_get_display_info(&mut self) -> Option<(u32, u32)> {
        let hdr = GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(&hdr as *const _ as *const u8, core::mem::size_of::<GpuCtrlHdr>())
        };

        let resp_type = self.send_ctrl_cmd(cmd_bytes);
        if resp_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
            crate::serial_println!("  VirtIO GPU: GET_DISPLAY_INFO failed (resp={:#x})", resp_type);
            return None;
        }

        // Parse response
        let resp = unsafe { &*(self.resp_buf as *const RespDisplayInfo) };
        for i in 0..16 {
            if resp.pmodes[i].enabled != 0 {
                let w = resp.pmodes[i].r_width;
                let h = resp.pmodes[i].r_height;
                crate::serial_println!("  VirtIO GPU: scanout {} enabled: {}x{}", i, w, h);
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
        }

        // Default if no enabled scanout found
        Some((1024, 768))
    }

    fn cmd_resource_create_2d(&mut self, resource_id: u32, format: u32, width: u32, height: u32) -> bool {
        let cmd = ResourceCreate2d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id,
            format,
            width,
            height,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<ResourceCreate2d>())
        };
        let resp = self.send_ctrl_cmd(bytes);
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_resource_unref(&mut self, resource_id: u32) {
        let cmd = ResourceUnref {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_UNREF),
            resource_id,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<ResourceUnref>())
        };
        self.send_ctrl_cmd(bytes);
    }

    fn cmd_attach_backing(&mut self, resource_id: u32, pages_phys: u64, num_pages: usize) -> bool {
        // Build command: header + entries in a temp buffer
        // The attach_backing header is followed by an array of MemEntry structs
        let hdr = ResourceAttachBacking {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
            resource_id,
            nr_entries: num_pages as u32,
        };

        let hdr_size = core::mem::size_of::<ResourceAttachBacking>();
        let entry_size = core::mem::size_of::<MemEntry>();
        let total = hdr_size + num_pages * entry_size;

        if total > 4096 {
            // Too many pages for a single command buffer — use a single large entry
            // (backing store IS contiguous)
            let hdr_single = ResourceAttachBacking {
                hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
                resource_id,
                nr_entries: 1,
            };
            let entry = MemEntry {
                addr: pages_phys,
                length: (num_pages * 4096) as u32,
                padding: 0,
            };

            // Write header + single entry to cmd buffer
            unsafe {
                let dst = self.cmd_buf as *mut u8;
                core::ptr::copy_nonoverlapping(&hdr_single as *const _ as *const u8, dst, hdr_size);
                core::ptr::copy_nonoverlapping(&entry as *const _ as *const u8, dst.add(hdr_size), entry_size);
            }

            let cmd_len = hdr_size + entry_size;
            // Zero response
            unsafe { core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24); }

            let common_cfg = self.device.common_cfg;
            let notify_addr = self.device.notify_base;
            let notify_off_mul = self.device.notify_off_mul;
            virtio::mmio_write16(common_cfg + 0x16, 0);
            let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
            let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

            let result = self.controlq.execute_sync(
                &[(self.cmd_buf, cmd_len as u32)],
                &[(self.resp_buf, 24)],
                || { virtio::mmio_write16(notify_virt, 0); },
            );

            let _ = virtio::mmio_read8(self.device.isr_addr);
            if result.is_none() { return false; }
            let resp = unsafe { core::ptr::read_volatile(self.resp_buf as *const u32) };
            return resp == VIRTIO_GPU_RESP_OK_NODATA;
        }

        // Write header to cmd buffer
        unsafe {
            let dst = self.cmd_buf as *mut u8;
            core::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, dst, hdr_size);

            // Write entries: one per page
            for i in 0..num_pages {
                let entry = MemEntry {
                    addr: pages_phys + (i as u64) * 4096,
                    length: 4096,
                    padding: 0,
                };
                core::ptr::copy_nonoverlapping(
                    &entry as *const _ as *const u8,
                    dst.add(hdr_size + i * entry_size),
                    entry_size,
                );
            }
        }

        // Zero response
        unsafe { core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24); }

        let cmd_len = total;
        let common_cfg = self.device.common_cfg;
        let notify_addr = self.device.notify_base;
        let notify_off_mul = self.device.notify_off_mul;
        virtio::mmio_write16(common_cfg + 0x16, 0);
        let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
        let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

        let result = self.controlq.execute_sync(
            &[(self.cmd_buf, cmd_len as u32)],
            &[(self.resp_buf, 24)],
            || { virtio::mmio_write16(notify_virt, 0); },
        );

        let _ = virtio::mmio_read8(self.device.isr_addr);
        if result.is_none() { return false; }
        let resp = unsafe { core::ptr::read_volatile(self.resp_buf as *const u32) };
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_set_scanout(&mut self, scanout_id: u32, resource_id: u32, width: u32, height: u32) -> bool {
        let cmd = SetScanout {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_SET_SCANOUT),
            r_x: 0,
            r_y: 0,
            r_width: width,
            r_height: height,
            scanout_id,
            resource_id,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<SetScanout>())
        };
        let resp = self.send_ctrl_cmd(bytes);
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_transfer_to_host_2d(&mut self, resource_id: u32, x: u32, y: u32, w: u32, h: u32) -> bool {
        // VirtIO GPU TRANSFER_TO_HOST_2D reads the backing store as "tightly packed":
        // row stride = r_width * bpp. For the framebuffer resource, our backing store
        // has stride = resource_width * bpp (pitch). Transfer full-width rows so the
        // packed stride matches the framebuffer stride, with offset pointing to the
        // first row. For other resources (e.g. cursor), the backing store IS tightly
        // packed, so use the original rect and offset=0.
        let (r_x, r_y, r_width, offset) = if resource_id == self.scanout_resource_id {
            (0u32, y, self.width, (y as u64) * (self.pitch as u64))
        } else {
            (x, y, w, 0u64)
        };
        let cmd = TransferToHost2d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r_x,
            r_y,
            r_width,
            r_height: h,
            offset,
            resource_id,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<TransferToHost2d>())
        };
        let resp = self.send_ctrl_cmd(bytes);
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_resource_flush(&mut self, resource_id: u32, x: u32, y: u32, w: u32, h: u32) -> bool {
        let cmd = ResourceFlush {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            r_x: x,
            r_y: y,
            r_width: w,
            r_height: h,
            resource_id,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<ResourceFlush>())
        };
        let resp = self.send_ctrl_cmd(bytes);
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_detach_backing(&mut self, resource_id: u32) {
        let cmd = ResourceDetachBacking {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
            resource_id,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<ResourceDetachBacking>())
        };
        self.send_ctrl_cmd(bytes);
    }

    /// Allocate framebuffer pages and set up the display pipeline.
    fn setup_display(&mut self, width: u32, height: u32) -> bool {
        self.width = width;
        self.height = height;
        self.pitch = width * 4;

        let fb_size = (width as usize) * (height as usize) * 4;
        let num_pages = (fb_size + 4095) / 4096;

        // Allocate contiguous physical pages for framebuffer (identity-mapped)
        let fb_phys = match physical::alloc_contiguous(num_pages) {
            Some(p) => p.as_u64(),
            None => {
                crate::serial_println!("  VirtIO GPU: failed to allocate {} pages for framebuffer", num_pages);
                return false;
            }
        };

        // Zero the framebuffer
        unsafe {
            core::ptr::write_bytes(fb_phys as *mut u8, 0, num_pages * 4096);
        }

        self.fb_phys = fb_phys;
        self.fb_pages = num_pages;

        // Create 2D resource
        let res_id = self.next_resource_id;
        self.next_resource_id += 1;

        if !self.cmd_resource_create_2d(res_id, VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM, width, height) {
            crate::serial_println!("  VirtIO GPU: RESOURCE_CREATE_2D failed");
            return false;
        }

        // Attach backing store
        if !self.cmd_attach_backing(res_id, fb_phys, num_pages) {
            crate::serial_println!("  VirtIO GPU: RESOURCE_ATTACH_BACKING failed");
            self.cmd_resource_unref(res_id);
            return false;
        }

        // Set scanout
        if !self.cmd_set_scanout(0, res_id, width, height) {
            crate::serial_println!("  VirtIO GPU: SET_SCANOUT failed");
            self.cmd_resource_unref(res_id);
            return false;
        }

        self.scanout_resource_id = res_id;

        crate::serial_println!("  VirtIO GPU: display {}x{} resource={} fb={:#x} ({} pages)",
            width, height, res_id, fb_phys, num_pages);

        true
    }
}

// ──────────────────────────────────────────────
// GpuDriver Trait Implementation
// ──────────────────────────────────────────────

impl GpuDriver for VirtioGpu {
    fn name(&self) -> &str {
        "VirtIO GPU"
    }

    fn set_mode(&mut self, width: u32, height: u32, _bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // Tear down old display if active
        if self.scanout_resource_id != 0 {
            // Disable scanout
            self.cmd_set_scanout(0, 0, 0, 0);
            self.cmd_detach_backing(self.scanout_resource_id);
            self.cmd_resource_unref(self.scanout_resource_id);
            self.scanout_resource_id = 0;

            // Free old framebuffer pages
            if self.fb_phys != 0 {
                for i in 0..self.fb_pages {
                    physical::free_frame(crate::memory::address::PhysAddr::new(
                        self.fb_phys + (i as u64) * 4096,
                    ));
                }
                self.fb_phys = 0;
                self.fb_pages = 0;
            }
        }

        if self.setup_display(width, height) {
            // Do an initial full transfer + flush
            self.cmd_transfer_to_host_2d(self.scanout_resource_id, 0, 0, width, height);
            self.cmd_resource_flush(self.scanout_resource_id, 0, 0, width, height);

            Some((self.width, self.height, self.pitch, self.fb_phys as u32))
        } else {
            None
        }
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys as u32)
    }

    fn supported_modes(&self) -> &[(u32, u32)] {
        &self.supported
    }

    fn has_accel(&self) -> bool {
        true // Software fill/copy directly on guest RAM framebuffer
    }

    fn accel_fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) -> bool {
        if self.fb_phys == 0 || w == 0 || h == 0 {
            return false;
        }
        // Clamp to display bounds
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        if w == 0 || h == 0 {
            return false;
        }

        let fb = self.fb_phys as *mut u32;
        let pitch_u32 = (self.pitch / 4) as usize;
        for row in y..(y + h) {
            let offset = (row as usize) * pitch_u32 + (x as usize);
            unsafe {
                let dst = fb.add(offset);
                for col in 0..(w as usize) {
                    core::ptr::write_volatile(dst.add(col), color);
                }
            }
        }
        true
    }

    fn accel_copy_rect(&mut self, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32) -> bool {
        if self.fb_phys == 0 || w == 0 || h == 0 {
            return false;
        }

        let fb = self.fb_phys as *mut u32;
        let pitch_u32 = (self.pitch / 4) as usize;

        // Copy bottom-to-top if destination is below source (avoid overwriting)
        if dy <= sy {
            for row in 0..(h as usize) {
                let src_off = (sy as usize + row) * pitch_u32 + sx as usize;
                let dst_off = (dy as usize + row) * pitch_u32 + dx as usize;
                unsafe { core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize); }
            }
        } else {
            for row in (0..(h as usize)).rev() {
                let src_off = (sy as usize + row) * pitch_u32 + sx as usize;
                let dst_off = (dy as usize + row) * pitch_u32 + dx as usize;
                unsafe { core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize); }
            }
        }
        true
    }

    fn update_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.scanout_resource_id == 0 || w == 0 || h == 0 {
            return;
        }

        // Clamp to display bounds
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);

        if w == 0 || h == 0 {
            return;
        }

        // Transfer dirty region from guest RAM to device resource
        self.cmd_transfer_to_host_2d(self.scanout_resource_id, x, y, w, h);
        // Flush to display
        self.cmd_resource_flush(self.scanout_resource_id, x, y, w, h);
    }

    fn transfer_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.scanout_resource_id == 0 || w == 0 || h == 0 {
            return;
        }
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        if w == 0 || h == 0 {
            return;
        }
        // Only transfer — no flush
        self.cmd_transfer_to_host_2d(self.scanout_resource_id, x, y, w, h);
    }

    fn flush_display(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.scanout_resource_id == 0 || w == 0 || h == 0 {
            return;
        }
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        if w == 0 || h == 0 {
            return;
        }
        // Only flush — all transfers already done
        self.cmd_resource_flush(self.scanout_resource_id, x, y, w, h);
    }

    fn has_hw_cursor(&self) -> bool {
        true
    }

    fn define_cursor(&mut self, w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
        // VirtIO GPU cursor must be 64x64 — pad smaller cursors
        let cursor_w: u32 = 64;
        let cursor_h: u32 = 64;
        let cursor_pages: usize = 4; // 64*64*4 = 16384 bytes = 4 pages

        let cursor_phys = self.cursor_buf_phys;
        if cursor_phys == 0 {
            return;
        }

        // Detach + unref old cursor resource FIRST (before reusing backing buffer)
        if self.cursor_resource_id != 0 {
            self.cmd_detach_backing(self.cursor_resource_id);
            self.cmd_resource_unref(self.cursor_resource_id);
            self.cursor_resource_id = 0;
        }

        // Create a new cursor resource
        let cursor_res = self.next_resource_id;
        self.next_resource_id += 1;

        if !self.cmd_resource_create_2d(cursor_res, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM, cursor_w, cursor_h) {
            return;
        }

        // Zero the pre-allocated cursor buffer (transparent)
        unsafe {
            core::ptr::write_bytes(cursor_phys as *mut u8, 0, cursor_pages * 4096);
        }

        // Copy pixel data into 64x64 buffer (src may be smaller)
        unsafe {
            let dst = cursor_phys as *mut u32;
            for row in 0..(h.min(cursor_h) as usize) {
                for col in 0..(w.min(cursor_w) as usize) {
                    let src_idx = row * (w as usize) + col;
                    let dst_idx = row * (cursor_w as usize) + col;
                    let pixel = if src_idx < pixels.len() { pixels[src_idx] } else { 0 };
                    core::ptr::write_volatile(dst.add(dst_idx), pixel);
                }
            }
        }

        // Attach pre-allocated backing
        if !self.cmd_attach_backing(cursor_res, cursor_phys, cursor_pages) {
            self.cmd_resource_unref(cursor_res);
            return;
        }

        // Transfer cursor pixels to host
        self.cmd_transfer_to_host_2d(cursor_res, 0, 0, cursor_w, cursor_h);

        self.cursor_resource_id = cursor_res;
        self.cursor_hot_x = hotx;
        self.cursor_hot_y = hoty;

        // Send UPDATE_CURSOR to set the cursor image at the current position
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
            pos: CursorPos { scanout_id: 0, x: self.cursor_x, y: self.cursor_y, padding: 0 },
            resource_id: cursor_res,
            hot_x: hotx,
            hot_y: hoty,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<UpdateCursor>())
        };
        self.send_cursor_cmd(bytes);
    }

    fn move_cursor(&mut self, x: u32, y: u32) {
        self.cursor_x = x;
        self.cursor_y = y;
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_MOVE_CURSOR),
            pos: CursorPos { scanout_id: 0, x, y, padding: 0 },
            resource_id: self.cursor_resource_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<UpdateCursor>())
        };
        self.send_cursor_cmd(bytes);
    }

    fn show_cursor(&mut self, visible: bool) {
        let res_id = if visible { self.cursor_resource_id } else { 0 };
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
            pos: CursorPos { scanout_id: 0, x: 0, y: 0, padding: 0 },
            resource_id: res_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<UpdateCursor>())
        };
        self.send_cursor_cmd(bytes);
    }

    fn has_double_buffer(&self) -> bool {
        false
    }
}

// ──────────────────────────────────────────────
// Initialization
// ──────────────────────────────────────────────

/// Initialize and register the VirtIO GPU driver.
/// Called from HAL factory during PCI probe.
pub fn init_and_register(pci_dev: &PciDevice) -> bool {
    crate::serial_println!("  VirtIO GPU: initializing (PCI {:02x}:{:02x}.{})",
        pci_dev.bus, pci_dev.device, pci_dev.function);

    // 1. Find PCI capabilities
    let caps = match virtio::find_capabilities(pci_dev) {
        Some(c) => c,
        None => return false,
    };

    // 2. Create device handle (maps BARs)
    let device = VirtioDevice::new(pci_dev, &caps);

    // 3-6. Initialize device (reset, negotiate features)
    let desired = VIRTIO_F_VERSION_1;
    match device.init_device(desired) {
        Ok(_negotiated) => {
            crate::serial_println!("  VirtIO GPU: features negotiated OK");
        }
        Err(e) => {
            crate::serial_println!("  VirtIO GPU: init failed: {}", e);
            return false;
        }
    }

    // 7. Set up virtqueues
    let controlq = match device.setup_queue(0) {
        Some(q) => q,
        None => {
            crate::serial_println!("  VirtIO GPU: failed to set up controlq");
            return false;
        }
    };

    let cursorq = match device.setup_queue(1) {
        Some(q) => q,
        None => {
            crate::serial_println!("  VirtIO GPU: failed to set up cursorq");
            return false;
        }
    };

    // Allocate DMA buffers for commands and responses
    let cmd_buf = match physical::alloc_frame() {
        Some(p) => {
            unsafe { core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4096); }
            p.as_u64()
        }
        None => {
            crate::serial_println!("  VirtIO GPU: failed to allocate cmd buffer");
            return false;
        }
    };

    let resp_buf = match physical::alloc_frame() {
        Some(p) => {
            unsafe { core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4096); }
            p.as_u64()
        }
        None => {
            crate::serial_println!("  VirtIO GPU: failed to allocate resp buffer");
            return false;
        }
    };

    // 8. Set DRIVER_OK
    device.set_driver_ok();

    // Clear any pending interrupt from device initialization
    let _ = virtio::mmio_read8(device.isr_addr);

    crate::serial_println!("  VirtIO GPU: device ready (DRIVER_OK)");

    // Pre-allocate cursor backing store (64x64x4 = 16 KiB = 4 pages).
    // MUST be allocated here during boot (kernel CR3 active, full 128 MiB identity map).
    // Runtime allocation during syscalls may land above 64 MiB — user CR3 only
    // identity-maps PD[0..31] (64 MiB), so writing to higher frames page-faults.
    let cursor_buf_phys = match physical::alloc_contiguous(4) {
        Some(p) => {
            unsafe { core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4 * 4096); }
            p.as_u64()
        }
        None => {
            crate::serial_println!("  VirtIO GPU: failed to allocate cursor buffer");
            0
        }
    };

    let mut gpu = VirtioGpu {
        device,
        controlq,
        cursorq,
        width: 0,
        height: 0,
        pitch: 0,
        fb_phys: 0,
        fb_pages: 0,
        scanout_resource_id: 0,
        cursor_resource_id: 0,
        next_resource_id: 1,
        cursor_hot_x: 0,
        cursor_hot_y: 0,
        cursor_x: 0,
        cursor_y: 0,
        cmd_buf,
        resp_buf,
        cursor_buf_phys,
        supported: Vec::new(),
    };

    // 9. Query native display resolution and build supported modes list.
    //
    // With hardware acceleration (KVM/HVF/WHPX), the guest boots so fast that
    // the EDID data from `edid=on,xres=...,yres=...` may not be ready when
    // GET_DISPLAY_INFO fires for the first time. The device then reports the
    // VGA-default 640x480 instead of the requested resolution.
    //
    // Retry up to 5 times with 50ms delays to give the host time to populate EDID.
    let mut native = gpu.cmd_get_display_info().unwrap_or((1024, 768));
    if native == (640, 480) {
        crate::serial_println!("  VirtIO GPU: got 640x480 (VGA default), retrying for EDID...");
        for attempt in 1..=5 {
            crate::arch::x86::pit::delay_ms(50);
            if let Some(res) = gpu.cmd_get_display_info() {
                if res != (640, 480) {
                    native = res;
                    crate::serial_println!("  VirtIO GPU: EDID ready after {}ms: {}x{}",
                        attempt * 50, res.0, res.1);
                    break;
                }
            }
        }
    }
    // Enforce minimum 1024x768 — never start with a smaller resolution.
    if native.0 < 1024 || native.1 < 768 {
        crate::serial_println!("  VirtIO GPU: {}x{} below minimum, forcing 1024x768",
            native.0, native.1);
        native = (1024, 768);
    }
    crate::serial_println!("  VirtIO GPU: native display {}x{}", native.0, native.1);

    // Build supported modes: start with COMMON_MODES, add native if not already present
    let mut modes: Vec<(u32, u32)> = super::COMMON_MODES.to_vec();
    if !modes.contains(&native) && native.0 > 0 && native.1 > 0 {
        modes.insert(0, native);
    }
    gpu.supported = modes;

    // Use VirtIO GPU's native display resolution (reported by host).
    // Unlike Bochs VGA / SVGA which inherit VBE boot resolution, VirtIO GPU
    // manages its own display pipeline and should use the native size.
    let (width, height) = native;

    // 10-13. Set up display pipeline
    if !gpu.setup_display(width, height) {
        crate::serial_println!("  VirtIO GPU: failed to set up display");
        return false;
    }

    // Update canonical framebuffer info to point at VirtIO's guest RAM buffer.
    // This triggers the boot_console change hook which re-renders the splash
    // logo centered for the new resolution — no manual copy needed.
    crate::drivers::framebuffer::update(
        gpu.fb_phys as u32,
        gpu.pitch,
        width,
        height,
        32,
    );

    // Initial transfer + flush
    gpu.cmd_transfer_to_host_2d(gpu.scanout_resource_id, 0, 0, width, height);
    gpu.cmd_resource_flush(gpu.scanout_resource_id, 0, 0, width, height);

    crate::serial_println!("[OK] VirtIO GPU: {}x{} (fb={:#x})", width, height, gpu.fb_phys);

    // Register as the active GPU driver
    super::register(Box::new(gpu));
    true
}

/// Probe: initialize VirtIO GPU and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("VirtIO GPU")
}
