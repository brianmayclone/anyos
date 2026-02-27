#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::println;
use anyos_std::process;

use libanyui_client as anyui;

anyos_std::entry!(main);

mod config;
mod events;
mod framebuffer;
mod render;
mod settings;
mod theme;
mod types;

use config::{
    ensure_finder, is_finder, load_dock_config, load_ico_icon, load_icons, load_icons_hires,
    save_dock_config,
};
use events::{unpack_event_name, SYSTEM_NAMES};
use framebuffer::Framebuffer;
use render::{dock_hit_test, render_dock, DragInfo, RenderState};
use settings::{DockSettings, POS_BOTTOM, POS_LEFT};
use theme::{geometry, set_geometry, DockGeometry};
use types::{DockItem, STILL_RUNNING};

/// Context menu item layouts per dock item state.
/// Items are pipe-separated. Index order must match handle_context_menu_click().
const MENU_PINNED_RUNNING: &str = "Show Window|Hide|-|Quit|-|Remove from Dock";
const MENU_PINNED_STOPPED: &str = "Open|-|Remove from Dock";
const MENU_TRANSIENT_RUNNING: &str = "Show Window|Hide|-|Quit|-|Keep in Dock";
// Finder: always pinned, cannot be removed
const MENU_FINDER_RUNNING: &str = "Show Window|Hide|-|Quit";
const MENU_FINDER_STOPPED: &str = "Open";
const MENU_EMPTY: &str = " ";

/// IPC channel name for dock reload notifications.
const DOCK_CHANNEL_NAME: &str = "dock";

/// Timer intervals for adaptive tick rate.
const TIMER_FAST_MS: u32 = 16;  // ~60 Hz for animations / drag
const TIMER_IDLE_MS: u32 = 200; // 5 Hz for idle polling

/// Duration of magnification enter/exit animation (milliseconds).
const MAG_ANIM_MS: u32 = 150;

/// Extra width for vertical dock windows (tooltip display area).
const TOOLTIP_EXTRA_W: u32 = 200;

struct DockApp {
    win: anyui::Window,
    canvas: anyui::Canvas,
    items: Vec<DockItem>,
    hovered_idx: Option<usize>,
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
    // Dock settings
    settings: DockSettings,
    // Magnification animation state
    mouse_in_dock: bool,
    mag_progress: i32,
    mag_start_val: i32,
    mag_target: i32,
    mag_start_time: u32,
}

static mut APP: Option<DockApp> = None;
fn app() -> &'static mut DockApp { unsafe { APP.as_mut().unwrap() } }

/// Compute the dock window rectangle based on position and screen size.
fn dock_window_rect(geom: &DockGeometry, screen_w: u32, screen_h: u32) -> (i32, i32, u32, u32) {
    match geom.position {
        POS_LEFT => (0, 0, geom.total_h + TOOLTIP_EXTRA_W, screen_h),
        POS_BOTTOM => (0, (screen_h - geom.total_h) as i32, screen_w, geom.total_h),
        _ => {
            // POS_RIGHT
            let w = geom.total_h + TOOLTIP_EXTRA_W;
            ((screen_w - w) as i32, 0, w, screen_h)
        }
    }
}

/// Check if the mouse cursor (in local/canvas coordinates) is within the dock zone.
fn mouse_in_dock_zone(lx: i32, ly: i32, fb_w: u32) -> bool {
    let geom = geometry();
    match geom.position {
        POS_BOTTOM => ly >= (geom.margin as i32 - 8),
        // Left: pill flush at x=0, zone extends pill_w + 8px into the window
        POS_LEFT => lx <= (geom.dock_height as i32 + 8),
        // Right: pill flush at right edge of framebuffer
        _ => lx >= (fb_w as i32 - geom.dock_height as i32 - 8),
    }
}

/// Returns true when the dock needs the fast 16ms timer.
fn needs_fast_timer() -> bool {
    let a = app();
    a.drag_active
        || a.drag_mouse_down
        || !a.bounce_items.is_empty()
        || a.mag_progress != a.mag_target
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

    // Load settings and initialize geometry
    let dock_settings = settings::load_dock_settings();
    set_geometry(DockGeometry::from_settings(&dock_settings));

    let (wx, wy, ww, wh) = dock_window_rect(geometry(), screen_width, screen_height);

    let flags = anyui::WIN_FLAG_BORDERLESS
        | anyui::WIN_FLAG_NOT_RESIZABLE
        | anyui::WIN_FLAG_ALWAYS_ON_TOP;

    let win = anyui::Window::new_with_flags("Dock", wx, wy, ww, wh, flags);

    let canvas = anyui::Canvas::new(ww, wh);
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
    load_icons(&mut items, dock_settings.icon_size);
    if dock_settings.magnification {
        load_icons_hires(&mut items, dock_settings.mag_size);
    }

    let fb = Framebuffer::new(ww, wh);
    let has_gpu = anyos_std::ui::window::gpu_has_accel();

    unsafe {
        APP = Some(DockApp {
            win,
            canvas,
            items,
            hovered_idx: None,
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
            settings: dock_settings,
            mouse_in_dock: false,
            mag_progress: 0,
            mag_start_val: 0,
            mag_target: 0,
            mag_start_time: 0,
        });
    }

    // Initial render
    {
        let a = app();
        let rs = RenderState {
            hover_idx: a.hovered_idx,
            bounce_items: &a.bounce_items,
            now: anyos_std::sys::uptime(),
            drag: None,
            mouse_along: 0,
            mag_progress: 0,
            settings: &a.settings,
        };
        render_dock(&mut a.fb, &a.items, a.screen_width, a.screen_height, &rs);
        a.canvas.copy_pixels_from(&a.fb.pixels);
        a.needs_redraw = false;
    }

    // Mouse move — immediate magnification render for smooth swiping
    app().canvas.on_mouse_move(|mx, my| {
        let a = app();
        // Only do immediate render when magnification is active
        if !a.settings.magnification || a.mag_progress <= 0 || a.drag_active {
            return;
        }
        let mouse_along = match geometry().position {
            POS_BOTTOM => mx,
            _ => my,
        };

        // Update hover
        let new_hover = dock_hit_test(
            mx, my, a.screen_width, a.screen_height,
            &a.items, &a.settings, mouse_along, a.mag_progress,
        );
        if new_hover != a.hovered_idx {
            a.hovered_idx = new_hover;
            update_context_menu();
        }

        // Immediate render — no waiting for next timer tick
        let now = anyos_std::sys::uptime();
        let rs = RenderState {
            hover_idx: a.hovered_idx,
            bounce_items: &a.bounce_items,
            now,
            drag: None,
            mouse_along,
            mag_progress: a.mag_progress,
            settings: &a.settings,
        };
        render_dock(&mut a.fb, &a.items, a.screen_width, a.screen_height, &rs);
        a.canvas.copy_pixels_from(&a.fb.pixels);
        a.needs_redraw = false;
    });

    // Mouse down — start potential drag (left button) or context menu prep (right button)
    app().canvas.on_mouse_down(|x, y, button| {
        let a = app();
        if button == 1 {
            // Left click — start potential drag
            let mouse_along = match geometry().position {
                POS_BOTTOM => x,
                _ => y,
            };
            if let Some(idx) = dock_hit_test(
                x, y, a.screen_width, a.screen_height,
                &a.items, &a.settings, mouse_along, a.mag_progress,
            ) {
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
    app().canvas.on_mouse_up(|x, y, _button| {
        let a = app();
        if a.drag_active {
            finalize_drag(x, y);
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
    let now = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz().max(1);

    // Check hover via mouse position
    let (mx, my, _) = a.canvas.get_mouse();
    let mouse_along = match geometry().position {
        POS_BOTTOM => mx,
        _ => my,
    };

    let new_hover = if a.drag_active {
        None // Suppress hover tooltip during drag
    } else {
        dock_hit_test(
            mx, my, a.screen_width, a.screen_height,
            &a.items, &a.settings, mouse_along, a.mag_progress,
        )
    };

    if new_hover != a.hovered_idx {
        a.hovered_idx = new_hover;
        a.needs_redraw = true;
        // Update context menu text for the hovered item
        update_context_menu();
    }

    // Magnification: check if mouse is in dock zone
    let in_zone = !a.items.is_empty()
        && a.settings.magnification
        && !a.drag_active
        && mouse_in_dock_zone(mx, my, a.fb.width);

    if in_zone != a.mouse_in_dock {
        a.mouse_in_dock = in_zone;
        a.mag_start_val = a.mag_progress;
        a.mag_target = if in_zone { 1000 } else { 0 };
        a.mag_start_time = now;
        if in_zone {
            ensure_fast_timer();
        }
    }

    // Animate magnification progress
    if a.mag_progress != a.mag_target {
        let elapsed_ms = now.wrapping_sub(a.mag_start_time) * 1000 / hz;
        if elapsed_ms >= MAG_ANIM_MS {
            a.mag_progress = a.mag_target;
        } else {
            let t = (elapsed_ms * 1000 / MAG_ANIM_MS) as i32;
            // EaseOut: t * (2 - t)
            let eased = t * (2000 - t) / 1000;
            let diff = a.mag_target - a.mag_start_val;
            a.mag_progress = a.mag_start_val + diff * eased / 1000;
        }
        a.needs_redraw = true;
    }

    // Magnification needs redraw each frame when active (mouse moves change icon sizes)
    if a.mouse_in_dock && a.settings.magnification && a.mag_progress > 0 {
        a.needs_redraw = true;
    }

    // Check drag activation (button held, moved > 5px along dock axis)
    if a.drag_mouse_down && !a.drag_active {
        let drag_dist = match geometry().position {
            POS_BOTTOM => (mx - a.drag_start_x).abs(),
            _ => (my - a.drag_start_y).abs(),
        };
        if drag_dist > 5 {
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

    // Clean up finished bounces (>2 seconds)
    let a = app();
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
            let (dmx, dmy, _) = a.canvas.get_mouse();
            Some(DragInfo {
                source_idx: a.drag_idx,
                mouse_x: dmx,
                mouse_y: dmy,
            })
        } else {
            None
        };

        let rs = RenderState {
            hover_idx: a.hovered_idx,
            bounce_items: &a.bounce_items,
            now,
            drag,
            mouse_along,
            mag_progress: a.mag_progress,
            settings: &a.settings,
        };
        render_dock(&mut a.fb, &a.items, a.screen_width, a.screen_height, &rs);
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
    let mouse_along = match geometry().position {
        POS_BOTTOM => lx,
        _ => ly,
    };
    let idx = match dock_hit_test(
        lx, ly, a.screen_width, a.screen_height,
        &a.items, &a.settings, mouse_along, a.mag_progress,
    ) {
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
        // "Show Window|Hide|-|Quit|-|Remove from Dock"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            3 => action_quit(item_idx),
            5 => action_unpin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_PINNED_STOPPED) {
        // "Open|-|Remove from Dock"
        match index {
            0 => action_open(item_idx),
            2 => action_unpin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_TRANSIENT_RUNNING) {
        // "Show Window|Hide|-|Quit|-|Keep in Dock"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            3 => action_quit(item_idx),
            5 => action_pin(item_idx),
            _ => {}
        }
    } else if core::ptr::eq(menu_text, MENU_FINDER_RUNNING) {
        // "Show Window|Hide|-|Quit"
        match index {
            0 => action_focus(item_idx),
            1 => action_hide(item_idx),
            3 => action_quit(item_idx),
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
fn finalize_drag(mouse_x: i32, mouse_y: i32) {
    let a = app();
    let src = a.drag_idx;
    if src >= a.items.len() { return; }

    let drop_idx = render::drag_drop_index(
        mouse_x, mouse_y, a.screen_width, a.screen_height, &a.items, src,
    );

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

    let icon_size = a.settings.icon_size;
    let mag_size = a.settings.mag_size;
    let magnification = a.settings.magnification;

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
    let icon = load_ico_icon(&icon_path, icon_size);
    let icon_hires = if magnification {
        load_ico_icon(&icon_path, mag_size)
    } else {
        None
    };

    let a = app();
    a.items.push(DockItem {
        name,
        bin_path,
        icon,
        icon_hires,
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

/// Poll the dock IPC channel for reload notifications.
fn poll_dock_channel() {
    let a = app();
    let chan = a.dock_chan;
    let sub = a.dock_sub;

    let mut buf = [0u32; 5];
    while anyos_std::ipc::evt_chan_poll(chan, sub, &mut buf) {
        match buf[0] {
            1 => reload_dock_items(),
            2 => reload_settings(),
            _ => {}
        }
    }
}

/// Reload dock config and merge new items into the current list.
fn reload_dock_items() {
    let new_items = load_dock_config();

    let a = app();
    let icon_size = a.settings.icon_size;
    let mag_size = a.settings.mag_size;
    let magnification = a.settings.magnification;

    for new_item in new_items {
        // Skip if we already have this app (by bin_path)
        let already = a.items.iter().any(|it| it.bin_path == new_item.bin_path);
        if already {
            continue;
        }

        // New pinned item — load its icon and add
        let icon_path = anyos_std::icons::app_icon_path(&new_item.bin_path);
        let icon = load_ico_icon(&icon_path, icon_size);
        let icon_hires = if magnification {
            load_ico_icon(&icon_path, mag_size)
        } else {
            None
        };

        let a = app();
        a.items.push(DockItem {
            name: new_item.name,
            bin_path: new_item.bin_path,
            icon,
            icon_hires,
            running: false,
            tid: 0,
            pinned: true,
        });
        a.needs_redraw = true;
    }

    ensure_finder(&mut app().items);
}

/// Reload dock settings from config file and apply changes.
fn reload_settings() {
    let new_settings = settings::load_dock_settings();
    let a = app();

    let size_changed = new_settings.icon_size != a.settings.icon_size;
    let mag_size_changed = new_settings.mag_size != a.settings.mag_size;
    let position_changed = new_settings.position != a.settings.position;
    let mag_changed = new_settings.magnification != a.settings.magnification;

    a.settings = new_settings;

    // Update geometry
    set_geometry(DockGeometry::from_settings(&a.settings));

    // Extract settings values before borrowing items
    let icon_size = a.settings.icon_size;
    let mag_size = a.settings.mag_size;
    let magnification = a.settings.magnification;

    // Reload icons if size changed
    if size_changed {
        load_icons(&mut a.items, icon_size);
    }
    if (size_changed || mag_size_changed || mag_changed) && magnification {
        load_icons_hires(&mut a.items, mag_size);
    }

    // Reposition/resize window if geometry changed
    if size_changed || position_changed {
        // Move off-screen first so the compositor marks the OLD area as dirty
        a.win.move_to(-10000, -10000);

        let (wx, wy, ww, wh) = dock_window_rect(geometry(), a.screen_width, a.screen_height);
        a.fb = Framebuffer::new(ww, wh);
        a.win.resize(ww, wh);
        a.canvas.set_size(ww, wh);
        a.win.move_to(wx, wy);
        a.bounce_items.clear();
    }

    a.needs_redraw = true;
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
                    let (wx, wy, ww, wh) = dock_window_rect(geometry(), new_w, new_h);
                    a.fb = Framebuffer::new(ww, wh);
                    a.bounce_items.clear();
                    a.needs_redraw = true;
                    a.win.resize(ww, wh);
                    a.win.move_to(wx, wy);
                    a.canvas.set_size(ww, wh);
                }
            }
            _ => {}
        }
    }
}
