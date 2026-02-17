//! User and group management syscall wrappers.

use crate::raw::*;

/// Add a new user. Root only.
/// Returns 0 on success, u32::MAX on error.
pub fn adduser(username: &str, password: &str, fullname: &str, homedir: &str) -> u32 {
    let mut ubuf = [0u8; 33];
    let ulen = username.len().min(32);
    ubuf[..ulen].copy_from_slice(&username.as_bytes()[..ulen]);
    ubuf[ulen] = 0;

    let mut pbuf = [0u8; 65];
    let plen = password.len().min(64);
    pbuf[..plen].copy_from_slice(&password.as_bytes()[..plen]);
    pbuf[plen] = 0;

    let mut fbuf = [0u8; 65];
    let flen = fullname.len().min(64);
    fbuf[..flen].copy_from_slice(&fullname.as_bytes()[..flen]);
    fbuf[flen] = 0;

    let mut hbuf = [0u8; 65];
    let hlen = homedir.len().min(64);
    hbuf[..hlen].copy_from_slice(&homedir.as_bytes()[..hlen]);
    hbuf[hlen] = 0;

    // Pack 4 pointers into a struct the kernel expects
    let ptrs: [u64; 4] = [
        ubuf.as_ptr() as u64,
        pbuf.as_ptr() as u64,
        fbuf.as_ptr() as u64,
        hbuf.as_ptr() as u64,
    ];
    syscall1(SYS_ADDUSER, ptrs.as_ptr() as u64)
}

/// Delete a user by UID. Root only.
pub fn deluser(uid: u16) -> u32 {
    syscall1(SYS_DELUSER, uid as u64)
}

/// List all users. Writes "uid:username\n..." to buf.
/// Returns bytes written.
pub fn listusers(buf: &mut [u8]) -> u32 {
    syscall2(SYS_LISTUSERS, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Add a new group. Root only.
/// Returns 0 on success.
pub fn addgroup(name: &str, gid: u16) -> u32 {
    let mut nbuf = [0u8; 33];
    let nlen = name.len().min(32);
    nbuf[..nlen].copy_from_slice(&name.as_bytes()[..nlen]);
    nbuf[nlen] = 0;

    // Pack name_ptr + gid
    let ptrs: [u64; 2] = [nbuf.as_ptr() as u64, gid as u64];
    syscall1(SYS_ADDGROUP, ptrs.as_ptr() as u64)
}

/// Delete a group by GID. Root only.
pub fn delgroup(gid: u16) -> u32 {
    syscall1(SYS_DELGROUP, gid as u64)
}

/// List all groups. Writes "gid:groupname\n..." to buf.
/// Returns bytes written.
pub fn listgroups(buf: &mut [u8]) -> u32 {
    syscall2(SYS_LISTGROUPS, buf.as_mut_ptr() as u64, buf.len() as u64)
}
