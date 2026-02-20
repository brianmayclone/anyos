//! VMware Backdoor (vmmouse) driver for absolute mouse in VMware/QEMU/VirtualBox.
//!
//! When VMware SVGA or VBoxVGA is active, the hypervisor intercepts PS/2 mouse data
//! and redirects it through the VMware backdoor port (0x5658). PS/2 IRQ12 still fires
//! but port 0x60 returns garbage — data must be read from the backdoor instead.
//!
//! Protocol:
//! - Backdoor port: 0x5658, magic: 0x564D5868
//! - Commands via EAX=magic, ECX=command, EDX=port, `in eax, dx`
//! - ABSPOINTER sub-commands: ENABLE, ABSOLUTE, RELATIVE, STATUS, DATA

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// VMware backdoor I/O port.
const VMWARE_PORT: u16 = 0x5658;
/// VMware backdoor magic value.
const VMWARE_MAGIC: u32 = 0x564D_5868;

// Backdoor commands
const CMD_GETVERSION: u32 = 10;
const CMD_ABSPOINTER_DATA: u32 = 39;
const CMD_ABSPOINTER_STATUS: u32 = 40;
const CMD_ABSPOINTER_COMMAND: u32 = 41;

// ABSPOINTER sub-commands (passed as arg to CMD_ABSPOINTER_COMMAND)
const ABSPOINTER_ENABLE: u32 = 0x4545_4152; // "EARE" — enable vmmouse
const ABSPOINTER_RELATIVE: u32 = 0xF5;       // disable / back to relative PS/2
const ABSPOINTER_ABSOLUTE: u32 = 0x5342_4152; // "SBAR" — switch to absolute mode

/// Whether vmmouse backdoor is active and should intercept IRQ12.
static VMMOUSE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Cached screen dimensions for coordinate scaling (updated from non-IRQ context).
static SCREEN_W: AtomicU32 = AtomicU32::new(1024);
static SCREEN_H: AtomicU32 = AtomicU32::new(768);

/// Result of a VMware backdoor call.
#[derive(Debug)]
struct BackdoorResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

/// Execute a VMware backdoor call.
///
/// Sets EAX=magic, EBX=arg, ECX=command, EDX=port, then executes `in eax, dx`.
/// The hypervisor intercepts this I/O and fills all registers with the response.
///
/// Note: RBX is reserved by LLVM so we save/restore it manually.
#[inline(always)]
fn backdoor(cmd: u32, arg: u32) -> BackdoorResult {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;

    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov ebx, {arg:e}",
            "in eax, dx",
            "mov {out_ebx:e}, ebx",
            "pop rbx",
            arg = in(reg) arg,
            out_ebx = out(reg) ebx,
            inout("eax") VMWARE_MAGIC => eax,
            inout("ecx") cmd => ecx,
            inout("edx") VMWARE_PORT as u32 => edx,
        );
    }

    BackdoorResult { eax, ebx, ecx, edx }
}

/// Detect if the VMware backdoor is present.
fn detect() -> bool {
    let r = backdoor(CMD_GETVERSION, 0);
    // If backdoor is present, EBX == VMWARE_MAGIC
    r.ebx == VMWARE_MAGIC
}

/// Enable the vmmouse in absolute mode.
fn enable() -> bool {
    // Step 1: Send ENABLE command
    backdoor(CMD_ABSPOINTER_COMMAND, ABSPOINTER_ENABLE);

    // Step 2: Check status — should return version info, not error
    let status = backdoor(CMD_ABSPOINTER_STATUS, 0);
    if status.eax == 0xFFFF_0000 {
        // Error — vmmouse not available
        return false;
    }

    // Step 3: Read any pending initial data (1 word)
    backdoor(CMD_ABSPOINTER_DATA, 1);

    // Step 4: Switch to absolute mode
    backdoor(CMD_ABSPOINTER_COMMAND, ABSPOINTER_ABSOLUTE);

    true
}

/// Disable vmmouse, revert to standard PS/2 relative mode.
pub fn disable() {
    if VMMOUSE_ACTIVE.load(Ordering::Relaxed) {
        backdoor(CMD_ABSPOINTER_COMMAND, ABSPOINTER_RELATIVE);
        VMMOUSE_ACTIVE.store(false, Ordering::Release);
        crate::serial_println!("[vmmouse] disabled, reverting to PS/2");
    }
}

/// Initialize vmmouse driver. Call after PS/2 mouse init.
/// Returns true if the backdoor was detected and vmmouse is now active.
pub fn init() -> bool {
    if !detect() {
        crate::serial_println!("[vmmouse] VMware backdoor not detected, using PS/2");
        return false;
    }

    crate::serial_println!("[vmmouse] VMware backdoor detected");

    if !enable() {
        crate::serial_println!("[WARN] vmmouse: enable failed, using PS/2");
        return false;
    }

    // Cache current screen resolution for coordinate scaling
    if let Some((w, h)) = crate::drivers::gpu::with_gpu(|g| {
        let (w, h, _, _) = g.get_mode();
        (w, h)
    }) {
        update_screen_size(w, h);
    }

    VMMOUSE_ACTIVE.store(true, Ordering::Release);
    crate::serial_println!("[OK] vmmouse: absolute mouse enabled via VMware backdoor ({}x{})",
        SCREEN_W.load(Ordering::Relaxed), SCREEN_H.load(Ordering::Relaxed));
    true
}

/// Check if vmmouse is active (IRQ handler should use backdoor instead of PS/2 data).
#[inline]
pub fn is_active() -> bool {
    VMMOUSE_ACTIVE.load(Ordering::Acquire)
}

/// Called from IRQ12 handler when vmmouse is active.
/// Reads the PS/2 byte to acknowledge the interrupt, then reads mouse data from backdoor.
pub fn handle_irq() {
    // Read and discard PS/2 byte to clear the IRQ (port 0x60 has garbage when vmmouse active)
    unsafe { crate::arch::x86::port::inb(0x60); }

    // Check how many 4-byte packets are pending
    let status = backdoor(CMD_ABSPOINTER_STATUS, 0);
    let words_available = status.eax & 0xFFFF;

    if words_available < 4 {
        return; // No complete packet
    }

    // Read the mouse data packet (4 words)
    let data = backdoor(CMD_ABSPOINTER_DATA, 4);

    // data.eax bits [15:0] = button flags:
    //   bit 5 (0x20) = left button
    //   bit 4 (0x10) = right button
    //   bit 3 (0x08) = middle button
    let btn_flags = data.eax & 0xFFFF;
    let buttons = super::mouse::MouseButtons {
        left: btn_flags & 0x20 != 0,
        right: btn_flags & 0x10 != 0,
        middle: btn_flags & 0x08 != 0,
    };

    // data.ebx = X coordinate (0..0xFFFF), data.ecx = Y coordinate (0..0xFFFF)
    let raw_x = data.ebx;
    let raw_y = data.ecx;

    // data.edx = scroll wheel delta (signed byte in low bits)
    let dz = data.edx as i8 as i32;

    // Scale from 0..0xFFFF to screen pixel coordinates
    let (screen_w, screen_h) = get_screen_size();

    let x = ((raw_x as u64 * screen_w as u64) / 0xFFFF) as i32;
    let y = ((raw_y as u64 * screen_h as u64) / 0xFFFF) as i32;

    // Inject as absolute event into the mouse buffer
    if dz != 0 {
        // Scroll event — inject with current position
        let scroll_event = super::mouse::MouseEvent {
            dx: x,
            dy: y,
            dz,
            buttons,
            event_type: super::mouse::MouseEventType::Scroll,
        };
        let mut buf = super::mouse::MOUSE_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(scroll_event);
        }
    }

    // Always inject position/button event
    super::mouse::inject_absolute(x, y, buttons);
}

/// Get cached screen dimensions for coordinate scaling.
/// Safe to call from IRQ context — uses atomics, no locks.
#[inline]
fn get_screen_size() -> (u32, u32) {
    (SCREEN_W.load(Ordering::Relaxed), SCREEN_H.load(Ordering::Relaxed))
}

/// Update cached screen dimensions. Call from non-IRQ context when resolution changes.
pub fn update_screen_size(w: u32, h: u32) {
    if w > 0 && h > 0 {
        SCREEN_W.store(w, Ordering::Relaxed);
        SCREEN_H.store(h, Ordering::Relaxed);
    }
}
