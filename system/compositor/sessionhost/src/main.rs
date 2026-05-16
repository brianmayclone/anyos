#![no_std]
#![no_main]

use anyos_std::ipc;
use anyos_std::process;

anyos_std::entry!(main);

mod crash;
mod launch;

/// IPC channel name — Dock/Finder send launch requests here.
const CHANNEL_NAME: &str = "sessionhost";

/// IPC commands
const CMD_LAUNCH_APP: u32 = 0x5001;
/// IPC responses
const EVT_LAUNCH_RESULT: u32 = 0x5002;

/// System event: process exited.
const EVT_PROCESS_EXITED: u32 = 0x0021;

fn main() {
    anyos_std::println!("[sessionhost] starting");

    // Register as the session host — enables .app spawning privilege
    if ipc::register_sessionhost() != 0 {
        anyos_std::println!("[sessionhost] FAILED — another sessionhost is already registered");
        return;
    }

    // System events (crash detection)
    let sys_sub = ipc::evt_sys_subscribe(0);

    // Launch request channel
    let chan = ipc::evt_chan_create(CHANNEL_NAME);
    let chan_sub = ipc::evt_chan_subscribe(chan, 0);

    anyos_std::println!("[sessionhost] channel '{}' ready", CHANNEL_NAME);

    loop {
        let mut evt = [0u32; 5];

        // Poll launch requests from Dock/Finder
        if ipc::evt_chan_poll(chan, chan_sub, &mut evt) {
            anyos_std::println!(
                "[sessionhost] evt={} req_sub={} shm_id={}",
                evt[0],
                evt[1],
                evt[2]
            );
            if evt[0] == CMD_LAUNCH_APP {
                // evt[1] = requester sub_id, evt[2] = shm_id with path
                let requester_sub = evt[1];
                let shm_id = evt[2];
                anyos_std::println!(
                    "[sessionhost] launch request requester_sub={} shm_id={}",
                    requester_sub,
                    shm_id
                );
                let tid = launch::launch_app(shm_id);
                anyos_std::println!(
                    "[sessionhost] launch result requester_sub={} tid={}",
                    requester_sub,
                    tid
                );
                // Send result back to requester
                let reply = [EVT_LAUNCH_RESULT, tid, 0, 0, 0];
                ipc::evt_chan_emit_to(chan, requester_sub, &reply);
                anyos_std::println!(
                    "[sessionhost] reply emitted requester_sub={} tid={}",
                    requester_sub,
                    tid
                );
            }
            continue;
        }

        // Poll system events (crash detection)
        if ipc::evt_sys_poll(sys_sub, &mut evt) {
            if evt[0] == EVT_PROCESS_EXITED {
                let exited_tid = evt[1];
                let exit_code = evt[2];
                if exit_code > 128 && exit_code < 256 {
                    crash::launch_crash_dialog(exited_tid);
                }
            }
            continue;
        }

        process::sleep(50);
    }
}
