#![no_std]
#![no_main]

use anyos_std::format;
use anyos_std::sys;
use anyos_std::process;
use anyos_std::ui::window;
use anyos_std::Vec;

anyos_std::entry!(main);

// ─── Colors (ARGB) ──────────────────────────────────────────────────────────

const COLOR_BG: u32 = 0xFF2D2D2D;
const COLOR_HEADER_BG: u32 = 0xFF383838;
const COLOR_ROW_ALT: u32 = 0xFF333333;
const COLOR_SEPARATOR: u32 = 0xFF4A4A4A;

const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_DIM: u32 = 0xFF969696;
const COLOR_TITLE: u32 = 0xFF00C8FF;

const COLOR_RUNNING: u32 = 0xFF27C93F;
const COLOR_READY: u32 = 0xFFFFBD2E;
const COLOR_BLOCKED: u32 = 0xFFFF5F56;

// ─── Layout ──────────────────────────────────────────────────────────────────

const FONT_W: u32 = 8;
const ROW_HEIGHT: i16 = 20;
const PAD: i16 = 8;

// Column X positions
const COL_TID: i16 = 10;
const COL_NAME: i16 = 60;
const COL_STATE: i16 = 240;
const COL_PRIO: i16 = 340;

// Key codes
const KEY_ESCAPE: u32 = 0x103;
const EVENT_KEY_DOWN: u32 = 1;
const EVENT_RESIZE: u32 = 3;

// ─── Data Structures ─────────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
}

struct MemInfo {
    total_frames: u32,
    free_frames: u32,
    heap_used: u32,
    heap_total: u32,
}

// ─── Data Fetching ───────────────────────────────────────────────────────────

fn fetch_tasks(buf: &mut [u8; 32 * 64]) -> Vec<TaskEntry> {
    let mut result = Vec::new();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX {
        return result;
    }
    for i in 0..count as usize {
        let off = i * 32;
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        result.push(TaskEntry { tid, name, name_len, state, priority: prio });
    }
    result
}

fn fetch_memory() -> Option<MemInfo> {
    let mut buf = [0u8; 16];
    if sys::sysinfo(0, &mut buf) != 0 {
        return None;
    }
    Some(MemInfo {
        total_frames: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        free_frames: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        heap_used: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        heap_total: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
    })
}

// ─── Number Formatting ──────────────────────────────────────────────────────

fn u32_to_str(mut val: u32, buf: &mut [u8; 12]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut i = 0;
    while val > 0 && i < 12 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    // Reverse
    let len = i;
    let mut a = 0;
    let mut b = len - 1;
    while a < b {
        buf.swap(a, b);
        a += 1;
        b -= 1;
    }
    len
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn draw_text_u32(win_id: u32, x: i16, y: i16, color: u32, val: u32) {
    let mut buf = [0u8; 12];
    let len = u32_to_str(val, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf[..len]) {
        window::draw_text(win_id, x, y, color, s);
    }
}

fn render(win_id: u32, tasks: &[TaskEntry], mem: &Option<MemInfo>, win_w: u32, win_h: u32) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, COLOR_BG);

    // ── System Stats Header ──
    let stats_h: i16 = 54;
    window::fill_rect(win_id, 0, 0, win_w as u16, stats_h as u16, COLOR_HEADER_BG);

    // Title
    window::draw_text(win_id, PAD, 4, COLOR_TITLE, "Activity Monitor");

    // Uptime
    let ticks = sys::uptime();
    let secs = ticks / 100;
    let mins = secs / 60;
    let uptime_str = format!("Uptime: {}m {}s", mins, secs % 60);
    let uptime_x = win_w as i16 - (uptime_str.len() as i16 * FONT_W as i16) - PAD;
    window::draw_text(win_id, uptime_x, 4, COLOR_TEXT_DIM, &uptime_str);

    // Memory bar
    if let Some(ref mem) = mem {
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        let used_kb = total_kb - free_kb;
        let heap_kb = mem.heap_used / 1024;
        let heap_total_kb = mem.heap_total / 1024;

        let mem_str = format!(
            "Phys: {} / {} MiB    Heap: {} / {} KiB",
            used_kb / 1024,
            total_kb / 1024,
            heap_kb,
            heap_total_kb
        );
        window::draw_text(win_id, PAD, 22, COLOR_TEXT, &mem_str);

        // Usage bar
        let bar_x: i16 = PAD;
        let bar_y: i16 = 40;
        let bar_w = (win_w as i16 - PAD * 2) as u16;
        let bar_h: u16 = 8;

        // Background
        window::fill_rect(win_id, bar_x, bar_y, bar_w, bar_h, COLOR_SEPARATOR);

        // Filled portion
        if total_kb > 0 {
            let pct = (used_kb as u64 * bar_w as u64 / total_kb as u64) as u16;
            if pct > 0 {
                let bar_color = if used_kb * 100 / total_kb > 80 {
                    COLOR_BLOCKED
                } else if used_kb * 100 / total_kb > 50 {
                    COLOR_READY
                } else {
                    COLOR_RUNNING
                };
                window::fill_rect(win_id, bar_x, bar_y, pct, bar_h, bar_color);
            }
        }
    }

    let mut y = stats_h + 2;

    // ── Column Headers ──
    window::fill_rect(win_id, 0, y, win_w as u16, ROW_HEIGHT as u16, COLOR_SEPARATOR);
    window::draw_text(win_id, COL_TID, y + 2, COLOR_TEXT, "TID");
    window::draw_text(win_id, COL_NAME, y + 2, COLOR_TEXT, "Process");
    window::draw_text(win_id, COL_STATE, y + 2, COLOR_TEXT, "State");
    window::draw_text(win_id, COL_PRIO, y + 2, COLOR_TEXT, "Priority");
    y += ROW_HEIGHT;

    // ── Task Rows ──
    let task_count_str = format!("{} processes", tasks.len());
    let count_x = win_w as i16 - (task_count_str.len() as i16 * FONT_W as i16) - PAD;
    window::draw_text(win_id, count_x, stats_h - ROW_HEIGHT + 4, COLOR_TEXT_DIM, &task_count_str);

    for (i, task) in tasks.iter().enumerate() {
        if y + ROW_HEIGHT > win_h as i16 {
            break;
        }

        // Alternate row background
        if i % 2 == 1 {
            window::fill_rect(win_id, 0, y, win_w as u16, ROW_HEIGHT as u16, COLOR_ROW_ALT);
        }

        let (state_str, state_color) = match task.state {
            0 => ("Ready", COLOR_READY),
            1 => ("Running", COLOR_RUNNING),
            2 => ("Blocked", COLOR_BLOCKED),
            3 => ("Terminated", COLOR_TEXT_DIM),
            4 => ("Dead", COLOR_TEXT_DIM),
            _ => ("Unknown", COLOR_TEXT_DIM),
        };

        // TID
        draw_text_u32(win_id, COL_TID, y + 2, COLOR_TEXT, task.tid);

        // Name
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            window::draw_text(win_id, COL_NAME, y + 2, COLOR_TEXT, name);
        }

        // State (color indicator dot + text)
        window::fill_rect(win_id, COL_STATE, y + 6, 8, 8, state_color);
        window::draw_text(win_id, COL_STATE + 12, y + 2, state_color, state_str);

        // Priority
        draw_text_u32(win_id, COL_PRIO, y + 2, COLOR_TEXT_DIM, task.priority as u32);

        y += ROW_HEIGHT;
    }

    window::present(win_id);
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let win_id = window::create("Activity Monitor", 150, 80, 420, 350);
    if win_id == u32::MAX {
        anyos_std::println!("taskmanager: failed to create window");
        return;
    }

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((420, 350));

    let mut thread_buf = [0u8; 32 * 64];
    let mut event = [0u32; 5];
    let mut last_update: u32 = 0;

    // Initial render
    let tasks = fetch_tasks(&mut thread_buf);
    let mem = fetch_memory();
    render(win_id, &tasks, &mem, win_w, win_h);

    loop {
        // Check for events
        if window::get_event(win_id, &mut event) == 1 {
            if event[0] == EVENT_KEY_DOWN && event[1] == KEY_ESCAPE {
                break;
            }
            if event[0] == EVENT_RESIZE {
                win_w = event[1];
                win_h = event[2];
                // Force immediate re-render with new size
                let tasks = fetch_tasks(&mut thread_buf);
                let mem = fetch_memory();
                render(win_id, &tasks, &mem, win_w, win_h);
                last_update = sys::uptime();
            }
        }

        // Auto-refresh every ~500ms (50 ticks at 100Hz)
        let now = sys::uptime();
        if now.wrapping_sub(last_update) >= 50 {
            let tasks = fetch_tasks(&mut thread_buf);
            let mem = fetch_memory();
            render(win_id, &tasks, &mem, win_w, win_h);
            last_update = now;
        }

        process::yield_cpu();
    }

    window::destroy(win_id);
}
