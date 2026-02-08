use crate::arch::x86::port::{inb, outb};
use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;

/// Mouse event
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub buttons: MouseButtons,
    pub event_type: MouseEventType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    Move,
    ButtonDown,
    ButtonUp,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

static MOUSE_BUFFER: Spinlock<VecDeque<MouseEvent>> = Spinlock::new(VecDeque::new());
static MOUSE_STATE: Spinlock<MouseState> = Spinlock::new(MouseState {
    cycle: 0,
    bytes: [0; 3],
    buttons: MouseButtons { left: false, right: false, middle: false },
});

struct MouseState {
    cycle: u8,
    bytes: [u8; 3],
    buttons: MouseButtons,
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

/// Initialize PS/2 mouse
pub fn init() {
    // Enable auxiliary mouse device
    mouse_wait_input();
    unsafe { outb(0x64, 0xA8); }

    // Enable interrupts
    mouse_wait_input();
    unsafe { outb(0x64, 0x20); }
    mouse_wait_output();
    let status = unsafe { inb(0x60) } | 0x02; // Enable IRQ12
    mouse_wait_input();
    unsafe { outb(0x64, 0x60); }
    mouse_wait_input();
    unsafe { outb(0x60, status); }

    // Set defaults
    mouse_write(0xF6);
    mouse_read(); // ACK

    // Enable data reporting
    mouse_write(0xF4);
    mouse_read(); // ACK

    crate::serial_println!("[OK] PS/2 mouse initialized");
}

/// Called from IRQ12 handler
pub fn handle_byte(byte: u8) {
    let mut state = MOUSE_STATE.lock();

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
            state.cycle = 0;

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

            // Decode buttons
            let new_buttons = MouseButtons {
                left: b[0] & 0x01 != 0,
                right: b[0] & 0x02 != 0,
                middle: b[0] & 0x04 != 0,
            };

            // Determine event type
            let event_type = if new_buttons.left != state.buttons.left
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
                buttons: new_buttons,
                event_type,
            };

            drop(state);

            let mut buf = MOUSE_BUFFER.lock();
            if buf.len() < 64 {
                buf.push_back(event);
            }
            return;
        }
        _ => {
            state.cycle = 0;
        }
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
