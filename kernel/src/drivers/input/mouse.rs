//! PS/2 mouse driver with IntelliMouse scroll wheel support.
//!
//! Processes IRQ12 byte streams into [`MouseEvent`]s with button state tracking.
//! Supports 3-byte standard packets and 4-byte IntelliMouse packets (scroll wheel).

use crate::arch::x86::port::{inb, outb};
use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;

/// Mouse event
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub dz: i32, // scroll wheel: -1 = scroll up, +1 = scroll down
    pub buttons: MouseButtons,
    pub event_type: MouseEventType,
}

/// Type of mouse event that occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    Move,
    ButtonDown,
    ButtonUp,
    Scroll,
    MoveAbsolute, // x/y are screen pixel coordinates (VMMDev absolute mouse)
}

/// State of the three mouse buttons.
#[derive(Debug, Clone, Copy, Default)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

/// Ring buffer of pending mouse events, capacity 64.
pub(crate) static MOUSE_BUFFER: Spinlock<VecDeque<MouseEvent>> = Spinlock::new(VecDeque::new());
static MOUSE_STATE: Spinlock<MouseState> = Spinlock::new(MouseState {
    cycle: 0,
    bytes: [0; 4],
    buttons: MouseButtons { left: false, right: false, middle: false },
    has_scroll: false,
});

struct MouseState {
    cycle: u8,
    bytes: [u8; 4],
    buttons: MouseButtons,
    has_scroll: bool,
}

fn mouse_wait_input() {
    for _ in 0..100_000 {
        if unsafe { inb(0x64) } & 0x02 == 0 {
            return;
        }
    }
}

fn mouse_wait_output() {
    for _ in 0..100_000 {
        if unsafe { inb(0x64) } & 0x01 != 0 {
            return;
        }
    }
}

fn mouse_write(data: u8) {
    mouse_wait_input();
    unsafe { outb(0x64, 0xD4); }
    mouse_wait_input();
    unsafe { outb(0x60, data); }
}

fn mouse_read() -> u8 {
    mouse_wait_output();
    unsafe { inb(0x60) }
}

/// Initialize PS/2 mouse with IntelliMouse scroll wheel support
pub fn init() {
    // Enable auxiliary mouse device
    mouse_wait_input();
    unsafe { outb(0x64, 0xA8); }

    // Read controller config — but do NOT enable IRQ12 yet.
    // IRQ12 must stay disabled during init so ACK bytes from mouse commands
    // don't leak into the IRQ handler's packet state machine.
    mouse_wait_input();
    unsafe { outb(0x64, 0x20); }
    mouse_wait_output();
    let status = unsafe { inb(0x60) };

    // Set defaults
    mouse_write(0xF6);
    mouse_read(); // ACK

    // Enable IntelliMouse scroll wheel: magic sequence
    // Set sample rate to 200, then 100, then 80, then read device ID
    mouse_write(0xF3); mouse_read(); // Set sample rate
    mouse_write(200);  mouse_read();
    mouse_write(0xF3); mouse_read();
    mouse_write(100);  mouse_read();
    mouse_write(0xF3); mouse_read();
    mouse_write(80);   mouse_read();

    // Read device ID — 0x03 means IntelliMouse (4-byte packets with scroll)
    mouse_write(0xF2); mouse_read(); // ACK
    let device_id = mouse_read();
    let has_scroll = device_id == 0x03;

    if has_scroll {
        MOUSE_STATE.lock().has_scroll = true;
    }

    // Enable data reporting
    mouse_write(0xF4);
    mouse_read(); // ACK

    // Flush any stale bytes from the controller buffer
    for _ in 0..16 {
        if unsafe { inb(0x64) } & 0x01 != 0 {
            unsafe { inb(0x60); } // discard
        } else {
            break;
        }
    }

    // NOW enable IRQ12 in the controller config — all init commands are done,
    // no more ACK bytes can leak into the IRQ handler.
    mouse_wait_input();
    unsafe { outb(0x64, 0x60); }
    mouse_wait_input();
    unsafe { outb(0x60, status | 0x02); } // Enable IRQ12

    if has_scroll {
        crate::serial_println!("[OK] PS/2 mouse initialized (IntelliMouse, scroll wheel)");
    } else {
        crate::serial_println!("[OK] PS/2 mouse initialized");
    }
}

/// Called from IRQ12 handler
pub fn handle_byte(byte: u8) {
    let mut state = MOUSE_STATE.lock();
    let packet_size: u8 = if state.has_scroll { 4 } else { 3 };

    match state.cycle {
        0 => {
            // First byte: buttons and sign bits
            // Bit 3 must always be set for valid first byte
            if byte & 0x08 != 0 {
                state.bytes[0] = byte;
                state.cycle = 1;
            }
        }
        1 => {
            state.bytes[1] = byte;
            state.cycle = 2;
        }
        2 => {
            state.bytes[2] = byte;
            if packet_size == 3 {
                state.cycle = 0;
                process_packet(&mut state, 0);
            } else {
                state.cycle = 3;
            }
        }
        3 => {
            state.bytes[3] = byte;
            state.cycle = 0;
            // Byte 3 is scroll wheel delta (signed)
            let dz = byte as i8 as i32;
            process_packet(&mut state, dz);
        }
        _ => {
            state.cycle = 0;
        }
    }
}

fn process_packet(state: &mut MouseState, dz: i32) {
    let b = state.bytes;

    // Decode movement
    let mut dx = b[1] as i32;
    let mut dy = b[2] as i32;

    // Apply sign extension
    if b[0] & 0x10 != 0 {
        dx -= 256;
    }
    if b[0] & 0x20 != 0 {
        dy -= 256;
    }
    // PS/2 mouse Y is inverted
    dy = -dy;

    // Boot splash: update HW cursor directly from IRQ (lag-free)
    crate::drivers::gpu::splash_cursor_move(dx, dy);

    // Decode buttons
    let new_buttons = MouseButtons {
        left: b[0] & 0x01 != 0,
        right: b[0] & 0x02 != 0,
        middle: b[0] & 0x04 != 0,
    };

    // Determine event type
    let event_type = if dz != 0 {
        MouseEventType::Scroll
    } else if new_buttons.left != state.buttons.left
        || new_buttons.right != state.buttons.right
        || new_buttons.middle != state.buttons.middle
    {
        if new_buttons.left && !state.buttons.left
            || new_buttons.right && !state.buttons.right
            || new_buttons.middle && !state.buttons.middle
        {
            MouseEventType::ButtonDown
        } else {
            MouseEventType::ButtonUp
        }
    } else {
        MouseEventType::Move
    };

    state.buttons = new_buttons;

    let event = MouseEvent {
        dx,
        dy,
        dz,
        buttons: new_buttons,
        event_type,
    };

    // Must drop state lock before taking buffer lock
    // (caller holds state lock, so we need to use a different approach)
    // Actually we can just grab the buffer lock here since we don't hold any other lock
    // that MOUSE_BUFFER depends on — but we DO hold MOUSE_STATE. Since these are
    // independent locks and no code path holds both in reverse order, this is safe.
    let mut buf = MOUSE_BUFFER.lock();
    if buf.len() < 256 {
        buf.push_back(event);
    }
}

/// Read a mouse event from the buffer (non-blocking)
pub fn read_event() -> Option<MouseEvent> {
    MOUSE_BUFFER.lock().pop_front()
}

/// Check if there are pending mouse events
pub fn has_event() -> bool {
    !MOUSE_BUFFER.lock().is_empty()
}

/// Drain all pending mouse events (used during splash→compositor transition).
pub fn clear_buffer() {
    MOUSE_BUFFER.lock().clear();
}

/// Get the current state of the mouse buttons.
pub fn get_current_buttons() -> MouseButtons {
    MOUSE_STATE.lock().buttons
}

/// Inject an absolute mouse position event (from VMMDev or USB tablet).
///
/// When buttons change, emits a MoveAbsolute event FIRST (for the position),
/// then the ButtonDown/ButtonUp event with dx=0,dy=0. This is necessary because
/// the compositor treats dx/dy in button events as relative deltas — absolute
/// pixel coordinates would send the cursor flying off-screen.
pub fn inject_absolute(x: i32, y: i32, buttons: MouseButtons) {
    static LOGGED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        crate::serial_println!("[mouse] inject_absolute first event: x={} y={}", x, y);
    }

    let mut state = MOUSE_STATE.lock();
    let old_buttons = state.buttons;

    let buttons_changed = buttons.left != old_buttons.left
        || buttons.right != old_buttons.right
        || buttons.middle != old_buttons.middle;

    state.buttons = buttons;

    let mut buf = MOUSE_BUFFER.lock();

    // Always emit the position update as MoveAbsolute
    if buf.len() < 256 {
        buf.push_back(MouseEvent {
            dx: x,
            dy: y,
            dz: 0,
            buttons,
            event_type: MouseEventType::MoveAbsolute,
        });
    }

    // If buttons changed, emit a separate ButtonDown/ButtonUp event (no position)
    if buttons_changed && buf.len() < 256 {
        let btn_event_type = if (buttons.left && !old_buttons.left)
            || (buttons.right && !old_buttons.right)
            || (buttons.middle && !old_buttons.middle)
        {
            MouseEventType::ButtonDown
        } else {
            MouseEventType::ButtonUp
        };
        buf.push_back(MouseEvent {
            dx: 0,
            dy: 0,
            dz: 0,
            buttons,
            event_type: btn_event_type,
        });
    }
}

/// Inject an absolute position-only event (no button state change).
///
/// Used by polling paths (VMMDev, SVGA cursor bypass) in sys_input_poll
/// where reading `get_current_buttons()` would race with IRQ-driven
/// `inject_absolute()` calls, causing spurious ButtonDown/ButtonUp events.
pub fn inject_position(x: i32, y: i32) {
    let mut buf = MOUSE_BUFFER.lock();
    if buf.len() < 256 {
        // Read current buttons for the event struct but do NOT compare/change them
        let buttons = MOUSE_STATE.lock().buttons;
        buf.push_back(MouseEvent {
            dx: x,
            dy: y,
            dz: 0,
            buttons,
            event_type: MouseEventType::MoveAbsolute,
        });
    }
}

/// PS/2 mouse IRQ handler (IRQ 12). Reads byte from port 0x60.
/// When the VMware backdoor (vmmouse) is active, delegates to vmmouse instead.
pub fn irq_handler(_irq: u8) {
    // If vmmouse is active, the backdoor intercepts PS/2 data — read from backdoor instead
    if super::vmmouse::is_active() {
        super::vmmouse::handle_irq();
        return;
    }

    let status = unsafe { crate::arch::x86::port::inb(0x64) };
    // Bit 0: Output Buffer Full — data is available
    // Bit 5: AUX data — byte is from mouse, not keyboard
    if status & 0x01 == 0 {
        return; // no data available (spurious IRQ)
    }
    let byte = unsafe { crate::arch::x86::port::inb(0x60) };
    if status & 0x20 == 0 {
        return; // not from auxiliary device (keyboard data, ignore)
    }
    handle_byte(byte);
}
