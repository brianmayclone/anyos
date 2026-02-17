//! User and group database for the kernel.
//!
//! Stores user/group entries parsed from `/System/users/passwd` and `/System/users/group`.
//! Provides authentication via MD5 password hashing.

use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// Maximum number of users supported.
const MAX_USERS: usize = 16;
/// Maximum number of groups supported.
const MAX_GROUPS: usize = 16;
/// Maximum members per group.
const MAX_GROUP_MEMBERS: usize = 8;

/// A user entry parsed from `/System/users/passwd`.
#[derive(Clone)]
pub struct UserEntry {
    pub username: [u8; 32],
    pub password_hash: [u8; 32], // MD5 hex string (32 ASCII chars), or all zeros for no password
    pub uid: u16,
    pub gid: u16,
    pub fullname: [u8; 64],
    pub homedir: [u8; 64],
    pub used: bool,
}

impl UserEntry {
    const fn empty() -> Self {
        UserEntry {
            username: [0u8; 32],
            password_hash: [0u8; 32],
            uid: 0,
            gid: 0,
            fullname: [0u8; 64],
            homedir: [0u8; 64],
            used: false,
        }
    }

    pub fn username_str(&self) -> &str {
        let len = self.username.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.username[..len]).unwrap_or("")
    }

    pub fn fullname_str(&self) -> &str {
        let len = self.fullname.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.fullname[..len]).unwrap_or("")
    }

    pub fn homedir_str(&self) -> &str {
        let len = self.homedir.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.homedir[..len]).unwrap_or("/")
    }

    fn hash_str(&self) -> &str {
        let len = self.password_hash.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.password_hash[..len]).unwrap_or("")
    }
}

/// A group entry parsed from `/System/users/group`.
#[derive(Clone)]
pub struct GroupEntry {
    pub name: [u8; 32],
    pub gid: u16,
    pub members: [[u8; 32]; MAX_GROUP_MEMBERS],
    pub member_count: u8,
    pub used: bool,
}

impl GroupEntry {
    const fn empty() -> Self {
        GroupEntry {
            name: [0u8; 32],
            gid: 0,
            members: [[0u8; 32]; MAX_GROUP_MEMBERS],
            member_count: 0,
            used: false,
        }
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

static USERS: Spinlock<[UserEntry; MAX_USERS]> = Spinlock::new({
    const E: UserEntry = UserEntry::empty();
    [E; MAX_USERS]
});

static GROUPS: Spinlock<[GroupEntry; MAX_GROUPS]> = Spinlock::new({
    const E: GroupEntry = GroupEntry::empty();
    [E; MAX_GROUPS]
});

fn copy_str_to_buf(src: &str, dst: &mut [u8]) {
    let len = src.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&src.as_bytes()[..len]);
    dst[len] = 0;
}

/// Initialize the user and group databases from `/System/users/passwd` and `/System/users/group`.
/// Should be called after VFS is available.
pub fn init() {
    crate::serial_println!("  Initializing user database...");

    // Parse passwd file
    if let Ok(data) = crate::fs::vfs::read_file_to_vec("/System/users/passwd") {
        if let Ok(text) = core::str::from_utf8(&data) {
            let mut users = USERS.lock();
            let mut idx = 0;
            for line in text.split('\n') {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if idx >= MAX_USERS {
                    break;
                }
                if let Some(entry) = parse_passwd_line(line) {
                    users[idx] = entry;
                    idx += 1;
                }
            }
            crate::serial_println!("    Loaded {} user(s)", idx);
        }
    } else {
        // No passwd file — create default root user in memory
        crate::serial_println!("    No passwd file found, creating default root user");
        let mut users = USERS.lock();
        let mut entry = UserEntry::empty();
        copy_str_to_buf("root", &mut entry.username);
        // Empty password hash = no password
        entry.uid = 0;
        entry.gid = 0;
        copy_str_to_buf("System Administrator", &mut entry.fullname);
        copy_str_to_buf("/", &mut entry.homedir);
        entry.used = true;
        users[0] = entry;
    }

    // Parse group file
    if let Ok(data) = crate::fs::vfs::read_file_to_vec("/System/users/group") {
        if let Ok(text) = core::str::from_utf8(&data) {
            let mut groups = GROUPS.lock();
            let mut idx = 0;
            for line in text.split('\n') {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if idx >= MAX_GROUPS {
                    break;
                }
                if let Some(entry) = parse_group_line(line) {
                    groups[idx] = entry;
                    idx += 1;
                }
            }
            crate::serial_println!("    Loaded {} group(s)", idx);
        }
    } else {
        crate::serial_println!("    No group file found, creating default root group");
        let mut groups = GROUPS.lock();
        let mut entry = GroupEntry::empty();
        copy_str_to_buf("root", &mut entry.name);
        entry.gid = 0;
        copy_str_to_buf("root", &mut entry.members[0]);
        entry.member_count = 1;
        entry.used = true;
        groups[0] = entry;
    }
}

/// Parse a passwd line: `username:md5hash:uid:gid:fullname:homedir`
fn parse_passwd_line(line: &str) -> Option<UserEntry> {
    let parts: Vec<&str> = line.splitn(6, ':').collect();
    if parts.len() < 6 {
        return None;
    }
    let uid: u16 = parts[2].parse().ok()?;
    let gid: u16 = parts[3].parse().ok()?;

    let mut entry = UserEntry::empty();
    copy_str_to_buf(parts[0], &mut entry.username);
    copy_str_to_buf(parts[1], &mut entry.password_hash);
    entry.uid = uid;
    entry.gid = gid;
    copy_str_to_buf(parts[4], &mut entry.fullname);
    copy_str_to_buf(parts[5], &mut entry.homedir);
    entry.used = true;
    Some(entry)
}

/// Parse a group line: `groupname:gid:member1,member2,...`
fn parse_group_line(line: &str) -> Option<GroupEntry> {
    let parts: Vec<&str> = line.splitn(3, ':').collect();
    if parts.len() < 2 {
        return None;
    }
    let gid: u16 = parts[1].parse().ok()?;

    let mut entry = GroupEntry::empty();
    copy_str_to_buf(parts[0], &mut entry.name);
    entry.gid = gid;

    if parts.len() >= 3 && !parts[2].is_empty() {
        let mut count = 0;
        for member in parts[2].split(',') {
            let member = member.trim();
            if !member.is_empty() && count < MAX_GROUP_MEMBERS {
                copy_str_to_buf(member, &mut entry.members[count]);
                count += 1;
            }
        }
        entry.member_count = count as u8;
    }
    entry.used = true;
    Some(entry)
}

/// Authenticate a user by username and password.
/// Returns `Some((uid, gid))` on success, `None` on failure.
pub fn authenticate(username: &str, password: &str) -> Option<(u16, u16)> {
    let users = USERS.lock();
    for user in users.iter() {
        if !user.used {
            continue;
        }
        if user.username_str() != username {
            continue;
        }
        // Check password
        let stored_hash = user.hash_str();
        if stored_hash.is_empty() {
            // Empty hash = no password required
            return Some((user.uid, user.gid));
        }
        // Compute MD5 of the provided password
        let computed = crate::crypto::md5::md5_hex(password.as_bytes());
        let computed_str = core::str::from_utf8(&computed).unwrap_or("");
        if stored_hash == computed_str {
            return Some((user.uid, user.gid));
        }
        return None; // Found user but wrong password
    }
    None // User not found
}

/// Look up a user by UID.
pub fn lookup_uid(uid: u16) -> Option<UserEntry> {
    let users = USERS.lock();
    for user in users.iter() {
        if user.used && user.uid == uid {
            return Some(user.clone());
        }
    }
    None
}

/// Look up the primary gid for a given uid. Returns 0 if not found.
pub fn lookup_gid_for_uid(uid: u16) -> u16 {
    match lookup_uid(uid) {
        Some(entry) => entry.gid,
        None => 0,
    }
}

/// Look up a user by username.
pub fn lookup_username(name: &str) -> Option<UserEntry> {
    let users = USERS.lock();
    for user in users.iter() {
        if user.used && user.username_str() == name {
            return Some(user.clone());
        }
    }
    None
}

/// Add a new user to the database. Returns true on success.
pub fn add_user(username: &str, password_hash: &str, uid: u16, gid: u16, fullname: &str, homedir: &str) -> bool {
    let mut users = USERS.lock();

    // Check for duplicate username or uid
    for user in users.iter() {
        if user.used && (user.username_str() == username || user.uid == uid) {
            return false;
        }
    }

    // Find empty slot
    let slot = match users.iter_mut().find(|u| !u.used) {
        Some(s) => s,
        None => return false,
    };

    copy_str_to_buf(username, &mut slot.username);
    copy_str_to_buf(password_hash, &mut slot.password_hash);
    slot.uid = uid;
    slot.gid = gid;
    copy_str_to_buf(fullname, &mut slot.fullname);
    copy_str_to_buf(homedir, &mut slot.homedir);
    slot.used = true;

    // Persist to disk (drop lock first to avoid nested lock)
    drop(users);
    persist_passwd();
    true
}

/// Remove a user by UID. Returns true on success.
pub fn remove_user(uid: u16) -> bool {
    if uid == 0 {
        return false; // Cannot remove root
    }
    let mut users = USERS.lock();
    for user in users.iter_mut() {
        if user.used && user.uid == uid {
            *user = UserEntry::empty();
            drop(users);
            persist_passwd();
            return true;
        }
    }
    false
}

/// Add a new group. Returns true on success.
pub fn add_group(name: &str, gid: u16) -> bool {
    let mut groups = GROUPS.lock();
    for group in groups.iter() {
        if group.used && (group.name_str() == name || group.gid == gid) {
            return false;
        }
    }
    let slot = match groups.iter_mut().find(|g| !g.used) {
        Some(s) => s,
        None => return false,
    };
    copy_str_to_buf(name, &mut slot.name);
    slot.gid = gid;
    slot.member_count = 0;
    slot.used = true;
    drop(groups);
    persist_group();
    true
}

/// Remove a group by GID. Returns true on success.
pub fn remove_group(gid: u16) -> bool {
    if gid == 0 {
        return false; // Cannot remove root group
    }
    let mut groups = GROUPS.lock();
    for group in groups.iter_mut() {
        if group.used && group.gid == gid {
            *group = GroupEntry::empty();
            drop(groups);
            persist_group();
            return true;
        }
    }
    false
}

/// Check if a user (by uid) is a member of a group (by gid).
pub fn is_member(uid: u16, gid: u16) -> bool {
    // Look up user's username
    let username = {
        let users = USERS.lock();
        let mut found = [0u8; 32];
        let mut found_any = false;
        for user in users.iter() {
            if user.used && user.uid == uid {
                found = user.username;
                found_any = true;
                break;
            }
        }
        if !found_any {
            return false;
        }
        found
    };
    let username_len = username.iter().position(|&b| b == 0).unwrap_or(32);
    let username_str = core::str::from_utf8(&username[..username_len]).unwrap_or("");

    let groups = GROUPS.lock();
    for group in groups.iter() {
        if !group.used || group.gid != gid {
            continue;
        }
        for i in 0..group.member_count as usize {
            let mlen = group.members[i].iter().position(|&b| b == 0).unwrap_or(32);
            if let Ok(mstr) = core::str::from_utf8(&group.members[i][..mlen]) {
                if mstr == username_str {
                    return true;
                }
            }
        }
    }
    false
}

/// List all users into a buffer. Returns bytes written.
/// Format: "uid:username:fullname\n" per user.
pub fn list_users(buf: &mut [u8]) -> usize {
    let users = USERS.lock();
    let mut written = 0;
    for user in users.iter() {
        if !user.used {
            continue;
        }
        let line = alloc::format!("{}:{}:{}\n", user.uid, user.username_str(), user.fullname_str());
        let bytes = line.as_bytes();
        if written + bytes.len() > buf.len() {
            break;
        }
        buf[written..written + bytes.len()].copy_from_slice(bytes);
        written += bytes.len();
    }
    written
}

/// List all groups into a buffer. Returns bytes written.
/// Format: "gid:groupname\n" per group.
pub fn list_groups(buf: &mut [u8]) -> usize {
    let groups = GROUPS.lock();
    let mut written = 0;
    for group in groups.iter() {
        if !group.used {
            continue;
        }
        let line = alloc::format!("{}:{}\n", group.gid, group.name_str());
        let bytes = line.as_bytes();
        if written + bytes.len() > buf.len() {
            break;
        }
        buf[written..written + bytes.len()].copy_from_slice(bytes);
        written += bytes.len();
    }
    written
}

/// Get username for a given UID. Returns bytes written to buf (0 if not found).
pub fn get_username(uid: u16, buf: &mut [u8]) -> usize {
    let users = USERS.lock();
    for user in users.iter() {
        if user.used && user.uid == uid {
            let name = user.username_str();
            let len = name.len().min(buf.len());
            buf[..len].copy_from_slice(&name.as_bytes()[..len]);
            return len;
        }
    }
    0
}

/// Return the next available UID (starting from 1000).
pub fn next_uid() -> u16 {
    let users = USERS.lock();
    let mut max_uid: u16 = 999;
    for user in users.iter() {
        if user.used && user.uid >= 1000 && user.uid > max_uid {
            max_uid = user.uid;
        }
    }
    max_uid + 1
}

/// Write the current user database to `/System/users/passwd`.
fn persist_passwd() {
    let users = USERS.lock();
    let mut content = alloc::string::String::new();
    for user in users.iter() {
        if !user.used {
            continue;
        }
        content.push_str(user.username_str());
        content.push(':');
        content.push_str(user.hash_str());
        content.push(':');
        // Use a simple integer-to-string conversion
        let mut uid_buf = [0u8; 8];
        let uid_str = format_u16(user.uid, &mut uid_buf);
        content.push_str(uid_str);
        content.push(':');
        let mut gid_buf = [0u8; 8];
        let gid_str = format_u16(user.gid, &mut gid_buf);
        content.push_str(gid_str);
        content.push(':');
        content.push_str(user.fullname_str());
        content.push(':');
        content.push_str(user.homedir_str());
        content.push('\n');
    }
    drop(users);

    // Write to VFS
    if let Ok(fd) = crate::fs::vfs::open("/System/users/passwd", crate::fs::file::FileFlags {
        read: false, write: true, append: false, create: true, truncate: true,
    }) {
        let _ = crate::fs::vfs::write(fd, content.as_bytes());
        let _ = crate::fs::vfs::close(fd);
    }
}

/// Write the current group database to `/System/users/group`.
fn persist_group() {
    let groups = GROUPS.lock();
    let mut content = alloc::string::String::new();
    for group in groups.iter() {
        if !group.used {
            continue;
        }
        content.push_str(group.name_str());
        content.push(':');
        let mut gid_buf = [0u8; 8];
        let gid_str = format_u16(group.gid, &mut gid_buf);
        content.push_str(gid_str);
        content.push(':');
        for i in 0..group.member_count as usize {
            if i > 0 {
                content.push(',');
            }
            let mlen = group.members[i].iter().position(|&b| b == 0).unwrap_or(32);
            if let Ok(mstr) = core::str::from_utf8(&group.members[i][..mlen]) {
                content.push_str(mstr);
            }
        }
        content.push('\n');
    }
    drop(groups);

    if let Ok(fd) = crate::fs::vfs::open("/System/users/group", crate::fs::file::FileFlags {
        read: false, write: true, append: false, create: true, truncate: true,
    }) {
        let _ = crate::fs::vfs::write(fd, content.as_bytes());
        let _ = crate::fs::vfs::close(fd);
    }
}

fn format_u16(val: u16, buf: &mut [u8; 8]) -> &str {
    if val == 0 {
        return "0";
    }
    let mut n = val;
    let mut i = 7;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if i == 0 { break; }
        i -= 1;
    }
    core::str::from_utf8(&buf[i + 1..8]).unwrap_or("0")
}
