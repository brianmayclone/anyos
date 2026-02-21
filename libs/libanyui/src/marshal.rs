//! Marshaller — cross-thread UI access via a dispatch queue.
//!
//! Worker threads cannot directly modify UI controls (the global AnyuiState
//! is not thread-safe). Instead, they push `UiCommand`s into a lock-free
//! ring buffer, which the main event loop drains at the start of each frame.
//!
//! # Usage
//! ```ignore
//! // From worker thread:
//! anyui_marshal_set_text(label_id, text_ptr, text_len);
//! anyui_marshal_set_visible(label_id, 1);
//! anyui_marshal_dispatch(my_callback, my_data);
//! ```

use crate::control::ControlId;

/// Maximum number of pending commands in the marshal queue.
const QUEUE_SIZE: usize = 256;

/// A buffered UI command from a worker thread.
#[derive(Clone, Copy)]
pub struct UiCommand {
    pub target_id: ControlId,
    pub kind: UiCommandKind,
}

/// The kind of UI modification to apply.
#[derive(Clone, Copy)]
pub enum UiCommandKind {
    /// Set the text of a control. Text is stored inline (max 128 bytes).
    SetText { buf: [u8; 128], len: u32 },
    /// Set the color of a control.
    SetColor { color: u32 },
    /// Set the state value of a control.
    SetState { value: u32 },
    /// Set visibility.
    SetVisible { visible: bool },
    /// Set position.
    SetPosition { x: i32, y: i32 },
    /// Set size.
    SetSize { w: u32, h: u32 },
    /// Execute an arbitrary callback on the UI thread.
    Dispatch { callback: extern "C" fn(u64), userdata: u64 },
}

/// Spinlock-based ring buffer for marshal commands.
///
/// Uses a simple spinlock (not interrupts-aware — this is user-space).
/// Both push and pop are O(1).
struct MarshalQueue {
    buf: [Option<UiCommand>; QUEUE_SIZE],
    head: usize, // next write position
    tail: usize, // next read position
    lock: core::sync::atomic::AtomicBool,
}

impl MarshalQueue {
    const fn new() -> Self {
        Self {
            buf: [None; QUEUE_SIZE],
            head: 0,
            tail: 0,
            lock: core::sync::atomic::AtomicBool::new(false),
        }
    }

    fn acquire(&self) {
        while self.lock.swap(true, core::sync::atomic::Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.lock.store(false, core::sync::atomic::Ordering::Release);
    }

    fn push(&mut self, cmd: UiCommand) -> bool {
        // Safety: we use interior mutability via raw pointer since
        // the spinlock protects concurrent access.
        let next = (self.head + 1) % QUEUE_SIZE;
        if next == self.tail {
            return false; // Queue full
        }
        self.buf[self.head] = Some(cmd);
        self.head = next;
        true
    }

    fn pop(&mut self) -> Option<UiCommand> {
        if self.tail == self.head {
            return None; // Queue empty
        }
        let cmd = self.buf[self.tail].take();
        self.tail = (self.tail + 1) % QUEUE_SIZE;
        cmd
    }
}

static mut QUEUE: MarshalQueue = MarshalQueue::new();

/// Push a command to the marshal queue (thread-safe).
fn marshal_push(cmd: UiCommand) {
    unsafe {
        QUEUE.acquire();
        let _ = QUEUE.push(cmd);
        QUEUE.release();
    }
}

/// Drain all pending marshal commands and apply them to UI state.
/// Called at the start of each `run_once()` frame on the main thread.
pub fn drain(st: &mut crate::AnyuiState) {
    loop {
        let cmd = unsafe {
            QUEUE.acquire();
            let c = QUEUE.pop();
            QUEUE.release();
            c
        };
        let cmd = match cmd {
            Some(c) => c,
            None => break,
        };

        match cmd.kind {
            UiCommandKind::SetText { buf, len } => {
                let text = &buf[..len as usize];
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_text(text);
                }
            }
            UiCommandKind::SetColor { color } => {
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_color(color);
                }
            }
            UiCommandKind::SetState { value } => {
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_state(value);
                }
            }
            UiCommandKind::SetVisible { visible } => {
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_visible(visible);
                }
            }
            UiCommandKind::SetPosition { x, y } => {
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_position(x, y);
                }
            }
            UiCommandKind::SetSize { w, h } => {
                if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == cmd.target_id) {
                    ctrl.set_size(w, h);
                }
            }
            UiCommandKind::Dispatch { callback, userdata } => {
                (callback)(userdata);
            }
        }
    }
}

// ── Exported C API (callable from worker threads) ───────────────────

#[no_mangle]
pub extern "C" fn anyui_marshal_set_text(id: ControlId, text: *const u8, len: u32) {
    let mut buf = [0u8; 128];
    let copy_len = (len as usize).min(128);
    if !text.is_null() && copy_len > 0 {
        unsafe { core::ptr::copy_nonoverlapping(text, buf.as_mut_ptr(), copy_len); }
    }
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetText { buf, len: copy_len as u32 },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_set_color(id: ControlId, color: u32) {
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetColor { color },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_set_state(id: ControlId, value: u32) {
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetState { value },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_set_visible(id: ControlId, visible: u32) {
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetVisible { visible: visible != 0 },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_set_position(id: ControlId, x: i32, y: i32) {
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetPosition { x, y },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_set_size(id: ControlId, w: u32, h: u32) {
    marshal_push(UiCommand {
        target_id: id,
        kind: UiCommandKind::SetSize { w, h },
    });
}

#[no_mangle]
pub extern "C" fn anyui_marshal_dispatch(callback: extern "C" fn(u64), userdata: u64) {
    marshal_push(UiCommand {
        target_id: 0,
        kind: UiCommandKind::Dispatch { callback, userdata },
    });
}
