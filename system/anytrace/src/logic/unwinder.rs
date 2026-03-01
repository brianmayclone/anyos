//! RBP-chain stack unwinding.

use alloc::vec::Vec;
use anyos_std::debug;

/// A single frame in the call stack.
#[derive(Clone)]
pub struct StackFrame {
    /// Index in the backtrace (0 = current frame).
    pub index: u32,
    /// Instruction pointer for this frame.
    pub rip: u64,
    /// Frame pointer (RBP) for this frame.
    pub rbp: u64,
    /// Symbol name if resolved (from ELF symbol table).
    pub symbol: Option<alloc::string::String>,
    /// Offset within the symbol.
    pub offset: u64,
}

/// Walk the RBP chain to produce a call stack.
///
/// Reads `[RBP+8]` for return address and `[RBP]` for next frame pointer.
/// Stops at NULL RBP or after `max_depth` frames.
pub fn unwind(tid: u32, rip: u64, rbp: u64, max_depth: usize) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut current_rip = rip;
    let mut current_rbp = rbp;

    for i in 0..max_depth {
        frames.push(StackFrame {
            index: i as u32,
            rip: current_rip,
            rbp: current_rbp,
            symbol: None,
            offset: 0,
        });

        // Stop if RBP is NULL or in kernel space
        if current_rbp == 0 || current_rbp >= 0x0000_8000_0000_0000 {
            break;
        }

        // Read next frame: [RBP] = saved RBP, [RBP+8] = return address
        let mut frame_data = [0u8; 16];
        let read = debug::read_mem(tid, current_rbp, &mut frame_data);
        if read < 16 {
            break;
        }

        let next_rbp = u64::from_le_bytes([
            frame_data[0], frame_data[1], frame_data[2], frame_data[3],
            frame_data[4], frame_data[5], frame_data[6], frame_data[7],
        ]);
        let ret_addr = u64::from_le_bytes([
            frame_data[8], frame_data[9], frame_data[10], frame_data[11],
            frame_data[12], frame_data[13], frame_data[14], frame_data[15],
        ]);

        // Sanity check: return address should be in user space
        if ret_addr == 0 || ret_addr >= 0x0000_8000_0000_0000 {
            break;
        }

        current_rip = ret_addr;
        current_rbp = next_rbp;
    }

    frames
}
