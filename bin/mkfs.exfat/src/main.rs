#![no_std]
#![no_main]

use anyos_std::{args, println, process, sys, vec, String};

anyos_std::entry!(main);

const SECTOR_SIZE: u32 = 512;
const BYTES_PER_SECTOR_SHIFT: u8 = 9;
const SECTORS_PER_CLUSTER_SHIFT: u8 = 3;
const SECTORS_PER_CLUSTER: u32 = 1 << SECTORS_PER_CLUSTER_SHIFT;
const CLUSTER_SIZE: usize = (SECTOR_SIZE * SECTORS_PER_CLUSTER) as usize;
const FAT_OFFSET: u32 = 32;
const EXFAT_EOC: u32 = 0xFFFF_FFFF;
const EXFAT_MEDIA: u32 = 0xFFFF_FFF8;
const EXFAT_MIN_CLUSTERS: u32 = 16;

#[derive(Clone)]
struct Config {
    device_id: u32,
    sectors: Option<u64>,
    label: String,
    json: bool,
}

#[derive(Clone, Copy)]
struct Layout {
    fs_sectors: u64,
    fat_length: u32,
    cluster_heap_offset: u32,
    cluster_count: u32,
    bitmap_cluster: u32,
    bitmap_clusters: u32,
    upcase_cluster: u32,
    root_cluster: u32,
}

fn main() -> u32 {
    match run() {
        Ok(()) => 0,
        Err(msg) => {
            println!("mkfs.exfat: {}", msg);
            1
        }
    }
}

fn run() -> Result<(), &'static str> {
    let cfg = parse_args()?;
    let sectors = match cfg.sectors {
        Some(value) => value,
        None => device_size_sectors(cfg.device_id)?,
    };
    let layout = compute_layout(sectors)?;

    zero_reserved_area(cfg.device_id)?;
    write_fat(cfg.device_id, &layout)?;
    write_bitmap(cfg.device_id, &layout)?;
    let upcase_checksum = write_upcase(cfg.device_id, &layout)?;
    write_root(cfg.device_id, &layout, upcase_checksum, &cfg.label)?;
    write_boot_regions(cfg.device_id, &layout)?;

    if cfg.json {
        println!(
            "{{\"device\":{},\"sectors\":{},\"clusters\":{},\"fat_sectors\":{},\"cluster_heap_offset\":{},\"root_cluster\":{}}}",
            cfg.device_id,
            layout.fs_sectors,
            layout.cluster_count,
            layout.fat_length,
            layout.cluster_heap_offset,
            layout.root_cluster
        );
    } else {
        println!(
            "mkfs.exfat: device {} formatiert: {} Sektoren, {} Cluster, FAT {} Sektoren",
            cfg.device_id, layout.fs_sectors, layout.cluster_count, layout.fat_length
        );
    }
    Ok(())
}

fn parse_args() -> Result<Config, &'static str> {
    let mut args_buf = [0u8; 512];
    let raw = process::args(&mut args_buf);
    let argv = args::tokenize(raw);
    let mut device_id: Option<u32> = None;
    let mut sectors: Option<u64> = None;
    let mut label = String::from("anyOS");
    let mut json = false;
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
            "--label" => {
                i += 1;
                if i >= argv.len() {
                    return Err("--label braucht einen Namen");
                }
                label = sanitize_label(argv[i].as_str());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                print_usage();
                return Err("usage");
            }
            _ if argv[i].starts_with("--device=") => {
                device_id = resolve_device_arg(&argv[i][9..]);
            }
            _ if argv[i].starts_with("--sectors=") => {
                sectors = parse_u64(&argv[i][10..]);
            }
            _ if argv[i].starts_with("--label=") => {
                label = sanitize_label(&argv[i][8..]);
            }
            _ => return Err("unbekannte Option"),
        }
        i += 1;
    }

    let device_id = device_id.ok_or("--device fehlt")?;
    if label.is_empty() {
        label = String::from("anyOS");
    }
    Ok(Config {
        device_id,
        sectors,
        label,
        json,
    })
}

fn print_usage() {
    println!("Usage: mkfs.exfat --device ID [--sectors N] [--label LABEL] [--json]");
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

fn sanitize_label(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        if out.len() >= 11 {
            break;
        }
        if b.is_ascii_graphic() || b == b' ' {
            out.push(b as char);
        }
    }
    out
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

fn compute_layout(fs_sectors: u64) -> Result<Layout, &'static str> {
    if fs_sectors <= 256 {
        return Err("Volume zu klein");
    }
    if fs_sectors > u32::MAX as u64 {
        return Err("Volume groesser als mkfs.exfat aktuell unterstuetzt");
    }

    let sectors = fs_sectors as u32;
    let est_clusters = sectors.saturating_sub(FAT_OFFSET) / SECTORS_PER_CLUSTER;
    let mut fat_length = div_ceil((est_clusters + 2) * 4, SECTOR_SIZE);
    let mut cluster_heap_offset = FAT_OFFSET + fat_length;
    let mut cluster_count = sectors.saturating_sub(cluster_heap_offset) / SECTORS_PER_CLUSTER;
    fat_length = div_ceil((cluster_count + 2) * 4, SECTOR_SIZE);
    cluster_heap_offset = FAT_OFFSET + fat_length;
    cluster_count = sectors.saturating_sub(cluster_heap_offset) / SECTORS_PER_CLUSTER;

    if cluster_count < EXFAT_MIN_CLUSTERS {
        return Err("Volume hat zu wenige Cluster");
    }

    let bitmap_bytes = div_ceil(cluster_count, 8);
    let bitmap_clusters = div_ceil(bitmap_bytes, CLUSTER_SIZE as u32).max(1);
    let bitmap_cluster = 2;
    let upcase_cluster = bitmap_cluster + bitmap_clusters;
    let root_cluster = upcase_cluster + 1;
    if root_cluster > cluster_count + 1 {
        return Err("Volume hat keinen Platz fuer Metadaten");
    }

    Ok(Layout {
        fs_sectors,
        fat_length,
        cluster_heap_offset,
        cluster_count,
        bitmap_cluster,
        bitmap_clusters,
        upcase_cluster,
        root_cluster,
    })
}

fn zero_reserved_area(device_id: u32) -> Result<(), &'static str> {
    let zero = [0u8; SECTOR_SIZE as usize];
    for sector in 24..FAT_OFFSET {
        write_sector(device_id, sector as u64, &zero)?;
    }
    Ok(())
}

fn write_fat(device_id: u32, layout: &Layout) -> Result<(), &'static str> {
    let entries_per_sector = SECTOR_SIZE as usize / 4;
    let mut sector = [0u8; SECTOR_SIZE as usize];
    for s in 0..layout.fat_length {
        sector.fill(0);
        let first_entry = s as usize * entries_per_sector;
        for i in 0..entries_per_sector {
            let entry = (first_entry + i) as u32;
            let value = fat_entry_value(entry, layout);
            if value != 0 {
                write_le32(&mut sector, i * 4, value);
            }
        }
        write_sector(device_id, (FAT_OFFSET + s) as u64, &sector)?;
    }
    Ok(())
}

fn fat_entry_value(entry: u32, layout: &Layout) -> u32 {
    if entry == 0 {
        EXFAT_MEDIA
    } else if entry == 1 || entry == layout.upcase_cluster || entry == layout.root_cluster {
        EXFAT_EOC
    } else {
        0
    }
}

fn write_bitmap(device_id: u32, layout: &Layout) -> Result<(), &'static str> {
    let bitmap_bytes = div_ceil(layout.cluster_count, 8) as usize;
    let mut bitmap = vec![0u8; bitmap_bytes.max(1)];
    for cluster in layout.bitmap_cluster..=layout.root_cluster {
        mark_allocated(&mut bitmap, cluster)?;
    }

    for idx in 0..layout.bitmap_clusters {
        let start = idx as usize * CLUSTER_SIZE;
        let end = (start + CLUSTER_SIZE).min(bitmap.len());
        let mut cluster_data = vec![0u8; CLUSTER_SIZE];
        if start < end {
            cluster_data[..end - start].copy_from_slice(&bitmap[start..end]);
        }
        write_cluster(
            device_id,
            layout,
            layout.bitmap_cluster + idx,
            &cluster_data,
        )?;
    }
    Ok(())
}

fn mark_allocated(bitmap: &mut [u8], cluster: u32) -> Result<(), &'static str> {
    if cluster < 2 {
        return Err("ungueltiger Cluster");
    }
    let bit = (cluster - 2) as usize;
    let byte = bit / 8;
    if byte >= bitmap.len() {
        return Err("Bitmap zu klein");
    }
    bitmap[byte] |= 1 << (bit % 8);
    Ok(())
}

fn write_upcase(device_id: u32, layout: &Layout) -> Result<u32, &'static str> {
    let mut upcase = vec![0u8; CLUSTER_SIZE];
    for i in 0u16..128 {
        let upper = if (0x61..=0x7A).contains(&i) {
            i - 0x20
        } else {
            i
        };
        write_le16(&mut upcase, i as usize * 2, upper);
    }

    let mut checksum = 0u32;
    for b in &upcase[..256] {
        checksum = checksum.rotate_right(1).wrapping_add(*b as u32);
    }
    write_cluster(device_id, layout, layout.upcase_cluster, &upcase)?;
    Ok(checksum)
}

fn write_root(
    device_id: u32,
    layout: &Layout,
    upcase_checksum: u32,
    label: &str,
) -> Result<(), &'static str> {
    let mut root = vec![0u8; CLUSTER_SIZE];

    root[0] = 0x81;
    root[1] = 0;
    write_le32(&mut root, 20, layout.bitmap_cluster);
    write_le64(&mut root, 24, div_ceil(layout.cluster_count, 8) as u64);

    let upcase = 32usize;
    root[upcase] = 0x82;
    write_le32(&mut root, upcase + 4, upcase_checksum);
    write_le32(&mut root, upcase + 20, layout.upcase_cluster);
    write_le64(&mut root, upcase + 24, 256);

    let label_entry = 64usize;
    let label_bytes = label.as_bytes();
    root[label_entry] = 0x83;
    root[label_entry + 1] = label_bytes.len().min(11) as u8;
    for (i, b) in label_bytes.iter().take(11).enumerate() {
        write_le16(&mut root, label_entry + 2 + i * 2, *b as u16);
    }

    write_cluster(device_id, layout, layout.root_cluster, &root)
}

fn write_boot_regions(device_id: u32, layout: &Layout) -> Result<(), &'static str> {
    let mut sectors = vec![0u8; 12 * SECTOR_SIZE as usize];
    build_main_boot_sector(&mut sectors[..SECTOR_SIZE as usize], layout);

    for idx in 1..=8usize {
        sectors[idx * 512 + 510] = 0x55;
        sectors[idx * 512 + 511] = 0xAA;
    }

    let checksum = boot_checksum(&sectors[..11 * SECTOR_SIZE as usize]);
    for i in 0..128usize {
        write_le32(&mut sectors[11 * 512..12 * 512], i * 4, checksum);
    }

    for base in [0u32, 12u32] {
        for s in 0..12u32 {
            let off = s as usize * SECTOR_SIZE as usize;
            let mut sector = [0u8; SECTOR_SIZE as usize];
            sector.copy_from_slice(&sectors[off..off + SECTOR_SIZE as usize]);
            write_sector(device_id, (base + s) as u64, &sector)?;
        }
    }
    Ok(())
}

fn build_main_boot_sector(sector: &mut [u8], layout: &Layout) {
    sector.fill(0);
    sector[0] = 0xEB;
    sector[1] = 0x76;
    sector[2] = 0x90;
    sector[3..11].copy_from_slice(b"EXFAT   ");
    write_le64(sector, 64, 0);
    write_le64(sector, 72, layout.fs_sectors);
    write_le32(sector, 80, FAT_OFFSET);
    write_le32(sector, 84, layout.fat_length);
    write_le32(sector, 88, layout.cluster_heap_offset);
    write_le32(sector, 92, layout.cluster_count);
    write_le32(sector, 96, layout.root_cluster);
    write_le32(sector, 100, 0x414E_594F);
    write_le16(sector, 104, 0x0100);
    write_le16(sector, 106, 0);
    sector[108] = BYTES_PER_SECTOR_SHIFT;
    sector[109] = SECTORS_PER_CLUSTER_SHIFT;
    sector[110] = 1;
    sector[111] = 0x80;
    sector[112] = 0xFF;
    sector[510] = 0x55;
    sector[511] = 0xAA;
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

fn write_cluster(
    device_id: u32,
    layout: &Layout,
    cluster: u32,
    data: &[u8],
) -> Result<(), &'static str> {
    if data.len() != CLUSTER_SIZE {
        return Err("cluster write size");
    }
    let lba = layout.cluster_heap_offset as u64 + (cluster - 2) as u64 * SECTORS_PER_CLUSTER as u64;
    for s in 0..SECTORS_PER_CLUSTER {
        let off = s as usize * SECTOR_SIZE as usize;
        let mut sector = [0u8; SECTOR_SIZE as usize];
        sector.copy_from_slice(&data[off..off + SECTOR_SIZE as usize]);
        write_sector(device_id, lba + s as u64, &sector)?;
    }
    Ok(())
}

fn write_sector(device_id: u32, lba: u64, sector: &[u8; 512]) -> Result<(), &'static str> {
    let rc = sys::disk_write(device_id, lba, 1, sector);
    if rc == 1 {
        Ok(())
    } else {
        Err("disk_write fehlgeschlagen")
    }
}

fn div_ceil(n: u32, d: u32) -> u32 {
    (n + d - 1) / d
}

fn read_le64(buf: &[u8], off: usize) -> u64 {
    let mut value = 0u64;
    for i in 0..8 {
        value |= (buf[off + i] as u64) << (i * 8);
    }
    value
}

fn write_le16(buf: &mut [u8], off: usize, val: u16) {
    buf[off] = val as u8;
    buf[off + 1] = (val >> 8) as u8;
}

fn write_le32(buf: &mut [u8], off: usize, val: u32) {
    for i in 0..4 {
        buf[off + i] = (val >> (i * 8)) as u8;
    }
}

fn write_le64(buf: &mut [u8], off: usize, val: u64) {
    for i in 0..8 {
        buf[off + i] = (val >> (i * 8)) as u8;
    }
}
