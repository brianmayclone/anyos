#![no_std]
#![no_main]

use anyos_std::{args, println, process, sys, vec, String, Vec};

anyos_std::entry!(main);

const SECTOR_SIZE: usize = 512;
const EXFAT_EOC_MIN: u32 = 0xFFFF_FFF8;
const EXFAT_BAD: u32 = 0xFFFF_FFF7;
const ENTRY_EOD: u8 = 0x00;
const ENTRY_BITMAP: u8 = 0x81;
const ENTRY_UPCASE: u8 = 0x82;
const ENTRY_LABEL: u8 = 0x83;
const ENTRY_FILE: u8 = 0x85;
const ENTRY_STREAM: u8 = 0xC0;
const ENTRY_FILENAME: u8 = 0xC1;
const ATTR_DIRECTORY: u16 = 0x0010;
const FLAG_NO_FAT_CHAIN: u8 = 0x02;

#[derive(Clone)]
struct Config {
    device_id: u32,
    sectors: Option<u64>,
    json: bool,
    repair: bool,
}

#[derive(Clone, Copy)]
struct Boot {
    sector_shift: u8,
    sectors_per_cluster_shift: u8,
    fat_offset: u32,
    fat_length: u32,
    cluster_heap_offset: u32,
    cluster_count: u32,
    root_cluster: u32,
    volume_length: u64,
}

impl Boot {
    fn bytes_per_sector(&self) -> usize {
        1usize << self.sector_shift
    }

    fn sectors_per_cluster(&self) -> u32 {
        1u32 << self.sectors_per_cluster_shift
    }

    fn cluster_size(&self) -> usize {
        self.bytes_per_sector() * self.sectors_per_cluster() as usize
    }

    fn cluster_lba(&self, cluster: u32) -> u64 {
        self.cluster_heap_offset as u64 + (cluster - 2) as u64 * self.sectors_per_cluster() as u64
    }

    fn valid_cluster(&self, cluster: u32) -> bool {
        cluster >= 2 && cluster < self.cluster_count + 2
    }
}

struct Report {
    device_id: u32,
    sectors: u64,
    dirs_checked: u32,
    files_checked: u32,
    clusters_referenced: u32,
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Report {
    fn new(device_id: u32, sectors: u64) -> Self {
        Self {
            device_id,
            sectors,
            dirs_checked: 0,
            files_checked: 0,
            clusters_referenced: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn error(&mut self, msg: &str) {
        self.errors.push(String::from(msg));
    }

    fn warn(&mut self, msg: &str) {
        self.warnings.push(String::from(msg));
    }

    fn clean(&self) -> bool {
        self.errors.is_empty()
    }
}

fn main() -> u32 {
    match run() {
        Ok((report, json)) => {
            if json {
                print_json(&report);
            } else {
                print_report(&report);
            }
            if report.clean() {
                0
            } else {
                4
            }
        }
        Err(msg) => {
            println!("fsck.exfat: {}", msg);
            1
        }
    }
}

fn run() -> Result<(Report, bool), &'static str> {
    let cfg = parse_args()?;
    if cfg.repair {
        return Err("--repair wird fuer exFAT noch nicht unterstuetzt");
    }
    let sectors = match cfg.sectors {
        Some(value) => value,
        None => device_size_sectors(cfg.device_id)?,
    };
    let json = cfg.json;
    let mut report = Report::new(cfg.device_id, sectors);
    let boot = read_boot(cfg.device_id, sectors, &mut report)?;
    check_boot_regions(cfg.device_id, &mut report)?;

    let mut referenced = Vec::new();
    check_root(
        cfg.device_id,
        &boot,
        boot.root_cluster,
        &mut referenced,
        &mut report,
        0,
    )?;
    check_allocation_bitmap(cfg.device_id, &boot, &referenced, &mut report)?;

    Ok((report, json))
}

fn parse_args() -> Result<Config, &'static str> {
    let mut args_buf = [0u8; 512];
    let raw = process::args(&mut args_buf);
    let argv = args::tokenize(raw);
    let mut device_id = None;
    let mut sectors = None;
    let mut json = false;
    let mut repair = false;
    let mut i = 0usize;

    while i < argv.len() {
        match argv[i].as_str() {
            "--device" => {
                i += 1;
                if i >= argv.len() {
                    return Err("--device braucht eine ID");
                }
                device_id = resolve_device_arg(argv[i].as_str());
            }
            "--sectors" => {
                i += 1;
                if i >= argv.len() {
                    return Err("--sectors braucht einen Wert");
                }
                sectors = parse_u64(argv[i].as_str());
            }
            "--json" => json = true,
            "--repair" => repair = true,
            "--help" | "-h" => {
                usage();
                return Err("usage");
            }
            _ if argv[i].starts_with("--device=") => {
                device_id = resolve_device_arg(&argv[i][9..]);
            }
            _ if argv[i].starts_with("--sectors=") => {
                sectors = parse_u64(&argv[i][10..]);
            }
            _ => return Err("unbekannte Option"),
        }
        i += 1;
    }

    Ok(Config {
        device_id: device_id.ok_or("--device fehlt")?,
        sectors,
        json,
        repair,
    })
}

fn usage() {
    println!("Usage: fsck.exfat --device ID [--sectors N] [--json] [--repair]");
}

fn read_boot(device_id: u32, sectors: u64, report: &mut Report) -> Result<Boot, &'static str> {
    let mut sector = [0u8; SECTOR_SIZE];
    read_sector(device_id, 0, &mut sector)?;
    if &sector[3..11] != b"EXFAT   " {
        return Err("kein exFAT Bootsektor");
    }
    if sector[510] != 0x55 || sector[511] != 0xAA {
        report.error("Bootsektor-Signatur fehlt");
    }

    let sector_shift = sector[108];
    let sectors_per_cluster_shift = sector[109];
    if sector_shift != 9 {
        report.error("nur 512-Byte-Sektoren werden aktuell unterstuetzt");
    }
    if sectors_per_cluster_shift > 25 {
        report.error("ungueltige Sektoren-pro-Cluster-Schiebung");
    }

    let boot = Boot {
        sector_shift,
        sectors_per_cluster_shift,
        fat_offset: read_le32(&sector, 80),
        fat_length: read_le32(&sector, 84),
        cluster_heap_offset: read_le32(&sector, 88),
        cluster_count: read_le32(&sector, 92),
        root_cluster: read_le32(&sector, 96),
        volume_length: read_le64(&sector, 72),
    };

    if boot.volume_length == 0 || boot.volume_length > sectors {
        report.error("VolumeLength liegt ausserhalb des Blockdevices");
    }
    if boot.fat_offset < 24 || boot.fat_length == 0 {
        report.error("ungueltige FAT-Lage");
    }
    if boot.cluster_heap_offset as u64 >= boot.volume_length {
        report.error("Cluster-Heap liegt ausserhalb des Volumes");
    }
    if boot.cluster_count == 0 {
        report.error("ClusterCount ist null");
    }
    if !boot.valid_cluster(boot.root_cluster) {
        report.error("RootDirectoryCluster liegt ausserhalb des Cluster-Heaps");
    }

    Ok(boot)
}

fn check_boot_regions(device_id: u32, report: &mut Report) -> Result<(), &'static str> {
    let mut main = vec![0u8; 12 * SECTOR_SIZE];
    read_sectors(device_id, 0, 12, &mut main)?;
    check_boot_checksum("Main boot region", &main, report);

    let mut backup = vec![0u8; 12 * SECTOR_SIZE];
    read_sectors(device_id, 12, 12, &mut backup)?;
    if backup[3..11] == main[3..11] {
        check_boot_checksum("Backup boot region", &backup, report);
        if backup[..SECTOR_SIZE] != main[..SECTOR_SIZE] {
            report.warn("Backup-Bootsektor unterscheidet sich vom Main-Bootsektor");
        }
    } else {
        report.warn("Backup-Bootregion fehlt oder ist nicht exFAT");
    }
    Ok(())
}

fn check_boot_checksum(name: &str, region: &[u8], report: &mut Report) {
    let checksum = boot_checksum(&region[..11 * SECTOR_SIZE]);
    for i in 0..128usize {
        let stored = read_le32(&region[11 * SECTOR_SIZE..], i * 4);
        if stored != checksum {
            let mut msg = String::from(name);
            msg.push_str(": Boot-Checksumme stimmt nicht");
            report.error(&msg);
            return;
        }
    }
}

fn check_root(
    device_id: u32,
    boot: &Boot,
    first_cluster: u32,
    referenced: &mut Vec<u32>,
    report: &mut Report,
    depth: u32,
) -> Result<(), &'static str> {
    if depth > 32 {
        report.error("Verzeichnistiefe ueberschreitet Sicherheitslimit");
        return Ok(());
    }
    let clusters = cluster_chain(device_id, boot, first_cluster, false, None, report)?;
    for cluster in &clusters {
        push_unique(referenced, *cluster);
    }
    report.dirs_checked += 1;

    let mut saw_bitmap = depth != 0;
    let mut saw_upcase = depth != 0;
    for cluster in clusters {
        let data = read_cluster(device_id, boot, cluster)?;
        let mut offset = 0usize;
        while offset + 32 <= data.len() {
            let entry_type = data[offset];
            if entry_type == ENTRY_EOD {
                break;
            }
            match entry_type {
                ENTRY_BITMAP => {
                    saw_bitmap = true;
                    let first = read_le32(&data, offset + 20);
                    let size = read_le64(&data, offset + 24);
                    if !boot.valid_cluster(first) || size == 0 {
                        report.error("Allocation-Bitmap-Eintrag ist ungueltig");
                    }
                    mark_chain(device_id, boot, first, size, true, referenced, report)?;
                }
                ENTRY_UPCASE => {
                    saw_upcase = true;
                    let first = read_le32(&data, offset + 20);
                    let size = read_le64(&data, offset + 24);
                    if !boot.valid_cluster(first) || size == 0 {
                        report.error("Upcase-Table-Eintrag ist ungueltig");
                    }
                    mark_chain(device_id, boot, first, size, true, referenced, report)?;
                }
                ENTRY_LABEL => {
                    if data[offset + 1] > 11 {
                        report.error("Volume-Label-Eintrag hat ungueltige Laenge");
                    }
                }
                ENTRY_FILE => {
                    offset =
                        check_file_set(device_id, boot, &data, offset, referenced, report, depth)?;
                    continue;
                }
                t if t & 0x80 == 0 => {}
                _ => report.warn("unbekannter aktiver Directory-Eintrag"),
            }
            offset += 32;
        }
    }
    if depth == 0 {
        if !saw_bitmap {
            report.error("Root Directory enthaelt keine Allocation Bitmap");
        }
        if !saw_upcase {
            report.error("Root Directory enthaelt keine Upcase Table");
        }
    }
    Ok(())
}

fn check_file_set(
    device_id: u32,
    boot: &Boot,
    data: &[u8],
    offset: usize,
    referenced: &mut Vec<u32>,
    report: &mut Report,
    depth: u32,
) -> Result<usize, &'static str> {
    let secondary = data[offset + 1] as usize;
    if secondary < 2 || offset + (secondary + 1) * 32 > data.len() {
        report.error("Datei-Entry-Set ist abgeschnitten oder zu kurz");
        return Ok(offset + 32);
    }
    let stream = offset + 32;
    if data[stream] != ENTRY_STREAM {
        report.error("Datei-Entry-Set ohne Stream Extension");
        return Ok(offset + (secondary + 1) * 32);
    }

    let attrs = read_le16(data, offset + 4);
    let flags = data[stream + 1];
    let name_len = data[stream + 3] as usize;
    let first = read_le32(data, stream + 20);
    let data_len = read_le64(data, stream + 24);
    let filename_entries = secondary - 1;
    let max_name_units = filename_entries * 15;

    if name_len > max_name_units {
        report.error("Dateiname ist laenger als seine Filename-Eintraege");
    }
    for idx in 0..filename_entries {
        if data[offset + 64 + idx * 32] != ENTRY_FILENAME {
            report.error("Datei-Entry-Set enthaelt ungueltigen Filename-Eintrag");
            break;
        }
    }

    let is_dir = attrs & ATTR_DIRECTORY != 0;
    if is_dir {
        if !boot.valid_cluster(first) {
            report.error("Verzeichnis referenziert ungueltigen ersten Cluster");
        } else {
            check_root(device_id, boot, first, referenced, report, depth + 1)?;
        }
    } else if data_len > 0 {
        if !boot.valid_cluster(first) {
            report.error("Datei referenziert ungueltigen ersten Cluster");
        } else {
            let contiguous = flags & FLAG_NO_FAT_CHAIN != 0;
            mark_chain(
                device_id, boot, first, data_len, contiguous, referenced, report,
            )?;
        }
    }
    report.files_checked += 1;
    Ok(offset + (secondary + 1) * 32)
}

fn mark_chain(
    device_id: u32,
    boot: &Boot,
    first: u32,
    data_len: u64,
    contiguous: bool,
    referenced: &mut Vec<u32>,
    report: &mut Report,
) -> Result<(), &'static str> {
    let clusters = cluster_chain(device_id, boot, first, contiguous, Some(data_len), report)?;
    for cluster in clusters {
        push_unique(referenced, cluster);
    }
    Ok(())
}

fn cluster_chain(
    device_id: u32,
    boot: &Boot,
    first: u32,
    contiguous: bool,
    data_len: Option<u64>,
    report: &mut Report,
) -> Result<Vec<u32>, &'static str> {
    let mut out = Vec::new();
    if !boot.valid_cluster(first) {
        report.error("Clusterkette startet ausserhalb des Cluster-Heaps");
        return Ok(out);
    }
    let expected = data_len
        .map(|len| div_ceil_u64(len, boot.cluster_size() as u64) as u32)
        .unwrap_or(boot.cluster_count)
        .max(1);

    if contiguous {
        for i in 0..expected {
            let cluster = first.saturating_add(i);
            if !boot.valid_cluster(cluster) {
                report.error("kontigue Clusterkette verlaesst den Cluster-Heap");
                break;
            }
            out.push(cluster);
        }
        report.clusters_referenced += out.len() as u32;
        return Ok(out);
    }

    let mut cluster = first;
    for _ in 0..boot.cluster_count {
        if !boot.valid_cluster(cluster) {
            report.error("FAT-Clusterkette verlaesst den Cluster-Heap");
            break;
        }
        if contains_u32(&out, cluster) {
            report.error("FAT-Clusterkette enthaelt einen Zyklus");
            break;
        }
        out.push(cluster);
        if data_len.is_some() && out.len() >= expected as usize {
            break;
        }
        let next = read_fat(device_id, boot, cluster)?;
        if next >= EXFAT_EOC_MIN {
            break;
        }
        if next == EXFAT_BAD {
            report.error("FAT-Clusterkette enthaelt einen Bad-Cluster-Marker");
            break;
        }
        if next == 0 {
            report.error("FAT-Clusterkette zeigt auf freien Cluster");
            break;
        }
        cluster = next;
    }
    report.clusters_referenced += out.len() as u32;
    Ok(out)
}

fn check_allocation_bitmap(
    device_id: u32,
    boot: &Boot,
    referenced: &[u32],
    report: &mut Report,
) -> Result<(), &'static str> {
    let root = read_cluster(device_id, boot, boot.root_cluster)?;
    let mut bitmap_first = 0u32;
    let mut bitmap_size = 0u64;
    for entry in root.chunks(32) {
        if entry[0] == ENTRY_BITMAP {
            bitmap_first = read_le32(entry, 20);
            bitmap_size = read_le64(entry, 24);
            break;
        }
        if entry[0] == ENTRY_EOD {
            break;
        }
    }
    if bitmap_first == 0 {
        return Ok(());
    }

    let bitmap = read_file_data(device_id, boot, bitmap_first, bitmap_size, true, report)?;
    for cluster in referenced {
        if !bitmap_allocated(&bitmap, *cluster) {
            report.error("referenzierter Cluster ist in der Allocation Bitmap frei");
            break;
        }
    }
    for cluster in 2..boot.cluster_count + 2 {
        if bitmap_allocated(&bitmap, cluster) && !contains_u32(referenced, cluster) {
            report.warn("Allocation Bitmap enthaelt belegte, aber nicht referenzierte Cluster");
            break;
        }
    }
    Ok(())
}

fn read_file_data(
    device_id: u32,
    boot: &Boot,
    first: u32,
    len: u64,
    contiguous: bool,
    report: &mut Report,
) -> Result<Vec<u8>, &'static str> {
    let clusters = cluster_chain(device_id, boot, first, contiguous, Some(len), report)?;
    let mut out = Vec::new();
    for cluster in clusters {
        let data = read_cluster(device_id, boot, cluster)?;
        out.extend_from_slice(&data);
    }
    out.truncate(len as usize);
    Ok(out)
}

fn bitmap_allocated(bitmap: &[u8], cluster: u32) -> bool {
    if cluster < 2 {
        return false;
    }
    let bit = (cluster - 2) as usize;
    let byte = bit / 8;
    byte < bitmap.len() && (bitmap[byte] & (1 << (bit % 8))) != 0
}

fn read_cluster(device_id: u32, boot: &Boot, cluster: u32) -> Result<Vec<u8>, &'static str> {
    let mut data = vec![0u8; boot.cluster_size()];
    read_sectors(
        device_id,
        boot.cluster_lba(cluster),
        boot.sectors_per_cluster(),
        &mut data,
    )?;
    Ok(data)
}

fn read_fat(device_id: u32, boot: &Boot, cluster: u32) -> Result<u32, &'static str> {
    let fat_byte = cluster as u64 * 4;
    let sector = boot.fat_offset as u64 + fat_byte / SECTOR_SIZE as u64;
    let offset = (fat_byte % SECTOR_SIZE as u64) as usize;
    if sector >= boot.fat_offset as u64 + boot.fat_length as u64 {
        return Err("FAT-Leseposition ausserhalb der FAT");
    }
    let mut buf = [0u8; SECTOR_SIZE];
    read_sector(device_id, sector, &mut buf)?;
    Ok(read_le32(&buf, offset))
}

fn read_sector(
    device_id: u32,
    lba: u64,
    sector: &mut [u8; SECTOR_SIZE],
) -> Result<(), &'static str> {
    let rc = sys::disk_read(device_id, lba, 1, sector);
    if rc == 1 {
        Ok(())
    } else {
        Err("disk_read fehlgeschlagen")
    }
}

fn read_sectors(device_id: u32, lba: u64, count: u32, buf: &mut [u8]) -> Result<(), &'static str> {
    let expected = count as usize * SECTOR_SIZE;
    if buf.len() < expected {
        return Err("Lesepuffer zu klein");
    }
    let rc = sys::disk_read(device_id, lba, count, &mut buf[..expected]);
    if rc == count {
        Ok(())
    } else {
        Err("disk_read fehlgeschlagen")
    }
}

fn resolve_device_arg(value: &str) -> Option<u32> {
    parse_u32(value).or_else(|| resolve_device_path(value))
}

fn resolve_device_path(path: &str) -> Option<u32> {
    let name = path.strip_prefix("/dev/").unwrap_or(path);
    let (disk_id, partition) = parse_storage_name(name)?;
    let mut buf = [0u8; 4096];
    let count = sys::disk_list(&mut buf);
    if count == u32::MAX {
        return None;
    }
    let wanted_part = partition.unwrap_or(0xFF);
    let n = (count as usize).min(buf.len() / 32);
    for idx in 0..n {
        let base = idx * 32;
        if buf[base + 1] == disk_id && buf[base + 2] == wanted_part {
            return Some(buf[base] as u32);
        }
    }
    None
}

fn parse_storage_name(name: &str) -> Option<(u8, Option<u8>)> {
    if let Some(rest) = name.strip_prefix("sd") {
        let b = rest.as_bytes();
        if b.is_empty() || !b[0].is_ascii_lowercase() {
            return None;
        }
        let disk_id = b[0] - b'a';
        if rest.len() == 1 {
            return Some((disk_id, None));
        }
        let part = parse_u32(&rest[1..])?;
        if part == 0 || part > u8::MAX as u32 {
            return None;
        }
        return Some((disk_id, Some((part - 1) as u8)));
    }
    if let Some(rest) = name.strip_prefix("hd") {
        if let Some(pos) = rest.find('p') {
            let disk_id = parse_u32(&rest[..pos])?;
            let part = parse_u32(&rest[pos + 1..])?;
            if disk_id > u8::MAX as u32 || part == 0 || part > u8::MAX as u32 {
                return None;
            }
            return Some((disk_id as u8, Some((part - 1) as u8)));
        }
        let disk_id = parse_u32(rest)?;
        if disk_id > u8::MAX as u32 {
            return None;
        }
        return Some((disk_id as u8, None));
    }
    None
}

fn device_size_sectors(device_id: u32) -> Result<u64, &'static str> {
    let mut buf = [0u8; 4096];
    let count = sys::disk_list(&mut buf);
    if count == u32::MAX {
        return Err("disk_list fehlgeschlagen");
    }
    let n = (count as usize).min(buf.len() / 32);
    for idx in 0..n {
        let base = idx * 32;
        if buf[base] as u32 == device_id {
            return Ok(read_le64(&buf, base + 16));
        }
    }
    Err("device nicht gefunden")
}

fn print_report(report: &Report) {
    println!("fsck.exfat report");
    println!("-----------------");
    println!("device id          : {}", report.device_id);
    println!("sectors            : {}", report.sectors);
    println!("directories checked: {}", report.dirs_checked);
    println!("files checked      : {}", report.files_checked);
    println!("clusters referenced: {}", report.clusters_referenced);
    println!("errors             : {}", report.errors.len());
    println!("warnings           : {}", report.warnings.len());
    println!(
        "status             : {}",
        if report.clean() { "CLEAN" } else { "DIRTY" }
    );
    for err in &report.errors {
        println!("  [error] {}", err);
    }
    for warn in &report.warnings {
        println!("  [warning] {}", warn);
    }
}

fn print_json(report: &Report) {
    println!(
        "{{\"device\":{},\"sectors\":{},\"clean\":{},\"directories_checked\":{},\"files_checked\":{},\"clusters_referenced\":{},\"errors\":{},\"warnings\":{}}}",
        report.device_id,
        report.sectors,
        if report.clean() { "true" } else { "false" },
        report.dirs_checked,
        report.files_checked,
        report.clusters_referenced,
        report.errors.len(),
        report.warnings.len()
    );
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn parse_u64(s: &str) -> Option<u64> {
    let mut value = 0u64;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(value)
}

fn push_unique(values: &mut Vec<u32>, value: u32) {
    if !contains_u32(values, value) {
        values.push(value);
    }
}

fn contains_u32(values: &[u32], value: u32) -> bool {
    values.iter().any(|v| *v == value)
}

fn div_ceil_u64(n: u64, d: u64) -> u64 {
    if n == 0 {
        0
    } else {
        1 + (n - 1) / d
    }
}

fn boot_checksum(data: &[u8]) -> u32 {
    let mut checksum = 0u32;
    for (idx, b) in data.iter().enumerate() {
        if idx == 106 || idx == 107 || idx == 112 {
            continue;
        }
        checksum = checksum.rotate_right(1).wrapping_add(*b as u32);
    }
    checksum
}

fn read_le16(buf: &[u8], off: usize) -> u16 {
    buf[off] as u16 | ((buf[off + 1] as u16) << 8)
}

fn read_le32(buf: &[u8], off: usize) -> u32 {
    let mut value = 0u32;
    for i in 0..4 {
        value |= (buf[off + i] as u32) << (i * 8);
    }
    value
}

fn read_le64(buf: &[u8], off: usize) -> u64 {
    let mut value = 0u64;
    for i in 0..8 {
        value |= (buf[off + i] as u64) << (i * 8);
    }
    value
}
