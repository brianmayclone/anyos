use alloc::format;
use alloc::string::String;

use libgit::{Oid, Repository};

pub fn resolve_rev(repo: &Repository, rev: &str) -> Option<Oid> {
    if rev == "HEAD" {
        return repo.head().ok();
    }

    if let Ok(oid) = libgit::refs::resolve_ref(repo, &format!("refs/heads/{}", rev)) {
        return Some(oid);
    }

    if let Ok(oid) = libgit::refs::resolve_ref(repo, &format!("refs/remotes/{}", rev)) {
        return Some(oid);
    }

    if let Ok(oid) = libgit::refs::resolve_ref(repo, &format!("refs/tags/{}", rev)) {
        return Some(oid);
    }

    if rev.starts_with("refs/") {
        if let Ok(oid) = libgit::refs::resolve_ref(repo, rev) {
            return Some(oid);
        }
    }

    Oid::from_hex(rev)
}

pub fn get_user_info(repo: &Repository) -> (String, String) {
    let config = libgit::config::read_config(repo).ok();
    let global_config = libgit::config::read_global_config();
    let name = config
        .as_ref()
        .and_then(|c| c.user_name())
        .or_else(|| global_config.as_ref().and_then(|c| c.user_name()))
        .unwrap_or("Unknown");
    let email = config
        .as_ref()
        .and_then(|c| c.user_email())
        .or_else(|| global_config.as_ref().and_then(|c| c.user_email()))
        .unwrap_or("unknown@unknown");
    (String::from(name), String::from(email))
}

pub fn get_timestamp() -> u64 {
    #[cfg(feature = "host")]
    {
        return std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    #[cfg(not(feature = "host"))]
    {
        let mut buf = [0u8; 8];
        anyos_std::sys::time(&mut buf);
        let year = buf[0] as u64 | ((buf[1] as u64) << 8);
        let month = buf[2] as u64;
        let day = buf[3] as u64;
        let hour = buf[4] as u64;
        let min = buf[5] as u64;
        let sec = buf[6] as u64;
        if year < 1970 || month == 0 || day == 0 {
            return 0;
        }
        let days = (year - 1970) * 365 + (year - 1969) / 4 + month_days(month) + day - 1;
        days * 86400 + hour * 3600 + min * 60 + sec
    }
}

#[cfg(not(feature = "host"))]
fn month_days(month: u64) -> u64 {
    const C: [u64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    if month < 13 {
        C[month as usize]
    } else {
        0
    }
}
