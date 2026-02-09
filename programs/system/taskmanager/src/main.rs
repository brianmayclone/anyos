#![no_std]
#![no_main]

use anyos_std::sys;
use anyos_std::process;
use anyos_std::ipc;
use anyos_std::ui::window;
use anyos_std::Vec;

anyos_std::entry!(main);

use uisys_client::*;

// ─── Layout ──────────────────────────────────────────────────────────────────

const ROW_H: i32 = 22;
const PAD: i32 = 8;
const STATS_H: i32 = 54;
const TOOLBAR_H: i32 = 32;
const HEADER_Y_OFFSET: i32 = STATS_H + TOOLBAR_H;

// Column X positions
const COL_TID: i32 = 10;
const COL_NAME: i32 = 60;
const COL_STATE: i32 = 190;
const COL_ARCH: i32 = 280;
const COL_CPU: i32 = 340;
const COL_PRIO: i32 = 410;

// Selected row highlight color
const SEL_BG: u32 = 0xFF0A4A8A;

// ─── Data Structures ─────────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    arch: u8,       // 0=x86_64, 1=x86
    cpu_ticks: u32,
}

struct MemInfo {
    total_frames: u32,
    free_frames: u32,
    heap_used: u32,
    heap_total: u32,
}

// ─── Data Fetching ───────────────────────────────────────────────────────────

fn fetch_tasks(buf: &mut [u8; 36 * 64]) -> Vec<TaskEntry> {
    let mut result = Vec::new();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX {
        return result;
    }
    for i in 0..count as usize {
        let off = i * 36;
        if off + 36 > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6]; // 0=x86_64, 1=x86
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let cpu_ticks = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        result.push(TaskEntry { tid, name, name_len, state, priority: prio, arch, cpu_ticks });
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

fn fetch_cpu_load(pipe_id: u32) -> u32 {
    if pipe_id == 0 { return 0; }
    let mut buf = [0u8; 64];
    let n = ipc::pipe_read(pipe_id, &mut buf) as usize;
    if n >= 4 {
        let off = n - 4;
        u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    } else {
        0
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render(
    win_id: u32,
    tasks: &[TaskEntry],
    mem: &Option<MemInfo>,
    cpu_pct: u32,
    selected: Option<usize>,
    total_ticks: u32,
    kill_btn: &UiToolbarButton,
    win_w: u32,
    win_h: u32,
) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG);

    // ── System Stats Header ──
    card(win_id, 0, 0, win_w, STATS_H as u32);

    label(win_id, PAD, 4, "Activity Monitor", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);

    // Uptime
    let ticks = sys::uptime();
    let secs = ticks / 100;
    let mins = secs / 60;
    let mut ubuf = [0u8; 24];
    let ustr = fmt_uptime(&mut ubuf, mins, secs % 60);
    label(win_id, win_w as i32 - (ustr.len() as i32 * 7) - PAD, 4, ustr, colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    // Memory info + CPU load
    if let Some(ref mem) = mem {
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        let used_kb = total_kb - free_kb;
        let heap_kb = mem.heap_used / 1024;
        let heap_total_kb = mem.heap_total / 1024;

        let mut mbuf = [0u8; 80];
        let ms = fmt_mem_cpu_line(&mut mbuf, used_kb / 1024, total_kb / 1024, heap_kb, heap_total_kb, cpu_pct);
        label(win_id, PAD, 22, ms, colors::TEXT, FontSize::Normal, TextAlign::Left);

        // Memory bar (left half) + CPU bar (right half)
        let half_w = ((win_w as i32 - PAD * 3) / 2) as u32;
        if total_kb > 0 {
            let mem_pct = (used_kb as u64 * 100 / total_kb as u64) as u32;
            progress(win_id, PAD, 40, half_w, 8, mem_pct);
        }
        progress(win_id, PAD + half_w as i32 + PAD, 40, half_w, 8, cpu_pct);
    }

    // ── Toolbar ──
    toolbar(win_id, 0, STATS_H, win_w, TOOLBAR_H as u32);
    kill_btn.render(win_id, "Kill Process");

    // Task count
    let mut cbuf = [0u8; 16];
    let cs = fmt_process_count(&mut cbuf, tasks.len());
    label(win_id, win_w as i32 - (cs.len() as i32 * 7) - PAD, STATS_H + 8, cs, colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let mut y = HEADER_Y_OFFSET;

    // ── Column Headers ──
    window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF4A4A4A);
    label(win_id, COL_TID, y + 3, "TID", colors::TEXT, FontSize::Small, TextAlign::Left);
    label(win_id, COL_NAME, y + 3, "Process", colors::TEXT, FontSize::Small, TextAlign::Left);
    label(win_id, COL_STATE, y + 3, "State", colors::TEXT, FontSize::Small, TextAlign::Left);
    label(win_id, COL_ARCH, y + 3, "Arch", colors::TEXT, FontSize::Small, TextAlign::Left);
    label(win_id, COL_CPU, y + 3, "CPU", colors::TEXT, FontSize::Small, TextAlign::Left);
    label(win_id, COL_PRIO, y + 3, "Priority", colors::TEXT, FontSize::Small, TextAlign::Left);
    y += ROW_H;

    // ── Task Rows ──
    for (i, task) in tasks.iter().enumerate() {
        if y + ROW_H > win_h as i32 {
            break;
        }

        // Selection highlight or alternating background
        if selected == Some(i) {
            window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, SEL_BG);
        } else if i % 2 == 1 {
            window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF333333);
        }

        let text_color = if selected == Some(i) { 0xFFFFFFFF } else { colors::TEXT };

        let (state_str, state_kind) = match task.state {
            0 => ("Ready", StatusKind::Warning),
            1 => ("Running", StatusKind::Online),
            2 => ("Blocked", StatusKind::Error),
            3 => ("Terminated", StatusKind::Offline),
            _ => ("Unknown", StatusKind::Offline),
        };

        // TID
        let mut tbuf = [0u8; 8];
        label(win_id, COL_TID, y + 3, fmt_u32(&mut tbuf, task.tid), text_color, FontSize::Small, TextAlign::Left);

        // Name
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            label(win_id, COL_NAME, y + 3, name, text_color, FontSize::Small, TextAlign::Left);
        }

        // State
        status_indicator(win_id, COL_STATE, y + 3, state_kind, state_str);

        // Architecture
        let arch_str = if task.arch == 1 { "x86" } else { "x86_64" };
        let arch_color = if task.arch == 1 { 0xFFFF9500 } else { colors::TEXT_SECONDARY };
        label(win_id, COL_ARCH, y + 3, arch_str, arch_color, FontSize::Small, TextAlign::Left);

        // CPU ticks (as percentage of total if > 0)
        let mut cpubuf = [0u8; 12];
        let cpu_str = if total_ticks > 0 && task.cpu_ticks > 0 {
            let pct_x10 = (task.cpu_ticks as u64 * 1000 / total_ticks as u64) as u32;
            fmt_pct(&mut cpubuf, pct_x10)
        } else {
            "0.0%"
        };
        label(win_id, COL_CPU, y + 3, cpu_str, colors::TEXT_SECONDARY, FontSize::Small, TextAlign::Left);

        // Priority
        let mut pbuf = [0u8; 8];
        label(win_id, COL_PRIO, y + 3, fmt_u32(&mut pbuf, task.priority as u32), colors::TEXT_SECONDARY, FontSize::Small, TextAlign::Left);

        y += ROW_H;
    }

    window::present(win_id);
}

// ─── Formatting ──────────────────────────────────────────────────────────────

fn fmt_u32<'a>(buf: &'a mut [u8; 8], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 8]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

/// Format pct_x10 (e.g. 125 = 12.5%) as "12.5%"
fn fmt_pct<'a>(buf: &'a mut [u8; 12], pct_x10: u32) -> &'a str {
    let whole = pct_x10 / 10;
    let frac = pct_x10 % 10;
    let mut p = 0;
    let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, whole);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'.'; p += 1;
    buf[p] = b'0' + frac as u8; p += 1;
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_uptime<'a>(buf: &'a mut [u8; 24], mins: u32, secs: u32) -> &'a str {
    let mut p = 0;
    buf[p..p + 8].copy_from_slice(b"Uptime: "); p += 8;
    let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, mins); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b"m "); p += 2;
    let s = fmt_u32(&mut t, secs); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b's'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mem_cpu_line<'a>(buf: &'a mut [u8; 80], used_mb: u32, total_mb: u32, heap_kb: u32, heap_total_kb: u32, cpu_pct: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 8];
    buf[p..p + 5].copy_from_slice(b"Mem: "); p += 5;
    let s = fmt_u32(&mut t, used_mb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut t, total_mb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 8].copy_from_slice(b"M  Heap:"); p += 8;
    let s = fmt_u32(&mut t, heap_kb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut t, heap_total_kb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 7].copy_from_slice(b"K  CPU:"); p += 7;
    let s = fmt_u32(&mut t, cpu_pct); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_process_count<'a>(buf: &'a mut [u8; 16], count: usize) -> &'a str {
    let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, count as u32);
    let mut p = 0;
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 10].copy_from_slice(b" processes"); p += 10;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let win_id = window::create("Activity Monitor", 120, 80, 500, 380);
    if win_id == u32::MAX {
        return;
    }

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((500, 380));

    // Open CPU load pipe
    let cpu_pipe_id = ipc::pipe_open("sys:cpu_load");

    // Kill button in the toolbar
    let kill_btn = UiToolbarButton::new(PAD, STATS_H + 4, 100, 24);

    let mut thread_buf = [0u8; 36 * 64];
    let mut event = [0u32; 5];
    let mut last_update: u32 = 0;
    let mut selected: Option<usize> = None;

    // Initial render
    let tasks = fetch_tasks(&mut thread_buf);
    let mem = fetch_memory();
    let cpu_pct = fetch_cpu_load(cpu_pipe_id);
    let total_ticks: u32 = tasks.iter().map(|t| t.cpu_ticks).sum();
    render(win_id, &tasks, &mem, cpu_pct, selected, total_ticks, &kill_btn, win_w, win_h);

    loop {
        // Check for events
        if window::get_event(win_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);

            if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                break;
            }

            if event[0] == EVENT_WINDOW_CLOSE {
                break;
            }

            if event[0] == EVENT_RESIZE {
                win_w = event[1];
                win_h = event[2];
                last_update = 0; // Force redraw
            }

            // Handle mouse click on task rows
            if ev.is_mouse_down() {
                let (_mx, my) = ev.mouse_pos();

                // Check kill button
                if kill_btn.handle_event(&ev) {
                    if let Some(sel_idx) = selected {
                        let tasks = fetch_tasks(&mut thread_buf);
                        if sel_idx < tasks.len() {
                            let tid = tasks[sel_idx].tid;
                            // Don't allow killing compositor (TID 3) or cpu_monitor
                            if tid > 3 {
                                process::kill(tid);
                                selected = None;
                                last_update = 0; // Force redraw
                            }
                        }
                    }
                } else {
                    // Check if clicked on a task row
                    let row_start_y = HEADER_Y_OFFSET + ROW_H; // After column headers
                    if my >= row_start_y {
                        let row_idx = ((my - row_start_y) / ROW_H) as usize;
                        let tasks = fetch_tasks(&mut thread_buf);
                        if row_idx < tasks.len() {
                            selected = Some(row_idx);
                        } else {
                            selected = None;
                        }
                        last_update = 0; // Force redraw
                    }
                }
            }
        }

        // Auto-refresh every ~500ms (50 ticks at 100Hz)
        let now = sys::uptime();
        if now.wrapping_sub(last_update) >= 50 {
            let tasks = fetch_tasks(&mut thread_buf);
            // Clamp selection if task list shrank
            if let Some(sel) = selected {
                if sel >= tasks.len() {
                    selected = if tasks.is_empty() { None } else { Some(tasks.len() - 1) };
                }
            }
            let mem = fetch_memory();
            let cpu_pct = fetch_cpu_load(cpu_pipe_id);
            let total_ticks: u32 = tasks.iter().map(|t| t.cpu_ticks).sum();
            render(win_id, &tasks, &mem, cpu_pct, selected, total_ticks, &kill_btn, win_w, win_h);
            last_update = now;
        }

        process::yield_cpu();
    }

    window::destroy(win_id);
}
