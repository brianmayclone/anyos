//! Syscall wrappers for libhttp — delegates to libsyscall.

pub use libsyscall::{
    exit, sleep, sbrk, mmap, munmap,
    open, close, read, write, file_size,
    dns_resolve, tcp_connect, tcp_send, tcp_recv, tcp_close, tcp_recv_available, tcp_status,
    random, log,
    O_WRITE, O_CREATE, O_TRUNC,
};
