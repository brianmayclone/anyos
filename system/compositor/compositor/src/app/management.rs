//! Management thread event loop.

use anyos_std::ipc;
use anyos_std::println;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::config;
use crate::render::{acquire_lock, desktop_ref, release_lock, signal_render};
use libconf::{ConfClient, ConfValue, RegistryScope};

use super::ipc::{emit_to_registered_apps, emit_to_target, handle_ipc_commands};
use super::session::perform_logout;
use super::shutdown::perform_shutdown;
use super::system_events::handle_system_events;

const SERVICE_STATE_IDLE: u32 = 0;
const SERVICE_STATE_STARTING: u32 = 1;
const SERVICE_STATE_DONE: u32 = 2;
const SERVICE_STATE_FAILED: u32 = 3;

static SESSIONHOST_STATE: AtomicU32 = AtomicU32::new(SERVICE_STATE_IDLE);
static SESSIONHOST_TID: AtomicU32 = AtomicU32::new(u32::MAX);

fn sessionhost_spawner_entry() {
    println!("compositor: starting required service '/System/Sessionhost'...");
    let tid = process::spawn("/System/Sessionhost", "");
    if tid != 0 && tid != u32::MAX {
        let _ = process::detach(tid);
        SESSIONHOST_TID.store(tid, Ordering::Release);
        SESSIONHOST_STATE.store(SERVICE_STATE_DONE, Ordering::Release);
        println!(
            "compositor: required service launched '/System/Sessionhost' (TID={})",
            tid
        );
    } else {
        SESSIONHOST_TID.store(u32::MAX, Ordering::Release);
        SESSIONHOST_STATE.store(SERVICE_STATE_FAILED, Ordering::Release);
        println!("compositor: WARNING — required service failed '/System/Sessionhost'");
    }
}

fn launch_sessionhost_async() {
    if SESSIONHOST_STATE
        .compare_exchange(
            SERVICE_STATE_IDLE,
            SERVICE_STATE_STARTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    let stack_size: usize = 32 * 1024;
    let stack_vec = alloc::vec![0u8; stack_size];
    let stack_base = stack_vec.as_ptr() as usize;
    core::mem::forget(stack_vec);
    #[cfg(target_arch = "x86_64")]
    let stack_top = ((stack_base + stack_size) & !0xF) - 8;
    #[cfg(target_arch = "aarch64")]
    let stack_top = (stack_base + stack_size) & !0xF;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        *(stack_top as *mut usize) = process::thread_exit_stub_addr();
    }

    let tid = process::thread_create_with_priority(
        sessionhost_spawner_entry,
        stack_top,
        "compositor/sessionhost-spawn",
        80,
    );
    if tid == 0 {
        SESSIONHOST_STATE.store(SERVICE_STATE_FAILED, Ordering::Release);
        println!("compositor: WARNING — could not create Sessionhost spawner thread");
    }
}

fn collect_sessionhost_tid(service_tids: &mut Vec<u32>) {
    if SESSIONHOST_STATE.load(Ordering::Acquire) != SERVICE_STATE_DONE {
        return;
    }
    let tid = SESSIONHOST_TID.swap(u32::MAX, Ordering::AcqRel);
    if tid != u32::MAX && !service_tids.contains(&tid) {
        service_tids.push(tid);
    }
}

fn reset_sessionhost_tracking_after_logout() {
    if SESSIONHOST_STATE.load(Ordering::Acquire) == SERVICE_STATE_STARTING {
        return;
    }
    SESSIONHOST_TID.store(u32::MAX, Ordering::Release);
    SESSIONHOST_STATE.store(SERVICE_STATE_IDLE, Ordering::Release);
}

pub(crate) fn management_loop(
    compositor_channel: u32,
    compositor_sub: u32,
    sys_sub: u32,
    init_pending: &mut bool,
    login_tid: &mut u32,
    login_pending: &mut bool,
    dock_spawned: &mut bool,
    service_tids: &mut Vec<u32>,
) {
    let mut events_buf = [[0u32; 5]; 256];
    let mut ipc_buf = [0u32; 5];
    let mut login_retries: u32 = 0;
    let mut login_failed = false;
    const MAX_LOGIN_RETRIES: u32 = 10;

    let boot_start_ms: u32 = sys::uptime_ms();
    let boot_redraw_schedule: [u32; 6] = [100, 250, 500, 1000, 2000, 4000];
    let mut boot_redraw_idx: usize = 0;

    let mut mgmt_loops: u32 = 0;
    let mut mgmt_input: u32 = 0;
    let mut mgmt_ipc: u32 = 0;
    let mut mgmt_sys: u32 = 0;
    let mut mgmt_idle: u32 = 0;
    let mut mgmt_last_report: u32 = sys::uptime_ms();
    let mut last_reap_ms: u32 = 0;
    const REAP_INTERVAL_MS: u32 = 250;
    let mut last_display_poll_ms: u32 = 0;
    const DISPLAY_POLL_INTERVAL_MS: u32 = 500;
    let mut last_input_cfg_poll_ms: u32 = 0;
    const INPUT_CFG_POLL_INTERVAL_MS: u32 = 500;
    let mut last_cursor_report_ms: u32 = sys::uptime_ms();
    const CURSOR_REPORT_INTERVAL_MS: u32 = 15_000;
    const BOOT_EVENT_WAIT_MS: u32 = 50;
    const IDLE_INPUT_WAIT_MS: u32 = 16;
    const ACTIVE_INPUT_WAIT_MS: u32 = 8;
    const INPUT_WAIT_STALL_LOG_MS: u32 = 75;
    const INPUT_PROCESS_STALL_LOG_MS: u32 = 50;
    const INPUT_SUBMIT_STALL_LOG_MS: u32 = 50;

    loop {
        let now_ms = sys::uptime_ms();
        collect_sessionhost_tid(service_tids);
        if now_ms.wrapping_sub(mgmt_last_report) >= 30000 {
            println!(
                "MGMT-STATS: loops={} input={} ipc={} sys={} idle={}",
                mgmt_loops, mgmt_input, mgmt_ipc, mgmt_sys, mgmt_idle
            );
            mgmt_loops = 0;
            mgmt_input = 0;
            mgmt_ipc = 0;
            mgmt_sys = 0;
            mgmt_idle = 0;
            mgmt_last_report = now_ms;
        }
        mgmt_loops += 1;

        // Periodic cursor heartbeat: where is the mouse and which
        // monitor does the compositor currently believe owns it?
        // Fires every 15 s so the serial log carries a baseline trace
        // alongside the per-event [cursor] output transitions.
        if now_ms.wrapping_sub(last_cursor_report_ms) >= CURSOR_REPORT_INTERVAL_MS {
            last_cursor_report_ms = now_ms;
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let mx = desktop.mouse_x;
            let my = desktop.mouse_y;
            let outputs_len = desktop.compositor.outputs.len();
            let mut hit: Option<(u32, i32, i32, u32, u32)> = None;
            for o in desktop.compositor.outputs.iter() {
                if mx >= o.virtual_x
                    && my >= o.virtual_y
                    && mx < o.virtual_x + o.fb_width as i32
                    && my < o.virtual_y + o.fb_height as i32
                {
                    hit = Some((
                        o.id,
                        mx - o.virtual_x,
                        my - o.virtual_y,
                        o.fb_width,
                        o.fb_height,
                    ));
                    break;
                }
            }
            release_lock();
            match hit {
                Some((id, lx, ly, w, h)) => println!(
                    "[cursor-hb] virt=({},{}) on M{} ({}x{}) local=({},{}) outputs={}",
                    mx, my, id, w, h, lx, ly, outputs_len
                ),
                None => println!(
                    "[cursor-hb] virt=({},{}) off-screen (no output contains it) outputs={}",
                    mx, my, outputs_len
                ),
            }
        }

        // Poll the kernel for display events (hot-plug, layout
        // changes from displayd's profile auto-switch). At 500ms
        // cadence — fast enough that the user feels the layout
        // settle when they plug a monitor, slow enough that idle
        // CPU is essentially zero. SYS_DISPLAY_POLL_EVENT only
        // works on the registered compositor (us).
        if now_ms.wrapping_sub(last_display_poll_ms) >= DISPLAY_POLL_INTERVAL_MS {
            last_display_poll_ms = now_ms;
            loop {
                let ev = anyos_std::display::poll_event();
                match ev {
                    anyos_std::display::DisplayEvent::None => break,
                    anyos_std::display::DisplayEvent::HotplugChanged
                    | anyos_std::display::DisplayEvent::LayoutApplied
                    | anyos_std::display::DisplayEvent::PreferredModeChanged { .. } => {
                        // displayd's set_layout path can change the
                        // primary output's mode (a per-output Resolution
                        // change in Settings) but does NOT fire
                        // EVT_RESOLUTION_CHANGED, so handle_resolution_change
                        // (the path that actually calls resize_fb) is
                        // never invoked along this route. Bridge the two:
                        // re-query the kernel framebuffer here, and if the
                        // primary's size changed since the last compose,
                        // route through handle_resolution_change before
                        // refreshing per-output state. Without this the
                        // back buffer stride goes out of sync with the
                        // hardware framebuffer pitch on every Settings →
                        // Display → Resolution change, producing the
                        // tiled / striped artifacts users see.
                        if let Some(fb) = anyos_std::ipc::map_framebuffer() {
                            acquire_lock();
                            let desktop = unsafe { desktop_ref() };
                            if fb.width != desktop.screen_width
                                || fb.height != desktop.screen_height
                            {
                                desktop.handle_resolution_change(fb.width, fb.height);
                            }
                            release_lock();
                        }
                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        desktop.refresh_outputs_and_reflow();
                        desktop.compositor.damage_all();
                        release_lock();
                        signal_render();
                    }
                }
            }
        }

        // NOTE: Earlier we re-read the input prefs from confd here every
        // 500 ms so live changes from the settings app would propagate
        // without a compositor restart. That turned out to be very
        // expensive in practice — `read_i64` on a key that confd hasn't
        // answered yet blocks for `SINGLE_LINE_TIMEOUT_MS` (5 s) per
        // call, and this poll fires on the same management thread that
        // dispatches `process_input`. With confd briefly unreachable
        // (e.g. during first boot, before the daemon has loaded its DB)
        // every tick blocked the input pipeline for tens of seconds —
        // visible as cursor "spotting" instead of a continuous trail.
        //
        // The initial values are loaded once in `bootstrap.rs` after
        // confd's brief readiness window, which is sufficient until we
        // wire a confd-watch / push-notification path. Settings-app
        // changes currently require a compositor restart; that will be
        // fixed when the watch path is in.
        let _ = last_input_cfg_poll_ms;
        let _ = INPUT_CFG_POLL_INTERVAL_MS;

        if boot_redraw_idx < boot_redraw_schedule.len() {
            let elapsed = now_ms.wrapping_sub(boot_start_ms);
            if elapsed >= boot_redraw_schedule[boot_redraw_idx] {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                desktop.compositor.damage_all();
                release_lock();
                signal_render();
                println!(
                    "compositor: boot watchdog redraw #{} ({}ms)",
                    boot_redraw_idx + 1,
                    elapsed
                );
                boot_redraw_idx += 1;
            }
        }

        let timeout = if *init_pending || *login_pending {
            BOOT_EVENT_WAIT_MS
        } else {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let pointer_active = desktop.dragging.is_some()
                || desktop.resizing.is_some()
                || desktop.global_drag.is_some()
                || desktop.mouse_buttons != 0;
            release_lock();
            if pointer_active {
                ACTIVE_INPUT_WAIT_MS
            } else {
                IDLE_INPUT_WAIT_MS
            }
        };
        let wait_start_ms = sys::uptime_ms();
        ipc::evt_chan_wait(compositor_channel, compositor_sub, timeout);
        let wait_elapsed_ms = sys::uptime_ms().wrapping_sub(wait_start_ms);

        // Check whether init has exited.  The management thread performs the
        // reap itself so the subsequent login spawn happens from the same
        // thread that owns the boot state transition.
        if *init_pending && super::bootstrap::poll_init_done() {
            let code = super::bootstrap::init_exit_code();
            println!("compositor: init completed (exit={})", code);
            *init_pending = false;

            // Now spawn the login screen
            println!("compositor: spawning login...");
            let new_login = process::spawn("/System/login", "");
            if new_login != u32::MAX {
                *login_tid = new_login;
                *login_pending = true;
                println!(
                    "compositor: login spawned (TID={}), waiting for authentication...",
                    new_login
                );
            } else {
                println!("compositor: WARNING — /System/login could not be spawned");
            }
        }

        if *login_pending {
            let status = process::try_waitpid(*login_tid);
            if status != process::STILL_RUNNING {
                let is_crash = status > 128 && status < 256;
                let is_cancelled = status == u32::MAX;

                if is_crash || is_cancelled {
                    login_retries += 1;
                    if is_crash {
                        println!(
                            "compositor: login crashed (exit={}), attempt {}/{}",
                            status, login_retries, MAX_LOGIN_RETRIES
                        );
                    } else {
                        println!(
                            "compositor: login exited without authentication, attempt {}/{}",
                            login_retries, MAX_LOGIN_RETRIES
                        );
                    }

                    if login_retries >= MAX_LOGIN_RETRIES {
                        println!(
                            "compositor: FATAL — login failed {} times, giving up",
                            MAX_LOGIN_RETRIES
                        );
                        *login_pending = false;
                        login_failed = true;
                    } else {
                        let new_tid = process::spawn("/System/login", "");
                        if new_tid != u32::MAX {
                            *login_tid = new_tid;
                            println!("compositor: login re-spawned (TID={})", new_tid);
                        } else {
                            println!("compositor: FATAL — cannot re-spawn login");
                            *login_pending = false;
                            login_failed = true;
                        }
                    }
                } else {
                    *login_pending = false;
                    let exit_uid = status;
                    println!("compositor: authentication complete, uid={}", exit_uid);

                    if exit_uid != 0 {
                        process::set_identity(exit_uid as u16);
                        println!("compositor: identity switched to uid={}", exit_uid);
                    }

                    let uid = process::getuid();
                    let mut name_buf = [0u8; 32];
                    let nlen = process::getusername(uid, &mut name_buf);
                    if nlen != u32::MAX && nlen > 0 {
                        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
                            persist_last_login_user(username);
                            anyos_std::env::set("USER", username);
                            let home = alloc::format!("/Users/{}", username);
                            anyos_std::env::set("HOME", &home);
                        }
                    }
                }
            }
        }

        if !*init_pending && !*login_pending && !*dock_spawned && !login_failed {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.on_process_exit(*login_tid);
            desktop.set_menubar_visible(true);
            desktop.compositor.damage_all();
            release_lock();
            signal_render();

            launch_sessionhost_async();

            println!("compositor: spawning dock...");
            let dock_tid = process::spawn("/System/compositor/dock", "");
            if dock_tid != u32::MAX {
                service_tids.push(dock_tid);
            }
            println!("compositor: dock spawned");
            service_tids.extend(config::launch_autostart());
            *dock_spawned = true;
        }

        let event_count = ipc::input_poll(&mut events_buf) as usize;
        if event_count > 0 {
            mgmt_input += 1;
            if wait_elapsed_ms >= INPUT_WAIT_STALL_LOG_MS {
                println!(
                    "[compositor/input] wait {} ms before {} events timeout={}ms",
                    wait_elapsed_ms, event_count, timeout
                );
            }
            let process_start_ms = sys::uptime_ms();
            let cursor_cmds = {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                desktop.process_input(&events_buf, event_count);
                desktop.damage_cursor();
                desktop.compositor.prepare_flush();
                let cmds = desktop.compositor.drain_gpu_cmds();
                release_lock();
                cmds
            };
            let process_elapsed_ms = sys::uptime_ms().wrapping_sub(process_start_ms);
            if process_elapsed_ms >= INPUT_PROCESS_STALL_LOG_MS {
                println!(
                    "[compositor/input] process {} events took {} ms",
                    event_count, process_elapsed_ms
                );
            }
            let submit_start_ms = sys::uptime_ms();
            crate::compositor::Compositor::submit_cmds(cursor_cmds);
            let submit_elapsed_ms = sys::uptime_ms().wrapping_sub(submit_start_ms);
            if submit_elapsed_ms >= INPUT_SUBMIT_STALL_LOG_MS {
                println!(
                    "[compositor/input] submit after {} events took {} ms",
                    event_count, submit_elapsed_ms
                );
            }
            signal_render();
        }

        let had_ipc = handle_ipc_commands(compositor_channel, compositor_sub, &mut ipc_buf);
        let had_sys = handle_system_events(compositor_channel, sys_sub);
        let should_reap = now_ms.wrapping_sub(last_reap_ms) >= REAP_INTERVAL_MS;
        let reaped_tids = if should_reap {
            last_reap_ms = now_ms;
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let exited = desktop.reap_exited_processes();
            release_lock();
            exited
        } else {
            Vec::new()
        };

        if had_ipc {
            mgmt_ipc += 1;
        }
        if had_sys {
            mgmt_sys += 1;
        }
        if !reaped_tids.is_empty() {
            mgmt_sys += 1;
            for tid in &reaped_tids {
                emit_to_registered_apps(&[crate::ipc_protocol::EVT_WINDOW_CLOSED, *tid, 0, 0, 0]);
            }
        }
        if event_count == 0 && !had_ipc && !had_sys && reaped_tids.is_empty() {
            mgmt_idle += 1;
        }

        if had_ipc || had_sys || !reaped_tids.is_empty() {
            signal_render();
        }

        {
            let wallpaper_pending = {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                let p = desktop.wallpaper_pending;
                release_lock();
                p
            };
            if wallpaper_pending {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                desktop.process_deferred_wallpaper();
                release_lock();
                signal_render();
            }
        }

        let had_work = event_count > 0 || had_ipc || had_sys || !reaped_tids.is_empty();
        if had_work {
            let ipc_events = {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                let events = desktop.drain_ipc_events();
                release_lock();
                events
            };
            for (target_sub, evt) in &ipc_events {
                if let Some(target) = target_sub {
                    emit_to_target(*target, evt);
                }
            }
        }

        {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let logout = desktop.logout_requested;
            if logout {
                desktop.logout_requested = false;
            }
            release_lock();

            if logout {
                perform_logout(login_tid, login_pending, dock_spawned, service_tids);
                reset_sessionhost_tracking_after_logout();
            }
        }

        {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let mode = desktop.shutdown_mode;
            if mode != 0 {
                desktop.shutdown_mode = 0;
            }
            release_lock();

            if mode != 0 {
                perform_shutdown(mode, service_tids);
            }
        }
    }
}

fn persist_last_login_user(username: &str) {
    if username.is_empty() {
        println!("compositor: persist_last_login_user skipped (empty username)");
        return;
    }
    if let Ok(mut client) = ConfClient::connect("compositor") {
        match client.set(
            RegistryScope::System,
            "system/login/state/last_user",
            ConfValue::String(alloc::string::String::from(username)),
        ) {
            Ok(_) => println!("compositor: persisted last_user='{}' via confd", username),
            Err(err) => println!(
                "compositor: FAILED to persist last_user='{}' via confd: {:?}",
                username, err
            ),
        }
    } else {
        println!(
            "compositor: FAILED to connect to confd for last_user='{}'",
            username
        );
    }
}
