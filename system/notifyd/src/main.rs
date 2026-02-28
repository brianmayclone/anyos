//! notifyd — Notification daemon for anyOS.
//!
//! Subscribes to the compositor event channel and listens for `CMD_SHOW_NOTIFICATION`
//! events. Renders iOS-style notification banners in a borderless always-on-top window,
//! positioned top-right below the menubar (like macOS).
//!
//! Architecture: single anyui Canvas window. Banners slide in from the right.
//! The window is moved off-screen when no notifications are active.

#![no_std]
#![no_main]

use alloc::vec::Vec;

use anyos_std::println;

use libanyui_client as anyui;

anyos_std::entry!(main);

mod framebuffer;
mod render;

use framebuffer::Framebuffer;
use render::{BANNER_W, BANNER_H, STACK_GAP, MARGIN_TOP, MAX_VISIBLE, MARGIN_RIGHT};

// ── IPC Constants ───────────────────────────────────────────────────────────

/// CMD_SHOW_NOTIFICATION from compositor IPC protocol.
const CMD_SHOW_NOTIFICATION: u32 = 0x1020;
/// CMD_DISMISS_NOTIFICATION from compositor IPC protocol.
const CMD_DISMISS_NOTIFICATION: u32 = 0x1021;
/// EVT_RESOLUTION_CHANGED from system events.
const EVT_RESOLUTION_CHANGED: u32 = 0x0040;

// ── Timer Constants ─────────────────────────────────────────────────────────

/// Fast timer for animations (~60 Hz).
const TIMER_FAST_MS: u32 = 16;
/// Idle timer for polling (5 Hz).
const TIMER_IDLE_MS: u32 = 200;

// ── Animation Constants ─────────────────────────────────────────────────────

/// Duration of slide-in animation (milliseconds).
const SLIDE_IN_MS: u32 = 250;
/// Duration of slide-out animation (milliseconds).
const SLIDE_OUT_MS: u32 = 200;
/// Default auto-dismiss timeout (milliseconds).
const DEFAULT_TIMEOUT_MS: u32 = 5000;

// ── Window Size ─────────────────────────────────────────────────────────────

/// Window width: banner + right margin + extra space for slide-in origin.
const WIN_W: u32 = BANNER_W + MARGIN_RIGHT;
/// Window height: enough for MAX_VISIBLE stacked banners.
const WIN_H: u32 = MARGIN_TOP + (BANNER_H + STACK_GAP) * MAX_VISIBLE as u32 + STACK_GAP;

/// Menubar height (logical pixels). Must match compositor's menubar.
const MENUBAR_H: i32 = 25;
/// Gap between menubar bottom and first notification.
const MENUBAR_GAP: i32 = 4;

// ── Notification Data ───────────────────────────────────────────────────────

/// A queued notification with its display state.
pub struct Notification {
    /// Unique notification ID.
    pub id: u32,
    /// Title text (max 64 bytes).
    pub title: [u8; 64],
    pub title_len: usize,
    /// Message text (max 128 bytes).
    pub msg: [u8; 128],
    pub msg_len: usize,
    /// Optional 16×16 ARGB icon.
    pub icon: Option<[u32; 256]>,
    /// Auto-dismiss timeout in ticks (from sys::uptime).
    pub dismiss_at: u32,
    /// Current X offset in the canvas (animated, 0 = fully visible, BANNER_W = off-screen right).
    pub x_offset: i32,
    /// Target X offset (0 when visible, BANNER_W when sliding out).
    pub target_x: i32,
    /// Vertical stack position (slot index, computed directly).
    pub slot: usize,
    /// Animation start time (uptime ticks).
    pub anim_start: u32,
    /// Animation start X position.
    pub anim_start_x: i32,
    /// Animation duration in milliseconds.
    pub anim_duration_ms: u32,
    /// Whether this notification is currently visible.
    pub visible: bool,
    /// Whether this notification is sliding out (being dismissed).
    pub dismissing: bool,
    /// TID of the sender app.
    pub sender_tid: u32,
}

impl Notification {
    /// Y position in the canvas based on slot index.
    pub fn y_pos(&self) -> i32 {
        MARGIN_TOP as i32 + self.slot as i32 * (BANNER_H as i32 + STACK_GAP as i32)
    }
}

// ── App State ───────────────────────────────────────────────────────────────

struct NotifyApp {
    win: anyui::Window,
    canvas: anyui::Canvas,
    fb: Framebuffer,
    notifications: Vec<Notification>,
    next_id: u32,
    screen_width: u32,
    screen_height: u32,
    /// Compositor event channel and subscription.
    comp_chan: u32,
    comp_sub: u32,
    /// System event subscription.
    sys_sub: u32,
    /// Current timer ID.
    timer_id: u32,
    /// Whether we're using the fast timer.
    fast_timer: bool,
    /// Whether the window is currently on-screen.
    on_screen: bool,
    /// Whether a redraw is needed.
    needs_redraw: bool,
}

static mut APP: Option<NotifyApp> = None;
fn app() -> &'static mut NotifyApp { unsafe { APP.as_mut().unwrap() } }

// ── Entry Point ─────────────────────────────────────────────────────────────

fn main() {
    println!("notifyd: starting...");

    if !anyui::init() {
        println!("notifyd: FAILED to init anyui");
        return;
    }

    let (screen_width, screen_height) = anyui::screen_size();
    if screen_width == 0 || screen_height == 0 {
        println!("notifyd: FAILED — screen size is zero");
        return;
    }

    // Create borderless, always-on-top, non-resizable window
    let flags = anyui::WIN_FLAG_BORDERLESS
        | anyui::WIN_FLAG_NOT_RESIZABLE
        | anyui::WIN_FLAG_ALWAYS_ON_TOP;

    // Create at a positive off-screen position (negative coords become CW_USEDEFAULT
    // due to u16 packing in libcompositor). Then immediately move_to off-screen.
    let win = anyui::Window::new_with_flags(
        "notifyd",
        screen_width as i32 + 1000,
        screen_height as i32 + 1000,
        WIN_W, WIN_H, flags,
    );
    win.move_to(screen_width as i32 + 1000, screen_height as i32 + 1000);

    let canvas = anyui::Canvas::new(WIN_W, WIN_H);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_interactive(true);
    win.add(&canvas);

    // Subscribe to compositor channel to receive CMD_SHOW_NOTIFICATION events
    let comp_chan = anyos_std::ipc::evt_chan_create("compositor");
    let comp_sub = anyos_std::ipc::evt_chan_subscribe(comp_chan, 0);

    // Subscribe to system events (resolution changes)
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);

    let fb = Framebuffer::new(WIN_W, WIN_H);

    unsafe {
        APP = Some(NotifyApp {
            win,
            canvas,
            fb,
            notifications: Vec::with_capacity(8),
            next_id: 1,
            screen_width,
            screen_height,
            comp_chan,
            comp_sub,
            sys_sub,
            timer_id: 0,
            fast_timer: false,
            on_screen: false,
            needs_redraw: false,
        });
    }

    // Click-to-dismiss
    app().canvas.on_mouse_down(|_x, y, _button| {
        handle_click(y);
    });

    // Start with idle timer
    app().timer_id = anyui::set_timer(TIMER_IDLE_MS, || { tick(); });

    app().win.on_close(|_| {
        anyui::quit();
    });

    println!("notifyd: ready (screen {}x{})", screen_width, screen_height);
    anyui::run();
}

// ── Tick (Timer Callback) ───────────────────────────────────────────────────

fn tick() {
    let now = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz().max(1);

    // Poll events
    poll_compositor_channel();
    poll_system_events();

    // Update slide animations (horizontal)
    update_animations(now, hz);

    // Auto-dismiss expired notifications
    check_timeouts(now);

    // Remove fully dismissed notifications
    let a = app();
    a.notifications.retain(|n| n.visible || !n.dismissing);

    // Recompute slot positions after removals
    recompute_slots();

    // Move window on/off screen
    let a = app();
    let has_active = a.notifications.iter().any(|n| n.visible);
    if has_active && !a.on_screen {
        // Position: top-right, below menubar
        let wx = a.screen_width as i32 - WIN_W as i32;
        let wy = MENUBAR_H + MENUBAR_GAP;
        a.win.move_to(wx, wy);
        a.on_screen = true;
    } else if !has_active && a.on_screen {
        a.win.move_to(a.screen_width as i32 + 1000, a.screen_height as i32 + 1000);
        a.on_screen = false;
    }

    // Render if needed
    let a = app();
    if a.needs_redraw {
        render::render_all(&mut a.fb, &a.notifications);
        a.canvas.copy_pixels_from(&a.fb.pixels);
        a.needs_redraw = false;
    }

    // Adaptive timer
    let a = app();
    let needs_fast = a.notifications.iter().any(|n| {
        n.visible && (n.x_offset != n.target_x || n.dismissing)
    });

    if needs_fast && !a.fast_timer {
        anyui::kill_timer(a.timer_id);
        a.timer_id = anyui::set_timer(TIMER_FAST_MS, || { tick(); });
        a.fast_timer = true;
    } else if !needs_fast && a.fast_timer {
        anyui::kill_timer(a.timer_id);
        a.timer_id = anyui::set_timer(TIMER_IDLE_MS, || { tick(); });
        a.fast_timer = false;
    }
}

// ── Event Polling ───────────────────────────────────────────────────────────

/// Poll the compositor event channel for CMD_SHOW_NOTIFICATION events.
fn poll_compositor_channel() {
    let a = app();
    let chan = a.comp_chan;
    let sub = a.comp_sub;

    let mut buf = [0u32; 5];
    while anyos_std::ipc::evt_chan_poll(chan, sub, &mut buf) {
        match buf[0] {
            CMD_SHOW_NOTIFICATION => {
                let sender_tid = buf[1];
                let shm_id = buf[2];
                let timeout_ms = buf[3];
                handle_show_notification(sender_tid, shm_id, timeout_ms);
            }
            CMD_DISMISS_NOTIFICATION => {
                let notif_id = buf[1];
                dismiss_notification(notif_id);
            }
            _ => {}
        }
    }
}

/// Poll system events (resolution changes).
fn poll_system_events() {
    let a = app();
    let sub = a.sys_sub;

    let mut buf = [0u32; 5];
    while anyos_std::ipc::evt_sys_poll(sub, &mut buf) {
        if buf[0] == EVT_RESOLUTION_CHANGED {
            let new_w = buf[1];
            let new_h = buf[2];
            let a = app();
            if new_w != a.screen_width || new_h != a.screen_height {
                a.screen_width = new_w;
                a.screen_height = new_h;
                if a.on_screen {
                    let wx = new_w as i32 - WIN_W as i32;
                    let wy = MENUBAR_H + MENUBAR_GAP;
                    a.win.move_to(wx, wy);
                }
            }
        }
    }
}

// ── Notification Handling ───────────────────────────────────────────────────

/// Process a CMD_SHOW_NOTIFICATION event: map SHM, parse data, create notification.
fn handle_show_notification(sender_tid: u32, shm_id: u32, timeout_ms: u32) {
    if shm_id == 0 { return; }

    let shm_addr = anyos_std::ipc::shm_map(shm_id);
    if shm_addr == 0 { return; }

    // Parse SHM layout: [title_len:u16, msg_len:u16, has_icon:u8, pad:3, title..., msg..., icon...]
    let data = unsafe {
        core::slice::from_raw_parts(shm_addr as *const u8, 4096)
    };

    let title_len = (data[0] as u16 | ((data[1] as u16) << 8)).min(64) as usize;
    let msg_len = (data[2] as u16 | ((data[3] as u16) << 8)).min(128) as usize;
    let has_icon = data[4] != 0;

    let title_start = 8;
    let title_end = title_start + title_len;
    let msg_start = title_end;
    let msg_end = msg_start + msg_len;

    // Copy data into notification struct before unmapping
    let mut title = [0u8; 64];
    let tlen = title_len.min(64);
    if tlen > 0 && title_end <= 4096 {
        title[..tlen].copy_from_slice(&data[title_start..title_start + tlen]);
    }

    let mut msg = [0u8; 128];
    let mlen = msg_len.min(128);
    if mlen > 0 && msg_end <= 4096 {
        msg[..mlen].copy_from_slice(&data[msg_start..msg_start + mlen]);
    }

    let icon = if has_icon {
        let icon_start = (msg_end + 3) & !3;
        if icon_start + 1024 <= 4096 {
            let icon_slice = unsafe {
                core::slice::from_raw_parts(
                    (shm_addr as usize + icon_start) as *const u32,
                    256,
                )
            };
            let mut icon_buf = [0u32; 256];
            icon_buf.copy_from_slice(icon_slice);
            Some(icon_buf)
        } else {
            None
        }
    } else {
        None
    };

    // Unmap SHM immediately — we copied everything we need
    anyos_std::ipc::shm_unmap(shm_id);

    // Validate
    if core::str::from_utf8(&title[..tlen]).is_err() || tlen == 0 {
        return;
    }

    let a = app();
    let id = a.next_id;
    a.next_id = a.next_id.wrapping_add(1);

    let now = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz().max(1);
    let timeout = if timeout_ms == 0 { DEFAULT_TIMEOUT_MS } else { timeout_ms };
    let timeout_ticks = timeout * hz / 1000;

    // Slot = number of currently active (non-dismissing) notifications
    let slot = a.notifications.iter().filter(|n| n.visible && !n.dismissing).count();

    a.notifications.push(Notification {
        id,
        title,
        title_len: tlen,
        msg,
        msg_len: mlen,
        icon,
        dismiss_at: now.wrapping_add(timeout_ticks),
        x_offset: BANNER_W as i32,  // start off-screen right
        target_x: 0,                 // slide to visible position
        slot,
        anim_start: now,
        anim_start_x: BANNER_W as i32,
        anim_duration_ms: SLIDE_IN_MS,
        visible: true,
        dismissing: false,
        sender_tid,
    });

    a.needs_redraw = true;

    // Switch to fast timer for animation
    if !a.fast_timer {
        anyui::kill_timer(a.timer_id);
        a.timer_id = anyui::set_timer(TIMER_FAST_MS, || { tick(); });
        a.fast_timer = true;
    }

    println!("notifyd: show notification #{} from tid={}", id, sender_tid);
}

/// Start dismissing a notification by ID (slide out to the right).
fn dismiss_notification(notif_id: u32) {
    let a = app();
    let now = anyos_std::sys::uptime();

    if let Some(notif) = a.notifications.iter_mut().find(|n| n.id == notif_id && n.visible) {
        notif.dismissing = true;
        notif.anim_start = now;
        notif.anim_start_x = notif.x_offset;
        notif.target_x = BANNER_W as i32; // slide out to the right
        notif.anim_duration_ms = SLIDE_OUT_MS;
        a.needs_redraw = true;
    }
}

// ── Animation ───────────────────────────────────────────────────────────────

/// Update horizontal slide animations for all active notifications.
fn update_animations(now: u32, hz: u32) {
    let a = app();

    for notif in a.notifications.iter_mut() {
        if !notif.visible { continue; }
        if notif.x_offset == notif.target_x { continue; }

        let elapsed_ticks = now.wrapping_sub(notif.anim_start);
        let elapsed_ms = elapsed_ticks * 1000 / hz;

        if elapsed_ms >= notif.anim_duration_ms {
            notif.x_offset = notif.target_x;
            if notif.dismissing {
                notif.visible = false;
            }
        } else {
            let t = (elapsed_ms * 1000 / notif.anim_duration_ms) as i32;

            // EaseOut for slide-in, EaseIn for slide-out
            let eased = if notif.dismissing {
                // EaseIn: t^2
                t * t / 1000
            } else {
                // EaseOut: t * (2 - t)
                t * (2000 - t) / 1000
            };

            let diff = notif.target_x - notif.anim_start_x;
            notif.x_offset = notif.anim_start_x + diff * eased / 1000;
        }
        a.needs_redraw = true;
    }
}

/// Check for expired notifications and start dismissing them.
fn check_timeouts(now: u32) {
    let a = app();

    let mut to_dismiss = [0u32; 8];
    let mut count = 0;

    for notif in a.notifications.iter() {
        if notif.visible && !notif.dismissing && now.wrapping_sub(notif.dismiss_at) < 0x8000_0000 {
            if count < 8 {
                to_dismiss[count] = notif.id;
                count += 1;
            }
        }
    }

    for i in 0..count {
        dismiss_notification(to_dismiss[i]);
    }
}

/// Recompute slot (vertical position) for all visible, non-dismissing notifications.
fn recompute_slots() {
    let a = app();
    let mut slot = 0usize;

    for notif in a.notifications.iter_mut() {
        if !notif.visible || notif.dismissing { continue; }

        if notif.slot != slot {
            notif.slot = slot;
            a.needs_redraw = true;
        }
        slot += 1;
    }
}

// ── Click Handling ──────────────────────────────────────────────────────────

/// Handle a click — dismiss the clicked banner (identified by Y position).
fn handle_click(y: i32) {
    let a = app();

    let mut clicked_id = None;
    for notif in a.notifications.iter() {
        if !notif.visible || notif.dismissing { continue; }
        let ny = notif.y_pos();
        if y >= ny && y < ny + BANNER_H as i32 {
            clicked_id = Some(notif.id);
            break;
        }
    }

    if let Some(id) = clicked_id {
        dismiss_notification(id);
    }
}
