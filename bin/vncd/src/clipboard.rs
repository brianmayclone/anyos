//! Compositor clipboard IPC for vncd.
//!
//! Replicates the SHM-based clipboard protocol used by libcompositor,
//! allowing vncd to get/set the system clipboard without linking libanyui.

use anyos_std::{ipc, process};

const CMD_SET_CLIPBOARD: u32 = 0x1011;
const CMD_GET_CLIPBOARD: u32 = 0x1012;
const RESP_CLIPBOARD_DATA: u32 = 0x2010;
const MAX_CLIPBOARD_SIZE: usize = 65536;

/// Set the compositor clipboard from VNC client text.
pub fn set_compositor_clipboard(comp_chan: u32, text: &[u8]) {
    if text.is_empty() || text.len() > MAX_CLIPBOARD_SIZE {
        return;
    }
    let data_len = text.len() as u32;
    let shm_id = ipc::shm_create(data_len);
    if shm_id == 0 {
        return;
    }
    let shm_addr = ipc::shm_map(shm_id);
    if shm_addr == 0 {
        ipc::shm_destroy(shm_id);
        return;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), shm_addr as *mut u8, text.len());
    }
    let cmd: [u32; 5] = [CMD_SET_CLIPBOARD, shm_id, data_len, 0 /* text/plain */, 0];
    ipc::evt_chan_emit(comp_chan, &cmd);
    process::sleep(32);
    ipc::shm_unmap(shm_id);
    ipc::shm_destroy(shm_id);
}

/// Get the compositor clipboard contents. Returns number of bytes written to `buf`.
pub fn get_compositor_clipboard(comp_chan: u32, sub_id: u32, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let cap = buf.len().min(MAX_CLIPBOARD_SIZE) as u32;
    let shm_id = ipc::shm_create(cap);
    if shm_id == 0 {
        return 0;
    }
    let shm_addr = ipc::shm_map(shm_id);
    if shm_addr == 0 {
        ipc::shm_destroy(shm_id);
        return 0;
    }
    let tid = process::getpid();
    let cmd: [u32; 5] = [CMD_GET_CLIPBOARD, shm_id, cap, tid, 0];
    ipc::evt_chan_emit(comp_chan, &cmd);

    let mut response = [0u32; 5];
    for _ in 0..50 {
        while ipc::evt_chan_poll(comp_chan, sub_id, &mut response) {
            if response[0] == RESP_CLIPBOARD_DATA && response[4] == tid {
                let result_len = response[2] as usize;
                let copy_len = result_len.min(buf.len());
                if copy_len > 0 {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            shm_addr as *const u8,
                            buf.as_mut_ptr(),
                            copy_len,
                        );
                    }
                }
                ipc::shm_unmap(shm_id);
                ipc::shm_destroy(shm_id);
                return copy_len;
            }
        }
        process::sleep(5);
    }

    // Timeout
    ipc::shm_unmap(shm_id);
    ipc::shm_destroy(shm_id);
    0
}
