//! Lightweight packet tracing for userspace diagnostics (`dcpdump`).

use crate::sync::spinlock::Spinlock;

pub const DIR_RX: u8 = 0;
pub const DIR_TX: u8 = 1;
pub const ENTRY_SIZE: usize = 24;
const TRACE_CAPACITY: usize = 256;

#[derive(Clone, Copy)]
struct TraceEntry {
    timestamp_ms: u32,
    length: u16,
    ethertype: u16,
    src_port: u16,
    dst_port: u16,
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    direction: u8,
    protocol: u8,
    ttl: u8,
    flags: u8,
}

impl TraceEntry {
    const EMPTY: Self = Self {
        timestamp_ms: 0,
        length: 0,
        ethertype: 0,
        src_port: 0,
        dst_port: 0,
        src_ip: [0; 4],
        dst_ip: [0; 4],
        direction: DIR_RX,
        protocol: 0,
        ttl: 0,
        flags: 0,
    };

    fn write_to(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&self.timestamp_ms.to_le_bytes());
        out[4..6].copy_from_slice(&self.length.to_le_bytes());
        out[6..8].copy_from_slice(&self.ethertype.to_le_bytes());
        out[8..10].copy_from_slice(&self.src_port.to_le_bytes());
        out[10..12].copy_from_slice(&self.dst_port.to_le_bytes());
        out[12..16].copy_from_slice(&self.src_ip);
        out[16..20].copy_from_slice(&self.dst_ip);
        out[20] = self.direction;
        out[21] = self.protocol;
        out[22] = self.ttl;
        out[23] = self.flags;
    }
}

struct TraceState {
    enabled: bool,
    head: usize,
    len: usize,
    entries: [TraceEntry; TRACE_CAPACITY],
}

impl TraceState {
    const fn new() -> Self {
        Self {
            enabled: false,
            head: 0,
            len: 0,
            entries: [TraceEntry::EMPTY; TRACE_CAPACITY],
        }
    }

    fn push(&mut self, entry: TraceEntry) {
        self.entries[self.head] = entry;
        self.head = (self.head + 1) % TRACE_CAPACITY;
        if self.len < TRACE_CAPACITY {
            self.len += 1;
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

static TRACE_STATE: Spinlock<TraceState> = Spinlock::new(TraceState::new());

pub fn set_enabled(enabled: bool) {
    TRACE_STATE.lock().enabled = enabled;
}

pub fn clear() {
    TRACE_STATE.lock().clear();
}

pub fn read_and_clear(buf: &mut [u8]) -> u32 {
    let mut state = TRACE_STATE.lock();
    let capacity = buf.len() / ENTRY_SIZE;
    let count = state.len.min(capacity);
    let start = if state.len == TRACE_CAPACITY {
        state.head
    } else {
        0
    };

    for i in 0..count {
        let idx = (start + i) % TRACE_CAPACITY;
        let off = i * ENTRY_SIZE;
        state.entries[idx].write_to(&mut buf[off..off + ENTRY_SIZE]);
    }

    state.clear();
    count as u32
}

pub fn record_frame(direction: u8, data: &[u8]) {
    let mut state = TRACE_STATE.lock();
    if !state.enabled {
        return;
    }

    let mut entry = TraceEntry::EMPTY;
    entry.timestamp_ms = crate::arch::hal::timer_current_ticks();
    entry.length = data.len().min(u16::MAX as usize) as u16;
    entry.direction = direction;

    if data.len() < 14 {
        entry.flags |= 0x02;
        state.push(entry);
        return;
    }

    entry.ethertype = u16::from_be_bytes([data[12], data[13]]);
    if entry.ethertype == 0x0800 {
        parse_ipv4(&data[14..], &mut entry);
    } else if entry.ethertype == 0x86DD {
        entry.flags |= 0x04;
    }

    state.push(entry);
}

fn parse_ipv4(data: &[u8], entry: &mut TraceEntry) {
    if data.len() < 20 {
        entry.flags |= 0x02;
        return;
    }

    let ihl = (data[0] & 0x0f) as usize;
    let header_len = ihl * 4;
    if header_len < 20 || data.len() < header_len {
        entry.flags |= 0x02;
        return;
    }

    entry.protocol = data[9];
    entry.ttl = data[8];
    entry.src_ip.copy_from_slice(&data[12..16]);
    entry.dst_ip.copy_from_slice(&data[16..20]);

    let payload = &data[header_len..];
    if (entry.protocol == 6 || entry.protocol == 17) && payload.len() >= 4 {
        entry.src_port = u16::from_be_bytes([payload[0], payload[1]]);
        entry.dst_port = u16::from_be_bytes([payload[2], payload[3]]);
        if entry.src_port == 53 || entry.dst_port == 53 {
            entry.flags |= 0x01;
        }
    }
}
