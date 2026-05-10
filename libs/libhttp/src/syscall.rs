//! Syscall wrappers for libhttp — delegates to libsyscall.

pub use libsyscall::{
    close, dns_resolve, exit, file_size, fsync, log, mmap, munmap, open, random, read, sbrk, sleep,
    tcp_close, tcp_connect, tcp_recv, tcp_recv_available, tcp_send, tcp_status, write, yield_cpu,
    O_APPEND, O_CREATE, O_TRUNC, O_WRITE,
};
