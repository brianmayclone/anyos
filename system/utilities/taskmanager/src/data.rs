use crate::types::*;
use alloc::vec::Vec;
use anyos_std::net;
use anyos_std::sys;

pub fn fetch_tasks(
    buf: &mut [u8; THREAD_ENTRY_SIZE * MAX_TASKS],
    prev: &mut PrevTicks,
    total_sched_ticks: u32,
    result: &mut Vec<TaskEntry>,
) {
    result.clear();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX {
        return;
    }

    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6];
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages =
            u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        let cpu_ticks =
            u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
        let io_read_bytes = u64::from_le_bytes([
            buf[off + 40],
            buf[off + 41],
            buf[off + 42],
            buf[off + 43],
            buf[off + 44],
            buf[off + 45],
            buf[off + 46],
            buf[off + 47],
        ]);
        let io_write_bytes = u64::from_le_bytes([
            buf[off + 48],
            buf[off + 49],
            buf[off + 50],
            buf[off + 51],
            buf[off + 52],
            buf[off + 53],
            buf[off + 54],
            buf[off + 55],
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
        let parent_tid =
            u32::from_le_bytes([buf[off + 60], buf[off + 61], buf[off + 62], buf[off + 63]]);
        let is_child_thread = buf[off + 7] != 0; // pd_shared flag from kernel

        // I/O rates: delta bytes over the 1s refresh interval.
        let (prev_read, prev_write) = prev.io_entries[..prev.count]
            .iter()
            .find(|e| e.0 == tid)
            .map(|e| (e.1, e.2))
            .unwrap_or((io_read_bytes, io_write_bytes));
        let io_read_bps = io_read_bytes.wrapping_sub(prev_read).min(u32::MAX as u64) as u32;
        let io_write_bps = io_write_bytes.wrapping_sub(prev_write).min(u32::MAX as u64) as u32;

        // Network bytes (tx at offset 64, rx at offset 72)
        let net_tx = u64::from_le_bytes([
            buf[off + 64],
            buf[off + 65],
            buf[off + 66],
            buf[off + 67],
            buf[off + 68],
            buf[off + 69],
            buf[off + 70],
            buf[off + 71],
        ]);
        let net_rx = u64::from_le_bytes([
            buf[off + 72],
            buf[off + 73],
            buf[off + 74],
            buf[off + 75],
            buf[off + 76],
            buf[off + 77],
            buf[off + 78],
            buf[off + 79],
        ]);
        // Network rate: delta bytes over 1000ms -> kbit/s.
        let (prev_tx, prev_rx) = prev.net_entries[..prev.count]
            .iter()
            .find(|e| e.0 == tid)
            .map(|e| (e.1, e.2))
            .unwrap_or((net_tx, net_rx));
        let d_tx = net_tx.wrapping_sub(prev_tx);
        let d_rx = net_rx.wrapping_sub(prev_rx);
        let d_net = d_tx.wrapping_add(d_rx);
        let net_kbit = if d_net > 0 {
            (d_net * 8 / 1000).min(u32::MAX as u64) as u32 // 1000ms refresh
        } else {
            0
        };
        let net_tx_bps = d_tx.min(u32::MAX as u64) as u32;
        let net_rx_bps = d_rx.min(u32::MAX as u64) as u32;

        result.push(TaskEntry {
            tid,
            name,
            name_len,
            state,
            priority: prio,
            arch,
            uid,
            user_pages,
            cpu_pct_x10,
            io_read_bytes,
            io_write_bytes,
            io_read_bps,
            io_write_bps,
            parent_tid,
            is_child_thread,
            net_kbit,
            net_rx_bps,
            net_tx_bps,
        });
    }

    prev.count = 0;
    for i in 0..count as usize {
        if prev.count >= MAX_TASKS {
            break;
        }
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let cpu_ticks =
            u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
        let net_tx = u64::from_le_bytes([
            buf[off + 64],
            buf[off + 65],
            buf[off + 66],
            buf[off + 67],
            buf[off + 68],
            buf[off + 69],
            buf[off + 70],
            buf[off + 71],
        ]);
        let net_rx = u64::from_le_bytes([
            buf[off + 72],
            buf[off + 73],
            buf[off + 74],
            buf[off + 75],
            buf[off + 76],
            buf[off + 77],
            buf[off + 78],
            buf[off + 79],
        ]);
        prev.entries[prev.count] = (tid, cpu_ticks);
        prev.net_entries[prev.count] = (tid, net_tx, net_rx);
        let io_read_bytes = u64::from_le_bytes([
            buf[off + 40],
            buf[off + 41],
            buf[off + 42],
            buf[off + 43],
            buf[off + 44],
            buf[off + 45],
            buf[off + 46],
            buf[off + 47],
        ]);
        let io_write_bytes = u64::from_le_bytes([
            buf[off + 48],
            buf[off + 49],
            buf[off + 50],
            buf[off + 51],
            buf[off + 52],
            buf[off + 53],
            buf[off + 54],
            buf[off + 55],
        ]);
        prev.io_entries[prev.count] = (tid, io_read_bytes, io_write_bytes);
        prev.count += 1;
    }
    prev.prev_total = total_sched_ticks;
}

pub fn sort_tasks_by_activity(tasks: &mut Vec<TaskEntry>) {
    for i in 1..tasks.len() {
        let mut j = i;
        while j > 0 && task_rank(&tasks[j]) > task_rank(&tasks[j - 1]) {
            tasks.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn task_rank(t: &TaskEntry) -> u64 {
    let cpu = (t.cpu_pct_x10 as u64) << 40;
    let io = ((t.io_read_bps as u64 + t.io_write_bps as u64).min(0xFFFFF)) << 20;
    let mem = (t.user_pages as u64).min(0xFFFFF);
    cpu | io | mem
}

/// Build the flat display list from grouped tasks.
///
/// Groups child threads (pd_shared) under their leader process.
/// Single-thread processes appear as standalone rows.
/// Multi-thread processes appear as collapsible group headers.
pub fn build_display_list(tasks: &[TaskEntry], expanded: &[u32], display: &mut Vec<DisplayRow>) {
    display.clear();
    let n = tasks.len().min(MAX_TASKS);
    if n == 0 {
        return;
    }

    // Step 1: Find the ultimate leader for each task.
    let mut leader = [0u16; MAX_TASKS];
    for i in 0..n {
        leader[i] = i as u16;
    }

    // Direct parent resolution
    for i in 0..n {
        if tasks[i].is_child_thread && tasks[i].parent_tid != 0 {
            for j in 0..n {
                if tasks[j].tid == tasks[i].parent_tid {
                    leader[i] = j as u16;
                    break;
                }
            }
        }
    }

    // Transitive closure (follow parent chains)
    for _ in 0..10 {
        let mut changed = false;
        for i in 0..n {
            let l = leader[i] as usize;
            if l < n && l != i && (leader[l] as usize) != l {
                leader[i] = leader[l];
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Step 2: Count members per leader
    let mut member_count = [0u16; MAX_TASKS];
    for i in 0..n {
        member_count[leader[i] as usize] += 1;
    }

    // Step 3: Build display list
    let mut processed = [false; MAX_TASKS];
    for i in 0..n {
        let l = leader[i] as usize;
        if processed[l] {
            continue;
        }
        processed[l] = true;

        let count = member_count[l];
        if count <= 1 {
            // Standalone process (single thread)
            display.push(DisplayRow {
                kind: 0,
                task_idx: l as u16,
                thread_count: 0,
                agg_cpu: tasks[l].cpu_pct_x10,
                agg_net: tasks[l].net_kbit,
                agg_pages: tasks[l].user_pages,
                agg_read_bps: tasks[l].io_read_bps,
                agg_write_bps: tasks[l].io_write_bps,
                agg_state: tasks[l].state,
            });
        } else {
            // Multi-thread process: compute aggregates
            let mut agg_cpu = 0u32;
            let mut agg_net = 0u32;
            let mut agg_pages = 0u32;
            let mut agg_read_bps = 0u32;
            let mut agg_write_bps = 0u32;
            let mut best_state = 2u8; // default blocked
            for j in 0..n {
                if leader[j] as usize == l {
                    agg_cpu += tasks[j].cpu_pct_x10;
                    agg_net += tasks[j].net_kbit;
                    agg_pages = agg_pages.saturating_add(tasks[j].user_pages);
                    agg_read_bps = agg_read_bps.saturating_add(tasks[j].io_read_bps);
                    agg_write_bps = agg_write_bps.saturating_add(tasks[j].io_write_bps);
                    match tasks[j].state {
                        1 => best_state = 1,                    // Running
                        0 if best_state != 1 => best_state = 0, // Ready
                        _ => {}
                    }
                }
            }

            display.push(DisplayRow {
                kind: 1,
                task_idx: l as u16,
                thread_count: count,
                agg_cpu,
                agg_net,
                agg_pages,
                agg_read_bps,
                agg_write_bps,
                agg_state: best_state,
            });

            // If expanded, add individual thread rows
            let is_expanded = expanded.iter().any(|&tid| tid == tasks[l].tid);
            if is_expanded {
                for j in 0..n {
                    if leader[j] as usize == l {
                        display.push(DisplayRow {
                            kind: 2,
                            task_idx: j as u16,
                            thread_count: 0,
                            agg_cpu: 0,
                            agg_net: 0,
                            agg_pages: 0,
                            agg_read_bps: 0,
                            agg_write_bps: 0,
                            agg_state: 0,
                        });
                    }
                }
            }
        }
    }
}

pub fn fetch_net_totals() -> Option<NetTotals> {
    net::net_stats().map(|s| NetTotals {
        rx_bytes: s.rx_bytes,
        tx_bytes: s.tx_bytes,
        rx_packets: s.rx_packets,
        tx_packets: s.tx_packets,
        rx_errors: s.rx_errors,
        tx_errors: s.tx_errors,
        tcp_curr_established: s.tcp_curr_established,
        tcp_conn_errors: s.tcp_conn_errors,
    })
}

pub fn fetch_memory() -> Option<MemInfo> {
    let mut buf = [0u8; 28];
    if sys::sysinfo(0, &mut buf) != 0 {
        return None;
    }
    Some(MemInfo {
        total_frames: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        free_frames: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        heap_used: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        heap_total: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        swap_total_pages: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
        swap_free_pages: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        swap_areas: u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
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
        if off + 8 > buf.len() {
            break;
        }
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

    if let Some(freq) = sys::cpu_frequency_info() {
        state.avg_freq_mhz = freq.average_mhz;
        state.total_freq_mhz = freq.total_mhz;
        state.max_freq_mhz = freq.max_mhz;
        let n = (state.num_cpus as usize).min(MAX_CPUS);
        for i in 0..n {
            state.core_freq_mhz[i] = freq.per_core_mhz[i];
        }
        for i in n..MAX_CPUS {
            state.core_freq_mhz[i] = 0;
        }
    }
}

pub fn fetch_hwinfo() -> HwInfo {
    let mut buf = [0u8; 116];
    sys::sysinfo(4, &mut buf);
    let freq = sys::cpu_frequency_info().unwrap_or_default();
    let mut brand = [0u8; 48];
    let mut vendor = [0u8; 16];
    brand.copy_from_slice(&buf[0..48]);
    vendor.copy_from_slice(&buf[48..64]);
    let buf_profile = u32::from_le_bytes([buf[108], buf[109], buf[110], buf[111]]);
    let buf_driver = u32::from_le_bytes([buf[112], buf[113], buf[114], buf[115]]);
    HwInfo {
        brand,
        vendor,
        tsc_mhz: u32::from_le_bytes([buf[64], buf[65], buf[66], buf[67]]),
        cpu_count: u32::from_le_bytes([buf[68], buf[69], buf[70], buf[71]]),
        boot_mode: u32::from_le_bytes([buf[72], buf[73], buf[74], buf[75]]),
        total_mem_mib: u32::from_le_bytes([buf[76], buf[77], buf[78], buf[79]]),
        free_mem_mib: u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]),
        fb_width: u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]),
        fb_height: u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]),
        fb_bpp: u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]),
        cpu_freq_mhz: freq
            .average_mhz
            .max(u32::from_le_bytes([buf[96], buf[97], buf[98], buf[99]])),
        total_cpu_freq_mhz: freq.total_mhz,
        max_freq_mhz: freq
            .max_mhz
            .max(u32::from_le_bytes([buf[100], buf[101], buf[102], buf[103]])),
        power_features: freq.features
            | u32::from_le_bytes([buf[104], buf[105], buf[106], buf[107]]),
        power_profile: if freq.num_cpus > 0 {
            freq.profile
        } else {
            buf_profile
        },
        power_driver: if freq.driver > 0 {
            freq.driver
        } else {
            buf_driver
        },
        core_freq_mhz: freq.per_core_mhz,
    }
}
