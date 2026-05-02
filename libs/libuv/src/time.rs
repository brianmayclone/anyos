pub fn now() -> u64 {
    anyos_std::sys::uptime_ms() as u64
}
