//! Shared config file access helpers.

pub(super) const CONF_PATH: &str = "/System/compositor/compositor.conf";

pub(super) fn read_conf() -> Option<alloc::string::String> {
    use anyos_std::fs;

    let fd = fs::open(CONF_PATH, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut buf = [0u8; 2048];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 {
        return None;
    }

    match core::str::from_utf8(&buf[..n]) {
        Ok(s) => Some(alloc::string::String::from(s)),
        Err(_) => None,
    }
}
