//! Pending permission info — static array (NOT in Thread struct).
//!
//! Follows the same pattern as PENDING_PROGRAMS in loader.rs to avoid enlarging
//! the Thread struct (which changes heap layout and can trigger latent bugs).

use crate::sync::spinlock::Spinlock;

const MAX_PENDING_PERM: usize = 16;

struct PendingPermSlot {
    tid: u32,
    data: [u8; 512],
    len: u16,
    used: bool,
}

impl PendingPermSlot {
    const fn empty() -> Self {
        PendingPermSlot { tid: 0, data: [0u8; 512], len: 0, used: false }
    }
}

static PENDING_PERM_INFO: Spinlock<[PendingPermSlot; MAX_PENDING_PERM]> = Spinlock::new([
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
]);

/// Store pending permission info for the current thread.
/// Data is a UTF-8 byte slice: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path".
pub fn set_current_perm_pending(data: &[u8]) {
    let tid = super::current_tid();
    if tid == 0 { return; }
    let mut slots = PENDING_PERM_INFO.lock();
    // Overwrite existing slot for this TID, or allocate a new one
    let idx = slots.iter().position(|s| s.used && s.tid == tid)
        .or_else(|| slots.iter().position(|s| !s.used));
    if let Some(i) = idx {
        let len = data.len().min(512);
        slots[i].data[..len].copy_from_slice(&data[..len]);
        slots[i].len = len as u16;
        slots[i].tid = tid;
        slots[i].used = true;
    }
}

/// Read pending permission info for the current thread into `buf`.
/// Consumes (clears) the slot after reading.
/// Returns the number of bytes copied (0 if none).
pub fn current_perm_pending(buf: &mut [u8]) -> usize {
    let tid = super::current_tid();
    if tid == 0 { return 0; }
    let mut slots = PENDING_PERM_INFO.lock();
    if let Some(slot) = slots.iter_mut().find(|s| s.used && s.tid == tid) {
        let len = slot.len as usize;
        if len > 0 {
            let copy = len.min(buf.len());
            buf[..copy].copy_from_slice(&slot.data[..copy]);
            // Consume the slot so it can be reused
            slot.used = false;
            return copy;
        }
    }
    0
}
