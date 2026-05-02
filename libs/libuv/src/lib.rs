//! libuv-compatible event loop foundation for anyOS.
//!
//! The crate is structured like a small native runtime layer: loop/timer/TCP
//! handles live in focused modules, while this file is only the public facade.

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod error;
pub mod handle;
pub mod loop_;
pub mod tcp;
pub mod time;
pub mod timer;

pub use error::*;
pub use handle::UvHandleKind;
pub use loop_::{
    uv_default_loop, uv_loop_close, uv_loop_init, uv_now, uv_run, uv_stop, uv_update_time,
    EventLoop, ScheduledTask, TaskId, TaskKind, UvLoop, UvRunMode,
};
pub use tcp::{
    tcp_accept_nowait, tcp_close, tcp_connect, tcp_connect_host, tcp_listen, tcp_read, tcp_write,
    uv_close, uv_tcp_accept_nowait, uv_tcp_bind_listen, uv_tcp_connect_ipv4, uv_tcp_init,
    uv_tcp_read, uv_tcp_write, UvTcp,
};
pub use time::now;
pub use timer::{
    uv_timer_init, uv_timer_start, uv_timer_stop, TimerQueue, UvTimer, UvTimerCallback,
};
