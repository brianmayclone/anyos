use crate::mir::MirBody;

pub struct RegAlloc {
    /// Stack offset for each local (negative from RBP)
    pub stack_slots: Vec<i32>,
    /// Total stack frame size (positive number)
    pub frame_size: i32,
}

pub fn allocate(body: &MirBody) -> RegAlloc {
    let mut offset = 0i32;
    let mut stack_slots = Vec::new();
    for _local in &body.locals {
        offset -= 8; // everything is 8 bytes for simplicity
        stack_slots.push(offset);
    }
    // Align to 16
    let frame_size = ((-offset + 15) / 16) * 16;
    RegAlloc { stack_slots, frame_size }
}
