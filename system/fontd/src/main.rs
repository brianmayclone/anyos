//! fontd — Font server daemon for anyOS.
//!
//! Loads TTF/OTF font files into shared memory on demand. Clients request
//! a font by filename (e.g. `"sfpro.ttf"`), fontd loads it from
//! `/System/fonts/` into SHM and returns the SHM ID. All subsequent
//! requests for the same file return the cached SHM — the font data
//! is loaded exactly once and shared across all processes.
//!
//! See `protocol.rs` for the IPC command/response format.

#![no_std]
#![no_main]

mod cache;
mod loader;
mod protocol;

use anyos_std::{ipc, println};

anyos_std::entry!(main);

const CHANNEL_NAME: &str = "fontd";

/// Broadcast event on the fontd channel so waiters know we're ready.
pub const EVT_FONTD_READY: u32 = 0x6000;

fn main() {
    println!("[fontd] starting");

    cache::init();

    let chan = ipc::evt_chan_create(CHANNEL_NAME);
    let sub = ipc::evt_chan_subscribe(chan, 0);

    // Broadcast ready signal — compositor waits for this before calling font_init()
    ipc::evt_chan_emit(chan, &[EVT_FONTD_READY, 0, 0, 0, 0]);

    println!("[fontd] ready (channel '{}')", CHANNEL_NAME);

    loop {
        ipc::evt_chan_wait(chan, sub, u32::MAX);

        let mut evt = [0u32; 5];
        while ipc::evt_chan_poll(chan, sub, &mut evt) {
            let resp = protocol::handle_request(&evt);
            let requester_sub = evt[1];
            ipc::evt_chan_emit_to(chan, requester_sub, &resp);
        }
    }
}
