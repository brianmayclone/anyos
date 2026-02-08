use crate::sync::spinlock::Spinlock;
use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

// ── Event type constants ──

// System events (0x0001 - 0x00FF) — only kernel can emit
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
const MAX_QUEUE_DEPTH: usize = 64;

/// Wire format: 5 u32 words = 20 bytes.
/// words[0] = event_type, words[1..4] = payload
#[derive(Clone, Copy)]
pub struct EventData {
    pub words: [u32; 5],
}

impl EventData {
    pub fn new(event_type: u32, p1: u32, p2: u32, p3: u32, p4: u32) -> Self {
        EventData {
            words: [event_type, p1, p2, p3, p4],
        }
    }

    pub fn event_type(&self) -> u32 {
        self.words[0]
    }
}

// ── Subscription ──

struct Subscription {
    id: u32,
    filter: Option<u32>, // None = all events, Some(type) = only that type
    queue: VecDeque<EventData>,
}

// ── System Bus ──

static SYSTEM_BUS: Spinlock<Vec<Subscription>> = Spinlock::new(Vec::new());
static NEXT_SUB_ID: AtomicU32 = AtomicU32::new(1);

/// Emit an event to the system bus (kernel-only).
pub fn system_emit(event: EventData) {
    let mut bus = SYSTEM_BUS.lock();
    for sub in bus.iter_mut() {
        if sub.filter.is_none() || sub.filter == Some(event.event_type()) {
            if sub.queue.len() >= MAX_QUEUE_DEPTH {
                sub.queue.pop_front(); // Drop oldest
            }
            sub.queue.push_back(event);
        }
    }
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
        });
    }
    sub_id
}

/// Emit an event to a module channel.
pub fn channel_emit(channel_id: u32, event: EventData) {
    let mut bus = MODULE_BUS.lock();
    if let Some(channel) = bus.get_mut(&channel_id) {
        for sub in channel.subs.iter_mut() {
            if sub.filter.is_none() || sub.filter == Some(event.event_type()) {
                if sub.queue.len() >= MAX_QUEUE_DEPTH {
                    sub.queue.pop_front();
                }
                sub.queue.push_back(event);
            }
        }
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
