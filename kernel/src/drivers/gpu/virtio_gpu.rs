//! VirtIO GPU driver (2D mode).
//!
//! PCI device: vendor 0x1AF4, device 0x1050 (modern VirtIO GPU).
//! Uses the VirtIO transport layer with two queues: controlq (display commands)
//! and cursorq (cursor updates). Supports damage-based display updates via
//! TRANSFER_TO_HOST_2D + RESOURCE_FLUSH, and full-color ARGB hardware cursor.
//!
//! QEMU: `-vga virtio` (virtio-vga with VGA BIOS compat) or `-device virtio-gpu-pci`.

use super::GpuDriver;
use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

// ──────────────────────────────────────────────
// VirtIO GPU Command Types
// ──────────────────────────────────────────────

const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x0102;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;

// 3D (virgl) commands
const VIRTIO_GPU_CMD_CTX_CREATE: u32 = 0x0200;
const VIRTIO_GPU_CMD_CTX_DESTROY: u32 = 0x0201;
const VIRTIO_GPU_CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
const VIRTIO_GPU_CMD_CTX_DETACH_RESOURCE: u32 = 0x0203;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_3D: u32 = 0x0205;
const VIRTIO_GPU_CMD_TRANSFER_FROM_HOST_3D: u32 = 0x0206;
const VIRTIO_GPU_CMD_SUBMIT_3D: u32 = 0x0207;

// EDID query (QEMU 3.1+, requires edid=on in device config)
const VIRTIO_GPU_CMD_GET_EDID: u32 = 0x010A;

const VIRTIO_GPU_CMD_UPDATE_CURSOR: u32 = 0x0300;
const VIRTIO_GPU_CMD_MOVE_CURSOR: u32 = 0x0301;

// Feature bits
const VIRTIO_GPU_F_VIRGL: u64 = 1 << 1;

// ──────────────────────────────────────────────
// VirtIO GPU Response Types
// ──────────────────────────────────────────────

const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const VIRTIO_GPU_RESP_OK_EDID: u32 = 0x1104;

static LAST_CTRL_CMD_TYPE: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_CMD_TYPE: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_RESOURCE_ID: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_X: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_Y: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_W: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_H: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_OFFSET_LO: AtomicU32 = AtomicU32::new(0);
static LAST_TRANSFER_OFFSET_HI: AtomicU32 = AtomicU32::new(0);
static LAST_CONTROL_Q_QSIZE: AtomicU32 = AtomicU32::new(0);
static LAST_CONTROL_Q_AVAIL: AtomicU32 = AtomicU32::new(0);
static LAST_CONTROL_Q_USED: AtomicU32 = AtomicU32::new(0);
static LAST_CONTROL_Q_FREE: AtomicU32 = AtomicU32::new(0);
static LAST_CONTROL_Q_BROKEN: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_Q_QSIZE: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_Q_AVAIL: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_Q_USED: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_Q_FREE: AtomicU32 = AtomicU32::new(0);
static LAST_CURSOR_Q_BROKEN: AtomicU32 = AtomicU32::new(0);

// ──────────────────────────────────────────────
// VirtIO GPU Pixel Formats
// ──────────────────────────────────────────────

const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;
const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32 = 1;

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

/// CTX_CREATE command.
#[repr(C)]
#[derive(Clone, Copy)]
struct CtxCreate {
    hdr: GpuCtrlHdr,
    nlen: u32,
    context_init: u32,
    debug_name: [u8; 64],
}

/// CTX_DESTROY command.
#[repr(C)]
#[derive(Clone, Copy)]
struct CtxDestroy {
    hdr: GpuCtrlHdr,
}

/// CTX_ATTACH_RESOURCE / CTX_DETACH_RESOURCE command.
#[repr(C)]
#[derive(Clone, Copy)]
struct CtxResource {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    padding: u32,
}

/// RESOURCE_CREATE_3D command.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceCreate3d {
    hdr: GpuCtrlHdr,
    resource_id: u32,
    target: u32,
    format: u32,
    bind: u32,
    width: u32,
    height: u32,
    depth: u32,
    array_size: u32,
    last_level: u32,
    nr_samples: u32,
    flags: u32,
    padding: u32,
}

/// TRANSFER_TO_HOST_3D command.
#[repr(C)]
#[derive(Clone, Copy)]
struct TransferToHost3d {
    hdr: GpuCtrlHdr,
    box_x: u32,
    box_y: u32,
    box_z: u32,
    box_w: u32,
    box_h: u32,
    box_d: u32,
    offset: u64,
    resource_id: u32,
    level: u32,
    stride: u32,
    layer_stride: u32,
}

/// TRANSFER_FROM_HOST_3D command.
#[repr(C)]
#[derive(Clone, Copy)]
struct TransferFromHost3d {
    hdr: GpuCtrlHdr,
    box_x: u32,
    box_y: u32,
    box_z: u32,
    box_w: u32,
    box_h: u32,
    box_d: u32,
    offset: u64,
    resource_id: u32,
    level: u32,
    stride: u32,
    layer_stride: u32,
}

/// SUBMIT_3D command header (variable-length data follows).
#[repr(C)]
#[derive(Clone, Copy)]
struct Submit3d {
    hdr: GpuCtrlHdr,
    size: u32,
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

/// GET_EDID command (QEMU 3.1+, requires `edid=on` in device config).
#[repr(C)]
#[derive(Clone, Copy)]
struct GetEdid {
    hdr: GpuCtrlHdr,
    scanout: u32,
    padding: u32,
}

/// GET_EDID response (header + size + 1024 bytes EDID data).
#[repr(C)]
#[derive(Clone, Copy)]
struct RespEdid {
    hdr: GpuCtrlHdr,
    size: u32,
    padding: u32,
    edid: [u8; 1024],
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
// Per-scanout state (multi-monitor)
// ──────────────────────────────────────────────
//
// Output 0 ("primary") historically lives in the inline `width / height /
// pitch / fb_phys / fb_pages / scanout_resource_id` fields of `VirtioGpu`
// — we keep that representation untouched so the fast paths (DMA back-
// buffer, accel_fill_rect, hardware cursor, etc.) stay intact at the cost
// of zero refactoring risk. Outputs 1..num_scanouts live in
// `extra_scanouts` and are configured lazily on the first
// `set_mode_for_output(id, …)` call. A scanout entry with `mirror_of =
// Some(other)` shares its scanned-out resource_id with another output
// (per the virtio-gpu spec's "create one framebuffer, link to all
// displays" mirroring idiom) and owns no framebuffer pages of its own.

#[derive(Debug, Clone, Copy)]
struct ScanoutState {
    width: u32,
    height: u32,
    pitch: u32,
    /// Guest-physical base of this scanout's framebuffer. `0` when the
    /// scanout is currently a mirror (no own backing store).
    fb_phys: u64,
    /// Number of pages backing `fb_phys`. `0` when this scanout mirrors
    /// another output.
    fb_pages: usize,
    /// virtio-gpu resource_id used for `SET_SCANOUT(id, resource_id, …)`.
    /// For mirror entries this is the *source* output's resource_id.
    /// `0` when this scanout is disabled.
    resource_id: u32,
    /// `Some(source_id)` when this scanout mirrors another output's
    /// framebuffer; `None` for own-framebuffer scanouts.
    mirror_of: Option<u32>,
}

impl ScanoutState {
    const fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            pitch: 0,
            fb_phys: 0,
            fb_pages: 0,
            resource_id: 0,
            mirror_of: None,
        }
    }
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

    // Display state for output 0 (primary scanout). See ScanoutState
    // comment above for why these inline fields stay in place.
    width: u32,
    height: u32,
    pitch: u32,
    fb_phys: u64,
    fb_pages: usize,

    /// Scanout state for outputs 1..num_scanouts_advertised. Index 0 in
    /// this Vec corresponds to output_id 1.
    extra_scanouts: alloc::vec::Vec<ScanoutState>,

    /// Number of scanouts the device advertises in its `virtio_gpu_config`
    /// (offset 8). Set during init from `device.device_cfg`. Never
    /// exceeds `output::MAX_OUTPUTS` (= 16, the spec's maximum).
    num_scanouts_advertised: u32,

    /// Pending hotplug / configuration events drained by the compositor
    /// via `SYS_DISPLAY_POLL_EVENT`. Pushed by the IRQ-driven `events_read`
    /// observer; popped by `poll_display_event()`.
    pending_events: alloc::collections::VecDeque<crate::drivers::gpu::output::DisplayEvent>,

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
    cmd_buf: u64,  // 1 page (4096 bytes) for command payloads
    resp_buf: u64, // 1 page (4096 bytes) for response payloads

    // Pre-allocated cursor backing store (64x64x4 = 16 KiB = 4 pages).
    // Allocated during init (under kernel CR3 with full identity mapping).
    // CRITICAL: user CR3 only identity-maps 64 MiB (PD[0..31]).
    // Runtime alloc_contiguous() may return frames above 64 MiB → page fault
    // when the kernel writes to them during a syscall under user CR3.
    cursor_buf_phys: u64,

    // Supported display modes (native first, then filtered COMMON_MODES)
    supported: Vec<(u32, u32)>,

    // Monitor detection: per-scanout display info cached from GET_DISPLAY_INFO
    display_infos: Vec<(u32, u32, bool)>, // (width, height, enabled) per scanout
    enabled_scanout_count: u32,

    // 3D (virgl) state
    virgl_capable: bool,
    virgl_ctx_id: u32, // active virgl rendering context (0 = none)
    cmd_3d_buf: u64,   // 64 KiB DMA buffer for 3D command submission

    /// IDs of 3D resources that are currently alive (created but not yet destroyed).
    /// Checked by dma_surface_download to reject requests for dead/freed surfaces,
    /// which would otherwise loop indefinitely returning all-zero data.
    live_3d_resources: Vec<u32>,
}

// VirtioGpu is accessed under the GPU Mutex (yields instead of spinning)
unsafe impl Send for VirtioGpu {}

impl VirtioGpu {
    pub fn last_command_types() -> (u32, u32) {
        (
            LAST_CTRL_CMD_TYPE.load(Ordering::Relaxed),
            LAST_CURSOR_CMD_TYPE.load(Ordering::Relaxed),
        )
    }

    pub fn last_transfer_info() -> (u32, u32, u32, u32, u32, u64) {
        let off_lo = LAST_TRANSFER_OFFSET_LO.load(Ordering::Relaxed) as u64;
        let off_hi = LAST_TRANSFER_OFFSET_HI.load(Ordering::Relaxed) as u64;
        (
            LAST_TRANSFER_RESOURCE_ID.load(Ordering::Relaxed),
            LAST_TRANSFER_X.load(Ordering::Relaxed),
            LAST_TRANSFER_Y.load(Ordering::Relaxed),
            LAST_TRANSFER_W.load(Ordering::Relaxed),
            LAST_TRANSFER_H.load(Ordering::Relaxed),
            off_lo | (off_hi << 32),
        )
    }

    pub fn queue_debug_info() -> ((u16, u16, u16, u16, bool), (u16, u16, u16, u16, bool)) {
        (
            (
                LAST_CONTROL_Q_QSIZE.load(Ordering::Relaxed) as u16,
                LAST_CONTROL_Q_AVAIL.load(Ordering::Relaxed) as u16,
                LAST_CONTROL_Q_USED.load(Ordering::Relaxed) as u16,
                LAST_CONTROL_Q_FREE.load(Ordering::Relaxed) as u16,
                LAST_CONTROL_Q_BROKEN.load(Ordering::Relaxed) != 0,
            ),
            (
                LAST_CURSOR_Q_QSIZE.load(Ordering::Relaxed) as u16,
                LAST_CURSOR_Q_AVAIL.load(Ordering::Relaxed) as u16,
                LAST_CURSOR_Q_USED.load(Ordering::Relaxed) as u16,
                LAST_CURSOR_Q_FREE.load(Ordering::Relaxed) as u16,
                LAST_CURSOR_Q_BROKEN.load(Ordering::Relaxed) != 0,
            ),
        )
    }

    fn snapshot_controlq(&self) {
        let (qs, avail, used, free, broken) = self.controlq.debug_state();
        LAST_CONTROL_Q_QSIZE.store(qs as u32, Ordering::Relaxed);
        LAST_CONTROL_Q_AVAIL.store(avail as u32, Ordering::Relaxed);
        LAST_CONTROL_Q_USED.store(used as u32, Ordering::Relaxed);
        LAST_CONTROL_Q_FREE.store(free as u32, Ordering::Relaxed);
        LAST_CONTROL_Q_BROKEN.store(broken as u32, Ordering::Relaxed);
    }

    fn snapshot_cursorq(&self) {
        let (qs, avail, used, free, broken) = self.cursorq.debug_state();
        LAST_CURSOR_Q_QSIZE.store(qs as u32, Ordering::Relaxed);
        LAST_CURSOR_Q_AVAIL.store(avail as u32, Ordering::Relaxed);
        LAST_CURSOR_Q_USED.store(used as u32, Ordering::Relaxed);
        LAST_CURSOR_Q_FREE.store(free as u32, Ordering::Relaxed);
        LAST_CURSOR_Q_BROKEN.store(broken as u32, Ordering::Relaxed);
    }

    // ── Command execution helpers ──

    /// Send a control command and wait for response.
    /// Returns the response type code.
    fn send_ctrl_cmd(&mut self, cmd: &[u8]) -> u32 {
        let cmd_len = cmd.len();
        if cmd_len > 4096 {
            crate::serial_verbose_println!("  VirtIO GPU: command too large ({} bytes)", cmd_len);
            return 0;
        }
        if cmd_len >= core::mem::size_of::<GpuCtrlHdr>() {
            let hdr = unsafe { &*(cmd.as_ptr() as *const GpuCtrlHdr) };
            LAST_CTRL_CMD_TYPE.store(hdr.type_, Ordering::Relaxed);
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
            || {
                virtio::mmio_write16(notify_virt, 0);
            },
        );
        self.snapshot_controlq();

        if result.is_none() {
            crate::serial_println!("[gpu] VirtIO GPU: control queue failed (type={:#x})", {
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
        if cmd_len >= core::mem::size_of::<GpuCtrlHdr>() {
            let hdr = unsafe { &*(cmd.as_ptr() as *const GpuCtrlHdr) };
            LAST_CURSOR_CMD_TYPE.store(hdr.type_, Ordering::Relaxed);
        }
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

        let result = self.cursorq.execute_sync(
            &[(cursor_buf, cmd_len as u32)],
            &[(cursor_resp, 24)],
            || {
                virtio::mmio_write16(notify_virt, 1);
            },
        );
        self.snapshot_cursorq();

        if result.is_none() {
            crate::serial_println!("[gpu] VirtIO GPU: cursor queue failed");
            return;
        }

        // Read ISR status to deassert any pending level-triggered PCI interrupt
        let _ = virtio::mmio_read8(self.device.isr_addr);
    }

    // ── GPU operations ──

    fn cmd_get_display_info(&mut self) -> Option<(u32, u32)> {
        let hdr = GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(
                &hdr as *const _ as *const u8,
                core::mem::size_of::<GpuCtrlHdr>(),
            )
        };

        let resp_type = self.send_ctrl_cmd(cmd_bytes);
        if resp_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
            crate::serial_verbose_println!(
                "  VirtIO GPU: GET_DISPLAY_INFO failed (resp={:#x})",
                resp_type
            );
            return None;
        }

        // Parse response
        let resp = unsafe { &*(self.resp_buf as *const RespDisplayInfo) };
        for i in 0..16 {
            if resp.pmodes[i].enabled != 0 {
                let w = resp.pmodes[i].r_width;
                let h = resp.pmodes[i].r_height;
                crate::serial_verbose_println!("  VirtIO GPU: scanout {} enabled: {}x{}", i, w, h);
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
        }

        // Default if no enabled scanout found
        Some((1024, 768))
    }

    /// Query GET_DISPLAY_INFO and cache all scanout infos.
    fn query_all_display_infos(&mut self) {
        let hdr = GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(
                &hdr as *const _ as *const u8,
                core::mem::size_of::<GpuCtrlHdr>(),
            )
        };
        let resp_type = self.send_ctrl_cmd(cmd_bytes);
        if resp_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
            return;
        }
        let resp = unsafe { &*(self.resp_buf as *const RespDisplayInfo) };
        self.display_infos.clear();
        self.enabled_scanout_count = 0;
        for i in 0..16 {
            let d = &resp.pmodes[i];
            let enabled = d.enabled != 0 && d.r_width > 0 && d.r_height > 0;
            self.display_infos.push((d.r_width, d.r_height, enabled));
            if enabled {
                self.enabled_scanout_count += 1;
            }
        }
    }

    /// Read EDID for a scanout via VIRTIO_GPU_CMD_GET_EDID (QEMU 3.1+, edid=on).
    fn cmd_get_edid(&mut self, scanout: u32) -> Option<[u8; 128]> {
        let cmd = GetEdid {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_EDID),
            scanout,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<GetEdid>(),
            )
        };
        let resp_type = self.send_ctrl_cmd(bytes);
        if resp_type != VIRTIO_GPU_RESP_OK_EDID {
            return None;
        }
        let resp = unsafe { &*(self.resp_buf as *const RespEdid) };
        if resp.size < 128 {
            return None;
        }
        let mut edid = [0u8; 128];
        edid.copy_from_slice(&resp.edid[..128]);
        Some(edid)
    }

    fn cmd_resource_create_2d(
        &mut self,
        resource_id: u32,
        format: u32,
        width: u32,
        height: u32,
    ) -> bool {
        let cmd = ResourceCreate2d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id,
            format,
            width,
            height,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<ResourceCreate2d>(),
            )
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
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<ResourceUnref>(),
            )
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
                core::ptr::copy_nonoverlapping(
                    &entry as *const _ as *const u8,
                    dst.add(hdr_size),
                    entry_size,
                );
            }

            let cmd_len = hdr_size + entry_size;
            // Zero response
            unsafe {
                core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24);
            }

            let common_cfg = self.device.common_cfg;
            let notify_addr = self.device.notify_base;
            let notify_off_mul = self.device.notify_off_mul;
            virtio::mmio_write16(common_cfg + 0x16, 0);
            let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
            let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

            let result = self.controlq.execute_sync(
                &[(self.cmd_buf, cmd_len as u32)],
                &[(self.resp_buf, 24)],
                || {
                    virtio::mmio_write16(notify_virt, 0);
                },
            );

            let _ = virtio::mmio_read8(self.device.isr_addr);
            if result.is_none() {
                return false;
            }
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
        unsafe {
            core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24);
        }

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
            || {
                virtio::mmio_write16(notify_virt, 0);
            },
        );

        let _ = virtio::mmio_read8(self.device.isr_addr);
        if result.is_none() {
            return false;
        }
        let resp = unsafe { core::ptr::read_volatile(self.resp_buf as *const u32) };
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_set_scanout(
        &mut self,
        scanout_id: u32,
        resource_id: u32,
        width: u32,
        height: u32,
    ) -> bool {
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
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<SetScanout>(),
            )
        };
        let resp = self.send_ctrl_cmd(bytes);
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }

    fn cmd_transfer_to_host_2d(
        &mut self,
        resource_id: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> bool {
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
        LAST_TRANSFER_RESOURCE_ID.store(resource_id, Ordering::Relaxed);
        LAST_TRANSFER_X.store(r_x, Ordering::Relaxed);
        LAST_TRANSFER_Y.store(r_y, Ordering::Relaxed);
        LAST_TRANSFER_W.store(r_width, Ordering::Relaxed);
        LAST_TRANSFER_H.store(h, Ordering::Relaxed);
        LAST_TRANSFER_OFFSET_LO.store(offset as u32, Ordering::Relaxed);
        LAST_TRANSFER_OFFSET_HI.store((offset >> 32) as u32, Ordering::Relaxed);
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
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<TransferToHost2d>(),
            )
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
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<ResourceFlush>(),
            )
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
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<ResourceDetachBacking>(),
            )
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
                crate::serial_verbose_println!(
                    "  VirtIO GPU: failed to allocate {} pages for framebuffer",
                    num_pages
                );
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
            crate::serial_verbose_println!("  VirtIO GPU: RESOURCE_CREATE_2D failed");
            return false;
        }

        // Attach backing store
        if !self.cmd_attach_backing(res_id, fb_phys, num_pages) {
            crate::serial_verbose_println!("  VirtIO GPU: RESOURCE_ATTACH_BACKING failed");
            self.cmd_resource_unref(res_id);
            return false;
        }

        // Set scanout
        if !self.cmd_set_scanout(0, res_id, width, height) {
            crate::serial_verbose_println!("  VirtIO GPU: SET_SCANOUT failed");
            self.cmd_resource_unref(res_id);
            return false;
        }

        self.scanout_resource_id = res_id;

        crate::serial_verbose_println!(
            "  VirtIO GPU: display {}x{} resource={} fb={:#x} ({} pages)",
            width,
            height,
            res_id,
            fb_phys,
            num_pages
        );

        true
    }

    // ── 3D (virgl) operations ──

    /// Create a virgl rendering context.
    fn cmd_ctx_create(&mut self, ctx_id: u32) -> bool {
        let mut cmd = CtxCreate {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_CTX_CREATE),
            nlen: 6,
            context_init: 0,
            debug_name: [0u8; 64],
        };
        cmd.hdr.ctx_id = ctx_id;
        cmd.debug_name[..6].copy_from_slice(b"anyOS\0");
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<CtxCreate>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Destroy a virgl rendering context.
    fn cmd_ctx_destroy(&mut self, ctx_id: u32) -> bool {
        let mut cmd = CtxDestroy {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_CTX_DESTROY),
        };
        cmd.hdr.ctx_id = ctx_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<CtxDestroy>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Attach a resource to a context.
    fn cmd_ctx_attach_resource(&mut self, ctx_id: u32, resource_id: u32) -> bool {
        let mut cmd = CtxResource {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_CTX_ATTACH_RESOURCE),
            resource_id,
            padding: 0,
        };
        cmd.hdr.ctx_id = ctx_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<CtxResource>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Detach a resource from a context.
    fn cmd_ctx_detach_resource(&mut self, ctx_id: u32, resource_id: u32) -> bool {
        let mut cmd = CtxResource {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_CTX_DETACH_RESOURCE),
            resource_id,
            padding: 0,
        };
        cmd.hdr.ctx_id = ctx_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<CtxResource>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Create a 3D resource (texture, buffer, etc.).
    fn cmd_resource_create_3d(
        &mut self,
        resource_id: u32,
        target: u32,
        format: u32,
        bind: u32,
        width: u32,
        height: u32,
        depth: u32,
        array_size: u32,
        last_level: u32,
        nr_samples: u32,
        flags: u32,
    ) -> bool {
        let cmd = ResourceCreate3d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_3D),
            resource_id,
            target,
            format,
            bind,
            width,
            height,
            depth,
            array_size,
            last_level,
            nr_samples,
            flags,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<ResourceCreate3d>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Transfer data from guest to host for a 3D resource.
    fn cmd_transfer_to_host_3d(
        &mut self,
        resource_id: u32,
        x: u32,
        y: u32,
        z: u32,
        w: u32,
        h: u32,
        d: u32,
        offset: u64,
        level: u32,
        stride: u32,
        layer_stride: u32,
        ctx_id: u32,
    ) -> bool {
        let mut cmd = TransferToHost3d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_3D),
            box_x: x,
            box_y: y,
            box_z: z,
            box_w: w,
            box_h: h,
            box_d: d,
            offset,
            resource_id,
            level,
            stride,
            layer_stride,
        };
        cmd.hdr.ctx_id = ctx_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<TransferToHost3d>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Transfer data from host to guest for a 3D resource.
    fn cmd_transfer_from_host_3d(
        &mut self,
        resource_id: u32,
        x: u32,
        y: u32,
        z: u32,
        w: u32,
        h: u32,
        d: u32,
        offset: u64,
        level: u32,
        stride: u32,
        layer_stride: u32,
        ctx_id: u32,
    ) -> bool {
        let mut cmd = TransferFromHost3d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_FROM_HOST_3D),
            box_x: x,
            box_y: y,
            box_z: z,
            box_w: w,
            box_h: h,
            box_d: d,
            offset,
            resource_id,
            level,
            stride,
            layer_stride,
        };
        cmd.hdr.ctx_id = ctx_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<TransferFromHost3d>(),
            )
        };
        self.send_ctrl_cmd(bytes) == VIRTIO_GPU_RESP_OK_NODATA
    }

    /// Submit a virgl command buffer to the host renderer.
    /// `data` contains raw Gallium/virgl command words.
    fn cmd_submit_3d(&mut self, data: &[u8], ctx_id: u32) -> bool {
        if data.is_empty() || self.cmd_3d_buf == 0 {
            return false;
        }

        // Max payload = 64 KiB buffer minus header
        let hdr_size = core::mem::size_of::<Submit3d>();
        let max_data = 64 * 1024 - hdr_size;
        if data.len() > max_data {
            return false;
        }

        let mut hdr = Submit3d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_SUBMIT_3D),
            size: data.len() as u32,
            padding: 0,
        };
        hdr.hdr.ctx_id = ctx_id;

        // Copy header + data into the 3D DMA buffer
        unsafe {
            let dst = self.cmd_3d_buf as *mut u8;
            core::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, dst, hdr_size);
            core::ptr::copy_nonoverlapping(data.as_ptr(), dst.add(hdr_size), data.len());
        }

        let total_len = (hdr_size + data.len()) as u32;

        // Zero response
        unsafe {
            core::ptr::write_bytes(self.resp_buf as *mut u8, 0, 24);
        }

        let common_cfg = self.device.common_cfg;
        let notify_addr = self.device.notify_base;
        let notify_off_mul = self.device.notify_off_mul;
        virtio::mmio_write16(common_cfg + 0x16, 0);
        let notify_off = virtio::mmio_read16(common_cfg + 0x1E);
        let notify_virt = notify_addr + (notify_off as u64) * (notify_off_mul as u64);

        let result = self.controlq.execute_sync(
            &[(self.cmd_3d_buf, total_len)],
            &[(self.resp_buf, 24)],
            || {
                virtio::mmio_write16(notify_virt, 0);
            },
        );

        let _ = virtio::mmio_read8(self.device.isr_addr);
        if result.is_none() {
            return false;
        }
        let resp = unsafe { core::ptr::read_volatile(self.resp_buf as *const u32) };
        resp == VIRTIO_GPU_RESP_OK_NODATA
    }
}

// ──────────────────────────────────────────────
// GpuDriver Trait Implementation
// ──────────────────────────────────────────────

impl GpuDriver for VirtioGpu {
    fn name(&self) -> &str {
        "VirtIO GPU"
    }

    fn driver_type_name(&self) -> &str {
        if self.virgl_capable {
            "virgl"
        } else {
            "none"
        }
    }

    fn has_3d(&self) -> bool {
        self.virgl_capable
    }

    fn sync(&mut self) {
        if !self.virgl_capable || self.virgl_ctx_id == 0 {
            return;
        }
        // Send a virgl NOP command (header only, length=0) and wait for the
        // synchronous response. This stalls until virglrenderer has processed
        // all previously submitted commands, ensuring rendered pixels are
        // committed before TRANSFER_FROM_HOST_3D reads them back.
        // Header: (length=0 << 16) | (obj=0 << 8) | cmd=0 = 0x00000000
        let nop_word: u32 = 0u32;
        let bytes = unsafe { core::slice::from_raw_parts(&nop_word as *const u32 as *const u8, 4) };
        self.cmd_submit_3d(bytes, self.virgl_ctx_id);
    }

    fn submit_3d_commands(&mut self, words: &[u32]) -> bool {
        if !self.virgl_capable {
            return false;
        }

        // Ensure we have a virgl context
        if self.virgl_ctx_id == 0 {
            self.virgl_ctx_id = 1;
            if !self.cmd_ctx_create(self.virgl_ctx_id) {
                self.virgl_ctx_id = 0;
                return false;
            }
        }

        // Submit raw virgl command words as bytes
        let bytes =
            unsafe { core::slice::from_raw_parts(words.as_ptr() as *const u8, words.len() * 4) };
        self.cmd_submit_3d(bytes, self.virgl_ctx_id)
    }

    fn dma_surface_upload(&mut self, sid: u32, data: &[u8], width: u32, height: u32) -> bool {
        if !self.virgl_capable {
            return false;
        }

        let ctx_id = self.virgl_ctx_id;
        let row_bytes = (width * 4) as usize;
        let staging_cap = 64 * 1024usize;
        let rows_per_chunk = (staging_cap / row_bytes).max(1) as u32;
        let num_pages = (staging_cap + 4095) / 4096;

        let mut y = 0u32;
        while y < height {
            let chunk_h = rows_per_chunk.min(height - y);
            let chunk_bytes = (chunk_h as usize) * row_bytes;
            let data_offset = (y as usize) * row_bytes;

            if data_offset + chunk_bytes > data.len() {
                return false;
            }

            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(data_offset),
                    self.cmd_3d_buf as *mut u8,
                    chunk_bytes,
                );
            }

            if !self.cmd_attach_backing(sid, self.cmd_3d_buf, num_pages) {
                return false;
            }

            let ok = self.cmd_transfer_to_host_3d(
                sid,
                0,
                y,
                0,
                width,
                chunk_h,
                1,
                0,
                0,
                row_bytes as u32,
                0,
                ctx_id,
            );

            // Detach backing
            let detach = ResourceDetachBacking {
                hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
                resource_id: sid,
                padding: 0,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    &detach as *const _ as *const u8,
                    core::mem::size_of::<ResourceDetachBacking>(),
                )
            };
            self.send_ctrl_cmd(bytes);

            if !ok {
                return false;
            }
            y += chunk_h;
        }

        true
    }

    fn dma_surface_download(&mut self, sid: u32, buf: &mut [u8], width: u32, height: u32) -> bool {
        if !self.virgl_capable {
            return false;
        }
        // Reject downloads for destroyed/unknown surfaces immediately.
        // Without this check a freed surface keeps the loop running forever,
        // consuming CPU while returning all-zero data ("zombie surface").
        if !self.live_3d_resources.contains(&sid) {
            return false;
        }

        let ctx_id = self.virgl_ctx_id;
        let row_bytes = (width * 4) as usize;
        let staging_cap = 64 * 1024usize;
        // How many rows fit in the 64 KiB staging buffer?
        let rows_per_chunk = (staging_cap / row_bytes).max(1) as u32;

        let num_pages = (staging_cap + 4095) / 4096;

        // Sync the virgl pipeline — ensures the host renderer has finalized all
        // pending rendering before we read back.  (RESOURCE_FLUSH is a 2D scanout
        // command and does nothing for 3D virgl resources.)
        self.sync();

        let mut y = 0u32;
        while y < height {
            let chunk_h = rows_per_chunk.min(height - y);
            let chunk_bytes = (chunk_h as usize) * row_bytes;
            let buf_offset = (y as usize) * row_bytes;

            unsafe {
                core::ptr::write_bytes(self.cmd_3d_buf as *mut u8, 0, chunk_bytes);
            }

            if !self.cmd_attach_backing(sid, self.cmd_3d_buf, num_pages) {
                return false;
            }

            let ok = self.cmd_transfer_from_host_3d(
                sid,
                0,
                y,
                0,
                width,
                chunk_h,
                1,
                0,
                0,
                row_bytes as u32,
                0,
                ctx_id,
            );

            if ok {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        self.cmd_3d_buf as *const u8,
                        buf.as_mut_ptr().add(buf_offset),
                        chunk_bytes,
                    );
                }
            }

            // Detach backing
            let detach = ResourceDetachBacking {
                hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
                resource_id: sid,
                padding: 0,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    &detach as *const _ as *const u8,
                    core::mem::size_of::<ResourceDetachBacking>(),
                )
            };
            self.send_ctrl_cmd(bytes);

            if !ok {
                return false;
            }
            y += chunk_h;
        }

        true
    }

    fn create_3d_resource(
        &mut self,
        target: u32,
        format: u32,
        bind: u32,
        width: u32,
        height: u32,
        depth: u32,
        array_size: u32,
        last_level: u32,
        nr_samples: u32,
        flags: u32,
    ) -> Option<u32> {
        if !self.virgl_capable {
            return None;
        }

        // Ensure virgl context exists
        if self.virgl_ctx_id == 0 {
            self.virgl_ctx_id = 1;
            if !self.cmd_ctx_create(self.virgl_ctx_id) {
                self.virgl_ctx_id = 0;
                return None;
            }
        }

        // Allocate resource ID from counter (starts at 1, shared with scanout/cursor)
        let resource_id = self.next_resource_id;
        self.next_resource_id += 1;

        if !self.cmd_resource_create_3d(
            resource_id,
            target,
            format,
            bind,
            width,
            height,
            depth,
            array_size,
            last_level,
            nr_samples,
            flags,
        ) {
            return None;
        }

        // Attach resource to the virgl rendering context
        if !self.cmd_ctx_attach_resource(self.virgl_ctx_id, resource_id) {
            self.cmd_resource_unref(resource_id);
            return None;
        }

        self.live_3d_resources.push(resource_id);
        Some(resource_id)
    }

    fn destroy_3d_resource(&mut self, resource_id: u32) -> bool {
        if !self.virgl_capable {
            return false;
        }
        // Remove from live set — subsequent dma_surface_download calls for this ID will fail fast.
        self.live_3d_resources.retain(|&id| id != resource_id);
        // Detach from virgl context if active
        if self.virgl_ctx_id != 0 {
            self.cmd_ctx_detach_resource(self.virgl_ctx_id, resource_id);
        }
        self.cmd_resource_unref(resource_id);
        true
    }

    fn set_mode(&mut self, width: u32, height: u32, _bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // Idempotent re-apply: when displayd pushes the same mode that
        // is already active (cold-boot layout matches the persisted
        // setup), reuse the existing scanout state instead of tearing
        // it down and re-allocating. The boot-time fb sits in the lower
        // 64 MiB identity-map region; freeing it and re-allocating now
        // (under user CR3 with the persisted layout call) hands back
        // pages above that boundary, after which setup_display's
        // write_bytes-on-phys-addr faults. Same root cause as the
        // secondary-scanout path in set_mode_for_output.
        if self.scanout_resource_id != 0
            && self.width == width
            && self.height == height
            && self.fb_phys != 0
        {
            return Some((self.width, self.height, self.pitch, self.fb_phys as u32));
        }

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
                unsafe {
                    core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize);
                }
            }
        } else {
            for row in (0..(h as usize)).rev() {
                let src_off = (sy as usize + row) * pitch_u32 + sx as usize;
                let dst_off = (dy as usize + row) * pitch_u32 + dx as usize;
                unsafe {
                    core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize);
                }
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
        // Only transfer — no flush.
        // Keep this path single-shot and side-effect minimal: if the control queue
        // is already unhealthy, retry/drain logic tends to touch more queue state
        // and makes post-mortem diagnosis harder. One failed transfer is safer
        // than compounding corruption with a second in-flight command.
        if !self.cmd_transfer_to_host_2d(self.scanout_resource_id, x, y, w, h) {
            crate::serial_println!(
                "[gpu] VirtIO TRANSFER_TO_HOST_2D failed: ({},{} {}x{}) res={}",
                x,
                y,
                w,
                h,
                self.scanout_resource_id
            );
        }
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
        // Only flush — all transfers already done.
        // Retry once on failure (same rationale and drain pattern as transfer_rect).
        if !self.cmd_resource_flush(self.scanout_resource_id, x, y, w, h) {
            for _ in 0..1_000_000u32 {
                core::hint::spin_loop();
            }
            while self.controlq.poll_used().is_some() {}
            if !self.cmd_resource_flush(self.scanout_resource_id, x, y, w, h) {
                crate::serial_println!(
                    "[gpu] VirtIO RESOURCE_FLUSH failed: ({},{} {}x{}) res={}",
                    x,
                    y,
                    w,
                    h,
                    self.scanout_resource_id
                );
            }
        }
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

        if !self.cmd_resource_create_2d(
            cursor_res,
            VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
            cursor_w,
            cursor_h,
        ) {
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
                    let pixel = if src_idx < pixels.len() {
                        pixels[src_idx]
                    } else {
                        0
                    };
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
            pos: CursorPos {
                scanout_id: 0,
                x: self.cursor_x,
                y: self.cursor_y,
                padding: 0,
            },
            resource_id: cursor_res,
            hot_x: hotx,
            hot_y: hoty,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<UpdateCursor>(),
            )
        };
        self.send_cursor_cmd(bytes);
    }

    fn move_cursor(&mut self, x: u32, y: u32) {
        self.cursor_x = x;
        self.cursor_y = y;
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_MOVE_CURSOR),
            pos: CursorPos {
                scanout_id: 0,
                x,
                y,
                padding: 0,
            },
            resource_id: self.cursor_resource_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<UpdateCursor>(),
            )
        };
        self.send_cursor_cmd(bytes);
    }

    fn show_cursor(&mut self, visible: bool) {
        let res_id = if visible { self.cursor_resource_id } else { 0 };
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
            pos: CursorPos {
                scanout_id: 0,
                x: 0,
                y: 0,
                padding: 0,
            },
            resource_id: res_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<UpdateCursor>(),
            )
        };
        self.send_cursor_cmd(bytes);
    }

    fn has_double_buffer(&self) -> bool {
        false
    }

    // ── Per-output cursor (multi-monitor) ──

    fn move_cursor_for_output(&mut self, output_id: u32, x: u32, y: u32) {
        if output_id >= self.num_scanouts_advertised {
            return;
        }
        self.cursor_x = x;
        self.cursor_y = y;
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_MOVE_CURSOR),
            pos: CursorPos {
                scanout_id: output_id,
                x,
                y,
                padding: 0,
            },
            resource_id: self.cursor_resource_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<UpdateCursor>(),
            )
        };
        self.send_cursor_cmd(bytes);
    }

    fn show_cursor_for_output(&mut self, output_id: u32, visible: bool) {
        if output_id >= self.num_scanouts_advertised {
            return;
        }
        // resource_id == 0 hides the cursor on the targeted scanout
        // per virtio-gpu spec; we keep cursor_resource_id intact so
        // the next show call finds the same image.
        let res_id = if visible { self.cursor_resource_id } else { 0 };
        let cmd = UpdateCursor {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_UPDATE_CURSOR),
            pos: CursorPos {
                scanout_id: output_id,
                x: self.cursor_x,
                y: self.cursor_y,
                padding: 0,
            },
            resource_id: res_id,
            hot_x: self.cursor_hot_x,
            hot_y: self.cursor_hot_y,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<UpdateCursor>(),
            )
        };
        self.send_cursor_cmd(bytes);
    }

    // ── Monitor / EDID ──

    fn display_count(&self) -> u32 {
        // Total scanouts the device advertises, regardless of how many
        // currently report a connected monitor. Multi-monitor user-space
        // (displayd, display-settings) iterates all advertised outputs to
        // distinguish "physically possible but disconnected" from "doesn't
        // exist at all".
        self.num_scanouts_advertised.max(1)
    }

    fn read_edid(&mut self, output: u32) -> Option<[u8; 128]> {
        self.cmd_get_edid(output)
    }

    fn display_info(&self, output: u32) -> Option<(u32, u32, bool)> {
        self.display_infos.get(output as usize).copied()
    }

    fn refresh_display_info(&mut self) {
        self.query_all_display_infos();
    }

    // ── Multi-monitor (per-output) implementations ────────────────────

    fn set_mode_for_output(
        &mut self,
        output_id: u32,
        width: u32,
        height: u32,
        bpp: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        if output_id >= self.num_scanouts_advertised {
            return None;
        }
        if output_id == 0 {
            return self.set_mode(width, height, bpp);
        }

        // width == 0 || height == 0 → disable scanout.
        if width == 0 || height == 0 {
            self.disable_extra_scanout(output_id);
            return Some((0, 0, 0, 0));
        }

        // Idempotent re-apply: when displayd pushes the same mode that
        // is already active (cold-boot layout matches the persisted
        // setup), reuse the existing scanout state instead of tearing
        // it down and re-allocating. The boot-time fb sits in the lower
        // 64 MiB identity-map region; freeing it and re-allocating now
        // (under user CR3 with the persisted layout call) can hand back
        // pages above that boundary, after which cmd_set_scanout fails
        // and mode_for_output returns None for the rest of the session.
        let idx = (output_id - 1) as usize;
        if let Some(s) = self.extra_scanouts.get(idx) {
            if s.resource_id != 0 && s.width == width && s.height == height && s.mirror_of.is_none()
            {
                return Some((s.width, s.height, s.pitch, s.fb_phys as u32));
            }
        }

        // Tear down whatever was on this scanout previously (mirror or own).
        self.disable_extra_scanout(output_id);

        // Allocate framebuffer pages + resource for this output.
        let pitch = width * 4;
        let fb_size = (width as usize) * (height as usize) * 4;
        let num_pages = (fb_size + 4095) / 4096;
        let fb_phys = match physical::alloc_contiguous(num_pages) {
            Some(p) => p.as_u64(),
            None => {
                crate::serial_verbose_println!(
                    "  VirtIO GPU: scanout {} fb alloc failed ({} pages)",
                    output_id,
                    num_pages
                );
                return None;
            }
        };
        unsafe {
            core::ptr::write_bytes(fb_phys as *mut u8, 0, num_pages * 4096);
        }
        let res_id = self.next_resource_id;
        self.next_resource_id += 1;
        if !self.cmd_resource_create_2d(res_id, VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM, width, height) {
            for i in 0..num_pages {
                physical::free_frame(crate::memory::address::PhysAddr::new(
                    fb_phys + (i as u64) * 4096,
                ));
            }
            return None;
        }
        if !self.cmd_attach_backing(res_id, fb_phys, num_pages) {
            self.cmd_resource_unref(res_id);
            for i in 0..num_pages {
                physical::free_frame(crate::memory::address::PhysAddr::new(
                    fb_phys + (i as u64) * 4096,
                ));
            }
            return None;
        }
        if !self.cmd_set_scanout(output_id, res_id, width, height) {
            self.cmd_detach_backing(res_id);
            self.cmd_resource_unref(res_id);
            for i in 0..num_pages {
                physical::free_frame(crate::memory::address::PhysAddr::new(
                    fb_phys + (i as u64) * 4096,
                ));
            }
            return None;
        }
        let idx = (output_id - 1) as usize;
        self.extra_scanouts[idx] = ScanoutState {
            width,
            height,
            pitch,
            fb_phys,
            fb_pages: num_pages,
            resource_id: res_id,
            mirror_of: None,
        };
        crate::serial_verbose_println!(
            "  VirtIO GPU: scanout {} active {}x{} resource={} fb={:#x}",
            output_id,
            width,
            height,
            res_id,
            fb_phys
        );
        // Initial transfer + flush so the host shows zeros instead of garbage.
        self.cmd_transfer_to_host_2d(res_id, 0, 0, width, height);
        self.cmd_resource_flush(res_id, 0, 0, width, height);
        Some((width, height, pitch, fb_phys as u32))
    }

    fn mode_for_output(&self, output_id: u32) -> Option<(u32, u32, u32, u32)> {
        if output_id == 0 {
            if self.width > 0 && self.height > 0 {
                Some((self.width, self.height, self.pitch, self.fb_phys as u32))
            } else {
                None
            }
        } else {
            let idx = (output_id - 1) as usize;
            let s = self.extra_scanouts.get(idx)?;
            if s.width == 0 || s.height == 0 {
                return None;
            }
            // Mirror entries report the source's fb_phys so callers can map it.
            let (fb_phys, pitch) = if let Some(src) = s.mirror_of {
                if src == 0 {
                    (self.fb_phys, self.pitch)
                } else {
                    let si = (src - 1) as usize;
                    let ss = &self.extra_scanouts[si];
                    (ss.fb_phys, ss.pitch)
                }
            } else {
                (s.fb_phys, s.pitch)
            };
            Some((s.width, s.height, pitch, fb_phys as u32))
        }
    }

    fn transfer_rect_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        if output_id == 0 {
            self.transfer_rect(x, y, w, h);
            return;
        }
        let idx = (output_id - 1) as usize;
        let s = match self.extra_scanouts.get(idx) {
            Some(s) if s.resource_id != 0 && w > 0 && h > 0 => *s,
            _ => return,
        };
        // Mirror outputs share the source resource_id, so transfers were
        // already done when the source did its transfer. Only flush below.
        if s.mirror_of.is_some() {
            return;
        }
        let x = x.min(s.width);
        let y = y.min(s.height);
        let w = w.min(s.width - x);
        let h = h.min(s.height - y);
        if w == 0 || h == 0 {
            return;
        }
        // Use the same row-major full-width transfer trick as the primary
        // scanout path (cmd_transfer_to_host_2d's special case keys on
        // resource_id == self.scanout_resource_id, so for extras we just
        // emit a rect transfer with the right offset directly).
        let offset = (y as u64) * (s.pitch as u64) + (x as u64) * 4;
        let cmd = TransferToHost2d {
            hdr: GpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r_x: x,
            r_y: y,
            r_width: w,
            r_height: h,
            offset,
            resource_id: s.resource_id,
            padding: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const _ as *const u8,
                core::mem::size_of::<TransferToHost2d>(),
            )
        };
        let _ = self.send_ctrl_cmd(bytes);
    }

    fn flush_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        if output_id == 0 {
            self.flush_display(x, y, w, h);
            return;
        }
        let idx = (output_id - 1) as usize;
        let s = match self.extra_scanouts.get(idx) {
            Some(s) if s.resource_id != 0 && w > 0 && h > 0 => *s,
            _ => return,
        };
        let x = x.min(s.width);
        let y = y.min(s.height);
        let w = w.min(s.width - x);
        let h = h.min(s.height - y);
        if w == 0 || h == 0 {
            return;
        }
        // For mirror entries, the resource_id belongs to the source —
        // RESOURCE_FLUSH against it shows the same pixels on this scanout
        // because SET_SCANOUT linked the resource to both outputs.
        let _ = self.cmd_resource_flush(s.resource_id, x, y, w, h);
    }

    fn update_rect_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        // Combined transfer + flush. Mirror outputs skip the transfer
        // (already done by the source) but still need a per-scanout flush.
        self.transfer_rect_for_output(output_id, x, y, w, h);
        self.flush_for_output(output_id, x, y, w, h);
    }

    fn set_output_mirror(&mut self, output_id: u32, source_output_id: u32) -> bool {
        if output_id == 0 || output_id >= self.num_scanouts_advertised {
            // Output 0 is always own-framebuffer in this driver layout —
            // applying a mirror to it would invalidate the inline scanout
            // fields that the legacy single-output paths still rely on.
            return false;
        }
        if source_output_id == output_id {
            return false;
        }
        // Resolve source (resource_id, width, height).
        let (src_res, src_w, src_h, src_pitch, src_fb) = if source_output_id == 0 {
            if self.scanout_resource_id == 0 {
                return false;
            }
            (
                self.scanout_resource_id,
                self.width,
                self.height,
                self.pitch,
                self.fb_phys,
            )
        } else {
            let si = (source_output_id - 1) as usize;
            let ss = match self.extra_scanouts.get(si) {
                Some(s) if s.resource_id != 0 && s.mirror_of.is_none() => *s,
                _ => return false,
            };
            (ss.resource_id, ss.width, ss.height, ss.pitch, ss.fb_phys)
        };
        // Tear down whatever this scanout had so we don't leak frames.
        self.disable_extra_scanout(output_id);

        if !self.cmd_set_scanout(output_id, src_res, src_w, src_h) {
            return false;
        }
        let idx = (output_id - 1) as usize;
        self.extra_scanouts[idx] = ScanoutState {
            width: src_w,
            height: src_h,
            pitch: src_pitch,
            fb_phys: src_fb,
            fb_pages: 0, // mirror owns no pages
            resource_id: src_res,
            mirror_of: Some(source_output_id),
        };
        true
    }

    fn output_info(&mut self, output_id: u32) -> Option<crate::drivers::gpu::output::OutputInfo> {
        use crate::drivers::gpu::output::{OutputInfo, OutputMode};
        if output_id >= self.num_scanouts_advertised {
            return None;
        }
        let mut info = OutputInfo::placeholder(output_id);
        // From cached GET_DISPLAY_INFO. Note: virtio-gpu reports `enabled=0`
        // for scanouts that have no resource attached yet, even when EDID
        // is valid and a host monitor is logically attached. Treat "has a
        // non-zero preferred mode" as the canonical "connected" signal so
        // user-space layout code (init_secondary_outputs, displayd) sees
        // the output before it has been activated for the first time.
        if let Some((w, h, _enabled)) = self.display_infos.get(output_id as usize).copied() {
            if w > 0 && h > 0 {
                info.preferred_mode = Some(OutputMode::new(w, h));
                info.connected = true;
            }
        }
        // Current mode (own state, more authoritative than the cached
        // GET_DISPLAY_INFO which may be stale after a mode change).
        if let Some((w, h, p, fb)) = self.mode_for_output(output_id) {
            let _ = (p, fb);
            if w > 0 && h > 0 {
                info.current_mode = Some(OutputMode::new(w, h));
            }
        }
        // Modes list = COMMON_MODES filtered to ≤ preferred (best effort).
        // Drivers without per-output mode lists treat the union as candidates;
        // displayd's UI shows only entries that fit the connected monitor.
        let cap = info
            .preferred_mode
            .map(|m| (m.width, m.height))
            .unwrap_or((u32::MAX, u32::MAX));
        for &(w, h) in super::COMMON_MODES {
            if w <= cap.0 && h <= cap.1 {
                info.modes.push(OutputMode::new(w, h));
            }
        }
        // Preferred mode may not be in COMMON_MODES (e.g. 1280×800 laptop
        // panels, ultrawide 3440×1440). Layout validation requires an exact
        // match against `modes`, so make sure the preferred entry is always
        // present — otherwise displayd's first apply gets rejected with
        // ModeUnsupported on cold boot.
        if let Some(pref) = info.preferred_mode {
            if !info
                .modes
                .iter()
                .any(|m| m.width == pref.width && m.height == pref.height)
            {
                info.modes.push(pref);
            }
        }
        if let Some(cur) = info.current_mode {
            if !info
                .modes
                .iter()
                .any(|m| m.width == cur.width && m.height == cur.height)
            {
                info.modes.push(cur);
            }
        }
        // EDID-derived metadata (manufacturer, physical size, hash).
        if let Some(edid) = self.cmd_get_edid(output_id) {
            info.edid_hash = crate::drivers::gpu::output::edid_hash(&edid);
            let raw = ((edid[8] as u16) << 8) | (edid[9] as u16);
            info.manufacturer[0] = b'A' + (((raw >> 10) & 0x1F) as u8).saturating_sub(1);
            info.manufacturer[1] = b'A' + (((raw >> 5) & 0x1F) as u8).saturating_sub(1);
            info.manufacturer[2] = b'A' + ((raw & 0x1F) as u8).saturating_sub(1);
            info.manufacturer[3] = 0;
            info.physical_mm = ((edid[21] as u16) * 10, (edid[22] as u16) * 10);

            // Parse EDID detailed timing #1 (bytes 54..71) as a fallback
            // preferred_mode source. virtio-gpu reports r_width = 0 for
            // scanouts that have no host-side resource attached yet, even
            // when EDID is fully populated and the host monitor would
            // happily run the timing — without this fallback secondary
            // outputs look "disconnected" to user-space layout code on
            // the very first cold boot.
            if info.preferred_mode.is_none() {
                let h_active = (edid[56] as u32) | (((edid[58] as u32) >> 4) << 8);
                let v_active = (edid[59] as u32) | (((edid[61] as u32) >> 4) << 8);
                if h_active >= 640 && v_active >= 480 {
                    info.preferred_mode = Some(OutputMode::new(h_active, v_active));
                    info.connected = true;
                }
            }
        }
        Some(info)
    }

    fn poll_display_event(&mut self) -> Option<crate::drivers::gpu::output::DisplayEvent> {
        // Lazy hotplug check: read events_read from device config every
        // poll. A future revision can wire this to the virtio config-change
        // ISR instead, but polling is correct (the spec only requires
        // events_read to be sticky until events_clear is written).
        if self.device.device_cfg != 0 {
            let events = unsafe { core::ptr::read_volatile(self.device.device_cfg as *const u32) };
            const VIRTIO_GPU_EVENT_DISPLAY: u32 = 1 << 0;
            if events & VIRTIO_GPU_EVENT_DISPLAY != 0 {
                // Ack the bit (write same value to events_clear at +4).
                unsafe {
                    core::ptr::write_volatile(
                        (self.device.device_cfg + 4) as *mut u32,
                        VIRTIO_GPU_EVENT_DISPLAY,
                    );
                }
                self.query_all_display_infos();
                self.pending_events
                    .push_back(crate::drivers::gpu::output::DisplayEvent::HotplugChanged);
            }
        }
        self.pending_events.pop_front()
    }
}

impl VirtioGpu {
    /// Tear down a non-primary scanout: disable the SET_SCANOUT, drop
    /// resource + backing if the scanout owned its framebuffer (mirror
    /// entries don't), free pages.
    fn disable_extra_scanout(&mut self, output_id: u32) {
        if output_id == 0 {
            return;
        }
        let idx = (output_id - 1) as usize;
        let prev = match self.extra_scanouts.get(idx) {
            Some(s) if s.resource_id != 0 => *s,
            _ => return,
        };
        // SET_SCANOUT(id, 0, 0,0) disables the scanout per spec.
        let _ = self.cmd_set_scanout(output_id, 0, 0, 0);
        if prev.mirror_of.is_none() {
            self.cmd_detach_backing(prev.resource_id);
            self.cmd_resource_unref(prev.resource_id);
            for i in 0..prev.fb_pages {
                physical::free_frame(crate::memory::address::PhysAddr::new(
                    prev.fb_phys + (i as u64) * 4096,
                ));
            }
        }
        self.extra_scanouts[idx] = ScanoutState::empty();
    }
}

// ──────────────────────────────────────────────
// Initialization
// ──────────────────────────────────────────────

/// Initialize and register the VirtIO GPU driver.
/// Called from HAL factory during PCI probe.
pub fn init_and_register(pci_dev: &PciDevice) -> bool {
    crate::serial_verbose_println!(
        "  VirtIO GPU: initializing (PCI {:02x}:{:02x}.{})",
        pci_dev.bus,
        pci_dev.device,
        pci_dev.function
    );

    // 1. Find PCI capabilities
    let caps = match virtio::find_capabilities(pci_dev) {
        Some(c) => c,
        None => return false,
    };

    // 2. Create device handle (maps BARs)
    let device = VirtioDevice::new(pci_dev, &caps);

    // 3-6. Initialize device (reset, negotiate features)
    let desired = VIRTIO_F_VERSION_1 | VIRTIO_GPU_F_VIRGL;
    let negotiated = match device.init_device(desired) {
        Ok(n) => {
            crate::serial_verbose_println!("  VirtIO GPU: features negotiated OK");
            n
        }
        Err(e) => {
            crate::serial_verbose_println!("  VirtIO GPU: init failed: {}", e);
            return false;
        }
    };
    let virgl_capable = (negotiated & VIRTIO_GPU_F_VIRGL) != 0;
    if virgl_capable {
        crate::serial_verbose_println!("  VirtIO GPU: VIRGL 3D acceleration available");
    }

    // 7. Set up virtqueues
    let controlq = match device.setup_queue(0) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  VirtIO GPU: failed to set up controlq");
            return false;
        }
    };

    let cursorq = match device.setup_queue(1) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  VirtIO GPU: failed to set up cursorq");
            return false;
        }
    };

    // Allocate DMA buffers for commands and responses
    let cmd_buf = match physical::alloc_frame() {
        Some(p) => {
            unsafe {
                core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4096);
            }
            p.as_u64()
        }
        None => {
            crate::serial_verbose_println!("  VirtIO GPU: failed to allocate cmd buffer");
            return false;
        }
    };

    let resp_buf = match physical::alloc_frame() {
        Some(p) => {
            unsafe {
                core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4096);
            }
            p.as_u64()
        }
        None => {
            crate::serial_verbose_println!("  VirtIO GPU: failed to allocate resp buffer");
            return false;
        }
    };

    // 8. Set DRIVER_OK
    device.set_driver_ok();

    // Clear any pending interrupt from device initialization
    let _ = virtio::mmio_read8(device.isr_addr);

    crate::serial_verbose_println!("  VirtIO GPU: device ready (DRIVER_OK)");

    // Pre-allocate cursor backing store (64x64x4 = 16 KiB = 4 pages).
    // MUST be allocated here during boot (kernel CR3 active, full 128 MiB identity map).
    // Runtime allocation during syscalls may land above 64 MiB — user CR3 only
    // identity-maps PD[0..31] (64 MiB), so writing to higher frames page-faults.
    let cursor_buf_phys = match physical::alloc_contiguous(4) {
        Some(p) => {
            unsafe {
                core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 4 * 4096);
            }
            p.as_u64()
        }
        None => {
            crate::serial_verbose_println!("  VirtIO GPU: failed to allocate cursor buffer");
            0
        }
    };

    // Allocate 64 KiB DMA buffer for 3D command submission (16 pages)
    let cmd_3d_buf = if virgl_capable {
        match physical::alloc_contiguous(16) {
            Some(p) => {
                unsafe {
                    core::ptr::write_bytes(p.as_u64() as *mut u8, 0, 16 * 4096);
                }
                p.as_u64()
            }
            None => {
                crate::serial_verbose_println!("  VirtIO GPU: failed to allocate 3D cmd buffer");
                0
            }
        }
    } else {
        0
    };

    // Read num_scanouts from device-specific config (virtio_gpu_config
    // layout: events_read u32 @ 0, events_clear u32 @ 4, num_scanouts u32 @ 8,
    // num_capsets u32 @ 12). Clamp to MAX_OUTPUTS — the spec already caps
    // it at 16 but defending against a misbehaving host is cheap.
    let num_scanouts_advertised = if device.device_cfg != 0 {
        let n = unsafe { core::ptr::read_volatile((device.device_cfg + 8) as *const u32) };
        n.clamp(1, super::output::MAX_OUTPUTS as u32)
    } else {
        1
    };
    crate::serial_verbose_println!(
        "  VirtIO GPU: device advertises {} scanout(s)",
        num_scanouts_advertised
    );

    let extra_count = num_scanouts_advertised.saturating_sub(1) as usize;
    let mut extra_scanouts = alloc::vec::Vec::with_capacity(extra_count);
    for _ in 0..extra_count {
        extra_scanouts.push(ScanoutState::empty());
    }

    let mut gpu = VirtioGpu {
        device,
        controlq,
        cursorq,
        width: 0,
        height: 0,
        pitch: 0,
        fb_phys: 0,
        fb_pages: 0,
        extra_scanouts,
        num_scanouts_advertised,
        pending_events: alloc::collections::VecDeque::new(),
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
        virgl_capable,
        virgl_ctx_id: 0,
        cmd_3d_buf,
        live_3d_resources: Vec::new(),
        display_infos: Vec::new(),
        enabled_scanout_count: 0,
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
        crate::serial_verbose_println!(
            "  VirtIO GPU: got 640x480 (VGA default), retrying for EDID..."
        );
        for attempt in 1..=5 {
            crate::arch::x86::pit::delay_ms(50);
            if let Some(res) = gpu.cmd_get_display_info() {
                if res != (640, 480) {
                    native = res;
                    crate::serial_verbose_println!(
                        "  VirtIO GPU: EDID ready after {}ms: {}x{}",
                        attempt * 50,
                        res.0,
                        res.1
                    );
                    break;
                }
            }
        }
    }
    // Enforce minimum 1024x768 — never start with a smaller resolution.
    if native.0 < 1024 || native.1 < 768 {
        crate::serial_verbose_println!(
            "  VirtIO GPU: {}x{} below minimum, forcing 1024x768",
            native.0,
            native.1
        );
        native = (1024, 768);
    }
    crate::serial_verbose_println!("  VirtIO GPU: native display {}x{}", native.0, native.1);

    // Cache all display/scanout info for monitor detection.
    gpu.query_all_display_infos();

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
        crate::serial_verbose_println!("  VirtIO GPU: failed to set up display");
        return false;
    }

    // Update canonical framebuffer info to point at VirtIO's guest RAM buffer.
    // This triggers the boot_console change hook which re-renders the splash
    // logo centered for the new resolution — no manual copy needed.
    crate::drivers::framebuffer::update(gpu.fb_phys as u64, gpu.pitch, width, height, 32);

    // Initial transfer + flush
    gpu.cmd_transfer_to_host_2d(gpu.scanout_resource_id, 0, 0, width, height);
    gpu.cmd_resource_flush(gpu.scanout_resource_id, 0, 0, width, height);

    crate::serial_verbose_println!(
        "[OK] VirtIO GPU: {}x{} (fb={:#x})",
        width,
        height,
        gpu.fb_phys
    );

    // Activate any additional advertised scanouts at their EDID-preferred
    // mode while we're still on the kernel CR3 (which has the full 128 MiB
    // identity map). Doing this lazily later from a SYS_DISPLAY_MAP_FB
    // syscall would run under user CR3 — that only identity-maps 64 MiB,
    // so allocating multi-MiB framebuffer pages and zeroing them faults
    // when alloc_contiguous returns physmem above 64 MiB. (Same constraint
    // as the cursor_buf_phys allocation comment higher up.)
    if gpu.num_scanouts_advertised > 1 {
        for output_id in 1..gpu.num_scanouts_advertised {
            // Pick preferred mode: from cached GET_DISPLAY_INFO if set,
            // otherwise EDID detailed timing #1, otherwise skip.
            let (pw, ph) = match gpu.display_infos.get(output_id as usize).copied() {
                Some((w, h, _)) if w >= 640 && h >= 480 => (w, h),
                _ => match gpu.cmd_get_edid(output_id) {
                    Some(edid) => {
                        let h_active = (edid[56] as u32) | (((edid[58] as u32) >> 4) << 8);
                        let v_active = (edid[59] as u32) | (((edid[61] as u32) >> 4) << 8);
                        if h_active >= 640 && v_active >= 480 {
                            (h_active, v_active)
                        } else {
                            (0, 0)
                        }
                    }
                    None => (0, 0),
                },
            };
            if pw == 0 {
                crate::serial_verbose_println!(
                    "  VirtIO GPU: scanout {} has no usable mode at boot, skipping",
                    output_id
                );
                continue;
            }
            match gpu.set_mode_for_output(output_id, pw, ph, 32) {
                Some((aw, ah, _, fb)) => {
                    crate::serial_println!(
                        "[OK] VirtIO GPU: scanout {} active {}x{} fb={:#x}",
                        output_id,
                        aw,
                        ah,
                        fb
                    );
                }
                None => {
                    crate::serial_println!("[!] VirtIO GPU: scanout {} setup failed", output_id);
                }
            }
        }
    }

    // Register as the active GPU driver
    super::register(Box::new(gpu));
    true
}

/// Probe: initialize VirtIO GPU and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("VirtIO GPU")
}
