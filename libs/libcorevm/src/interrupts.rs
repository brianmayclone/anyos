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
}

impl InterruptController {
    /// Create a new interrupt controller with no pending interrupts.
    pub fn new() -> Self {
        InterruptController {
            pending: [0u64; 4],
            interrupt_shadow: false,
            handling_exception: false,
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
        // Maskable interrupts require IF=1 and no shadow.
        if (rflags & flags::IF) == 0 || self.interrupt_shadow {
            return None;
        }
        // Scan words low-to-high for the lowest pending vector.
        for (word_idx, &word) in self.pending.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u8;
                return Some((word_idx as u8) * 64 + bit);
            }
        }
        None
    }

    /// Acknowledge delivery of an interrupt, clearing its pending bit.
    #[inline]
    pub fn acknowledge(&mut self, vector: u8) {
        self.clear_irq(vector);
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

    /// Read a 32-bit protected-mode IDT gate descriptor.
    ///
    /// Each entry is 8 bytes. The descriptor is parsed into an [`IdtEntry`]
    /// with the gate type, DPL, present bit, selector, and combined offset.
    ///
    /// Returns `Err(VmError::GeneralProtection)` if the vector exceeds the
    /// IDT limit.
    pub fn read_idt_entry_protected(
        &self,
        vector: u8,
        idtr_base: u64,
        idtr_limit: u16,
        mem: &dyn MemoryBus,
    ) -> Result<IdtEntry> {
        let entry_offset = (vector as u64) * 8;
        if entry_offset + 7 > idtr_limit as u64 {
            return Err(VmError::GeneralProtection(
                (vector as u32) * 8 + 2, // error code: IDT selector index + IDT flag
            ));
        }
        let addr = idtr_base + entry_offset;

        // Bytes [0..1]: offset low 16 bits
        let offset_low = mem.read_u16(addr)? as u64;
        // Bytes [2..3]: segment selector
        let selector = mem.read_u16(addr + 2)? as u16;
        // Byte [4]: reserved / IST (bits [2:0] in long mode, zero in 32-bit)
        let byte4 = mem.read_u8(addr + 4)?;
        // Byte [5]: type (bits [3:0]) + DPL (bits [6:5]) + P (bit 7)
        let type_attr = mem.read_u8(addr + 5)?;
        // Bytes [6..7]: offset high 16 bits
        let offset_high = mem.read_u16(addr + 6)? as u64;

        let offset = offset_low | (offset_high << 16);
        let gate_type_nibble = type_attr & 0x0F;
        let dpl = (type_attr >> 5) & 0x03;
        let present = (type_attr & 0x80) != 0;

        let gate_type = match gate_type_nibble {
            0x5 => GateType::Task,
            0x6 => GateType::Interrupt16,
            0x7 => GateType::Trap16,
            0xE => GateType::Interrupt32,
            0xF => GateType::Trap32,
            _ => {
                return Err(VmError::GeneralProtection(
                    (vector as u32) * 8 + 2,
                ));
            }
        };

        Ok(IdtEntry {
            offset,
            selector,
            gate_type,
            dpl,
            present,
            ist: byte4 & 0x07,
        })
    }

    /// Read a 64-bit long-mode IDT gate descriptor.
    ///
    /// Each entry is 16 bytes. The upper 8 bytes contain the high 32 bits
    /// of the handler offset, allowing handlers anywhere in the 64-bit
    /// address space.
    ///
    /// Returns `Err(VmError::GeneralProtection)` if the vector exceeds the
    /// IDT limit.
    pub fn read_idt_entry_long(
        &self,
        vector: u8,
        idtr_base: u64,
        idtr_limit: u16,
        mem: &dyn MemoryBus,
    ) -> Result<IdtEntry> {
        let entry_offset = (vector as u64) * 16;
        if entry_offset + 15 > idtr_limit as u64 {
            return Err(VmError::GeneralProtection(
                (vector as u32) * 16 + 2,
            ));
        }
        let addr = idtr_base + entry_offset;

        // Bytes [0..1]: offset bits [15:0]
        let offset_low = mem.read_u16(addr)? as u64;
        // Bytes [2..3]: segment selector
        let selector = mem.read_u16(addr + 2)? as u16;
        // Byte [4]: IST (bits [2:0]), rest reserved
        let ist = mem.read_u8(addr + 4)? & 0x07;
        // Byte [5]: type (bits [3:0]) + DPL (bits [6:5]) + P (bit 7)
        let type_attr = mem.read_u8(addr + 5)?;
        // Bytes [6..7]: offset bits [31:16]
        let offset_mid = mem.read_u16(addr + 6)? as u64;
        // Bytes [8..11]: offset bits [63:32]
        let offset_high = mem.read_u32(addr + 8)? as u64;
        // Bytes [12..15]: reserved (must be zero, not validated here)

        let offset = offset_low | (offset_mid << 16) | (offset_high << 32);
        let gate_type_nibble = type_attr & 0x0F;
        let dpl = (type_attr >> 5) & 0x03;
        let present = (type_attr & 0x80) != 0;

        let gate_type = match gate_type_nibble {
            0xE => GateType::Interrupt64,
            0xF => GateType::Trap64,
            _ => {
                return Err(VmError::GeneralProtection(
                    (vector as u32) * 16 + 2,
                ));
            }
        };

        Ok(IdtEntry {
            offset,
            selector,
            gate_type,
            dpl,
            present,
            ist,
        })
    }
}
