#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::anim::{AnimSet, Easing};
use anyos_std::println;
use anyos_std::process;

use libanyui_client as anyui;

anyos_std::entry!(main);

mod config;
mod events;
mod framebuffer;
mod render;
mod theme;
mod types;

use config::{ensure_finder, is_finder, load_dock_config, load_ico_icon, load_icons, save_dock_config};
use events::{unpack_event_name, SYSTEM_NAMES};
use framebuffer::Framebuffer;
use render::{dock_hit_test, render_dock, DragInfo, RenderState};
use theme::DOCK_TOTAL_H;
use types::{DockItem, STILL_RUNNING};

/// Context menu item layouts per dock item state.
/// Items are pipe-separated. Index order must match handle_context_menu_click().
const MENU_PINNED_RUNNING: &str = "Show Window|Hide|Quit|Remove from Dock";
const MENU_PINNED_STOPPED: &str = "Open|Remove from Dock";
const MENU_TRANSIENT_RUNNING: &str = "Show Window|Hide|Quit|Keep in Dock";
// Finder: always pinned, cannot be removed
const MENU_FINDER_RUNNING: &str = "Show Window|Hide|Quit";
const MENU_FINDER_STOPPED: &str = "Open";
const MENU_EMPTY: &str = " ";

/// IPC channel name for dock reload notifications.
const DOCK_CHANNEL_NAME: &str = "dock";

/// Timer intervals for adaptive tick rate.
const TIMER_FAST_MS: u32 = 16;  // ~60 Hz for animations / drag
const TIMER_IDLE_MS: u32 = 200; // 5 Hz for idle polling

struct DockApp {
    canvas: anyui::Canvas,
    items: Vec<DockItem>,
    hovered_idx: Option<usize>,
    anims: AnimSet,
    bounce_items: Vec<(usize, u32)>,
    screen_width: u32,
    screen_height: u32,
    fb: Framebuffer,
    sys_sub: u32,
    has_gpu: bool,
    tid_names: Vec<(u32, String)>,
    needs_redraw: bool,
    last_theme: bool,
    // Context menu
    ctx_menu_id: u32,
    ctx_menu_idx: Option<usize>,
    last_menu_text: &'static str,
    // Drag-and-drop
    drag_mouse_down: bool,
    drag_start_x: i32,
    drag_start_y: i32,
    drag_idx: usize,
    drag_active: bool,
    // IPC channel for reload notifications
    dock_chan: u32,
    dock_sub: u32,
    // Adaptive timer: 16ms when active, 200ms when idle
    timer_id: u32,
    fast_timer: bool,
}

static mut APP: Option<DockApp> = None;
fn app() -> &'static mut DockApp { unsafe { APP.as_mut().unwrap() } }

/// Returns true when the dock needs the fast 16ms timer (animations, drag, bounces).
fn needs_fast_timer() -> bool {
    let a = app();
    let now = anyos_std::sys::uptime();
    a.drag_active || a.drag_mouse_down || !a.bounce_items.is_empty() || a.anims.has_active(now)
}

/// Switch to fast (16ms) timer. No-op if already fast.
fn ensure_fast_timer() {
    let a = app();
    if a.fast_timer { return; }
    anyui::kill_timer(a.timer_id);
    a.timer_id = anyui::set_timer(TIMER_FAST_MS, || { tick(); });
    a.fast_timer = true;
}

/// Switch to idle (200ms) timer. No-op if already idle.
fn ensure_idle_timer() {
    let a = app();
    if !a.fast_timer { return; }
    anyui::kill_timer(a.timer_id);
    a.timer_id = anyui::set_timer(TIMER_IDLE_MS, || { tick(); });
    a.fast_timer = false;
}

fn main() {
    println!("dock: starting with anyui...");

    if !anyui::init() {
        println!("dock: FAILED to init anyui");
        return;
    }

    let (screen_width, screen_height) = anyui::screen_size();
    println!("dock: screen_size={}x{}", screen_width, screen_height);
    if screen_width == 0 || screen_height == 0 {
        println!("dock: FAILED — screen size is zero");
        return;
    }

    let flags = anyui::WIN_FLAG_BORDERLESS
        | anyui::WIN_FLAG_NOT_RESIZABLE
        | anyui::WIN_FLAG_ALWAYS_ON_TOP;

    let win = anyui::Window::new_with_flags(
        "Dock",
        0, (screen_height - DOCK_TOTAL_H) as i32,
        screen_width, DOCK_TOTAL_H,
        flags,
    );

    let canvas = anyui::Canvas::new(screen_width, DOCK_TOTAL_H);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_interactive(true);
    win.add(&canvas);

    // Context menu (attached to canvas, text updated dynamically in tick)
    let ctx_menu = anyui::ContextMenu::new(MENU_EMPTY);
    let ctx_menu_id = anyui::Widget::id(&ctx_menu);
    canvas.set_context_menu(&ctx_menu);
    win.add(&ctx_menu);

    ctx_menu.on_item_click(|e| {
        handle_context_menu_click(e.index);
    });

    // Subscribe to system events (process spawn/exit notifications)
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);

    // Create IPC channel for dock reload notifications (from Finder etc.)
    let dock_chan = anyos_std::ipc::evt_chan_create(DOCK_CHANNEL_NAME);
    let dock_sub = anyos_std::ipc::evt_chan_subscribe(dock_chan, 0);

    // Load dock items from config + icons (Finder is always present)
    let mut items = load_dock_config();
    ensure_finder(&mut items);
    load_icons(&mut items);

    let fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);
    let has_gpu = anyos_std::ui::window::gpu_has_accel();

    unsafe {
        APP = Some(DockApp {
            canvas,
            items,
            hovered_idx: None,
            anims: AnimSet::new(),
            bounce_items: Vec::new(),
            screen_width,
            screen_height,
            fb,
            sys_sub,
            has_gpu,
            tid_names: Vec::new(),
            needs_redraw: true,
            last_theme: theme::is_light(),
            ctx_menu_id,
            ctx_menu_idx: None,
            last_menu_text: MENU_EMPTY,
            drag_mouse_down: false,
            drag_start_x: 0,
            drag_start_y: 0,
            drag_idx: 0,
            drag_active: false,
            dock_chan,
            dock_sub,
            timer_id: 0,
            fast_timer: false,
        });
    }

    // Initial render
    {
        let a = app();
        let rs = RenderState {
            hover_idx: a.hovered_idx,
            anims: &a.anims,
            bounce_items: &a.bounce_items,
            now: anyos_std::sys::uptime(),
            drag: None,
        };
        render_dock(&mut a.fb, &a.items, a.screen_width, &rs);
        a.canvas.copy_pixels_from(&a.fb.pixels);
        a.needs_redraw = false;
    }

    // Mouse down — start potential drag (left button) or context menu prep (right button)
    app().canvas.on_mouse_down(|x, y, button| {
        let a = app();
        if button == 1 {
            // Left click — start potential drag
            if let Some(idx) = dock_hit_test(x, y, a.screen_width, &a.items) {
                a.drag_mouse_down = true;
                a.drag_start_x = x;
                a.drag_start_y = y;
                a.drag_idx = idx;
                ensure_fast_timer();
            }
        }
        // Right-click context menu is handled automatically by anyui
    });

    // Mouse up — finalize drag or handle click
    app().canvas.on_mouse_up(|x, _y, _button| {
        let a = app();
        if a.drag_active {
            finalize_drag(x);
        } else if a.drag_mouse_down {
            // Was a click (no drag movement)
            handle_dock_click(a.drag_start_x, a.drag_start_y);
        }
        a.drag_mouse_down = false;
        a.drag_active = false;
        a.needs_redraw = true;
    });

    // Window lifecycle: add transient items only when a window is actually opened
    anyui::on_window_opened(|app_tid| {
        handle_window_opened(app_tid);
        ensure_fast_timer(); // bounce animation
    });

    // Window lifecycle: remove transient items when last window closes
    anyui::on_window_closed(|app_tid| {
        handle_window_closed(app_tid);
    });

    // Start with idle timer — switches to fast (16ms) when animations/drag are active
    app().timer_id = anyui::set_timer(TIMER_IDLE_MS, || {
        tick();
    });

    win.on_close(|_| {
        anyui::quit();
    });

    anyui::run();
}

fn tick() {
    let a = app();

    // Check hover via mouse position
    let (mx, my, _) = a.canvas.get_mouse();
    let new_hover = if a.drag_active {
        None // Suppress hover tooltip during drag
    } else {
        dock_hit_test(mx, my, a.screen_width, &a.items)
    };

    if new_hover != a.hovered_idx {
        if a.has_gpu {
            let now = anyos_std::sys::uptime();
            if let Some(old) = a.hovered_idx {
                a.anims.start_at(100 + old as u32, 4000, 0, 200, Easing::EaseOut, now);
            }
            if let Some(new) = new_hover {
                a.anims.start_at(100 + new as u32, 0, 4000, 200, Easing::EaseOut, now);
            }
        }
        a.hovered_idx = new_hover;
        a.needs_redraw = true;

        // Update context menu text for the hovered item
        update_context_menu();
    }

    // Check drag activation (left button held, moved > 5px)
    if a.drag_mouse_down && !a.drag_active {
        let dx = mx - a.drag_start_x;
        if dx.abs() > 5 {
            a.drag_active = true;
            a.needs_redraw = true;
        }
    }

    // During drag, continuously update render
    if a.drag_active {
        a.needs_redraw = true;
    }

    // Check theme changes
    let current_theme = theme::is_light();
    if current_theme != a.last_theme {
        a.last_theme = current_theme;
        a.needs_redraw = true;
    }

    // Poll system events + dock IPC channel
    process_system_events();
    poll_dock_channel();

    // Tick animations
    let a = app();
    let now = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz().max(1);
    a.anims.remove_done(now);
    if a.anims.has_active(now) {
        a.needs_redraw = true;
    }

    // Clean up finished bounces (>2 seconds)
    a.bounce_items.retain(|(_, start)| {
        let elapsed_ms = now.wrapping_sub(*start) * 1000 / hz;
        elapsed_ms < 2000
    });
    if !a.bounce_items.is_empty() {
        a.needs_redraw = true;
    }

    // Redraw if needed
    if a.needs_redraw {
        let drag = if a.drag_active {
            let (mx, _, _) = a.canvas.get_mouse();
            Some(DragInfo {
                source_idx: a.drag_idx,
                mouse_x: mx,
            })
        } else {
            None
        };

        let rs = RenderState {
            hover_idx: a.hovered_idx,
            anims: &a.anims,
            bounce_items: &a.bounce_items,
            now,
            drag,
        };
        render_dock(&mut a.fb, &a.items, a.screen_width, &rs);
        a.canvas.copy_pixels_from(&a.fb.pixels);
        a.needs_redraw = false;
    }

    // Adaptive timer: switch between 16ms (active) and 200ms (idle)
    if needs_fast_timer() {
        ensure_fast_timer();
    } else {
        ensure_idle_timer();
    }
}

/// Update context menu text based on the currently hovered dock item.
fn update_context_menu() {
    let a = app();
    let new_text = match a.hovered_idx {
        Some(idx) => {
            if let Some(item) = a.items.get(idx) {
                a.ctx_menu_idx = Some(idx);
                if is_finder(item) {
                    if item.running { MENU_FINDER_RUNNING } else { MENU_FINDER_STOPPED }
                } else if item.pinned {
                    if item.running { MENU_PINNED_RUNNING } else { MENU_PINNED_STOPPED }
                } else {
                    MENU_TRANSIENT_RUNNING
                }
            } else {
                a.ctx_menu_idx = None;
                MENU_EMPTY
            }
        }
        None => {
            a.ctx_menu_idx = None;
            MENU_EMPTY
        }
    };

    if !core::ptr::eq(new_text, a.last_menu_text) {
        a.last_menu_text = new_text;
        anyui::Control::from_id(a.ctx_menu_id).set_text(new_text);
    }
}

/// Handle a click on a dock item — launch or focus the app.
fn handle_dock_click(lx: i32, ly: i32) {
    let a = app();
    let idx = match dock_hit_test(lx, ly, a.screen_width, &a.items) {
        Some(i) => i,
        None => return,
    };
    let item = match a.items.get_mut(idx) {
        Some(i) => i,
        None => return,
    };

    if item.tid != 0 {
        let status = process::try_waitpid(item.tid);
        if status == STILL_RUNNING {
            // App running — focus its window via compositor IPC
            let cmd: [u32; 5] = [0x100A, item.tid, 0, 0, 0]; // CMD_FOCUS_BY_TID
            anyos_std::ipc::evt_chan_emit(anyui::get_compositor_channel(), &cmd);
            return;
        }
        // App exited — reset and re-launch below
        item.running = false;
        item.tid = 0;
    }

    let tid = process::spawn(&item.bin_path, "");
    if tid != 0 {
        item.tid = tid;
        item.running = true;
        if a.has_gpu {
            a.bounce_items.push((idx, anyos_std::sys::uptime()));
            ensure_fast_timer();
        }
        a.needs_redraw = true;
    }
}

/// Handle context menu item click.
fn handle_context_menu_click(index: u32) {
    let a = app();
    let item_idx = match a.ctx_menu_idx {
        Some(i) => i,
        None => return,
    };

    let menu_text = a.last_menu_text;

    if core::ptr::eq(menu_text, MENU_PINNED_RUNNING) {
        // "Show Window|Hide|Quit|Remove from Dock"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            2 => action_quit(item_idx),
            3 => action_unpin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_PINNED_STOPPED) {
        // "Open|Remove from Dock"
        match index {
            0 => action_open(item_idx),
            1 => action_unpin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_TRANSIENT_RUNNING) {
        // "Show Window|Hide|Quit|Keep in Dock"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            2 => action_quit(item_idx),
            3 => action_pin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_FINDER_RUNNING) {
        // "Show Window|Hide|Quit"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            2 => action_quit(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_FINDER_STOPPED) {
        // "Open"
        match index {
            0 => action_open(item_idx),
            _ => {}
        }
    }
}

fn action_focus(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get(idx) {
        if item.tid != 0 {
            let cmd: [u32; 5] = [0x100A, item.tid, 0, 0, 0]; // CMD_FOCUS_BY_TID
            anyos_std::ipc::evt_chan_emit(anyui::get_compositor_channel(), &cmd);
        }
    }
}

fn action_hide(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get(idx) {
        if item.tid != 0 {
            let cmd: [u32; 5] = [0x1014, item.tid, 0, 0, 0]; // CMD_HIDE_BY_TID
            anyos_std::ipc::evt_chan_emit(anyui::get_compositor_channel(), &cmd);
        }
    }
}

fn action_quit(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get(idx) {
        if item.tid != 0 {
            process::kill(item.tid);
        }
    }
}

fn action_open(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get_mut(idx) {
        let tid = process::spawn(&item.bin_path, "");
        if tid != 0 {
            item.tid = tid;
            item.running = true;
            if a.has_gpu {
                a.bounce_items.push((idx, anyos_std::sys::uptime()));
                ensure_fast_timer();
            }
            a.needs_redraw = true;
        }
    }
}

fn action_pin(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get_mut(idx) {
        item.pinned = true;
        a.needs_redraw = true;
    }
    save_dock_config(&app().items);
}

fn action_unpin(idx: usize) {
    let a = app();
    if let Some(item) = a.items.get(idx) {
        if is_finder(item) { return; } // Finder cannot be removed
    }
    if let Some(item) = a.items.get_mut(idx) {
        if item.running {
            // Keep in dock as transient (will be removed when app exits)
            item.pinned = false;
        } else {
            // Not running — remove immediately
            a.items.remove(idx);
        }
        a.needs_redraw = true;
    }
    save_dock_config(&app().items);
}

/// Finalize a drag operation — reorder items.
fn finalize_drag(mouse_x: i32) {
    let a = app();
    let src = a.drag_idx;
    if src >= a.items.len() { return; }

    let drop_idx = render::drag_drop_index(mouse_x, a.screen_width, &a.items, src);

    if drop_idx != src {
        let item = a.items.remove(src);
        let insert_at = if drop_idx > src { drop_idx - 1 } else { drop_idx };
        let insert_at = insert_at.min(a.items.len());
        a.items.insert(insert_at, item);
        a.needs_redraw = true;
        save_dock_config(&a.items);
    }
}

/// Handle window opened event — add transient dock item for windowed apps.
fn handle_window_opened(app_tid: u32) {
    let a = app();

    // Already tracked (pinned or previously added)?
    if a.items.iter().any(|it| it.tid == app_tid) {
        return;
    }

    // Look up cached TID→name from EVT_PROCESS_SPAWNED
    let name = match a.tid_names.iter().find(|(t, _)| *t == app_tid) {
        Some((_, n)) => n.clone(),
        None => return, // Unknown process, skip
    };

    // Skip system threads
    if SYSTEM_NAMES.iter().any(|&s| s == name.as_str()) {
        return;
    }

    let bin_path = {
        let app_path = alloc::format!("/Applications/{}.app", name);
        let mut stat_buf = [0u32; 7];
        if anyos_std::fs::stat(&app_path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            app_path
        } else {
            alloc::format!("/System/bin/{}", name)
        }
    };
    let icon_path = anyos_std::icons::app_icon_path(&bin_path);
    let icon = load_ico_icon(&icon_path);

    let a = app();
    a.items.push(DockItem {
        name,
        bin_path,
        icon,
        running: true,
        tid: app_tid,
        pinned: false,
    });
    if a.has_gpu {
        let idx = a.items.len() - 1;
        a.bounce_items.push((idx, anyos_std::sys::uptime()));
    }
    a.needs_redraw = true;
}

/// Handle window closed event — remove transient dock item when last window closes.
fn handle_window_closed(app_tid: u32) {
    let a = app();

    // Mark pinned items as not running
    for item in a.items.iter_mut() {
        if item.tid == app_tid && item.pinned {
            item.running = false;
            item.tid = 0;
            a.needs_redraw = true;
        }
    }

    // Remove transient items
    let mut i = 0;
    while i < a.items.len() {
        if a.items[i].tid == app_tid && !a.items[i].pinned {
            a.items.remove(i);
            a.needs_redraw = true;
        } else {
            i += 1;
        }
    }
}

/// Poll the dock IPC channel for reload notifications (e.g. from Finder).
fn poll_dock_channel() {
    let a = app();
    let chan = a.dock_chan;
    let sub = a.dock_sub;

    let mut buf = [0u32; 5];
    while anyos_std::ipc::evt_chan_poll(chan, sub, &mut buf) {
        if buf[0] == 1 {
            // Reload signal: re-read config and merge new items
            reload_dock_items();
        }
    }
}

/// Reload dock config and merge new items into the current list.
fn reload_dock_items() {
    let new_items = load_dock_config();

    let a = app();
    for new_item in new_items {
        // Skip if we already have this app (by bin_path)
        let already = a.items.iter().any(|it| it.bin_path == new_item.bin_path);
        if already {
            continue;
        }

        // New pinned item — load its icon and add
        let icon_path = anyos_std::icons::app_icon_path(&new_item.bin_path);
        let icon = load_ico_icon(&icon_path);

        let a = app();
        a.items.push(DockItem {
            name: new_item.name,
            bin_path: new_item.bin_path,
            icon,
            running: false,
            tid: 0,
            pinned: true,
        });
        a.needs_redraw = true;
    }

    ensure_finder(&mut app().items);
}

/// Handle system events (process spawn/exit, resolution change).
fn process_system_events() {
    let a = app();
    let sys_sub = a.sys_sub;

    let mut sys_buf = [0u32; 5];
    while anyos_std::ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
        match sys_buf[0] {
            0x0020 => {
                // EVT_PROCESS_SPAWNED
                let spawned_tid = sys_buf[1];
                let name = unpack_event_name(sys_buf[2], sys_buf[3], sys_buf[4]);
                if name.is_empty() { continue; }

                let a = app();

                // Cache TID→name for later lookup
                if !a.tid_names.iter().any(|(t, _)| *t == spawned_tid) {
                    a.tid_names.push((spawned_tid, name.clone()));
                }

                // Skip system threads
                if SYSTEM_NAMES.iter().any(|&s| s == name.as_str()) {
                    continue;
                }

                // Match pinned items by binary basename
                for item in a.items.iter_mut() {
                    if item.pinned && item.tid == 0 {
                        let raw_basename = item.bin_path.rsplit('/').next().unwrap_or("");
                        let basename = raw_basename.strip_suffix(".app").unwrap_or(raw_basename);
                        if basename == name.as_str() {
                            item.tid = spawned_tid;
                            item.running = true;
                            a.needs_redraw = true;
                            break;
                        }
                    }
                }

                // Transient items are added via on_window_opened callback,
                // so only programs with actual windows appear in the dock.
            }
            0x0021 => {
                // EVT_PROCESS_EXITED
                let exited_tid = sys_buf[1];
                let a = app();

                // Mark pinned items as not running
                for item in a.items.iter_mut() {
                    if item.tid == exited_tid && item.pinned {
                        item.running = false;
                        item.tid = 0;
                        a.needs_redraw = true;
                    }
                }

                // Remove transient items
                let mut i = 0;
                while i < a.items.len() {
                    if a.items[i].tid == exited_tid && !a.items[i].pinned {
                        a.items.remove(i);
                        a.needs_redraw = true;
                    } else {
                        i += 1;
                    }
                }

                a.tid_names.retain(|(t, _)| *t != exited_tid);
            }
            0x0040 => {
                // EVT_RESOLUTION_CHANGED
                let new_w = sys_buf[1];
                let new_h = sys_buf[2];
                let a = app();
                if new_w != a.screen_width || new_h != a.screen_height {
                    a.screen_width = new_w;
                    a.screen_height = new_h;
                    a.fb = Framebuffer::new(new_w, DOCK_TOTAL_H);
                    a.bounce_items.clear();
                    a.needs_redraw = true;
                    // Reposition and resize canvas via anyui
                    a.canvas.set_size(new_w, DOCK_TOTAL_H);
                }
            }
            _ => {}
        }
    }
}
