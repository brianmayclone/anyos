#![no_std]
#![no_main]

use anyos_std::ipc;
use anyos_std::process;
use anyos_std::ui::window;

anyos_std::entry!(main);

#[path = "ipc.rs"]
mod shell_ipc;

fn main() {
    // Ensure compositor connection is initialized (needed to get channel IDs)
    let _ = window::screen_size();

    // Get compositor channel for receiving focus-change broadcasts
    let (comp_chan, comp_sub) = window::compositor_channel();

    // Subscribe to system events (process spawn/exit)
    let sys_sub = ipc::evt_sys_subscribe(0);

    // Create our "shell" channel — apps send menu registrations here
    let shell_chan = ipc::evt_chan_create("shell");
    let shell_sub = ipc::evt_chan_subscribe(shell_chan, 0);

    loop {
        let mut evt = [0u32; 5];

        // Poll shell channel (menu registrations from apps)
        if ipc::evt_chan_poll(shell_chan, shell_sub, &mut evt) {
            handle_shell_event(&evt);
            continue;
        }

        // Poll compositor events (focus changes)
        if comp_chan != 0 && ipc::evt_chan_poll(comp_chan, comp_sub, &mut evt) {
            handle_compositor_event(comp_chan, &evt);
            continue;
        }

        // Poll system events
        if ipc::evt_sys_poll(sys_sub, &mut evt) {
            // Could track running processes here
            continue;
        }

        process::sleep(50);
    }
}

fn handle_shell_event(_evt: &[u32; 5]) {
    match _evt[0] {
        shell_ipc::CMD_SET_MENU => {
            // evt[1] = sender tid, evt[2] = shm_id with MenuBarDef data
            // TODO: store menu data per-app, forward to compositor on focus
        }
        shell_ipc::CMD_UPDATE_MENU_ITEM => {
            // evt[1] = sender tid, evt[2] = item_id, evt[3] = new_flags
        }
        _ => {}
    }
}

fn handle_compositor_event(_comp_chan: u32, evt: &[u32; 5]) {
    match evt[0] {
        shell_ipc::EVT_FOCUS_CHANGED => {
            let _tid = evt[1];
            // TODO: look up stored menu data for _tid, forward to compositor
            // via CMD_SET_MENU so the compositor renders the right menus
        }
        _ => {}
    }
}
