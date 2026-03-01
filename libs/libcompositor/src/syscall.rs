//! Syscall wrappers for libcompositor DLL — delegates to libsyscall.

pub use libsyscall::{
    get_tid, sleep, screen_size,
    shm_create, shm_map, shm_unmap, shm_destroy,
    evt_chan_create, evt_chan_subscribe, evt_chan_emit, evt_chan_poll, evt_chan_emit_to,
};
