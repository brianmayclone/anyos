use anyos_std::net;

pub type TlsHandle = u32;

fn ensure_initialized() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    if !INITIALIZED.swap(true, Ordering::SeqCst) {
        libtls::set_transport(tcp_send, tcp_recv, sleep, random);
    }
}

fn tcp_send(fd: u32, data: &[u8]) -> i32 {
    let n = net::tcp_send(fd, data);
    if n == u32::MAX {
        -1
    } else {
        n as i32
    }
}

fn tcp_recv(fd: u32, buf: &mut [u8]) -> i32 {
    let start = anyos_std::sys::uptime_ms();
    loop {
        let avail = net::tcp_recv_available(fd);
        match avail {
            u32::MAX => return -1,
            0xFFFF_FFFE => return 0,
            n if n > 0 => {
                let read_len = (n as usize).min(buf.len());
                let n = net::tcp_recv(fd, &mut buf[..read_len]);
                if n == 0 {
                    return 0;
                }
                if n != u32::MAX {
                    return n as i32;
                }
            }
            _ => {}
        }
        if anyos_std::sys::uptime_ms().wrapping_sub(start) >= 30_000 {
            return -1;
        }
        anyos_std::process::sleep(10);
    }
}

fn sleep(ms: u32) {
    anyos_std::process::sleep(ms);
}

fn random(buf: &mut [u8]) -> u32 {
    anyos_std::sys::random(buf) as u32
}

pub fn connect(fd: u32, host: &str) -> i32 {
    ensure_initialized();
    libtls::connect(fd, host)
}

pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
    libtls::send(handle, data)
}
pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
    libtls::recv(handle, buf)
}
pub fn close(handle: TlsHandle) {
    libtls::close(handle);
}
