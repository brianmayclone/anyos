//! VirtIO Input driver over MMIO transport for ARM64.
//!
//! Handles VirtIO keyboard and mouse devices (DeviceID = 18).
//! Uses Linux evdev-compatible event format (EV_KEY, EV_REL, EV_ABS).
//! Stores events in ring buffers accessible by the compositor/shell.

use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicBool, Ordering, fence};

use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;

use super::VirtioMmioDevice;
use super::virtqueue::{VirtQueue, VRING_DESC_F_WRITE};

// ---------------------------------------------------------------------------
// VirtIO Input Config Select values (Spec 5.8.2)
// ---------------------------------------------------------------------------

const VIRTIO_INPUT_CFG_ID_NAME: u8 = 0x01;
const VIRTIO_INPUT_CFG_EV_BITS: u8 = 0x11;

// ---------------------------------------------------------------------------
// Linux evdev event types
// ---------------------------------------------------------------------------

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;

// Relative axes (mouse)
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;

// Absolute axes (tablet/touchscreen)
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

// ---------------------------------------------------------------------------
// VirtIO Input Event (8 bytes, matches spec 5.8.6)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VirtioInputEvent {
    pub type_: u16,
    pub code: u16,
    pub value: u32,
}

// ---------------------------------------------------------------------------
// Event Buffers (ring buffers for keyboard and mouse events)
// ---------------------------------------------------------------------------

const KEY_BUF_SIZE: usize = 256;
const MOUSE_BUF_SIZE: usize = 256;

/// Keyboard event: scancode + pressed flag.
#[derive(Clone, Copy, Default)]
pub struct KeyEvent {
    pub code: u16,
    pub pressed: bool,
}

/// Mouse event: relative movement or button.
#[derive(Clone, Copy, Default)]
pub struct MouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub buttons: u8,
    pub is_move: bool,
}

static KEY_BUF: Spinlock<KeyRingBuf> = Spinlock::new(KeyRingBuf::new());
static MOUSE_BUF: Spinlock<MouseRingBuf> = Spinlock::new(MouseRingBuf::new());

struct KeyRingBuf {
    buf: [KeyEvent; KEY_BUF_SIZE],
    head: usize,
    tail: usize,
}

impl KeyRingBuf {
    const fn new() -> Self {
        KeyRingBuf { buf: [KeyEvent { code: 0, pressed: false }; KEY_BUF_SIZE], head: 0, tail: 0 }
    }
    fn push(&mut self, ev: KeyEvent) {
        let next = (self.head + 1) % KEY_BUF_SIZE;
        if next != self.tail {
            self.buf[self.head] = ev;
            self.head = next;
        }
    }
    fn pop(&mut self) -> Option<KeyEvent> {
        if self.head == self.tail { return None; }
        let ev = self.buf[self.tail];
        self.tail = (self.tail + 1) % KEY_BUF_SIZE;
        Some(ev)
    }
    fn is_empty(&self) -> bool { self.head == self.tail }
}

struct MouseRingBuf {
    buf: [MouseEvent; MOUSE_BUF_SIZE],
    head: usize,
    tail: usize,
}

impl MouseRingBuf {
    const fn new() -> Self {
        MouseRingBuf {
            buf: [MouseEvent { dx: 0, dy: 0, buttons: 0, is_move: false }; MOUSE_BUF_SIZE],
            head: 0, tail: 0,
        }
    }
    fn push(&mut self, ev: MouseEvent) {
        let next = (self.head + 1) % MOUSE_BUF_SIZE;
        if next != self.tail {
            self.buf[self.head] = ev;
            self.head = next;
        }
    }
    fn pop(&mut self) -> Option<MouseEvent> {
        if self.head == self.tail { return None; }
        let ev = self.buf[self.tail];
        self.tail = (self.tail + 1) % MOUSE_BUF_SIZE;
        Some(ev)
    }
}

// ---------------------------------------------------------------------------
// Input device state
// ---------------------------------------------------------------------------

/// Accumulated mouse state for batching relative events.
static MOUSE_DX: AtomicU32 = AtomicU32::new(0);
static MOUSE_DY: AtomicU32 = AtomicU32::new(0);
static MOUSE_BUTTONS: AtomicU32 = AtomicU32::new(0);

/// Number of initialized input devices.
static INPUT_COUNT: AtomicU32 = AtomicU32::new(0);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

struct VirtioInput {
    base: usize,
    eventq: VirtQueue,
    /// Physical address of the event buffer page.
    event_buf_phys: u64,
    event_buf_virt: usize,
    /// Number of pre-posted event buffers.
    num_events: u16,
    /// Device name for logging.
    is_keyboard: bool,
}

// We support up to 4 input devices (keyboard, mouse, touchpad, etc.)
static INPUT_DEVICES: Spinlock<[Option<VirtioInput>; 4]> = Spinlock::new([None, None, None, None]);

#[inline]
fn phys_to_virt(phys: u64) -> usize {
    (phys + 0xFFFF_0000_4000_0000) as usize
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize a VirtIO input device.
pub fn init(dev: &VirtioMmioDevice) {
    // Feature negotiation
    if dev.init_device(0).is_none() {
        crate::serial_println!("  virtio-input: feature negotiation failed");
        return;
    }

    // Read device name from config.
    // VirtIO Input config has byte-sized fields: select(0), subsel(1), size(2).
    // Use byte-level MMIO access to avoid unaligned 32-bit writes on ARM64.
    dev.write_config_u8(0, VIRTIO_INPUT_CFG_ID_NAME); // select
    dev.write_config_u8(1, 0); // subsel
    // Read size (byte 2)
    let size = dev.read_config_u8(2) as u32;
    let is_keyboard = if size > 0 {
        // Read first byte of name (data starts at byte offset 8 in config)
        let first_char = dev.read_config_u8(8);
        // Keyboards typically start with 'Q' (QEMU Virtio Keyboard) or 'k'
        first_char == b'Q' || first_char == b'k' || first_char == b'K'
    } else {
        false
    };

    let kind = if is_keyboard { "keyboard" } else { "mouse/pointer" };

    // Set up eventq (queue 0)
    let eventq = match VirtQueue::new(0, 64) {
        Some(q) => q,
        None => {
            crate::serial_println!("  virtio-input({}): failed to allocate eventq", kind);
            return;
        }
    };

    let (desc_phys, avail_phys, used_phys) = eventq.phys_addrs();
    if !dev.setup_queue_raw(0, 64, desc_phys, avail_phys, used_phys) {
        crate::serial_println!("  virtio-input({}): failed to setup eventq", kind);
        return;
    }

    // Allocate event buffer (one page = many 8-byte events)
    let event_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => return,
    };
    let event_buf_phys = event_frame.0;
    let event_buf_virt = phys_to_virt(event_buf_phys);
    unsafe { ptr::write_bytes(event_buf_virt as *mut u8, 0, FRAME_SIZE); }

    let mut input = VirtioInput {
        base: dev.base(),
        eventq,
        event_buf_phys,
        event_buf_virt,
        num_events: 0,
        is_keyboard,
    };

    // Pre-post event buffers to the eventq
    let event_size = core::mem::size_of::<VirtioInputEvent>() as u32;
    let max_events = (FRAME_SIZE / event_size as usize).min(64) as u16;
    for i in 0..max_events {
        let offset = i as u64 * event_size as u64;
        let phys = event_buf_phys + offset;
        if input.eventq.push_buf(phys, event_size, VRING_DESC_F_WRITE).is_none() {
            break;
        }
        input.num_events += 1;
    }

    dev.driver_ok();

    // Register IRQ handler
    let irq = dev.irq();
    let idx = INPUT_COUNT.load(Ordering::Relaxed) as usize;
    crate::arch::arm64::gic::enable_irq(irq);
    crate::arch::arm64::exceptions::register_irq(irq, input_irq_handler);

    // Store device
    let mut devices = INPUT_DEVICES.lock();
    if idx < 4 {
        devices[idx] = Some(input);
        INPUT_COUNT.store((idx + 1) as u32, Ordering::Relaxed);
    }

    INITIALIZED.store(true, Ordering::Relaxed);
    crate::serial_println!("  virtio-input({}): initialized, {} event buffers, IRQ {}",
        kind, max_events, irq);
}

// ---------------------------------------------------------------------------
// IRQ Handler
// ---------------------------------------------------------------------------

/// IRQ handler for all VirtIO input devices.
fn input_irq_handler() {
    let mut devices = INPUT_DEVICES.lock();
    let count = INPUT_COUNT.load(Ordering::Relaxed) as usize;

    for i in 0..count {
        if let Some(ref mut dev) = devices[i] {
            // Acknowledge interrupt
            let base = dev.base;
            let status = unsafe { ptr::read_volatile((base + 0x060) as *const u32) };
            if status == 0 { continue; }
            unsafe { ptr::write_volatile((base + 0x064) as *mut u32, status); }

            // Process completed events
            while let Some((desc_idx, _len)) = dev.eventq.pop_used() {
                let event_offset = desc_idx as usize * core::mem::size_of::<VirtioInputEvent>();
                if event_offset + 8 <= FRAME_SIZE {
                    let event = unsafe {
                        ptr::read_volatile(
                            (dev.event_buf_virt + event_offset) as *const VirtioInputEvent
                        )
                    };
                    process_event(&event, dev.is_keyboard);
                }

                // Re-post the buffer
                let phys = dev.event_buf_phys + desc_idx as u64 * 8;
                dev.eventq.push_buf(phys, 8, VRING_DESC_F_WRITE);
            }

            // Notify device that we re-posted buffers
            unsafe { ptr::write_volatile((base + 0x050) as *mut u32, 0); }
        }
    }
}

/// Process a single evdev event.
fn process_event(event: &VirtioInputEvent, is_keyboard: bool) {
    match event.type_ {
        EV_KEY => {
            let pressed = event.value != 0;
            if is_keyboard || event.code < 256 {
                // Keyboard key event
                KEY_BUF.lock().push(KeyEvent { code: event.code, pressed });
            } else {
                // Mouse button (BTN_LEFT=0x110, BTN_RIGHT=0x111, BTN_MIDDLE=0x112)
                let btn = match event.code {
                    0x110 => 1u8,  // Left
                    0x111 => 2u8,  // Right
                    0x112 => 4u8,  // Middle
                    _ => 0,
                };
                if btn != 0 {
                    if pressed {
                        MOUSE_BUTTONS.fetch_or(btn as u32, Ordering::Relaxed);
                    } else {
                        MOUSE_BUTTONS.fetch_and(!(btn as u32), Ordering::Relaxed);
                    }
                    let buttons = MOUSE_BUTTONS.load(Ordering::Relaxed) as u8;
                    MOUSE_BUF.lock().push(MouseEvent { dx: 0, dy: 0, buttons, is_move: false });
                }
            }
        }
        EV_REL => {
            match event.code {
                REL_X => {
                    let dx = event.value as i32;
                    MOUSE_BUF.lock().push(MouseEvent {
                        dx,
                        dy: 0,
                        buttons: MOUSE_BUTTONS.load(Ordering::Relaxed) as u8,
                        is_move: true,
                    });
                }
                REL_Y => {
                    let dy = event.value as i32;
                    MOUSE_BUF.lock().push(MouseEvent {
                        dx: 0,
                        dy,
                        buttons: MOUSE_BUTTONS.load(Ordering::Relaxed) as u8,
                        is_move: true,
                    });
                }
                _ => {}
            }
        }
        EV_ABS => {
            // Absolute positioning — treat as relative for now
            // TODO: proper absolute coordinate handling
        }
        EV_SYN => {
            // Sync event — marks end of a batch of events
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Pop the next keyboard event, if any.
pub fn pop_key_event() -> Option<KeyEvent> {
    KEY_BUF.lock().pop()
}

/// Pop the next mouse event, if any.
pub fn pop_mouse_event() -> Option<MouseEvent> {
    MOUSE_BUF.lock().pop()
}

/// Check if VirtIO input is initialized.
pub fn is_available() -> bool {
    INITIALIZED.load(Ordering::Relaxed)
}

/// Get the number of initialized input devices.
pub fn device_count() -> u32 {
    INPUT_COUNT.load(Ordering::Relaxed)
}
