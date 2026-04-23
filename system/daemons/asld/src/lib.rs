#![cfg_attr(not(target_os = "linux"), no_std)]

extern crate alloc;

pub mod agent;
pub mod config;
pub mod distro;
pub mod errors;
pub mod image;
pub mod ipc;
pub mod model;
pub mod mounts;
pub mod network;
pub mod runtime;
pub mod schema;
pub mod status;
pub mod store;
pub mod vm;

use anyos_std::{println, process};
use libsvc::ServiceLifecycle;

use crate::config::ConfdStore;
use crate::ipc::IpcState;
use crate::runtime::RuntimeService;

pub const PIPE_NAME: &str = "asld";

pub fn run() {
    println!("asld: starting");

    let mut lifecycle = ServiceLifecycle::connect("asld").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    if let Err(err) = schema::register_manifest() {
        println!("asld: failed to register confd manifest: {:?}", err);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("manifest_register_failed");
        }
        return;
    }

    let mut store = match ConfdStore::connect("asld") {
        Ok(store) => store,
        Err(err) => {
            println!("asld: failed to connect to confd: {}", err.message());
            if let Some(lifecycle) = lifecycle.as_mut() {
                let _ = lifecycle.notify_failed("confd_connect_failed");
            }
            return;
        }
    };

    let old_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        anyos_std::ipc::pipe_close(old_pipe);
    }

    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("asld: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut runtime = RuntimeService::new();
    let mut ipc_state = IpcState::new();
    let mut pipe_buf = [0u8; 2048];

    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }

    println!("asld: ready (pipe='{}')", PIPE_NAME);

    loop {
        let active = ipc::handle_requests(&mut runtime, &mut store, &mut ipc_state, pipe_id, &mut pipe_buf);
        runtime.tick();
        process::sleep(if active { 20 } else { 100 });
    }
}
