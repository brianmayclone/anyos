//! Data structures and kernel data fetching for htop.

use crate::{MAX_TASKS, MAX_CPUS, THREAD_ENTRY_SIZE};

pub struct PrevTicks {
    pub entries:    [(u32, u32); MAX_TASKS],
    pub count:      usize,
    pub prev_total: u32,
}

pub struct CpuState {
    pub num_cpus:          u32,
    pub total_sched_ticks: u32,
    pub core_pct:          [u32; MAX_CPUS],
    pub prev_total:        u32,
    pub prev_idle:         u32,
    pub prev_core_total:   [u32; MAX_CPUS],
    pub prev_core_idle:    [u32; MAX_CPUS],
}

impl CpuState {
    pub const fn new() -> Self {
        CpuState {
            num_cpus: 1, total_sched_ticks: 0,
            core_pct: [0; MAX_CPUS],
            prev_total: 0, prev_idle: 0,
            prev_core_total: [0; MAX_CPUS],
            prev_core_idle:  [0; MAX_CPUS],
        }
    }
}

#[derive(Clone, Copy)]
pub struct TaskEntry {
    pub tid:         u32,
    pub name:        [u8; 24],
    pub name_len:    usize,
    pub state:       u8,
    pub priority:    i8,
    pub uid:         u16,
    pub user_pages:  u32,
    pub cpu_pct_x10: u32,
}

pub fn fetch_cpu(state: &mut CpuState) {
    let mut buf = [0u8; 16 + 8 * MAX_CPUS];
    anyos_std::sys::sysinfo(3, &mut buf);

    let total = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let idle  = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ncpu  = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    state.num_cpus = ncpu.max(1).min(MAX_CPUS as u32);
    state.total_sched_ticks = total;

    // Compute overall % from delta since last call, then update saved values
    let _dt = total.wrapping_sub(state.prev_total);
    let _di = idle.wrapping_sub(state.prev_idle);
    state.prev_total = total;
    state.prev_idle  = idle;

    for i in 0..(state.num_cpus as usize).min(MAX_CPUS) {
        let off = 16 + i * 8;
        if off + 8 > buf.len() { break; }
        let ct  = u32::from_le_bytes([buf[off],   buf[off+1], buf[off+2], buf[off+3]]);
        let ci  = u32::from_le_bytes([buf[off+4], buf[off+5], buf[off+6], buf[off+7]]);
        let dct = ct.wrapping_sub(state.prev_core_total[i]);
        let dci = ci.wrapping_sub(state.prev_core_idle[i]);
        state.core_pct[i] = if dct > 0 {
            100u32.saturating_sub(dci.saturating_mul(100) / dct)
        } else { 0 };
        state.prev_core_total[i] = ct;
        state.prev_core_idle[i]  = ci;
    }
}

pub fn fetch_tasks(
    raw:               &mut [u8],
    prev:              &mut PrevTicks,
    total_sched_ticks: u32,
    out:               &mut [TaskEntry; MAX_TASKS],
) -> usize {
    let count = anyos_std::sys::sysinfo(1, raw);
    if count == u32::MAX || count == 0 { return 0; }
    let n = (count as usize).min(MAX_TASKS);
    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid        = u32::from_le_bytes([raw[off],   raw[off+1], raw[off+2], raw[off+3]]);
        let prio       = raw[off + 4] as i8;
        let state_byte = raw[off + 5];
        let mut name   = [0u8; 24];
        name.copy_from_slice(&raw[off + 8..off + 32]);
        let name_len   = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages = u32::from_le_bytes([raw[off+32], raw[off+33], raw[off+34], raw[off+35]]);
        let cpu_ticks  = u32::from_le_bytes([raw[off+36], raw[off+37], raw[off+38], raw[off+39]]);
        let uid        = u16::from_le_bytes([raw[off+56], raw[off+57]]);

        let prev_ticks = prev.entries[..prev.count]
            .iter().find(|e| e.0 == tid).map(|e| e.1).unwrap_or(cpu_ticks);
        let d_ticks = cpu_ticks.wrapping_sub(prev_ticks);
        let cpu_pct_x10 = if dt > 0 && d_ticks > 0 {
            (d_ticks as u64 * 1000 / dt as u64).min(9999) as u32
        } else { 0 };

        out[i] = TaskEntry { tid, name, name_len, state: state_byte,
                             priority: prio, uid, user_pages, cpu_pct_x10 };
    }

    prev.count = n;
    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid       = u32::from_le_bytes([raw[off],   raw[off+1], raw[off+2], raw[off+3]]);
        let cpu_ticks = u32::from_le_bytes([raw[off+36], raw[off+37], raw[off+38], raw[off+39]]);
        prev.entries[i] = (tid, cpu_ticks);
    }
    prev.prev_total = total_sched_ticks;
    n
}

pub fn sort_by_cpu_desc(tasks: &mut [TaskEntry], n: usize) {
    for i in 1..n {
        let mut j = i;
        while j > 0 && tasks[j].cpu_pct_x10 > tasks[j - 1].cpu_pct_x10 {
            tasks.swap(j, j - 1); j -= 1;
        }
    }
}
