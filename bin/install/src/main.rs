#![no_std]
#![no_main]

use anyos_std::format;
use anyos_std::String;
use anyos_std::vec;
use anyos_std::Vec;
use anyos_std::{print, println};
use anyos_std::fs;
use anyos_std::sys;
use anyos_std::process;

anyos_std::entry!(main);

// ── Constants ──────────────────────────────────────────────────────────────

const SECTOR_SIZE: u32 = 512;
const PARTITION_START: u32 = 128; // exFAT partition starts at sector 128
const FAT_OFFSET: u32 = 32;      // FAT starts at sector 32 within partition
const SPC: u32 = 8;              // sectors per cluster (4 KB clusters)
const SPC_SHIFT: u8 = 3;         // log2(SPC)
const CLUSTER_SIZE: u32 = SPC * SECTOR_SIZE; // 4096

const FS_TYPE_EXFAT: u32 = 7;    // fs_type for mount syscall

// ── Disk listing helpers ───────────────────────────────────────────────────

struct DiskInfo {
    device_id: u8,
    disk_id: u8,
    partition: u8, // 0xFF = whole disk
    start_lba: u64,
    size_sectors: u64,
}

fn list_block_devices() -> Vec<DiskInfo> {
    let mut buf = [0u8; 32 * 32]; // max 32 devices
    let count = sys::disk_list(&mut buf);
    let mut devices = Vec::new();
    for i in 0..count as usize {
        let off = i * 32;
        let device_id = buf[off];
        let disk_id = buf[off + 1];
        let partition = buf[off + 2];
        let start_lba = u64::from_le_bytes([
            buf[off+4], buf[off+5], buf[off+6], buf[off+7],
            buf[off+8], buf[off+9], buf[off+10], buf[off+11],
        ]);
        let size_sectors = u64::from_le_bytes([
            buf[off+12], buf[off+13], buf[off+14], buf[off+15],
            buf[off+16], buf[off+17], buf[off+18], buf[off+19],
        ]);
        devices.push(DiskInfo { device_id, disk_id, partition, start_lba, size_sectors });
    }
    devices
}

fn format_size(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GB", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

// ── ExFAT Formatter ────────────────────────────────────────────────────────

fn write_le16(buf: &mut [u8], off: usize, val: u16) {
    buf[off] = val as u8;
    buf[off + 1] = (val >> 8) as u8;
}

fn write_le32(buf: &mut [u8], off: usize, val: u32) {
    buf[off] = val as u8;
    buf[off + 1] = (val >> 8) as u8;
    buf[off + 2] = (val >> 16) as u8;
    buf[off + 3] = (val >> 24) as u8;
}

fn write_le64(buf: &mut [u8], off: usize, val: u64) {
    for i in 0..8 {
        buf[off + i] = (val >> (i * 8)) as u8;
    }
}

struct ExFatParams {
    fs_start: u32,      // absolute LBA on disk
    fs_sectors: u32,    // total sectors in partition
    fat_length: u32,    // FAT size in sectors
    cluster_heap_offset: u32,
    cluster_count: u32,
    root_cluster: u32,
}

fn compute_exfat_params(fs_start: u32, fs_sectors: u32) -> ExFatParams {
    // First estimate
    let est_clusters = (fs_sectors - FAT_OFFSET) / SPC;
    let fat_bytes = (est_clusters + 2) * 4;
    let mut fat_length = (fat_bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let mut cluster_heap_offset = FAT_OFFSET + fat_length;

    // Recompute with final cluster_count
    let cluster_count = (fs_sectors - cluster_heap_offset) / SPC;
    let fat_bytes = (cluster_count + 2) * 4;
    fat_length = (fat_bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
    cluster_heap_offset = FAT_OFFSET + fat_length;

    // Clusters: 2=bitmap, 3=upcase, 4=root_dir
    let root_cluster = 4;

    ExFatParams {
        fs_start, fs_sectors, fat_length, cluster_heap_offset,
        cluster_count, root_cluster,
    }
}

/// Format an exFAT filesystem on the given disk, starting at `fs_start` LBA.
fn format_exfat(disk_id: u8, fs_start: u32, fs_sectors: u32) -> bool {
    let p = compute_exfat_params(fs_start, fs_sectors);
    println!("  exFAT: {} clusters, {} bytes/cluster", p.cluster_count, CLUSTER_SIZE);
    println!("  exFAT: FAT at +{}, data at +{}", FAT_OFFSET, p.cluster_heap_offset);

    // ── Build VBR ──
    let mut vbr = [0u8; 512];
    vbr[0] = 0xEB; vbr[1] = 0x76; vbr[2] = 0x90;
    vbr[3..11].copy_from_slice(b"EXFAT   ");
    write_le64(&mut vbr, 64, p.fs_start as u64);  // PartitionOffset
    write_le64(&mut vbr, 72, p.fs_sectors as u64); // VolumeLength
    write_le32(&mut vbr, 80, FAT_OFFSET);          // FatOffset
    write_le32(&mut vbr, 84, p.fat_length);        // FatLength
    write_le32(&mut vbr, 88, p.cluster_heap_offset); // ClusterHeapOffset
    write_le32(&mut vbr, 92, p.cluster_count);     // ClusterCount
    write_le32(&mut vbr, 96, p.root_cluster);      // FirstClusterOfRootDirectory
    write_le32(&mut vbr, 100, 0x414E594F);         // VolumeSerialNumber "ANYO"
    write_le16(&mut vbr, 104, 0x0100);             // FileSystemRevision
    write_le16(&mut vbr, 106, 0);                  // VolumeFlags
    vbr[108] = 9;                                   // BytesPerSectorShift
    vbr[109] = SPC_SHIFT;                           // SectorsPerClusterShift
    vbr[110] = 1;                                   // NumberOfFats
    vbr[111] = 0x80;                                // DriveSelect
    vbr[112] = 0xFF;                                // PercentInUse
    vbr[510] = 0x55; vbr[511] = 0xAA;

    let mut ext = [0u8; 512];
    ext[510] = 0x55; ext[511] = 0xAA;

    let oem = [0u8; 512];
    let reserved = [0u8; 512];

    // Compute boot region checksum over sectors 0-10
    let mut checksum: u32 = 0;
    let regions: [&[u8; 512]; 11] = [
        &vbr, &ext, &ext, &ext, &ext, &ext, &ext, &ext, &ext,
        array_ref_512(&oem), array_ref_512(&reserved),
    ];
    for (sec_idx, sector) in regions.iter().enumerate() {
        for (byte_idx, &b) in sector.iter().enumerate() {
            let abs_idx = sec_idx * 512 + byte_idx;
            if abs_idx == 106 || abs_idx == 107 || abs_idx == 112 {
                continue;
            }
            checksum = (checksum.rotate_right(1)).wrapping_add(b as u32);
        }
    }

    let mut cs_sector = [0u8; 512];
    for i in 0..128 {
        write_le32(&mut cs_sector, i * 4, checksum);
    }

    // Write boot region (main + backup)
    let dev = disk_id as u32;
    for base in [0u32, 12] {
        disk_write_sector(dev, p.fs_start + base, &vbr);
        for i in 0..8u32 {
            disk_write_sector(dev, p.fs_start + base + 1 + i, &ext);
        }
        disk_write_sector(dev, p.fs_start + base + 9, &oem);
        disk_write_sector(dev, p.fs_start + base + 10, &reserved);
        disk_write_sector(dev, p.fs_start + base + 11, &cs_sector);
    }

    // ── Build FAT ──
    let fat_entries = p.cluster_count + 2;
    let fat_bytes = fat_entries * 4;
    let mut fat = vec![0u8; fat_bytes as usize];
    write_le32(&mut fat, 0, 0xFFFFFFF8); // media descriptor
    write_le32(&mut fat, 4, 0xFFFFFFFF); // reserved
    write_le32(&mut fat, 8, 0xFFFFFFFF); // cluster 2: bitmap (single cluster)
    write_le32(&mut fat, 12, 0xFFFFFFFF); // cluster 3: upcase (single cluster for minimal table)
    write_le32(&mut fat, 16, 0xFFFFFFFF); // cluster 4: root dir (single cluster)

    // Write FAT to disk
    let fat_abs = p.fs_start + FAT_OFFSET;
    let mut off = 0usize;
    for s in 0..p.fat_length {
        let mut sector = [0u8; 512];
        let end = (off + 512).min(fat.len());
        let copy_len = end - off;
        sector[..copy_len].copy_from_slice(&fat[off..end]);
        disk_write_sector(dev, fat_abs + s, &sector);
        off += 512;
    }

    // ── Build upcase table (minimal: 128 entries = 256 bytes) ──
    let mut upcase = vec![0u8; CLUSTER_SIZE as usize];
    for i in 0u16..128 {
        let upper = if i >= 0x61 && i <= 0x7A { i - 0x20 } else { i };
        write_le16(&mut upcase, i as usize * 2, upper);
    }
    let upcase_len: u32 = 128 * 2; // 256 bytes

    // Compute upcase checksum
    let mut upcase_checksum: u32 = 0;
    for i in 0..upcase_len as usize {
        upcase_checksum = upcase_checksum.rotate_right(1).wrapping_add(upcase[i] as u32);
    }

    // Write upcase to cluster 3
    write_cluster(dev, &p, 3, &upcase);

    // ── Build root directory ──
    let mut root = vec![0u8; CLUSTER_SIZE as usize];
    let mut pos = 0usize;

    // Allocation Bitmap entry (0x81)
    let bitmap_size = (p.cluster_count + 7) / 8;
    root[pos] = 0x81;
    root[pos + 1] = 0; // first bitmap
    write_le32(&mut root, pos + 20, 2); // FirstCluster = 2
    write_le64(&mut root, pos + 24, bitmap_size as u64);
    pos += 32;

    // Upcase Table entry (0x82)
    root[pos] = 0x82;
    write_le32(&mut root, pos + 4, upcase_checksum);
    write_le32(&mut root, pos + 20, 3); // FirstCluster = 3
    write_le64(&mut root, pos + 24, upcase_len as u64);
    pos += 32;

    // Volume Label entry (0x83): "anyOS"
    root[pos] = 0x83;
    root[pos + 1] = 5; // CharacterCount
    let label = b"anyOS";
    for (i, &ch) in label.iter().enumerate() {
        write_le16(&mut root, pos + 2 + i * 2, ch as u16);
    }

    // Write root dir to cluster 4
    write_cluster(dev, &p, 4, &root);

    // ── Build allocation bitmap ──
    let mut bitmap = vec![0u8; CLUSTER_SIZE as usize];
    // Clusters 2, 3, 4 are allocated (bits 0, 1, 2)
    bitmap[0] = 0x07; // bits 0-2 set
    write_cluster(dev, &p, 2, &bitmap);

    // Zero out a few more sectors after the structures to be safe
    let zero = [0u8; 512];
    for s in 24..FAT_OFFSET {
        disk_write_sector(dev, p.fs_start + s, &zero);
    }

    true
}

fn array_ref_512(arr: &[u8; 512]) -> &[u8; 512] { arr }

fn disk_write_sector(device_id: u32, lba: u32, data: &[u8; 512]) {
    sys::disk_write(device_id, lba as u64, 1, data);
}

fn write_cluster(device_id: u32, p: &ExFatParams, cluster: u32, data: &[u8]) {
    let lba = p.fs_start + p.cluster_heap_offset + (cluster - 2) * SPC;
    let sectors = (data.len() as u32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    for s in 0..sectors.min(SPC) {
        let off = (s * SECTOR_SIZE) as usize;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(data.len());
        sector[..end - off].copy_from_slice(&data[off..end]);
        sys::disk_write(device_id, (lba + s) as u64, 1, &sector);
    }
}

// ── Bootloader installation ────────────────────────────────────────────────

fn install_bootloader(disk_id: u8) -> bool {
    let dev = disk_id as u32;

    // Read stage1 (MBR boot code)
    let stage1_fd = fs::open("/boot/stage1.bin", 0);
    if stage1_fd == u32::MAX {
        // Try alternate path
        let stage1_fd = fs::open("/System/boot/stage1.bin", 0);
        if stage1_fd == u32::MAX {
            println!("  ERROR: stage1.bin not found");
            return false;
        }
        return install_bootloader_with_fd(dev, stage1_fd);
    }
    install_bootloader_with_fd(dev, stage1_fd)
}

fn install_bootloader_with_fd(dev: u32, stage1_fd: u32) -> bool {
    let mut stage1 = [0u8; 512];
    let n = fs::read(stage1_fd, &mut stage1);
    fs::close(stage1_fd);
    if n == 0 || n == u32::MAX {
        println!("  ERROR: failed to read stage1.bin");
        return false;
    }

    // Read stage2
    let stage2_fd = fs::open("/boot/stage2.bin", 0);
    let stage2_fd = if stage2_fd == u32::MAX {
        let fd = fs::open("/System/boot/stage2.bin", 0);
        if fd == u32::MAX {
            println!("  ERROR: stage2.bin not found");
            return false;
        }
        fd
    } else { stage2_fd };

    let mut stage2 = vec![0u8; 63 * 512]; // max 63 sectors
    let mut total = 0usize;
    loop {
        let n = fs::read(stage2_fd, &mut stage2[total..]);
        if n == 0 || n == u32::MAX { break; }
        total += n as usize;
    }
    fs::close(stage2_fd);
    if total == 0 {
        println!("  ERROR: failed to read stage2.bin");
        return false;
    }

    // Read current MBR to preserve any existing data, then overlay stage1 boot code
    let mut mbr = [0u8; 512];
    sys::disk_read(dev, 0, 1, &mut mbr);

    // Copy boot code (bytes 0-439) from stage1
    mbr[..440].copy_from_slice(&stage1[..440]);
    // Boot signature
    mbr[510] = 0x55;
    mbr[511] = 0xAA;

    // Write MBR
    sys::disk_write(dev, 0, 1, &mbr);

    // Write stage2 (sectors 1-63)
    let stage2_sectors = (total + 511) / 512;
    for s in 0..stage2_sectors {
        let off = s * 512;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(total);
        sector[..end - off].copy_from_slice(&stage2[off..end]);
        sys::disk_write(dev, (1 + s) as u64, 1, &sector);
    }

    println!("  Bootloader: stage1 (440 bytes) + stage2 ({} sectors)", stage2_sectors);
    true
}

// ── MBR partition table ────────────────────────────────────────────────────

fn create_partition(disk_id: u32, total_sectors: u64) {
    let part_size = total_sectors - PARTITION_START as u64;
    let mut entry = [0u8; 16];
    entry[0] = 0;     // index (partition 0)
    entry[1] = 0x07;  // type: exFAT
    entry[2] = 0x80;  // bootable
    entry[3] = 0;     // reserved
    write_le32(&mut entry, 4, PARTITION_START);
    write_le32(&mut entry, 8, part_size as u32);
    sys::partition_create(disk_id, &entry);
}

// ── Case mapping for ISO→exFAT copy ───────────────────────────────────────

const CASE_MAP: &[(&str, &str)] = &[
    ("system", "System"),
    ("applications", "Applications"),
    ("users", "Users"),
    ("libraries", "Libraries"),
    ("info.conf", "Info.conf"),
    ("icon.ico", "Icon.ico"),
];

fn fix_case(name: &str) -> String {
    for &(lower, proper) in CASE_MAP {
        if name == lower {
            return String::from(proper);
        }
    }
    // .app bundles: capitalize first letter of each word
    if name.ends_with(".app") {
        return capitalize_words(name);
    }
    // .dlib: capitalize first letter
    if name.ends_with(".dlib") {
        return capitalize_first(name);
    }
    String::from(name)
}

fn capitalize_first(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut first = true;
    for ch in s.chars() {
        if first && ch.is_ascii_lowercase() {
            result.push((ch as u8 - 32) as char);
            first = false;
        } else {
            result.push(ch);
            first = false;
        }
    }
    result
}

fn capitalize_words(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for ch in s.chars() {
        if ch == ' ' || ch == '-' || ch == '_' {
            result.push(ch);
            capitalize_next = true;
        } else if capitalize_next && ch.is_ascii_lowercase() {
            result.push((ch as u8 - 32) as char);
            capitalize_next = false;
        } else {
            result.push(ch);
            capitalize_next = false;
        }
    }
    result
}

// ── Recursive file copy ────────────────────────────────────────────────────

fn copy_recursive(src: &str, dst: &str, depth: u32) -> u32 {
    if depth > 16 { return 0; }

    let mut buf = [0u8; 256 * 64]; // max 256 entries
    let count = fs::readdir(src, &mut buf);
    if count == u32::MAX { return 0; }

    let mut copied = 0u32;
    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || name_len > 56 { continue; }

        let name = match core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Skip . and ..
        if name == "." || name == ".." { continue; }
        // Skip some paths we don't need on the installed system
        if depth == 0 && (name == "src" || name == "apps") { continue; }

        let fixed_name = fix_case(name);
        let child_src = format!("{}/{}", src, name);
        let child_dst = format!("{}/{}", dst, fixed_name);

        if entry_type == 1 {
            // Directory
            fs::mkdir(&child_dst);
            copied += copy_recursive(&child_src, &child_dst, depth + 1);
        } else {
            // File
            if copy_file(&child_src, &child_dst) {
                copied += 1;
            }
        }
    }
    copied
}

fn copy_file(src: &str, dst: &str) -> bool {
    let src_fd = fs::open(src, 0);
    if src_fd == u32::MAX { return false; }

    // Read in chunks to handle large files
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = fs::read(src_fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(src_fd);

    fs::write_bytes(dst, &data).is_ok()
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let mut _help_buf = [0u8; 256];
    let _help_raw = anyos_std::process::args(&mut _help_buf);
    if _help_raw.contains("--help") {
        anyos_std::println!("install - Install anyOS to disk\n\nUsage: install [DISK_ID]\n\nOptions:\n  -l             List available disks");
        return;
    }

    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"l");


    if args.has(b'l') {
        cmd_list();
        return;
    }

    let first_arg = args.pos(0);
    if let Some("list") = first_arg {
        cmd_list();
        return;
    }

    if first_arg.is_none() {
        println!("anyOS Installer v1.0");
        println!("");
        println!("Usage:");
        println!("  install <disk_id>   Install anyOS to the specified disk");
        println!("  install -l          List available disks");
        println!("");
        println!("Example: install 0");
        return;
    }

    let disk_id_str = first_arg.unwrap();
    let disk_id: u8 = match disk_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            println!("ERROR: Invalid disk ID '{}'", disk_id_str);
            return;
        }
    };

    cmd_install(disk_id);
}

fn cmd_list() {
    let devices = list_block_devices();

    println!("Available block devices:");
    println!("");
    for dev in &devices {
        let kind = if dev.partition == 0xFF { "disk" } else { "part" };
        let part_str = if dev.partition == 0xFF {
            String::from("-")
        } else {
            format!("p{}", dev.partition)
        };
        println!("  ID={:<3} disk={} {}  start={:<10} size={} ({})",
            dev.device_id, dev.disk_id, part_str,
            dev.start_lba, dev.size_sectors, format_size(dev.size_sectors));
        let _ = kind;
    }
}

fn cmd_install(target_disk: u8) {
    println!("anyOS Installer v1.0");
    println!("");

    // Find the whole-disk device
    let devices = list_block_devices();
    let disk_dev = devices.iter().find(|d| d.disk_id == target_disk && d.partition == 0xFF);
    let disk = match disk_dev {
        Some(d) => d,
        None => {
            println!("ERROR: Disk {} not found. Use 'install -l' to list devices.", target_disk);
            return;
        }
    };

    let total_sectors = disk.size_sectors;
    if total_sectors < 1024 * 64 { // min ~32 MB
        println!("ERROR: Disk {} is too small ({}).", target_disk, format_size(total_sectors));
        return;
    }

    println!("Target: Disk {} ({}, {} sectors)", target_disk, format_size(total_sectors), total_sectors);

    // Check for partitions
    let partitions: Vec<&DiskInfo> = devices.iter()
        .filter(|d| d.disk_id == target_disk && d.partition != 0xFF)
        .collect();
    if !partitions.is_empty() {
        println!("  Existing partitions: {}", partitions.len());
        for p in &partitions {
            println!("    Partition {}: start={} size={}", p.partition, p.start_lba, format_size(p.size_sectors));
        }
    }

    println!("");
    println!("WARNING: ALL data on disk {} will be ERASED!", target_disk);
    println!("Type 'yes' to continue:");

    let mut confirm_buf = [0u8; 16];
    let confirm_len = sys::con_read_line(&mut confirm_buf);
    let confirm = core::str::from_utf8(&confirm_buf[..confirm_len]).unwrap_or("").trim();
    if confirm != "yes" {
        println!("Aborted.");
        return;
    }

    println!("");
    let disk_dev_id = disk.device_id as u32;
    let fs_sectors = (total_sectors - PARTITION_START as u64) as u32;

    // Step 1: Create MBR partition table
    print!("[1/5] Creating partition table...       ");
    create_partition(disk_dev_id, total_sectors);
    println!("OK");

    // Step 2: Install bootloader
    print!("[2/5] Installing bootloader...          ");
    if !install_bootloader(target_disk) {
        println!("FAILED");
        println!("Installation aborted. Bootloader files not found.");
        println!("Ensure /boot/stage1.bin and /boot/stage2.bin exist.");
        return;
    }
    println!("OK");

    // Step 3: Format exFAT
    print!("[3/5] Formatting exFAT filesystem...    ");
    if !format_exfat(target_disk, PARTITION_START, fs_sectors) {
        println!("FAILED");
        return;
    }
    println!("OK");

    // Step 4: Rescan + mount
    print!("[4/5] Mounting target filesystem...      ");
    sys::partition_rescan(disk_dev_id);
    // Find the new partition device
    let new_devices = list_block_devices();
    let part_dev = new_devices.iter().find(|d| d.disk_id == target_disk && d.partition == 0);
    let part_dev_id = match part_dev {
        Some(d) => d.device_id,
        None => {
            println!("FAILED (partition not found after rescan)");
            return;
        }
    };

    let dev_id_str = format!("{}", part_dev_id);
    let mount_result = fs::mount("/mnt/target", &dev_id_str, FS_TYPE_EXFAT);
    if mount_result != 0 {
        println!("FAILED (mount error={})", mount_result);
        return;
    }
    println!("OK (device={})", part_dev_id);

    // Step 5: Copy system files
    println!("[5/5] Copying system files...");
    fs::mkdir("/mnt/target/System");
    fs::mkdir("/mnt/target/Applications");
    fs::mkdir("/mnt/target/Users");
    fs::mkdir("/mnt/target/boot");
    fs::mkdir("/mnt/target/media");

    let mut total_copied = 0u32;

    // Copy each top-level directory
    let dirs_to_copy: &[(&str, &str)] = &[
        ("/System", "/mnt/target/System"),
        ("/Applications", "/mnt/target/Applications"),
        ("/Users", "/mnt/target/Users"),
        ("/boot", "/mnt/target/boot"),
        ("/media", "/mnt/target/media"),
    ];

    for &(src, dst) in dirs_to_copy {
        print!("  {}... ", src);
        let count = copy_recursive(src, dst, 1);
        println!("{} files", count);
        total_copied += count;
    }

    // Also copy any Libraries if present
    let mut stat_buf = [0u32; 7];
    if fs::stat("/Libraries", &mut stat_buf) == 0 {
        fs::mkdir("/mnt/target/Libraries");
        print!("  /Libraries... ");
        let count = copy_recursive("/Libraries", "/mnt/target/Libraries", 1);
        println!("{} files", count);
        total_copied += count;
    }

    println!("");
    println!("  Total: {} files copied", total_copied);

    // Unmount
    fs::umount("/mnt/target");

    println!("");
    println!("Installation complete!");
    println!("Remove the installation media and reboot to start anyOS from disk.");
}
