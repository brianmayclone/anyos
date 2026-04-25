//! System event handling for the compositor.

use anyos_std::ipc;

use crate::config;
use crate::ipc_protocol;
use crate::render::{acquire_lock, desktop_ref, release_lock};

use super::ipc::emit_to_registered_apps;

pub(crate) fn handle_system_events(_compositor_channel: u32, sys_sub: u32) -> bool {
    let mut sys_buf = [0u32; 5];
    let mut had_work = false;
    while ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
        had_work = true;
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        if sys_buf[0] == 0x0021 {
            let exited_tid = sys_buf[1];
            desktop.on_process_exit(exited_tid);

            release_lock();
            emit_to_registered_apps(&[ipc_protocol::EVT_WINDOW_CLOSED, exited_tid, 0, 0, 0]);
        } else if sys_buf[0] == 0x0040 {
            let new_w = sys_buf[1];
            let new_h = sys_buf[2];
            desktop.handle_resolution_change(new_w, new_h);
            release_lock();
            config::save_resolution(new_w, new_h);
        } else {
            release_lock();
        }
    }
    had_work
}
