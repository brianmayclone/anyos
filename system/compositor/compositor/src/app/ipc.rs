//! Compositor IPC command handling.

use anyos_std::ipc;
use anyos_std::Vec;

use crate::config;
use crate::desktop::{self, AppIpcTarget};
use crate::ipc_protocol;
use crate::render::{acquire_lock, desktop_ref, release_lock, signal_render};

pub(crate) fn emit_to_target(target: AppIpcTarget, event: &[u32; 5]) {
    ipc::evt_chan_emit_to(target.channel_id, target.sub_id, event);
}

pub(crate) fn emit_to_registered_apps(event: &[u32; 5]) {
    let targets = {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        let mut out = Vec::with_capacity(desktop.app_subs.len());
        for &(_, target) in &desktop.app_subs {
            out.push(target);
        }
        release_lock();
        out
    };
    for target in targets {
        emit_to_target(target, event);
    }
}

pub(crate) fn handle_ipc_commands(
    compositor_channel: u32,
    compositor_sub: u32,
    ipc_buf: &mut [u32; 5],
) -> bool {
    let mut cmds = [[0u32; 5]; 64];
    let mut cmd_count = 0usize;
    for i in 0..cmds.len() {
        if !ipc::evt_chan_poll(compositor_channel, compositor_sub, ipc_buf) {
            break;
        }
        cmds[i] = *ipc_buf;
        cmd_count += 1;
    }
    if cmd_count == 0 {
        return false;
    }

    let mut responses: Vec<(Option<AppIpcTarget>, [u32; 5])> = Vec::new();

    let mut i = 0;
    while i < cmd_count {
        let cmd = cmds[i];
        if cmd[0] < 0x1000 || cmd[0] >= 0x2000 {
            i += 1;
            continue;
        }

        match cmd[0] {
            ipc_protocol::CMD_CREATE_WINDOW => {
                let app_tid = cmd[1];
                let wh = cmd[2];
                let width = wh >> 16;
                let height = wh & 0xFFFF;
                let xy = cmd[3];
                let raw_x = (xy >> 16) as u16;
                let raw_y = (xy & 0xFFFF) as u16;
                let shm_id_and_flags = cmd[4];
                let shm_id = shm_id_and_flags >> 16;
                let flags = shm_id_and_flags & 0xFFFF;

                let valid = {
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    let ok = shm_id != 0 && desktop.validate_window_surface(width, height, flags);
                    release_lock();
                    ok
                };

                if valid {
                    let shm_addr = ipc::shm_map(shm_id);
                    if shm_addr != 0 {
                        let borderless = flags & desktop::WIN_FLAG_BORDERLESS != 0;
                        let full_h = if borderless {
                            height
                        } else {
                            height + desktop::title_bar_height()
                        };

                        let mut pre_pixels = alloc::vec![0u32; (width * full_h) as usize];

                        if !borderless {
                            desktop::pre_render_chrome_ex(
                                &mut pre_pixels,
                                width,
                                full_h,
                                "Window",
                                true,
                                flags,
                            );
                            crate::desktop::window::copy_shm_to_pixels(
                                &mut pre_pixels,
                                width,
                                desktop::title_bar_height(),
                                shm_addr as *const u32,
                                width,
                                height,
                            );
                        }

                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        let win_id = desktop.create_ipc_window_fast(
                            app_tid,
                            width,
                            height,
                            flags,
                            shm_id,
                            shm_addr as *mut u32,
                            pre_pixels,
                            raw_x,
                            raw_y,
                        );
                        let target = desktop.get_ipc_target_for_tid(app_tid);
                        release_lock();

                        responses.push((
                            target,
                            [
                                ipc_protocol::RESP_WINDOW_CREATED,
                                win_id,
                                shm_id,
                                app_tid,
                                0,
                            ],
                        ));
                    }
                }
                i += 1;
            }
            ipc_protocol::CMD_RESIZE_SHM => {
                let new_shm_id = cmd[2];
                let shm_addr = if new_shm_id > 0 {
                    ipc::shm_map(new_shm_id)
                } else {
                    0
                };
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                if let Some(resp) = desktop.handle_resize_shm_pre_mapped(&cmd, shm_addr as usize) {
                    responses.push(resp);
                }
                release_lock();
                i += 1;
            }
            ipc_protocol::CMD_SET_FONT_SMOOTHING => {
                let new_mode = cmd[1].min(2);
                let old_mode = desktop::theme::read_font_smoothing();
                if new_mode != old_mode {
                    desktop::set_font_smoothing(new_mode);
                    config::save_font_smoothing(new_mode);
                    emit_to_registered_apps(&[
                        ipc_protocol::EVT_FONT_SMOOTHING_CHANGED,
                        new_mode,
                        0,
                        0,
                        0,
                    ]);
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    desktop.compositor.damage_all();
                    release_lock();
                    signal_render();
                }
                i += 1;
            }
            ipc_protocol::CMD_SET_THEME => {
                let new_theme = cmd[1].min(1);
                let old_theme = unsafe { core::ptr::read_volatile(0x0400_000C as *const u32) };
                if new_theme != old_theme {
                    desktop::set_theme(new_theme);
                    config::save_theme(if new_theme == 0 { "dark" } else { "light" }, "");
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    desktop.on_theme_change();
                    release_lock();
                    emit_to_registered_apps(&[
                        ipc_protocol::EVT_THEME_CHANGED,
                        new_theme,
                        old_theme,
                        0,
                        0,
                    ]);
                    signal_render();
                }
                i += 1;
            }
            ipc_protocol::CMD_SET_SCALE => {
                let new_scale = cmd[1];
                let old_scale = desktop::theme::read_scale_factor();
                if new_scale != old_scale && (100..=300).contains(&new_scale) {
                    desktop::theme::set_scale_factor(new_scale);
                    config::save_scale_factor(new_scale);
                    emit_to_registered_apps(&[
                        ipc_protocol::EVT_SCALE_CHANGED,
                        new_scale,
                        old_scale,
                        0,
                        0,
                    ]);
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    desktop.handle_scale_change();
                    release_lock();
                    signal_render();
                }
                i += 1;
            }
            ipc_protocol::CMD_RELOAD_SHORTCUTS => {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                desktop.shortcuts = config::read_shortcuts();
                release_lock();
                i += 1;
            }
            ipc_protocol::CMD_LIST_WINDOW_TIDS => {
                let requester_tid = cmd[1];
                let mut tids: Vec<u32> = Vec::new();
                {
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    for win in &desktop.windows {
                        if win.owner_tid != 0 && !tids.contains(&win.owner_tid) {
                            tids.push(win.owner_tid);
                        }
                    }
                    release_lock();
                }
                let target = {
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    let t = desktop.get_ipc_target_for_tid(requester_tid);
                    release_lock();
                    t
                };
                for &tid in &tids {
                    let entry = [ipc_protocol::EVT_WINDOW_LIST_ENTRY, tid, 0, 0, 0];
                    if let Some(target) = target {
                        emit_to_target(target, &entry);
                    }
                }
                let end = [
                    ipc_protocol::EVT_WINDOW_LIST_END,
                    tids.len() as u32,
                    0,
                    0,
                    0,
                ];
                if let Some(target) = target {
                    emit_to_target(target, &end);
                }
                i += 1;
            }
            _ => {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                while i < cmd_count {
                    let c = cmds[i];
                    if c[0] < 0x1000 || c[0] >= 0x2000 {
                        i += 1;
                        continue;
                    }
                    match c[0] {
                        ipc_protocol::CMD_CREATE_WINDOW
                        | ipc_protocol::CMD_RESIZE_SHM
                        | ipc_protocol::CMD_SET_THEME
                        | ipc_protocol::CMD_SET_FONT_SMOOTHING
                        | ipc_protocol::CMD_SET_SCALE => break,
                        _ => {}
                    }
                    if let Some(resp) = desktop.handle_ipc_command(&c) {
                        responses.push(resp);
                    }
                    i += 1;
                }
                let ipc_gpu_cmds = desktop.compositor.drain_gpu_cmds();
                release_lock();
                crate::compositor::Compositor::submit_cmds(ipc_gpu_cmds);
            }
        }
    }

    for (target_sub, response) in &responses {
        if let Some(target) = target_sub {
            emit_to_target(*target, response);
        }

        if response[0] == ipc_protocol::RESP_WINDOW_CREATED {
            emit_to_registered_apps(&[
                ipc_protocol::EVT_WINDOW_OPENED,
                response[3],
                response[1],
                0,
                0,
            ]);
        } else if response[0] == ipc_protocol::RESP_WINDOW_DESTROYED {
            let app_tid = response[2];
            let remaining_windows = response[3];
            if remaining_windows == 0 {
                emit_to_registered_apps(&[ipc_protocol::EVT_WINDOW_CLOSED, app_tid, 0, 0, 0]);
            }
        }
    }
    true
}
