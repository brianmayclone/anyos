//! Management thread event loop.

use anyos_std::ipc;
use anyos_std::println;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::Vec;

use crate::config;
use crate::render::{acquire_lock, desktop_ref, release_lock, signal_render};

use super::ipc::handle_ipc_commands;
use super::session::perform_logout;
use super::shutdown::perform_shutdown;
use super::system_events::handle_system_events;

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

    loop {
        let now_ms = sys::uptime_ms();
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

        if boot_redraw_idx < boot_redraw_schedule.len() {
            let elapsed = now_ms.wrapping_sub(boot_start_ms);
            if elapsed >= boot_redraw_schedule[boot_redraw_idx] {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                desktop.compositor.damage_all();
                release_lock();
                signal_render();
                println!("compositor: boot watchdog redraw #{} ({}ms)", boot_redraw_idx + 1, elapsed);
                boot_redraw_idx += 1;
            }
        }

        let timeout = if *init_pending || *login_pending { 100 } else { 5000 };
        ipc::evt_chan_wait(compositor_channel, compositor_sub, timeout);

        // Check if init waiter thread signaled completion
        if *init_pending && super::bootstrap::is_init_done() {
            let code = super::bootstrap::init_exit_code();
            println!("compositor: init completed (exit={})", code);
            *init_pending = false;

            // Now spawn the login screen
            let new_login = process::spawn("/System/login", "");
            if new_login != u32::MAX {
                *login_tid = new_login;
                *login_pending = true;
                println!("compositor: login spawned (TID={}), waiting for authentication...", new_login);
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
                        println!("compositor: login crashed (exit={}), attempt {}/{}", status, login_retries, MAX_LOGIN_RETRIES);
                    } else {
                        println!("compositor: login exited without authentication, attempt {}/{}", login_retries, MAX_LOGIN_RETRIES);
                    }

                    if login_retries >= MAX_LOGIN_RETRIES {
                        println!("compositor: FATAL — login failed {} times, giving up", MAX_LOGIN_RETRIES);
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
            crate::compositor::Compositor::submit_cmds(cursor_cmds);
            signal_render();
        }

        let had_ipc = handle_ipc_commands(compositor_channel, compositor_sub, &mut ipc_buf);
        let had_sys = handle_system_events(compositor_channel, sys_sub);

        if had_ipc {
            mgmt_ipc += 1;
        }
        if had_sys {
            mgmt_sys += 1;
        }
        if event_count == 0 && !had_ipc && !had_sys {
            mgmt_idle += 1;
        }

        if had_ipc || had_sys {
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

        let had_work = event_count > 0 || had_ipc || had_sys;
        if had_work {
            let ipc_events = {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                let events = desktop.drain_ipc_events();
                release_lock();
                events
            };
            for (target_sub, evt) in &ipc_events {
                if let Some(sub_id) = target_sub {
                    ipc::evt_chan_emit_to(compositor_channel, *sub_id, evt);
                } else {
                    ipc::evt_chan_emit(compositor_channel, evt);
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
                perform_logout(
                    login_tid,
                    login_pending,
                    dock_spawned,
                    service_tids,
                );
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
