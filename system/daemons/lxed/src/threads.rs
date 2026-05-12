use anyos_std::sys;

const THREAD_ENTRY_SIZE: usize = 80;
const MAX_THREADS: usize = 256;

pub(crate) fn is_thread_alive(tid: u32) -> bool {
    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = sys::sysinfo(1, &mut buf);
    if count == u32::MAX {
        return false;
    }

    for idx in 0..count as usize {
        let off = idx * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let entry_tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if entry_tid == tid {
            let state = buf[off + 5];
            return state <= 2;
        }
    }
    false
}
