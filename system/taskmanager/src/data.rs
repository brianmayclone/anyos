use alloc::vec::Vec;
use anyos_std::sys;
use crate::types::*;

pub fn fetch_tasks(buf: &mut [u8; THREAD_ENTRY_SIZE * 64], prev: &mut PrevTicks, total_sched_ticks: u32, result: &mut Vec<TaskEntry>) {
    result.clear();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX { return; }

    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6];
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
        let io_read_bytes = u64::from_le_bytes([
            buf[off + 40], buf[off + 41], buf[off + 42], buf[off + 43],
            buf[off + 44], buf[off + 45], buf[off + 46], buf[off + 47],
        ]);
        let io_write_bytes = u64::from_le_bytes([
            buf[off + 48], buf[off + 49], buf[off + 50], buf[off + 51],
            buf[off + 52], buf[off + 53], buf[off + 54], buf[off + 55],
        ]);

        let prev_ticks = prev.entries[..prev.count]
            .iter()
            .find(|e| e.0 == tid)
            .map(|e| e.1)
            .unwrap_or(cpu_ticks);

        let d_ticks = cpu_ticks.wrapping_sub(prev_ticks);
        let cpu_pct_x10 = if dt > 0 && d_ticks > 0 {
            (d_ticks as u64 * 1000 / dt as u64).min(1000) as u32
        } else {
            0
        };

        let uid = u16::from_le_bytes([buf[off + 56], buf[off + 57]]);

        result.push(TaskEntry { tid, name, name_len, state, priority: prio, arch, uid, user_pages, cpu_pct_x10, io_read_bytes, io_write_bytes });
    }

    prev.count = 0;
    for i in 0..count as usize {
        if prev.count >= MAX_TASKS { break; }
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
        prev.entries[prev.count] = (tid, cpu_ticks);
        prev.count += 1;
    }
    prev.prev_total = total_sched_ticks;
}

pub fn fetch_memory() -> Option<MemInfo> {
    let mut buf = [0u8; 16];
    if sys::sysinfo(0, &mut buf) != 0 { return None; }
    Some(MemInfo {
        total_frames: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        free_frames: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        heap_used: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        heap_total: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
    })
}

pub fn fetch_cpu(state: &mut CpuState) {
    let mut buf = [0u8; 16 + 8 * MAX_CPUS];
    sys::sysinfo(3, &mut buf);

    let total = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let idle = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ncpu = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    state.num_cpus = ncpu.max(1).min(MAX_CPUS as u32);
    state.total_sched_ticks = total;

    let dt = total.wrapping_sub(state.prev_total);
    let di = idle.wrapping_sub(state.prev_idle);
    state.overall_pct = if dt > 0 {
        100u32.saturating_sub(di.saturating_mul(100) / dt)
    } else {
        0
    };
    state.prev_total = total;
    state.prev_idle = idle;

    for i in 0..(state.num_cpus as usize).min(MAX_CPUS) {
        let off = 16 + i * 8;
        if off + 8 > buf.len() { break; }
        let ct = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let ci = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let dct = ct.wrapping_sub(state.prev_core_total[i]);
        let dci = ci.wrapping_sub(state.prev_core_idle[i]);
        state.core_pct[i] = if dct > 0 {
            100u32.saturating_sub(dci.saturating_mul(100) / dct)
        } else {
            0
        };
        state.prev_core_total[i] = ct;
        state.prev_core_idle[i] = ci;
    }
}

pub fn fetch_hwinfo() -> HwInfo {
    let mut buf = [0u8; 96];
    sys::sysinfo(4, &mut buf);
    let mut brand = [0u8; 48];
    let mut vendor = [0u8; 16];
    brand.copy_from_slice(&buf[0..48]);
    vendor.copy_from_slice(&buf[48..64]);
    HwInfo {
        brand, vendor,
        tsc_mhz: u32::from_le_bytes([buf[64], buf[65], buf[66], buf[67]]),
        cpu_count: u32::from_le_bytes([buf[68], buf[69], buf[70], buf[71]]),
        boot_mode: u32::from_le_bytes([buf[72], buf[73], buf[74], buf[75]]),
        total_mem_mib: u32::from_le_bytes([buf[76], buf[77], buf[78], buf[79]]),
        free_mem_mib: u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]),
        fb_width: u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]),
        fb_height: u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]),
        fb_bpp: u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]),
    }
}
