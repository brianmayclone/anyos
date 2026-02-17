//! VFS permission check logic.
//!
//! Each file/directory has a 12-bit mode:
//! - Bits 0-3:  Others  (Read=1, Modify=2, Delete=4, Create=8)
//! - Bits 4-7:  Group   (Read=1, Modify=2, Delete=4, Create=8)
//! - Bits 8-11: Owner   (Read=1, Modify=2, Delete=4, Create=8)

pub const PERM_READ: u16   = 1;
pub const PERM_MODIFY: u16 = 2;
pub const PERM_DELETE: u16 = 4;
pub const PERM_CREATE: u16 = 8;

/// Default file mode: full access for everyone (backwards-compatible).
pub const DEFAULT_MODE: u16 = 0xFFF;

/// Check if the current thread can perform `needed` permission on a file/dir.
///
/// `file_uid`/`file_gid` are the owner of the file. `mode` is the 12-bit permission mask.
/// `needed` is a bitmask of PERM_READ / PERM_MODIFY / PERM_DELETE / PERM_CREATE.
///
/// Returns true if the operation is allowed.
pub fn check_permission(file_uid: u16, file_gid: u16, mode: u16, needed: u16) -> bool {
    let thread_uid = crate::task::scheduler::current_thread_uid();
    let thread_gid = crate::task::scheduler::current_thread_gid();

    // Root bypasses everything
    if thread_uid == 0 {
        return true;
    }

    // Check owner bits (bits 8-11)
    if thread_uid == file_uid {
        let owner_perms = (mode >> 8) & 0xF;
        return owner_perms & needed == needed;
    }

    // Check group bits (bits 4-7)
    if thread_gid == file_gid || crate::task::users::is_member(thread_uid, file_gid) {
        let group_perms = (mode >> 4) & 0xF;
        return group_perms & needed == needed;
    }

    // Check others bits (bits 0-3)
    let other_perms = mode & 0xF;
    other_perms & needed == needed
}
