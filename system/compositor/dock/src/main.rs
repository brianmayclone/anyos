#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::anim::{AnimSet, Easing};
use anyos_std::println;
use anyos_std::process;

use libcompositor_client::CompositorClient;

anyos_std::entry!(main);

mod config;
mod events;
mod framebuffer;
mod render;
mod theme;
mod types;

use config::{load_dock_config, load_ico_icon, load_icons};
use events::{query_thread_name, unpack_event_name, SYSTEM_NAMES};
use framebuffer::Framebuffer;
use render::{blit_to_surface, dock_hit_test, render_dock, RenderState};
use theme::DOCK_TOTAL_H;
use types::{DockItem, CMD_FOCUS_BY_TID, STILL_RUNNING};

fn main() {
    println!("dock: connecting to compositor...");
    let client = match CompositorClient::init() {
        Some(c) => c,
        None => {
            println!("dock: FAILED to init compositor client");
            return;
        }
    };

    let (screen_width, screen_height) = client.screen_size();
    println!("dock: screen_size={}x{}", screen_width, screen_height);
    if screen_width == 0 || screen_height == 0 {
        println!("dock: FAILED — screen size is zero");
        return;
    }

    // Borderless, not resizable, always on top
    let flags: u32 = 0x01 | 0x02 | 0x04;

    println!("dock: creating window ({}x{}, flags=0x{:x})", screen_width, DOCK_TOTAL_H, flags);
    let mut win = match client.create_window(0, (screen_height - DOCK_TOTAL_H) as i32, screen_width, DOCK_TOTAL_H, flags) {
        Some(w) => w,
        None => {
            println!("dock: FAILED to create window");
            return;
        }
    };

    client.set_title(&win, "Dock");

    // Subscribe to system events (process spawn/exit notifications)
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);
    let own_tid = anyos_std::process::getpid();

    // Load dock items from config + icons
    let mut items = load_dock_config();
    load_icons(&mut items);

    let mut screen_width = screen_width;
    let mut screen_height = screen_height;

    let mut fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);

    // Animation state
    let mut hovered_idx: Option<usize> = None;
    let mut anims = AnimSet::new();
    let mut bounce_items: Vec<(usize, u32)> = Vec::new();
    let has_gpu = anyos_std::ui::window::gpu_has_accel();
    // TID→name cache: populated from EVT_PROCESS_SPAWNED, consumed by EVT_WINDOW_OPENED
    let mut tid_names: Vec<(u32, String)> = Vec::new();

    // Initial render
    let rs = RenderState {
        hover_idx: hovered_idx,
        anims: &anims,
        bounce_items: &bounce_items,
        now: anyos_std::sys::uptime(),
    };
    render_dock(&mut fb, &items, screen_width, &rs);
    blit_to_surface(&fb, &win);
    client.present(&win);

    let mut needs_redraw = false;

    loop {
        process_compositor_events(
            &client, &mut win, &mut items, &mut hovered_idx, &mut anims,
            &mut bounce_items, &mut needs_redraw, &mut tid_names,
            screen_width, own_tid, has_gpu,
        );

        process_system_events(
            &client, &mut win, &mut items, &mut fb, &mut bounce_items,
            &mut tid_names, &mut needs_redraw, &mut screen_width,
            &mut screen_height, sys_sub,
        );

        // Tick animations
        let now = anyos_std::sys::uptime();
        let hz = anyos_std::sys::tick_hz().max(1);
        anims.remove_done(now);
        if anims.has_active(now) {
            needs_redraw = true;
        }

        // Clean up finished bounces (>2 seconds)
        bounce_items.retain(|(_, start)| {
            let elapsed_ms = now.wrapping_sub(*start) * 1000 / hz;
            elapsed_ms < 2000
        });
        if !bounce_items.is_empty() {
            needs_redraw = true;
        }

        if needs_redraw {
            let rs = RenderState {
                hover_idx: hovered_idx,
                anims: &anims,
                bounce_items: &bounce_items,
                now,
            };
            render_dock(&mut fb, &items, screen_width, &rs);
            blit_to_surface(&fb, &win);
            client.present(&win);
            needs_redraw = false;
        }

        let sleep_ms = if anims.has_active(now) || !bounce_items.is_empty() { 8 } else { 32 };
        process::sleep(sleep_ms);
    }
}

/// Handle compositor window events (mouse, theme, window open/close).
fn process_compositor_events(
    client: &CompositorClient,
    win: &mut libcompositor_client::WindowHandle,
    items: &mut Vec<DockItem>,
    hovered_idx: &mut Option<usize>,
    anims: &mut AnimSet,
    bounce_items: &mut Vec<(usize, u32)>,
    needs_redraw: &mut bool,
    tid_names: &mut Vec<(u32, String)>,
    screen_width: u32,
    own_tid: u32,
    has_gpu: bool,
) {
    while let Some(event) = client.poll_event(win) {
        match event.event_type {
            libcompositor_client::EVT_MOUSE_MOVE => {
                let lx = event.arg1 as i32;
                let ly = event.arg2 as i32;
                let new_hover = dock_hit_test(lx, ly, screen_width, items);
                if new_hover != *hovered_idx {
                    if has_gpu {
                        let now = anyos_std::sys::uptime();
                        if let Some(old) = *hovered_idx {
                            anims.start_at(100 + old as u32, 4000, 0, 200, Easing::EaseOut, now);
                        }
                        if let Some(new) = new_hover {
                            anims.start_at(100 + new as u32, 0, 4000, 200, Easing::EaseOut, now);
                        }
                    }
                    *hovered_idx = new_hover;
                    *needs_redraw = true;
                }
            }
            0x0050 => {
                // EVT_THEME_CHANGED
                *needs_redraw = true;
            }
            0x0060 => {
                // EVT_WINDOW_OPENED: create transient dock item for the app
                handle_window_opened(
                    items, tid_names, needs_redraw,
                    event.window_id, own_tid,
                );
            }
            0x0061 => {
                // EVT_WINDOW_CLOSED: remove transient dock item
                let exited_tid = event.window_id;
                let mut i = 0;
                while i < items.len() {
                    if items[i].tid == exited_tid && !items[i].pinned {
                        items.remove(i);
                        *needs_redraw = true;
                    } else {
                        i += 1;
                    }
                }
            }
            libcompositor_client::EVT_MOUSE_DOWN => {
                handle_dock_click(
                    client, items, bounce_items, needs_redraw,
                    event.arg1 as i32, event.arg2 as i32, screen_width, has_gpu,
                );
            }
            _ => {}
        }
    }
}

/// Handle a click on a dock item — launch or focus the app.
fn handle_dock_click(
    client: &CompositorClient,
    items: &mut Vec<DockItem>,
    bounce_items: &mut Vec<(usize, u32)>,
    needs_redraw: &mut bool,
    lx: i32,
    ly: i32,
    screen_width: u32,
    has_gpu: bool,
) {
    let idx = match dock_hit_test(lx, ly, screen_width, items) {
        Some(i) => i,
        None => return,
    };
    let item = match items.get_mut(idx) {
        Some(i) => i,
        None => return,
    };

    if item.tid != 0 {
        let status = process::try_waitpid(item.tid);
        if status == STILL_RUNNING {
            // App running — focus its window
            let cmd: [u32; 5] = [CMD_FOCUS_BY_TID, item.tid, 0, 0, 0];
            anyos_std::ipc::evt_chan_emit(client.channel_id, &cmd);
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
        if has_gpu {
            bounce_items.push((idx, anyos_std::sys::uptime()));
        }
        *needs_redraw = true;
    }
}

/// Handle EVT_WINDOW_OPENED: add transient dock item if appropriate.
fn handle_window_opened(
    items: &mut Vec<DockItem>,
    tid_names: &mut Vec<(u32, String)>,
    needs_redraw: &mut bool,
    app_tid: u32,
    own_tid: u32,
) {
    // Skip our own window and already-tracked items
    if app_tid == own_tid || items.iter().any(|it| it.tid == app_tid) {
        return;
    }

    let name = tid_names.iter()
        .find(|(t, _)| *t == app_tid)
        .map(|(_, n)| n.clone())
        .or_else(|| query_thread_name(app_tid))
        .unwrap_or_else(|| alloc::format!("app-{}", app_tid));

    if SYSTEM_NAMES.iter().any(|&s| s == name.as_str()) {
        return;
    }

    // Try /Applications/{Name}.app first, then /System/bin/{name}
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

    items.push(DockItem {
        name,
        bin_path,
        icon,
        running: true,
        tid: app_tid,
        pinned: false,
    });
    *needs_redraw = true;
}

/// Handle system events (process spawn/exit, resolution change).
fn process_system_events(
    client: &CompositorClient,
    win: &mut libcompositor_client::WindowHandle,
    items: &mut Vec<DockItem>,
    fb: &mut Framebuffer,
    bounce_items: &mut Vec<(usize, u32)>,
    tid_names: &mut Vec<(u32, String)>,
    needs_redraw: &mut bool,
    screen_width: &mut u32,
    screen_height: &mut u32,
    sys_sub: u32,
) {
    let mut sys_buf = [0u32; 5];
    while anyos_std::ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
        match sys_buf[0] {
            0x0020 => {
                // EVT_PROCESS_SPAWNED
                let spawned_tid = sys_buf[1];
                let name = unpack_event_name(sys_buf[2], sys_buf[3], sys_buf[4]);
                if name.is_empty() { continue; }

                // Cache TID→name for EVT_WINDOW_OPENED lookup
                if !tid_names.iter().any(|(t, _)| *t == spawned_tid) {
                    tid_names.push((spawned_tid, name.clone()));
                }

                // Skip system threads
                if SYSTEM_NAMES.iter().any(|&s| s == name.as_str()) {
                    continue;
                }

                // Match pinned items by binary basename
                for item in items.iter_mut() {
                    if item.pinned && item.tid == 0 {
                        let raw_basename = item.bin_path.rsplit('/').next().unwrap_or("");
                        let basename = raw_basename.strip_suffix(".app").unwrap_or(raw_basename);
                        if basename == name.as_str() {
                            item.tid = spawned_tid;
                            item.running = true;
                            *needs_redraw = true;
                            break;
                        }
                    }
                }
            }
            0x0040 => {
                // EVT_RESOLUTION_CHANGED
                let new_w = sys_buf[1];
                let new_h = sys_buf[2];
                if new_w != *screen_width || new_h != *screen_height {
                    *screen_width = new_w;
                    *screen_height = new_h;

                    if client.resize_window(win, *screen_width, DOCK_TOTAL_H) {
                        *fb = Framebuffer::new(*screen_width, DOCK_TOTAL_H);
                        client.move_window(win, 0, (*screen_height - DOCK_TOTAL_H) as i32);
                        // Reset bounces — positions invalid after resize
                        bounce_items.clear();
                        *needs_redraw = true;
                    }
                }
            }
            0x0021 => {
                // EVT_PROCESS_EXITED
                let exited_tid = sys_buf[1];
                for item in items.iter_mut() {
                    if item.tid == exited_tid && item.pinned {
                        item.running = false;
                        item.tid = 0;
                        *needs_redraw = true;
                    }
                }
                tid_names.retain(|(t, _)| *t != exited_tid);
            }
            _ => {}
        }
    }
}
