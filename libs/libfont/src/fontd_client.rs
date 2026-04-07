//! fontd IPC client — requests font data from the fontd daemon via SHM.
//!
//! On first use, connects to the "fontd" event channel.
//! Each font request sends a CMD_LOAD_BY_NAME with the filename in SHM,
//! waits for EVT_FONT_READY, and maps the returned SHM into the caller's
//! address space. The mapped data is a raw TTF/OTF byte slice.

use crate::syscall;

const CMD_LOAD_BY_NAME: u32 = 0x6001;
const EVT_FONT_READY: u32 = 0x6002;

/// Timeout waiting for fontd response (ms).
/// On CD-ROM, loading a 6 MB font can take 15-20s.
const FONTD_TIMEOUT_MS: u32 = 60_000;

/// Connection state (lazily initialized per process).
static mut CHAN_ID: u32 = 0;
static mut SUB_ID: u32 = 0;

/// Ensure we have an open connection to fontd.
fn ensure_connected() -> bool {
    unsafe {
        if CHAN_ID != 0 { return true; }
        let chan = syscall::evt_chan_open(b"fontd");
        if chan == 0 {
            syscall::log_str("[libfont] fontd: channel open returned 0");
            return false;
        }
        let sub = syscall::evt_chan_subscribe(chan);
        if sub == 0 {
            syscall::log_str("[libfont] fontd: subscribe returned 0");
            return false;
        }
        CHAN_ID = chan;
        SUB_ID = sub;

        let mut num_buf = [0u8; 16];
        syscall::log(b"[libfont] fontd: connected (chan=");
        syscall::log(crate::font_manager::format_u32(chan, &mut num_buf).as_bytes());
        syscall::log(b", sub=");
        syscall::log(crate::font_manager::format_u32(sub, &mut num_buf).as_bytes());
        syscall::log(b")\n");
        true
    }
}

/// Request a system font by filename (e.g. "sfpro.ttf") from fontd.
/// Returns the raw font data as a &'static [u8] slice (SHM-mapped, lives forever).
/// Returns None if fontd is not running or the font can't be loaded.
pub fn request_font(filename: &[u8]) -> Option<&'static [u8]> {
    if !ensure_connected() { return None; }

    let (chan, sub) = unsafe { (CHAN_ID, SUB_ID) };

    // Put the filename into a small SHM
    let name_len = filename.len().min(255);
    let name_shm = syscall::shm_create((name_len + 1) as u32);
    if name_shm == 0 { return None; }
    let name_addr = syscall::shm_map(name_shm);
    if name_addr == 0 { syscall::shm_destroy(name_shm); return None; }

    unsafe {
        let dst = core::slice::from_raw_parts_mut(name_addr as *mut u8, name_len + 1);
        dst[..name_len].copy_from_slice(&filename[..name_len]);
        dst[name_len] = 0;
    }

    syscall::log(b"[libfont] fontd: requesting ");
    syscall::log(filename);
    let mut nb = [0u8; 16];
    syscall::log(b" (name_shm=");
    syscall::log(crate::font_manager::format_u32(name_shm, &mut nb).as_bytes());
    syscall::log(b")\n");

    // Broadcast request (fontd + ourselves receive it)
    let req = [CMD_LOAD_BY_NAME, sub, name_shm, 0, 0];
    syscall::evt_chan_emit(chan, &req);

    // Wait for EVT_FONT_READY, skip any non-response events (our own echo)
    let mut resp = [0u32; 5];
    let mut got_response = false;
    for _ in 0..50 {
        syscall::evt_chan_wait(chan, sub, FONTD_TIMEOUT_MS);
        while syscall::evt_chan_poll(chan, sub, &mut resp) {
            if resp[0] == EVT_FONT_READY {
                got_response = true;
                break;
            }
        }
        if got_response { break; }
    }

    // Clean up name SHM
    syscall::shm_unmap(name_shm);
    syscall::shm_destroy(name_shm);

    if !got_response {
        syscall::log_str("[libfont] fontd: no response (timeout)");
        return None;
    }

    let font_shm = resp[1];
    let data_size = resp[2];
    if font_shm == 0 || data_size == 0 { return None; }

    // Map the font data SHM into our address space
    let addr = syscall::shm_map(font_shm);
    if addr == 0 { return None; }

    let data = unsafe { core::slice::from_raw_parts(addr as *const u8, data_size as usize) };

    syscall::log(b"[libfont] fontd: mapped ");
    syscall::log(filename);
    syscall::log(b" (");
    syscall::log(crate::font_manager::format_u32(data_size / 1024, &mut nb).as_bytes());
    syscall::log(b" KB, shm=");
    syscall::log(crate::font_manager::format_u32(font_shm, &mut nb).as_bytes());
    syscall::log(b")\n");

    Some(data)
}
