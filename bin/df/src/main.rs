#![no_std]
#![no_main]

anyos_std::entry!(main);

fn format_human(bytes: u64, buf: &mut [u8]) -> &str {
    if bytes >= 1024 * 1024 * 1024 {
        let gb10 = (bytes * 10 / (1024 * 1024 * 1024)) as u32;
        let whole = gb10 / 10;
        let frac = gb10 % 10;
        let len = fmt_u32(whole, buf);
        buf[len] = b'.';
        buf[len + 1] = b'0' + (frac as u8);
        buf[len + 2] = b'G';
        unsafe { core::str::from_utf8_unchecked(&buf[..len + 3]) }
    } else if bytes >= 1024 * 1024 {
        let mb10 = (bytes * 10 / (1024 * 1024)) as u32;
        let whole = mb10 / 10;
        let frac = mb10 % 10;
        let len = fmt_u32(whole, buf);
        buf[len] = b'.';
        buf[len + 1] = b'0' + (frac as u8);
        buf[len + 2] = b'M';
        unsafe { core::str::from_utf8_unchecked(&buf[..len + 3]) }
    } else if bytes >= 1024 {
        let kb = (bytes / 1024) as u32;
        let len = fmt_u32(kb, buf);
        buf[len] = b'K';
        unsafe { core::str::from_utf8_unchecked(&buf[..len + 1]) }
    } else {
        let len = fmt_u32(bytes as u32, buf);
        buf[len] = b'B';
        unsafe { core::str::from_utf8_unchecked(&buf[..len + 1]) }
    }
}

fn fmt_u32(mut n: u32, buf: &mut [u8]) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}

fn main() {
    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;

    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"h");
    let human = args.has(b'h');

    // Read mount list
    let mut mnt_buf = [0u8; 2048];
    let mnt_len = anyos_std::fs::list_mounts(&mut mnt_buf);
    if mnt_len == u32::MAX || mnt_len == 0 {
        anyos_std::println!("No filesystems mounted.");
        return;
    }

    anyos_std::println!("{:<20}{:>8}  {:>8}  {:>8}  {:>5}  {}",
        t("Filesystem"), t("Size"), t("Used"), t("Avail"), t("Use%"), t("Mounted on"));

    let mnt_str = match core::str::from_utf8(&mnt_buf[..mnt_len as usize]) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in mnt_str.split('\n') {
        let line = line.trim();
        if line.is_empty() { continue; }
        let mut parts = line.splitn(2, '\t');
        let mount_path = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        let fs_type = parts.next().unwrap_or("unknown");

        // Skip devfs
        if fs_type == "devfs" { continue; }

        let dev_name = match fs_type {
            "iso9660" => "/dev/cdrom",
            "exfat" | "fat16" | "ntfs" => "/dev/hda",
            "smb" => "smb",
            _ => fs_type,
        };

        if let Some(st) = anyos_std::fs::statfs(mount_path) {
            let total_kb = (st.total_bytes / 1024) as u32;
            let used_kb = (st.used_bytes / 1024) as u32;
            let free_kb = (st.free_bytes / 1024) as u32;
            let use_pct = if total_kb > 0 { used_kb as u64 * 100 / total_kb as u64 } else { 0 };

            if human {
                let mut b1 = [0u8; 16];
                let mut b2 = [0u8; 16];
                let mut b3 = [0u8; 16];
                let s_total = format_human(st.total_bytes, &mut b1);
                let s_used = format_human(st.used_bytes, &mut b2);
                let s_free = format_human(st.free_bytes, &mut b3);
                anyos_std::println!("{:<20}{:>8}  {:>8}  {:>8}  {:>3}%  {}",
                    dev_name, s_total, s_used, s_free, use_pct, mount_path);
            } else {
                anyos_std::println!("{:<20}{:>7}K {:>8}K {:>8}K  {:>3}%  {}",
                    dev_name, total_kb, used_kb, free_kb, use_pct, mount_path);
            }
        } else {
            // Skip mounts without stats (e.g. failed disk mounts)
            continue;
        }
    }
}
