//! App permission management — check, store, list, delete runtime permissions.
//!
//! Used by the PermissionDialog and Settings app to manage per-user, per-app
//! permission grants. Also used internally by `spawn()` to handle the
//! `PERM_NEEDED` sentinel.

use crate::raw::*;

/// Sentinel value returned by SYS_SPAWN when the app needs permission approval.
pub const PERM_NEEDED: u32 = u32::MAX - 2;

/// Check stored permissions for an app.
/// Returns the granted capability bitmask, or u32::MAX if no permission file exists.
/// `uid` = 0 means use the caller's uid.
pub fn perm_check(app_id: &str, uid: u16) -> u32 {
    let mut buf = [0u8; 129];
    let len = app_id.len().min(128);
    buf[..len].copy_from_slice(&app_id.as_bytes()[..len]);
    buf[len] = 0;
    syscall2(SYS_PERM_CHECK, buf.as_ptr() as u64, uid as u64)
}

/// Store granted permissions for an app.
/// Returns true on success.
/// `uid` = 0 means use the caller's uid.
pub fn perm_store(app_id: &str, granted: u32, uid: u16) -> bool {
    let mut buf = [0u8; 129];
    let len = app_id.len().min(128);
    buf[..len].copy_from_slice(&app_id.as_bytes()[..len]);
    buf[len] = 0;
    syscall3(SYS_PERM_STORE, buf.as_ptr() as u64, granted as u64, uid as u64) == 0
}

/// List all apps with stored permissions for the caller's uid.
/// Writes entries as "app_id\x1Fgranted_hex\n" into `buf`.
/// Returns the number of entries written.
pub fn perm_list(buf: &mut [u8]) -> u32 {
    syscall2(SYS_PERM_LIST, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Delete stored permissions for an app (caller's uid).
/// Returns true on success.
pub fn perm_delete(app_id: &str) -> bool {
    let mut buf = [0u8; 129];
    let len = app_id.len().min(128);
    buf[..len].copy_from_slice(&app_id.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_PERM_DELETE, buf.as_ptr() as u64) == 0
}

/// Read pending permission info from the calling thread.
/// Format: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path".
/// Returns bytes written (0 if no pending info).
pub fn perm_pending_info(buf: &mut [u8]) -> u32 {
    syscall2(SYS_PERM_PENDING_INFO, buf.as_mut_ptr() as u64, buf.len() as u64)
}
