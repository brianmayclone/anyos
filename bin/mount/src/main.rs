#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Filesystem type constants (must match kernel mount_fs() dispatch).
const FS_TYPE_FAT: u32 = 0;
const FS_TYPE_ISO9660: u32 = 1;
const FS_TYPE_SMB: u32 = 5;
const FS_TYPE_COREFS: u32 = 6;

fn main() {


    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.contains("--help") {
        anyos_std::println!("mount - Mount a filesystem");
        anyos_std::println!("");
        anyos_std::println!("Usage: mount [-t TYPE] DEVICE MOUNTPOINT");
        anyos_std::println!("       mount                (list mounts)");
        anyos_std::println!("");
        anyos_std::println!("Options:");
        anyos_std::println!("  -t TYPE   Filesystem type:");
        anyos_std::println!("              fat, vfat     FAT/exFAT filesystem");
        anyos_std::println!("              iso9660, iso  ISO 9660 (CD-ROM)");
        anyos_std::println!("              smb, cifs     SMB/CIFS network share");
        anyos_std::println!("              corefs        CoreFS filesystem");
        anyos_std::println!("");
        anyos_std::println!("Examples:");
        anyos_std::println!("  mount -t corefs 3 /mnt/data     Mount CoreFS partition (device 3)");
        anyos_std::println!("  mount -t fat 2 /mnt/usb         Mount FAT partition (device 2)");
        anyos_std::println!("  mount -t smb //10.0.0.1/share /mnt/share");
        anyos_std::println!("");
        anyos_std::println!("Device IDs can be found with 'lsblk' or 'sysinfo --disks'.");
        return;
    }


    if args.is_empty() {
        // No arguments: list all mount points
        let mut buf = [0u8; 2048];
        let n = anyos_std::fs::list_mounts(&mut buf);
        if n == u32::MAX {
            anyos_std::println!("mount: failed to list mount points");
            return;
        }
        if n == 0 {
            anyos_std::println!("No filesystems mounted.");
            return;
        }
        // Parse and display "path\tfstype\n" entries
        if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
            for line in text.lines() {
                if line.is_empty() { continue; }
                let parts: (&str, &str) = if let Some(tab) = line.find('\t') {
                    (&line[..tab], &line[tab+1..])
                } else {
                    (line, "unknown")
                };
                anyos_std::println!("{} type {}", parts.0, parts.1);
            }
        }
        return;
    }

    // Parse arguments: mount [-t fstype] device mountpoint
    // Or:              mount -t iso9660 /dev/cdrom0 /mnt/cdrom0
    let args_str = args.trim();
    let mut tokens = TokenIter::new(args_str);

    let mut fs_type_str: Option<&str> = None;
    let mut device: Option<&str> = None;
    let mut mount_point: Option<&str> = None;

    while let Some(tok) = tokens.next() {
        if tok == "-t" {
            fs_type_str = tokens.next();
        } else if device.is_none() {
            device = Some(tok);
        } else {
            mount_point = Some(tok);
        }
    }

    let (device, mount_point_str) = match (device, mount_point) {
        (Some(d), Some(m)) => (d, anyos_std::String::from(m)),
        (Some(d), None) => {
            // Auto-generate mount point for SMB: //ip/share → /mnt/share
            if d.starts_with("//") {
                let stripped = &d[2..]; // ip/share
                let share = if let Some(pos) = stripped.find('/') {
                    &stripped[pos + 1..]
                } else {
                    stripped
                };
                let mut mp = anyos_std::String::from("/mnt/");
                mp.push_str(share);
                (d, mp)
            } else {
                anyos_std::println!("Usage: mount [-t fstype] device mountpoint");
                anyos_std::println!("       mount                (list mounts)");
                anyos_std::println!("Types: fat, iso9660, smb");
                return;
            }
        }
        _ => {
            anyos_std::println!("Usage: mount [-t fstype] device mountpoint");
            anyos_std::println!("       mount                (list mounts)");
            anyos_std::println!("Types: fat, exfat, iso9660, smb, corefs");
            return;
        }
    };

    let fs_type = match fs_type_str {
        Some("fat") | Some("fat16") | Some("vfat") | Some("exfat") => FS_TYPE_FAT,
        Some("iso9660") | Some("iso") | Some("cdrom") => FS_TYPE_ISO9660,
        Some("smb") | Some("cifs") | Some("samba") => FS_TYPE_SMB,
        Some("corefs") => FS_TYPE_COREFS,
        Some(other) => {
            anyos_std::println!("mount: unknown filesystem type '{}'", other);
            anyos_std::println!("Supported types: fat, exfat, iso9660, smb, corefs");
            return;
        }
        None => {
            // Try to auto-detect from device name
            if device.contains("cdrom") || device.contains("dvd") {
                FS_TYPE_ISO9660
            } else if device.starts_with("//") {
                FS_TYPE_SMB
            } else {
                anyos_std::println!("mount: specify filesystem type with -t");
                anyos_std::println!("Supported types: fat, exfat, iso9660, smb, corefs");
                return;
            }
        }
    };

    let result = anyos_std::fs::mount(&mount_point_str, device, fs_type);
    if result == u32::MAX {
        anyos_std::println!("mount: failed to mount {} on {}", device, mount_point_str);
    }
}

/// Simple space-delimited token iterator for argument parsing.
struct TokenIter<'a> {
    remaining: &'a str,
}

impl<'a> TokenIter<'a> {
    fn new(s: &'a str) -> Self {
        Self { remaining: s.trim() }
    }

    fn next(&mut self) -> Option<&'a str> {
        let s = self.remaining.trim_start();
        if s.is_empty() {
            return None;
        }
        if let Some(pos) = s.find(' ') {
            let token = &s[..pos];
            self.remaining = &s[pos+1..];
            Some(token)
        } else {
            let token = s;
            self.remaining = "";
            Some(token)
        }
    }
}
