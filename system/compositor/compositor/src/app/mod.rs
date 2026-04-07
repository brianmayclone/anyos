//! Compositor runtime orchestration.

mod bootstrap;
mod ipc;
mod management;
mod session;
mod shutdown;
mod system_events;

pub fn run() {
    bootstrap::run();
}
