#![no_std]
#![no_main]

mod protocol;
mod service;
mod state;
mod threads;

use anyos_std::{println, process};
use libsvc::ServiceLifecycle;

use protocol::{create_or_open_pipe, handle_requests};
use service::publish_details;
use state::RuntimeState;

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 128];
    let raw = process::args(&mut args_buf);
    if raw.contains("--help") {
        println!("lxed - LXE runtime daemon\n\nUsage: lxed");
        return;
    }

    let mut lifecycle = ServiceLifecycle::connect("lxed").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let pipe_id = match create_or_open_pipe() {
        Some(pipe_id) => pipe_id,
        None => {
            println!("lxed: failed to create control pipe");
            if let Some(lifecycle) = lifecycle.as_mut() {
                let _ = lifecycle.notify_failed("pipe_create_failed");
            }
            return;
        }
    };

    let mut runtime = RuntimeState::new();
    runtime.prepare();
    publish_details(&mut lifecycle, &runtime);

    println!(
        "lxed: ready root={} rootfs={}",
        runtime.config.root, runtime.config.rootfs
    );
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }

    let mut buf = [0u8; 1024];
    loop {
        if handle_requests(&mut runtime, pipe_id, &mut buf) {
            publish_details(&mut lifecycle, &runtime);
        }
        process::sleep(25);
    }
}
