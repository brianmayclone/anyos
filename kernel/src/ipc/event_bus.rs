//! System and module event bus for publish/subscribe IPC.
//!
//! Two independent buses exist: the **system bus** (kernel-only emitters, e.g. process
//! lifecycle and hardware events) and the **module bus** (named channels that any process
//! can create, subscribe to, and emit events on). Each subscriber has a bounded per-sub
//! queue; oldest events are dropped when the queue is full.
//!
//! **Blocking wait support**: Subscribers can register a waiter TID. When an event is
//! emitted, blocked waiters are collected under the bus lock and woken *outside* the lock
//! via `scheduler::wake_thread()` (same pattern as pipe blocking).

use crate::sync::spinlock::Spinlock;
use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

// ── Event type constants ──

// System events (0x0001 - 0x00FF) -- only kernel can emit
pub const EVT_DEVICE_ATTACHED: u32 = 0x0001;
pub const EVT_DEVICE_DETACHED: u32 = 0x0002;
pub const EVT_NETWORK_LINK_UP: u32 = 0x0005;
pub const EVT_NETWORK_LINK_DOWN: u32 = 0x0006;
pub const EVT_NETWORK_DHCP_DONE: u32 = 0x0007;
pub const EVT_BOOT_COMPLETE: u32 = 0x0010;
pub const EVT_PROCESS_SPAWNED: u32 = 0x0020;
pub const EVT_PROCESS_EXITED: u32 = 0x0021;
pub const EVT_RESOLUTION_CHANGED: u32 = 0x0040;
pub const EVT_OUT_OF_MEMORY: u32 = 0x0030;
pub const EVT_DISK_ERROR: u32 = 0x0031;

// Module events (0x0100+) — apps can emit
pub const EVT_CUSTOM: u32 = 0x0100;
pub const EVT_DATA_READY: u32 = 0x0101;
pub const EVT_NOTIFICATION: u32 = 0x0103;

/// Maximum events queued per subscription before dropping oldest.
const MAX_QUEUE_DEPTH: usize = 512;

/// Fixed-size event payload: 5 x u32 words = 20 bytes.
///
/// `words[0]` holds the event type; `words[1..5]` carry type-specific payload data.
#[derive(Clone, Copy)]
pub struct EventData {
    pub words: [u32; 5],
}

impl EventData {
    /// Construct a new event with the given type and four payload words.
    pub fn new(event_type: u32, p1: u32, p2: u32, p3: u32, p4: u32) -> Self {
        EventData {
            words: [event_type, p1, p2, p3, p4],
        }
    }

    /// Return the event type code (first word).
    pub fn event_type(&self) -> u32 {
        self.words[0]
    }
}

// ── Subscription ──

struct Subscription {
    id: u32,
    filter: Option<u32>, // None = all events, Some(type) = only that type
    queue: VecDeque<EventData>,
    /// TID of thread blocked in `evt_chan_wait` / `evt_sys_wait` on this subscription.
    /// Cleared by emit (collected for wake) or by the waiter itself on timeout/return.
    waiter_tid: Option<u32>,
}

// ── System Bus ──

static SYSTEM_BUS: Spinlock<Vec<Subscription>> = Spinlock::new(Vec::new());
static NEXT_SUB_ID: AtomicU32 = AtomicU32::new(1);

/// Emit an event to the system bus (kernel-only).
///
/// Collects blocked waiter TIDs under the lock, wakes them outside
/// to avoid holding SYSTEM_BUS while acquiring SCHEDULER lock.
pub fn system_emit(event: EventData) {
    let mut tids_to_wake = [0u32; 8];
    let mut wake_count = 0usize;
    {
        let mut bus = SYSTEM_BUS.lock();
        for sub in bus.iter_mut() {
            if sub.filter.is_none() || sub.filter == Some(event.event_type()) {
                if sub.queue.len() >= MAX_QUEUE_DEPTH {
                    sub.queue.pop_front(); // Drop oldest
                }
                sub.queue.push_back(event);
                if let Some(tid) = sub.waiter_tid.take() {
                    if wake_count < 8 {
                        tids_to_wake[wake_count] = tid;
                        wake_count += 1;
                    }
                }
            }
        }
    }
    for i in 0..wake_count {
        crate::task::scheduler::wake_thread(tids_to_wake[i]);
    }
    // Also wake compositor — it may be blocked on a channel subscription,
    // not the system bus, but still needs to process system events.
    crate::syscall::handlers::wake_compositor_if_blocked();
}

/// Subscribe to system events. filter=0 means all events.
/// Returns a subscription ID.
pub fn system_subscribe(filter: u32) -> u32 {
    let id = NEXT_SUB_ID.fetch_add(1, Ordering::Relaxed);
    let filter = if filter == 0 { None } else { Some(filter) };
    let mut bus = SYSTEM_BUS.lock();
    bus.push(Subscription {
        id,
        filter,
        queue: VecDeque::new(),
        waiter_tid: None,
    });
    id
}

/// Poll for the next event on a system subscription.
pub fn system_poll(sub_id: u32) -> Option<EventData> {
    let mut bus = SYSTEM_BUS.lock();
    if let Some(sub) = bus.iter_mut().find(|s| s.id == sub_id) {
        sub.queue.pop_front()
    } else {
        None
    }
}

/// Unsubscribe from the system bus.
pub fn system_unsubscribe(sub_id: u32) {
    let mut bus = SYSTEM_BUS.lock();
    bus.retain(|s| s.id != sub_id);
}

// ── Module Bus ──

struct Channel {
    subs: Vec<Subscription>,
}

static MODULE_BUS: Spinlock<BTreeMap<u32, Channel>> = Spinlock::new(BTreeMap::new());

/// DJB2 hash for channel name strings.
fn djb2_hash(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 5381;
    for &b in bytes {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    hash
}

/// Create a named channel on the module bus.
/// Returns the channel ID (djb2 hash of name).
pub fn channel_create(name_bytes: &[u8]) -> u32 {
    let id = djb2_hash(name_bytes);
    let mut bus = MODULE_BUS.lock();
    bus.entry(id).or_insert(Channel {
        subs: Vec::new(),
    });
    id
}

/// Subscribe to a module channel. filter=0 means all events.
pub fn channel_subscribe(channel_id: u32, filter: u32) -> u32 {
    let sub_id = NEXT_SUB_ID.fetch_add(1, Ordering::Relaxed);
    let filter = if filter == 0 { None } else { Some(filter) };
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        channel.subs.push(Subscription {
            id: sub_id,
            filter,
            queue: VecDeque::new(),
            waiter_tid: None,
        });
    }
    sub_id
}

/// Emit an event to a module channel (broadcast to all matching subscribers).
///
/// Collects blocked waiter TIDs under the lock, wakes them outside
/// to avoid holding MODULE_BUS while acquiring SCHEDULER lock.
pub fn channel_emit(channel_id: u32, event: EventData) {
    let mut tids_to_wake = [0u32; 8];
    let mut wake_count = 0usize;
    {
        let mut bus = MODULE_BUS.lock();
        if let Some(channel) = bus.get_mut(&channel_id) {
            for sub in channel.subs.iter_mut() {
                if sub.filter.is_none() || sub.filter == Some(event.event_type()) {
                    if sub.queue.len() >= MAX_QUEUE_DEPTH {
                        sub.queue.pop_front();
                    }
                    sub.queue.push_back(event);
                    if let Some(tid) = sub.waiter_tid.take() {
                        if wake_count < 8 {
                            tids_to_wake[wake_count] = tid;
                            wake_count += 1;
                        }
                    }
                }
            }
        }
    }
    for i in 0..wake_count {
        crate::task::scheduler::wake_thread(tids_to_wake[i]);
    }
}

/// Emit an event to a specific subscriber on a module channel (unicast).
///
/// Used by the compositor to deliver window events only to the owning app,
/// preventing other apps from receiving keyboard/mouse events for windows
/// they don't own.
pub fn channel_emit_to(channel_id: u32, target_sub_id: u32, event: EventData) {
    let mut tid_to_wake: Option<u32> = None;
    {
        let mut bus = MODULE_BUS.lock();
        if let Some(channel) = bus.get_mut(&channel_id) {
            for sub in channel.subs.iter_mut() {
                if sub.id == target_sub_id {
                    if sub.queue.len() >= MAX_QUEUE_DEPTH {
                        sub.queue.pop_front();
                    }
                    sub.queue.push_back(event);
                    tid_to_wake = sub.waiter_tid.take();
                    break;
                }
            }
        }
    }
    if let Some(tid) = tid_to_wake {
        crate::task::scheduler::wake_thread(tid);
    }
}

/// Poll for the next event on a module channel subscription.
pub fn channel_poll(channel_id: u32, sub_id: u32) -> Option<EventData> {
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        if let Some(sub) = channel.subs.iter_mut().find(|s| s.id == sub_id) {
            return sub.queue.pop_front();
        }
    }
    None
}

/// Unsubscribe from a module channel.
pub fn channel_unsubscribe(channel_id: u32, sub_id: u32) {
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        channel.subs.retain(|s| s.id != sub_id);
    }
}

/// Destroy a module channel and all its subscriptions.
pub fn channel_destroy(channel_id: u32) {
    let mut bus = MODULE_BUS.lock();
    bus.remove(&channel_id);
}

/// Lock-free check if SYSTEM_BUS or MODULE_BUS lock is currently held.
pub fn is_any_bus_locked() -> bool {
    SYSTEM_BUS.is_locked() || MODULE_BUS.is_locked()
}

// ── Blocking wait helpers ──

/// Register a waiter TID on a channel subscription. Returns `false` if the
/// subscription already has queued events (caller should not block).
pub fn channel_register_waiter(channel_id: u32, sub_id: u32, tid: u32) -> bool {
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        if let Some(sub) = channel.subs.iter_mut().find(|s| s.id == sub_id) {
            if !sub.queue.is_empty() {
                return false; // Events already queued — don't block
            }
            sub.waiter_tid = Some(tid);
            return true; // Registered — caller should block
        }
    }
    false
}

/// Clear the waiter TID on a channel subscription (called after waking).
pub fn channel_unregister_waiter(channel_id: u32, sub_id: u32) {
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        if let Some(sub) = channel.subs.iter_mut().find(|s| s.id == sub_id) {
            sub.waiter_tid = None;
        }
    }
}

/// Non-blocking check if a channel subscription has queued events.
pub fn channel_has_events(channel_id: u32, sub_id: u32) -> bool {
    let bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get(&channel_id) {
        if let Some(sub) = channel.subs.iter().find(|s| s.id == sub_id) {
            return !sub.queue.is_empty();
        }
    }
    false
}
