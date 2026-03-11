//! IDT emulation and interrupt delivery.
//!
//! Manages a pending interrupt bitmask (vectors 0-255), an interrupt shadow
//! state (inhibits IRQ delivery for one instruction after `MOV SS`), and
//! double-fault detection. Provides helpers to read IDT entries from guest
//! memory in real mode (IVT), 32-bit protected mode, and 64-bit long mode.

use crate::error::{Result, VmError};
use crate::flags;
use crate::memory::MemoryBus;

/// IDT gate descriptor type, matching the x86 gate type field encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateType {
    /// Task gate (type 0x5) — triggers a task switch.
    Task,
    /// 16-bit interrupt gate (type 0x6) — clears IF on entry.
    Interrupt16,
    /// 16-bit trap gate (type 0x7) — IF unchanged.
    Trap16,
    /// 32-bit interrupt gate (type 0xE) — clears IF on entry.
    Interrupt32,
    /// 32-bit trap gate (type 0xF) — IF unchanged.
    Trap32,
    /// 64-bit interrupt gate (type 0xE in long mode) — clears IF on entry.
    Interrupt64,
    /// 64-bit trap gate (type 0xF in long mode) — IF unchanged.
    Trap64,
}

/// A decoded IDT gate descriptor.
#[derive(Debug, Clone, Copy)]
pub struct IdtEntry {
    /// Target code offset (combined from low/mid/high fields).
    pub offset: u64,
    /// Target code segment selector.
    pub selector: u16,
    /// Gate type (interrupt, trap, or task).
    pub gate_type: GateType,
    /// Descriptor privilege level (ring required to invoke via INT n).
    pub dpl: u8,
    /// Whether the descriptor is present.
    pub present: bool,
    /// Interrupt Stack Table index (0 = legacy stack switch, 1-7 = IST entry).
    /// Only meaningful in 64-bit mode.
    pub ist: u8,
}

/// Software interrupt controller managing pending IRQs and delivery state.
///
/// Uses a 256-bit bitmask (four `u64` words) to track which interrupt
/// vectors are pending. Vector 0 is the lowest-priority; delivery returns
/// the lowest-numbered pending vector first (matching typical PIC behavior
/// where lower vector = higher priority).
pub struct InterruptController {
    /// Bitmask of pending interrupt vectors (bit N of word N/64 = vector N).
    pending: [u64; 4],
    /// Interrupt shadow: when `true`, maskable IRQ delivery is suppressed
    /// for one instruction. Set after `MOV SS` or `POP SS` to keep the
    /// SS:RSP pair atomic.
    pub interrupt_shadow: bool,
    /// Set while dispatching an exception; used to detect double faults.
    /// If a second exception occurs while this is `true`, the controller
    /// raises `#DF` (vector 8) instead.
    pub handling_exception: bool,
    /// Set while delivering a double fault; used to detect triple faults.
    pub handling_double_fault: bool,
}

impl InterruptController {
    /// Create a new interrupt controller with no pending interrupts.
    pub fn new() -> Self {
        InterruptController {
            pending: [0u64; 4],
            interrupt_shadow: false,
            handling_exception: false,
            handling_double_fault: false,
        }
    }

    /// Raise an interrupt request for the given vector (0-255).
    ///
    /// The vector will remain pending until acknowledged or cleared.
    #[inline]
    pub fn raise_irq(&mut self, vector: u8) {
        let word = (vector >> 6) as usize; // vector / 64
        let bit = (vector & 63) as u64;
        self.pending[word] |= 1u64 << bit;
    }

    /// Clear a pending interrupt for the given vector.
    #[inline]
    pub fn clear_irq(&mut self, vector: u8) {
        let word = (vector >> 6) as usize;
        let bit = (vector & 63) as u64;
        self.pending[word] &= !(1u64 << bit);
    }

    /// Check for a deliverable interrupt.
    ///
    /// Returns `Some(vector)` if there is a pending interrupt that can be
    /// delivered right now. Delivery requires:
    /// - `IF` (interrupt enable) flag is set in `rflags`
    /// - The interrupt shadow is not active
    ///
    /// Returns the lowest-numbered (highest-priority) pending vector.
    pub fn pending_interrupt(&self, rflags: u64) -> Option<u8> {
        if self.interrupt_shadow {
            return None;
        }
        let if_set = (rflags & flags::IF) != 0;
        // Scan words low-to-high for the lowest pending vector.
        for (word_idx, &word) in self.pending.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u8;
                let vec = (word_idx as u8) * 64 + bit;
                if if_set {
                    return Some(vec);
                }
                return None;
            }
        }
        None
    }

    /// Acknowledge delivery of an interrupt, clearing its pending bit.
    #[inline]
    pub fn acknowledge(&mut self, vector: u8) {
        self.clear_irq(vector);
    }

    /// Return one raw 64-bit pending-vector word for diagnostics.
    ///
    /// `idx` selects the word: 0 => vectors 0..63, 1 => 64..127,
    /// 2 => 128..191, 3 => 192..255.
    pub fn pending_word(&self, idx: usize) -> u64 {
        self.pending.get(idx).copied().unwrap_or(0)
    }

    /// Return the lowest-numbered pending vector, ignoring IF/shadow state.
    pub fn lowest_pending_vector(&self) -> Option<u8> {
        for (word_idx, &word) in self.pending.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u8;
                return Some((word_idx as u8) * 64 + bit);
            }
        }
        None
    }

    /// Read an interrupt vector entry from the real-mode IVT.
    ///
    /// In real mode the IVT occupies linear addresses 0x0000-0x03FF. Each
    /// entry is 4 bytes: offset (u16) followed by segment (u16).
    ///
    /// Returns `(segment, offset)` for the requested vector.
    pub fn read_idt_entry_real(
        &self,
        vector: u8,
        mem: &dyn MemoryBus,
    ) -> Result<(u16, u16)> {
        let addr = (vector as u64) * 4;
        let offset = mem.read_u16(addr)? as u16;
        let segment = mem.read_u16(addr + 2)? as u16;
        Ok((segment, offset))
    }

}
