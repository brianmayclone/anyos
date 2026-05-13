#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::partition::{partition_type_name, KNOWN_PARTITION_TYPES};
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
    let (whole, frac, unit) = if bytes >= 1024 * 1024 * 1024 {
        let unit_size: u64 = 1024 * 1024 * 1024;
        (
            bytes / unit_size,
            (bytes % unit_size) * 10 / unit_size,
            "GiB",
        )
    } else if bytes >= 1024 * 1024 {
        let unit_size: u64 = 1024 * 1024;
        (
            bytes / unit_size,
            (bytes % unit_size) * 10 / unit_size,
            "MiB",
        )
    } else if bytes >= 1024 {
        let unit_size: u64 = 1024;
        (
            bytes / unit_size,
            (bytes % unit_size) * 10 / unit_size,
            "KiB",
        )
    } else {
        (bytes, 0, "B")
    };

    let mut pos = 0;
    // whole part
    let mut n = whole;
    if n == 0 {
        buf[pos] = b'0';
        pos += 1;
    } else {
        let start = pos;
        while n > 0 {
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
            pos += 1;
        }
        let (mut l, mut r) = (start, pos - 1);
        while l < r {
            buf.swap(l, r);
            l += 1;
            r -= 1;
        }
    }
    // one fractional digit for KiB/MiB/GiB if non-zero
    if unit != "B" && frac > 0 {
        buf[pos] = b'.';
        pos += 1;
        buf[pos] = b'0' + frac as u8;
        pos += 1;
    }
    buf[pos] = b' ';
    pos += 1;
    for &b in unit.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("?")
}

/// Read u64 LE from buffer.
fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
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

/// Find the block-device ID for a disk partition.
fn partition_device_id(disk_id: u32, partition_index: u8) -> Option<u32> {
    const ENTRY: usize = 64;
    const MAX_DEVS: usize = 16;
    let mut buf = [0u8; ENTRY * MAX_DEVS];
    let count = anyos_std::sys::disk_list(&mut buf);
    if count == u32::MAX {
        return None;
    }
    for i in 0..count as usize {
        let off = i * ENTRY;
        if off + ENTRY > buf.len() {
            break;
        }
        if buf[off + 1] as u32 == disk_id && buf[off + 2] == partition_index {
            return Some(buf[off] as u32);
        }
    }
    None
}

/// Probe the filesystem signature at the start of a partition device.
fn probe_fs(device_id: u32) -> &'static str {
    let mut sector = [0u8; 512];
    if anyos_std::sys::disk_read(device_id, 0, 1, &mut sector) != 0 {
        return "?";
    }

    if &sector[3..11] == b"EXFAT   " {
        return "exFAT";
    }
    if &sector[3..11] == b"NTFS    " {
        return "NTFS";
    }
    if sector[82..90].starts_with(b"FAT32") {
        return "FAT32";
    }
    if sector[54..62].starts_with(b"FAT16") {
        return "FAT16";
    }
    if sector[54..62].starts_with(b"FAT12") {
        return "FAT12";
    }
    if sector[1..6] == *b"CD001" {
        return "ISO9660";
    }

    "Unknown"
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

    println!("Disk sd{}: {} partitions", disk_letter(disk_id), count);
    println!(
        "{:<6} {:<4} {:<14} {:<8} {:>12} {:>12} {:>10}",
        "Part", "Boot", "PartType", "FS", "Start LBA", "Sectors", "Size"
    );
    println!(
        "{}",
        "--------------------------------------------------------------------------------"
    );

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
        let fs_name = partition_device_id(disk_id, index)
            .map(probe_fs)
            .unwrap_or("?");

        println!(
            "sd{}{:<3} {:<4} {:<14} {:<8} {:>12} {:>12} {:>10}",
            disk_letter(disk_id),
            index + 1,
            boot_str,
            partition_type_name(ptype),
            fs_name,
            start_lba,
            size_sectors,
            size_str
        );
    }
}

/// Convert disk_id (0-25) to its Linux-style device letter: 0 → 'a', 1 → 'b', …
fn disk_letter(disk_id: u32) -> char {
    if disk_id < 26 {
        (b'a' + disk_id as u8) as char
    } else {
        '?'
    }
}

/// List all block devices.
fn list_all() {
    // Entry size: 64 bytes per device (matches kernel's preferred format
    // which includes the 40-byte device label after the 24-byte header).
    const ENTRY: usize = 64;
    const MAX_DEVS: usize = 16;

    // First pass: find all whole-disk entries and rescan their partitions
    // so that the device list is always up-to-date.
    {
        let mut tmp = [0u8; ENTRY * MAX_DEVS];
        let n = anyos_std::sys::disk_list(&mut tmp);
        for i in 0..n as usize {
            let off = i * ENTRY;
            let disk_id = tmp[off + 1];
            let part = tmp[off + 2];
            if part == 0xFF {
                anyos_std::sys::partition_rescan(disk_id as u32);
            }
        }
    }

    let mut buf = [0u8; ENTRY * MAX_DEVS];
    let count = anyos_std::sys::disk_list(&mut buf);

    if count == 0 {
        println!("No block devices found.");
        return;
    }

    println!(
        "{:<10} {:>4} {:<6} {:<6} {:>12} {:>12} {:>10}",
        "Device", "ID", "Disk", "Part", "Start LBA", "Sectors", "Size"
    );
    println!(
        "{}",
        "--------------------------------------------------------------------"
    );

    let mut seen_disks = [false; 8];

    for i in 0..count as usize {
        let off = i * ENTRY;
        let id = buf[off];
        let disk_id = buf[off + 1];
        let part = buf[off + 2];
        let start_lba = read_u64_le(&buf, off + 8);
        let size_sectors = read_u64_le(&buf, off + 16);

        let mut size_buf = [0u8; 32];
        let size_str = format_size(size_sectors, &mut size_buf);

        if part == 0xFF {
            // Whole disk
            println!(
                "sd{:<8} {:>4} {:<6} {:<6} {:>12} {:>12} {:>10}",
                disk_letter(disk_id as u32),
                id,
                disk_id,
                "-",
                start_lba,
                size_sectors,
                size_str
            );
            seen_disks[disk_id as usize & 7] = true;
        } else {
            println!(
                "sd{}{:<6} {:>4} {:<6} {:<6} {:>12} {:>12} {:>10}",
                disk_letter(disk_id as u32),
                part + 1,
                id,
                disk_id,
                part + 1,
                start_lba,
                size_sectors,
                size_str
            );
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
    println!("fdisk: interactive mode for sd{}", disk_letter(disk_id));
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
                for info in KNOWN_PARTITION_TYPES {
                    println!("  {:02X}  {}", info.code, info.name);
                }
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

/// Get total sector count for a whole disk via disk_list.
fn disk_total_sectors(disk_id: u32) -> Option<u64> {
    const ENTRY: usize = 64;
    const MAX_DEVS: usize = 16;
    let mut buf = [0u8; ENTRY * MAX_DEVS];
    let count = anyos_std::sys::disk_list(&mut buf);
    for i in 0..count as usize {
        let off = i * ENTRY;
        let did = buf[off + 1];
        let part = buf[off + 2];
        if part == 0xFF && did as u32 == disk_id {
            return Some(read_u64_le(&buf, off + 16));
        }
    }
    None
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

    println!(
        "Using partition slot {} (sd{}{})",
        slot,
        disk_letter(disk_id),
        slot + 1
    );

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

    // How many sectors are free from start_lba to end of disk (for the "max" default).
    let max_available: u32 = disk_total_sectors(disk_id)
        .and_then(|total| total.checked_sub(start_lba as u64))
        .map(|v| v.min(u32::MAX as u64) as u32)
        .unwrap_or(u32::MAX);

    // Ask for size — empty input or "max"/"all" fills the remaining space.
    let mut max_hint = [0u8; 32];
    let hint = format_size(max_available as u64, &mut max_hint);
    print!("Size (sectors, e.g. 100M, 1G; empty or 'max' = {}): ", hint);
    let mut size_buf = [0u8; 32];
    let size_len = read_line(&mut size_buf);
    let size_str = core::str::from_utf8(&size_buf[..size_len])
        .unwrap_or("")
        .trim();
    let size_sectors: u32 = if size_str.is_empty()
        || size_str.eq_ignore_ascii_case("max")
        || size_str.eq_ignore_ascii_case("all")
        || size_str == "*"
    {
        if max_available == 0 {
            println!("No free space available at LBA {}.", start_lba);
            return;
        }
        max_available
    } else {
        match parse_size(size_str) {
            Some(v) => v as u32,
            None => {
                println!("Invalid size.");
                return;
            }
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

    // Validate: partition must fit on disk
    let end_lba = start_lba as u64 + size_sectors as u64;
    if let Some(disk_sectors) = disk_total_sectors(disk_id) {
        if end_lba > disk_sectors {
            let mut sb2 = [0u8; 32];
            let disk_size = format_size(disk_sectors, &mut sb2);
            println!("Error: partition exceeds disk size!");
            println!(
                "  Partition end: LBA {} (start {} + size {})",
                end_lba, start_lba, size_sectors
            );
            println!("  Disk capacity: {} sectors ({})", disk_sectors, disk_size);
            let avail = if (start_lba as u64) < disk_sectors {
                disk_sectors - start_lba as u64
            } else {
                0
            };
            let mut sb3 = [0u8; 32];
            let avail_str = format_size(avail, &mut sb3);
            println!(
                "  Available from LBA {}: {} sectors ({})",
                start_lba, avail, avail_str
            );
            return;
        }
    }

    // Validate: no overlap with existing partitions
    for i in 0..count as usize {
        let off = i * 32;
        let ex_start = read_u64_le(&part_buf, off + 8);
        let ex_size = read_u64_le(&part_buf, off + 16);
        let ex_end = ex_start + ex_size;
        let ex_idx = part_buf[off] as usize + 1;
        if (start_lba as u64) < ex_end && end_lba > ex_start {
            println!(
                "Error: overlaps with partition sd{}{} (LBA {}-{})!",
                disk_letter(disk_id),
                ex_idx,
                ex_start,
                ex_end - 1
            );
            return;
        }
    }

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
        println!(
            "Created partition sd{}{}: type=0x{:02X} ({}) start={} size={} ({})",
            disk_letter(disk_id),
            slot + 1,
            ptype,
            partition_type_name(ptype),
            start_lba,
            size_sectors,
            ss
        );
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
        println!(
            "Changed partition {} type to 0x{:02X} ({}).",
            num,
            ptype,
            partition_type_name(ptype)
        );
    } else {
        println!("Error changing type.");
    }
}

/// Create a new (empty) MBR disklabel.
fn cmd_new_disklabel(disk_id: u32) {
    println!(
        "WARNING: This will erase all partition entries on hd{}!",
        disk_id
    );
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

    if raw.contains("--help") {
        anyos_std::println!("fdisk - Disk partition manager\n\nUsage: fdisk [DISK_ID]\n\nOptions:\n  -l             List available disks");
        return;
    }

    // fdisk -l : list all disks and partitions
    if args.has(b'l') {
        list_all();
        return;
    }

    // fdisk /dev/sda  (Linux-style, letter) or fdisk /dev/hd0 (legacy, digit) or fdisk 0
    if args.pos_count > 0 {
        let target = args.positional[0];
        let disk_id = if let Some(rest) = target.strip_prefix("/dev/sd") {
            // Linux convention: /dev/sd<letter>[<part>] — letter selects the disk.
            let bytes = rest.as_bytes();
            if bytes.is_empty() || !(bytes[0] >= b'a' && bytes[0] <= b'z') {
                println!("fdisk: invalid device '{}'", target);
                return;
            }
            (bytes[0] - b'a') as u32
        } else if let Some(rest) = target.strip_prefix("/dev/hd") {
            // Legacy fallback: /dev/hd<digit>[p<part>] — digit is the disk id.
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
