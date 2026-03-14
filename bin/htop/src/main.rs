#![no_std]
#![no_main]

mod data;
mod render;

anyos_std::entry!(main);

pub const MAX_TASKS: usize = 256;
pub const MAX_CPUS:  usize = 32;
pub const THREAD_ENTRY_SIZE: usize = 64;
const REFRESH_MS: u32 = 1500;

fn get_terminal_size() -> (usize, usize) {
    let mut buf = [0u8; 8];
    let rows = {
        let n = anyos_std::env::get("LINES", &mut buf);
        if n != u32::MAX && n > 0 { parse_uint(&buf[..n as usize]).unwrap_or(24) } else { 24 }
    };
    let cols = {
        let n = anyos_std::env::get("COLUMNS", &mut buf);
        if n != u32::MAX && n > 0 { parse_uint(&buf[..n as usize]).unwrap_or(80) } else { 80 }
    };
    (rows.max(8), cols.max(40))
}

fn parse_uint(bytes: &[u8]) -> Option<usize> {
    let mut val: usize = 0;
    let mut any = false;
    for &b in bytes {
        if b >= b'0' && b <= b'9' { val = val * 10 + (b - b'0') as usize; any = true; }
        else { break; }
    }
    if any { Some(val) } else { None }
}

fn main() {
    use data::*;
    use render::*;

    anyos_std::sys::con_set_mode(
        anyos_std::sys::CON_MODE_HIDE_CURSOR | anyos_std::sys::CON_MODE_NO_AUTOSCROLL,
    );
    anyos_std::print!("\x1B[2J\x1B[H");

    static mut RAW_BUF: [u8; THREAD_ENTRY_SIZE * MAX_TASKS] =
        [0u8; THREAD_ENTRY_SIZE * MAX_TASKS];

    let mut prev = PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 };
    let mut cpu  = CpuState::new();

    const ET: TaskEntry = TaskEntry {
        tid: 0, name: [0; 24], name_len: 0, state: 0,
        priority: 0, uid: 0, user_pages: 0, cpu_pct_x10: 0,
    };
    let mut tasks = [ET; MAX_TASKS];
    let mut uid_cache: [(u16, [u8; 16], u8); 32] = [(0, [0u8; 16], 0); 32];

    fetch_cpu(&mut cpu);

    let mut elapsed_ms: u32 = 0;
    let mut need_redraw = true;

    loop {
        let key = anyos_std::sys::con_poll_key();
        if key != 0 && ((key & 0xFF) as u8 == b'q' || (key & 0xFF) as u8 == b'Q') {
            break;
        }

        anyos_std::process::sleep(50);
        elapsed_ms += 50;
        if elapsed_ms < REFRESH_MS && !need_redraw { continue; }
        elapsed_ms = 0;
        need_redraw = false;

        fetch_cpu(&mut cpu);
        let task_count = unsafe {
            fetch_tasks(&mut RAW_BUF, &mut prev, cpu.total_sched_ticks, &mut tasks)
        };
        sort_by_cpu_desc(&mut tasks, task_count);

        // Resolve usernames
        let mut uid_cache_len = 0usize;
        for i in 0..task_count {
            let uid = tasks[i].uid;
            if !uid_cache[..uid_cache_len].iter().any(|e| e.0 == uid) && uid_cache_len < 32 {
                let mut name_buf = [0u8; 16];
                let nlen = anyos_std::process::getusername(uid, &mut name_buf);
                let len = if nlen != u32::MAX && nlen > 0 { (nlen as u8).min(15) } else { 0 };
                uid_cache[uid_cache_len] = (uid, name_buf, len);
                uid_cache_len += 1;
            }
        }

        // Count states
        let mut n_running  = 0u32;
        let mut n_sleeping = 0u32;
        for i in 0..task_count {
            if tasks[i].state == 1 { n_running += 1; } else { n_sleeping += 1; }
        }

        // Memory
        let mut mem_buf = [0u8; 16];
        anyos_std::sys::sysinfo(0, &mut mem_buf);
        let total_frames = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free_frames  = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        let used_kb  = total_frames.saturating_sub(free_frames) * 4;
        let total_kb = total_frames * 4;

        // Uptime
        let ticks = anyos_std::sys::uptime();
        let hz    = anyos_std::sys::tick_hz();
        let total_secs = if hz > 0 { ticks / hz } else { 0 };

        let (term_rows, term_cols) = get_terminal_size();

        anyos_std::print!("\x1B[H");

        render_frame(&FrameData {
            cpu: &cpu,
            tasks: &tasks,
            task_count,
            uid_cache: &uid_cache,
            uid_cache_len,
            n_running,
            n_sleeping,
            used_kb,
            total_kb,
            hours: total_secs / 3600,
            mins:  (total_secs % 3600) / 60,
            secs:  total_secs % 60,
            term_rows,
            term_cols,
        });
    }

    anyos_std::sys::con_set_mode(0);
    anyos_std::print!("\x1B[2J\x1B[H{}", render::RESET);
}
