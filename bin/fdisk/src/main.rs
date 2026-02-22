#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{print, println};

/// Read a line from stdin (fd 0) into buf, echoing characters.
/// Returns number of bytes read (excluding newline).
fn read_line(buf: &mut [u8]) -> usize {
    let mut pos = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let n = anyos_std::fs::read(0, &mut byte);
        if n == 0 {
            anyos_std::process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            break;
        }
        match byte[0] {
            b'\n' | b'\r' => {
                print!("\n");
                break;
            }
            8 | 127 => {
                if pos > 0 {
                    pos -= 1;
                    print!("\x08 \x08");
                }
            }
            c if c >= b' ' => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    print!("{}", c as char);
                }
            }
            _ => {}
        }
    }
    pos
}

/// Parse a decimal number from a string slice.
fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

/// Parse a size string like "100M", "2G", or a plain number (in sectors).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let last = s.as_bytes()[s.len() - 1];
    if last == b'M' || last == b'm' {
        let num = parse_u32(&s[..s.len() - 1])? as u64;
        Some(num * 2048) // 1 MiB = 2048 sectors
    } else if last == b'G' || last == b'g' {
        let num = parse_u32(&s[..s.len() - 1])? as u64;
        Some(num * 2048 * 1024) // 1 GiB = 2097152 sectors
    } else if last == b'K' || last == b'k' {
        let num = parse_u32(&s[..s.len() - 1])? as u64;
        Some(num * 2) // 1 KiB = 2 sectors
    } else {
        Some(parse_u32(s)? as u64)
    }
}

/// Format sectors as a human-readable size string.
fn format_size(sectors: u64, buf: &mut [u8]) -> &str {
    let bytes = sectors * 512;
    let (val, unit) = if bytes >= 1024 * 1024 * 1024 {
        (bytes / (1024 * 1024 * 1024), "GiB")
    } else if bytes >= 1024 * 1024 {
        (bytes / (1024 * 1024), "MiB")
    } else if bytes >= 1024 {
        (bytes / 1024, "KiB")
    } else {
        (bytes, "B")
    };
    // Format into buf
    let mut pos = 0;
    let mut n = val;
    if n == 0 {
        buf[0] = b'0';
        pos = 1;
    } else {
        // Write digits in reverse
        let start = pos;
        while n > 0 {
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
            pos += 1;
        }
        // Reverse
        let end = pos;
        let mut l = start;
        let mut r = end - 1;
        while l < r {
            buf.swap(l, r);
            l += 1;
            r -= 1;
        }
    }
    buf[pos] = b' ';
    pos += 1;
    for &b in unit.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("?")
}

/// MBR partition type byte to name.
fn type_name(t: u8) -> &'static str {
    match t {
        0x00 => "Empty",
        0x01 => "FAT12",
        0x04 | 0x06 | 0x0E => "FAT16",
        0x0B | 0x0C => "FAT32",
        0x07 => "NTFS/exFAT",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0xEE => "GPT protective",
        0xEF => "EFI System",
        _ => "Unknown",
    }
}

/// Read u64 LE from buffer.
fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off], buf[off + 1], buf[off + 2], buf[off + 3],
        buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7],
    ])
}

/// Write u32 LE to buffer.
fn write_u32_le(buf: &mut [u8], off: usize, val: u32) {
    let b = val.to_le_bytes();
    buf[off] = b[0];
    buf[off + 1] = b[1];
    buf[off + 2] = b[2];
    buf[off + 3] = b[3];
}

/// Print the partition table for a disk.
fn print_partitions(disk_id: u32) {
    let mut buf = [0u8; 32 * 8]; // up to 8 partitions
    let count = anyos_std::sys::disk_partitions(disk_id, &mut buf);

    if count == u32::MAX {
        println!("Error reading partition table for disk {}", disk_id);
        return;
    }

    if count == 0 {
        println!("No partitions found on disk {}", disk_id);
        return;
    }

    println!("Disk hd{}: {} partitions", disk_id, count);
    println!("{:<6} {:<4} {:<12} {:>12} {:>12} {:>10}",
        "Part", "Boot", "Type", "Start LBA", "Sectors", "Size");
    println!("{}", "--------------------------------------------------------------");

    for i in 0..count as usize {
        let off = i * 32;
        let index = buf[off];
        let ptype = buf[off + 1];
        let bootable = buf[off + 2] != 0;
        let _scheme = buf[off + 3];
        let start_lba = read_u64_le(&buf, off + 8);
        let size_sectors = read_u64_le(&buf, off + 16);

        let boot_str = if bootable { "*" } else { " " };
        let mut size_buf = [0u8; 32];
        let size_str = format_size(size_sectors, &mut size_buf);

        println!("hd{}p{:<2} {:<4} {:<12} {:>12} {:>12} {:>10}",
            disk_id, index + 1, boot_str, type_name(ptype),
            start_lba, size_sectors, size_str);
    }
}

/// List all block devices.
fn list_all() {
    let mut buf = [0u8; 32 * 16]; // up to 16 devices
    let count = anyos_std::sys::disk_list(&mut buf);

    if count == 0 {
        println!("No block devices found.");
        return;
    }

    println!("{:<10} {:<6} {:<6} {:>12} {:>12} {:>10}",
        "Device", "Disk", "Part", "Start LBA", "Sectors", "Size");
    println!("{}", "--------------------------------------------------------------");

    let mut seen_disks = [false; 8];

    for i in 0..count as usize {
        let off = i * 32;
        let _id = buf[off];
        let disk_id = buf[off + 1];
        let part = buf[off + 2];
        let start_lba = read_u64_le(&buf, off + 8);
        let size_sectors = read_u64_le(&buf, off + 16);

        let mut size_buf = [0u8; 32];
        let size_str = format_size(size_sectors, &mut size_buf);

        if part == 0xFF {
            // Whole disk
            println!("hd{:<7} {:<6} {:<6} {:>12} {:>12} {:>10}",
                disk_id, disk_id, "-", start_lba, size_sectors, size_str);
            seen_disks[disk_id as usize & 7] = true;
        } else {
            println!("hd{}p{:<5} {:<6} {:<6} {:>12} {:>12} {:>10}",
                disk_id, part + 1, disk_id, part + 1, start_lba, size_sectors, size_str);
        }
    }

    println!();

    // Print partition tables for each disk
    for d in 0..8u8 {
        if seen_disks[d as usize] {
            println!();
            print_partitions(d as u32);
        }
    }
}

/// Interactive fdisk session for a disk.
fn interactive(disk_id: u32) {
    println!("fdisk: interactive mode for hd{}", disk_id);
    println!("Type 'h' for help.\n");

    loop {
        print!("fdisk> ");
        let mut line_buf = [0u8; 128];
        let len = read_line(&mut line_buf);
        if len == 0 {
            continue;
        }
        let cmd = core::str::from_utf8(&line_buf[..len]).unwrap_or("");
        let cmd = cmd.trim();

        match cmd.as_bytes().first().copied() {
            Some(b'h') => {
                println!("  p   Print partition table");
                println!("  n   Create new partition");
                println!("  d   Delete a partition");
                println!("  t   Change partition type");
                println!("  o   Create new MBR disklabel");
                println!("  l   List known partition types");
                println!("  w   Write changes and exit");
                println!("  q   Quit without saving");
            }
            Some(b'p') => {
                print_partitions(disk_id);
            }
            Some(b'n') => {
                cmd_new_partition(disk_id);
            }
            Some(b'd') => {
                cmd_delete_partition(disk_id);
            }
            Some(b't') => {
                cmd_change_type(disk_id);
            }
            Some(b'o') => {
                cmd_new_disklabel(disk_id);
            }
            Some(b'l') => {
                println!("Known partition types:");
                println!("  01  FAT12");
                println!("  06  FAT16");
                println!("  0B  FAT32");
                println!("  0C  FAT32 (LBA)");
                println!("  07  NTFS/exFAT");
                println!("  82  Linux swap");
                println!("  83  Linux");
                println!("  EF  EFI System");
            }
            Some(b'w') => {
                println!("Rescanning partition table...");
                let count = anyos_std::sys::partition_rescan(disk_id);
                println!("Found {} partitions.", count);
                println!("Done.");
                return;
            }
            Some(b'q') => {
                println!("Exiting without saving.");
                return;
            }
            _ => {
                println!("Unknown command '{}'. Type 'h' for help.", cmd);
            }
        }
    }
}

/// Create a new partition.
fn cmd_new_partition(disk_id: u32) {
    // Read current partitions to find free slots and space
    let mut part_buf = [0u8; 32 * 4];
    let count = anyos_std::sys::disk_partitions(disk_id, &mut part_buf);

    if count == u32::MAX {
        println!("Error reading partition table.");
        return;
    }

    // Find a free partition slot (0-3 for MBR)
    let mut used = [false; 4];
    for i in 0..count as usize {
        let idx = part_buf[i * 32] as usize;
        if idx < 4 {
            used[idx] = true;
        }
    }

    let slot = match used.iter().position(|&u| !u) {
        Some(s) => s,
        None => {
            println!("All 4 MBR partition slots are in use.");
            return;
        }
    };

    println!("Using partition slot {} (hd{}p{})", slot, disk_id, slot + 1);

    // Ask for start LBA
    print!("Start LBA (default: auto): ");
    let mut lba_buf = [0u8; 32];
    let lba_len = read_line(&mut lba_buf);
    let start_lba = if lba_len == 0 {
        // Find end of last partition
        let mut max_end: u64 = 2048; // default start at 1 MiB
        for i in 0..count as usize {
            let off = i * 32;
            let s = read_u64_le(&part_buf, off + 8);
            let sz = read_u64_le(&part_buf, off + 16);
            let end = s + sz;
            if end > max_end {
                max_end = end;
            }
        }
        // Align to 2048 sectors (1 MiB)
        let aligned = (max_end + 2047) & !2047;
        println!("Auto start: {}", aligned);
        aligned as u32
    } else {
        let s = core::str::from_utf8(&lba_buf[..lba_len]).unwrap_or("");
        match parse_u32(s) {
            Some(v) => v,
            None => {
                println!("Invalid LBA.");
                return;
            }
        }
    };

    // Ask for size
    print!("Size (sectors, or e.g. 100M, 1G): ");
    let mut size_buf = [0u8; 32];
    let size_len = read_line(&mut size_buf);
    if size_len == 0 {
        println!("No size specified.");
        return;
    }
    let size_str = core::str::from_utf8(&size_buf[..size_len]).unwrap_or("");
    let size_sectors = match parse_size(size_str) {
        Some(v) => v as u32,
        None => {
            println!("Invalid size.");
            return;
        }
    };

    // Ask for type
    print!("Partition type (hex, default 0B=FAT32): ");
    let mut type_buf = [0u8; 8];
    let type_len = read_line(&mut type_buf);
    let ptype = if type_len == 0 {
        0x0B // FAT32
    } else {
        let s = core::str::from_utf8(&type_buf[..type_len]).unwrap_or("");
        match parse_hex_u8(s.trim()) {
            Some(v) => v,
            None => {
                println!("Invalid type. Using 0x0B (FAT32).");
                0x0B
            }
        }
    };

    // Build the 16-byte entry
    let mut entry = [0u8; 16];
    entry[0] = slot as u8;
    entry[1] = ptype;
    entry[2] = 0; // not bootable
    write_u32_le(&mut entry, 4, start_lba);
    write_u32_le(&mut entry, 8, size_sectors);

    let ret = anyos_std::sys::partition_create(disk_id, &entry);
    if ret == 0 {
        let mut sb = [0u8; 32];
        let ss = format_size(size_sectors as u64, &mut sb);
        println!("Created partition hd{}p{}: type=0x{:02X} ({}) start={} size={} ({})",
            disk_id, slot + 1, ptype, type_name(ptype), start_lba, size_sectors, ss);
    } else {
        println!("Error creating partition.");
    }
}

/// Delete a partition.
fn cmd_delete_partition(disk_id: u32) {
    print!("Partition number (1-4): ");
    let mut buf = [0u8; 8];
    let len = read_line(&mut buf);
    let s = core::str::from_utf8(&buf[..len]).unwrap_or("");
    let num = match parse_u32(s) {
        Some(n) if n >= 1 && n <= 4 => n,
        _ => {
            println!("Invalid partition number.");
            return;
        }
    };

    let ret = anyos_std::sys::partition_delete(disk_id, num - 1);
    if ret == 0 {
        println!("Partition {} deleted.", num);
    } else {
        println!("Error deleting partition {}.", num);
    }
}

/// Change partition type.
fn cmd_change_type(disk_id: u32) {
    print!("Partition number (1-4): ");
    let mut buf = [0u8; 8];
    let len = read_line(&mut buf);
    let s = core::str::from_utf8(&buf[..len]).unwrap_or("");
    let num = match parse_u32(s) {
        Some(n) if n >= 1 && n <= 4 => n,
        _ => {
            println!("Invalid partition number.");
            return;
        }
    };

    // Read current partition to get its start/size
    let mut part_buf = [0u8; 32 * 4];
    let count = anyos_std::sys::disk_partitions(disk_id, &mut part_buf);
    if count == u32::MAX {
        println!("Error reading partition table.");
        return;
    }

    let idx = (num - 1) as usize;
    let mut found = false;
    let mut start: u32 = 0;
    let mut size: u32 = 0;
    for i in 0..count as usize {
        let off = i * 32;
        if part_buf[off] as usize == idx {
            start = read_u64_le(&part_buf, off + 8) as u32;
            size = read_u64_le(&part_buf, off + 16) as u32;
            found = true;
            break;
        }
    }

    if !found {
        println!("Partition {} not found.", num);
        return;
    }

    print!("New type (hex, e.g. 07, 0B, 83): ");
    let mut type_buf = [0u8; 8];
    let type_len = read_line(&mut type_buf);
    let s = core::str::from_utf8(&type_buf[..type_len]).unwrap_or("");
    let ptype = match parse_hex_u8(s.trim()) {
        Some(v) => v,
        None => {
            println!("Invalid type.");
            return;
        }
    };

    let mut entry = [0u8; 16];
    entry[0] = idx as u8;
    entry[1] = ptype;
    entry[2] = 0;
    write_u32_le(&mut entry, 4, start);
    write_u32_le(&mut entry, 8, size);

    let ret = anyos_std::sys::partition_create(disk_id, &entry);
    if ret == 0 {
        println!("Changed partition {} type to 0x{:02X} ({}).", num, ptype, type_name(ptype));
    } else {
        println!("Error changing type.");
    }
}

/// Create a new (empty) MBR disklabel.
fn cmd_new_disklabel(disk_id: u32) {
    println!("WARNING: This will erase all partition entries on hd{}!", disk_id);
    print!("Are you sure? (y/N): ");
    let mut buf = [0u8; 8];
    let len = read_line(&mut buf);
    if len == 0 || (buf[0] != b'y' && buf[0] != b'Y') {
        println!("Aborted.");
        return;
    }

    // Delete all 4 partitions
    for i in 0..4u32 {
        anyos_std::sys::partition_delete(disk_id, i);
    }
    println!("Created new empty MBR disklabel on hd{}.", disk_id);
}

/// Parse a hex byte from a string like "0B" or "07".
fn parse_hex_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    let s = if s.starts_with("0x") || s.starts_with("0X") {
        &s[2..]
    } else {
        s
    };
    if s.is_empty() || s.len() > 2 {
        return None;
    }
    let mut val: u8 = 0;
    for &b in s.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        val = val.checked_mul(16)?.checked_add(digit)?;
    }
    Some(val)
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"l");

    // fdisk -l : list all disks and partitions
    if args.has(b'l') {
        list_all();
        return;
    }

    // fdisk /dev/hd0  or  fdisk 0
    if args.pos_count > 0 {
        let target = args.positional[0];
        // Parse disk ID from path or plain number
        let disk_id = if target.starts_with("/dev/hd") {
            // Extract just the disk number (before any 'p')
            let rest = &target[7..];
            let end = rest.find('p').unwrap_or(rest.len());
            match parse_u32(&rest[..end]) {
                Some(d) => d,
                None => {
                    println!("fdisk: invalid device '{}'", target);
                    return;
                }
            }
        } else {
            match parse_u32(target) {
                Some(d) => d,
                None => {
                    println!("fdisk: invalid device '{}'", target);
                    return;
                }
            }
        };

        interactive(disk_id);
        return;
    }

    // Default: list
    list_all();
}
