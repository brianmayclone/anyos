use alloc::vec;
use alloc::vec::Vec;

/// File name size in table-loader commands (matches QEMU BIOS_LINKER_LOADER_FILESZ).
const FILESZ: usize = 56;

fn make_name(s: &str) -> [u8; FILESZ] {
    let mut buf = [0u8; FILESZ];
    let n = s.len().min(FILESZ - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf
}

const TABLES_NAME: &str = "etc/acpi/tables";
const RSDP_NAME: &str = "etc/acpi/rsdp";

fn write_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
    let b = val.to_le_bytes();
    buf[offset] = b[0];
    buf[offset + 1] = b[1];
}

fn write_u32(buf: &mut Vec<u8>, offset: usize, val: u32) {
    let b = val.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&b);
}

fn write_acpi_header(buf: &mut Vec<u8>, offset: usize, sig: &[u8; 4], length: u32, revision: u8) {
    buf[offset..offset + 4].copy_from_slice(sig);
    write_u32(buf, offset + 4, length);
    buf[offset + 8] = revision;
    // [9] checksum = 0 (loader patches it)
    buf[offset + 10..offset + 16].copy_from_slice(b"ANYOS\0");
    buf[offset + 16..offset + 24].copy_from_slice(b"ANYOSTBL");
    write_u32(buf, offset + 24, 1); // OEM revision
    buf[offset + 28..offset + 32].copy_from_slice(b"ANYS");
    write_u32(buf, offset + 32, 1); // creator revision
}

fn build_rsdp() -> Vec<u8> {
    let mut r = vec![0u8; 36];
    r[0..8].copy_from_slice(b"RSD PTR ");
    // [8] checksum = 0, patched by loader
    r[9..15].copy_from_slice(b"ANYOS\0");
    r[15] = 0; // revision 0 = ACPI 1.0
    // [16..20] RSDT address = 0, patched by loader
    r
}

fn build_rsdt() -> Vec<u8> {
    let mut t = vec![0u8; 44];
    write_acpi_header(&mut t, 0, b"RSDT", 44, 1);
    // [36..40] FADT pointer, [40..44] MADT pointer — patched by loader
    t
}

fn build_fadt() -> Vec<u8> {
    let mut t = vec![0u8; 116];
    write_acpi_header(&mut t, 0, b"FACP", 116, 1);
    // [36] FIRMWARE_CTRL, [40] DSDT — patched by loader
    t[44] = 1; // INT_MODEL
    // SCI_INT = 9
    write_u16(&mut t, 46, 9);
    // PM1a_EVT_BLK
    write_u32(&mut t, 56, 0xB000);
    // PM1a_CNT_BLK
    write_u32(&mut t, 64, 0xB004);
    // PM_TMR_BLK
    write_u32(&mut t, 76, 0xB008);
    // GPE0_BLK
    write_u32(&mut t, 80, 0xB020);
    // PM1_EVT_LEN
    t[88] = 4;
    // PM1_CNT_LEN
    t[89] = 2;
    // PM_TMR_LEN
    t[91] = 4;
    // GPE0_BLK_LEN
    t[92] = 4;
    // P_LVL2_LAT
    write_u16(&mut t, 96, 0x0065);
    // P_LVL3_LAT
    write_u16(&mut t, 98, 0x03E9);
    // IAPC_BOOT_ARCH
    write_u16(&mut t, 109, 0x0003);
    // FLAGS
    write_u32(&mut t, 112, 0x000000A5);
    t
}

fn build_facs() -> Vec<u8> {
    let mut t = vec![0u8; 64];
    t[0..4].copy_from_slice(b"FACS");
    write_u32(&mut t, 4, 64);
    t
}

fn build_dsdt() -> Vec<u8> {
    let mut t = vec![0u8; 36];
    write_acpi_header(&mut t, 0, b"DSDT", 36, 1);
    t
}

fn build_madt() -> Vec<u8> {
    let len: u32 = 114;
    let mut t = vec![0u8; len as usize];
    write_acpi_header(&mut t, 0, b"APIC", len, 1);
    // Local APIC address
    write_u32(&mut t, 36, 0xFEE0_0000);
    // Flags (PCAT_COMPAT)
    write_u32(&mut t, 40, 1);

    let mut off = 44;
    // Local APIC entry (type=0, len=8)
    t[off] = 0;
    t[off + 1] = 8;
    t[off + 2] = 0; // ACPI processor ID
    t[off + 3] = 0; // APIC ID
    write_u32(&mut t, off + 4, 1); // flags: enabled
    off += 8;

    // IOAPIC entry (type=1, len=12)
    t[off] = 1;
    t[off + 1] = 12;
    t[off + 2] = 0; // IOAPIC ID
    write_u32(&mut t, off + 4, 0xFEC0_0000);
    write_u32(&mut t, off + 8, 0); // GSI base
    off += 12;

    // Interrupt Source Overrides (type=2, len=10)
    let overrides: &[(u8, u32, u16)] = &[
        (0, 2, 0x0000),
        (5, 5, 0x000D),
        (9, 9, 0x000D),
        (10, 10, 0x000D),
        (11, 11, 0x000D),
    ];
    for &(source, gsi, flags) in overrides {
        t[off] = 2;
        t[off + 1] = 10;
        t[off + 2] = 0; // bus = ISA
        t[off + 3] = source;
        write_u32(&mut t, off + 4, gsi);
        write_u16(&mut t, off + 8, flags);
        off += 10;
    }

    t
}

// ── Table-loader command builders ───────────────────────────────────────────
// Each command is 128 bytes. Layout matches QEMU's BiosLinkerLoaderEntry:
//   [0..4]   command type (LE u32)
//   [4..128] union payload (124 bytes)
//
// ALLOCATE (cmd=1):  [4..60] file(56), [60..64] align(u32), [64] zone(u8)
// ADD_POINTER (cmd=2): [4..60] dest(56), [60..116] src(56), [116..120] offset(u32), [120] size(u8)
// ADD_CHECKSUM (cmd=3): [4..60] file(56), [60..64] offset(u32), [64..68] start(u32), [68..72] length(u32)

fn loader_allocate(file: &[u8; FILESZ], align: u32, zone: u8) -> [u8; 128] {
    let mut cmd = [0u8; 128];
    cmd[0..4].copy_from_slice(&1u32.to_le_bytes());
    cmd[4..60].copy_from_slice(file);
    cmd[60..64].copy_from_slice(&align.to_le_bytes());
    cmd[64] = zone;
    cmd
}

fn loader_add_pointer(dest: &[u8; FILESZ], src: &[u8; FILESZ], offset: u32, size: u8) -> [u8; 128] {
    let mut cmd = [0u8; 128];
    cmd[0..4].copy_from_slice(&2u32.to_le_bytes());
    cmd[4..60].copy_from_slice(dest);
    cmd[60..116].copy_from_slice(src);
    cmd[116..120].copy_from_slice(&offset.to_le_bytes());
    cmd[120] = size;
    cmd
}

fn loader_add_checksum(file: &[u8; FILESZ], offset: u32, start: u32, length: u32) -> [u8; 128] {
    let mut cmd = [0u8; 128];
    cmd[0..4].copy_from_slice(&3u32.to_le_bytes());
    cmd[4..60].copy_from_slice(file);
    cmd[60..64].copy_from_slice(&offset.to_le_bytes());
    cmd[64..68].copy_from_slice(&start.to_le_bytes());
    cmd[68..72].copy_from_slice(&length.to_le_bytes());
    cmd
}

/// Generate ACPI tables for SeaBIOS fw_cfg.
/// Returns (rsdp_data, tables_data, loader_data).
pub fn generate_acpi_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let tables_file = make_name(TABLES_NAME);
    let rsdp_file = make_name(RSDP_NAME);

    let rsdp = build_rsdp();

    let rsdt = build_rsdt();
    let fadt = build_fadt();
    let facs = build_facs();
    let dsdt = build_dsdt();
    let madt = build_madt();

    let rsdt_off: u32 = 0;
    let fadt_off = rsdt.len() as u32;
    let facs_off = fadt_off + fadt.len() as u32;
    let dsdt_off = facs_off + facs.len() as u32;
    let madt_off = dsdt_off + dsdt.len() as u32;
    let madt_len = madt.len() as u32;

    let mut tables = Vec::with_capacity((madt_off + madt_len) as usize);
    tables.extend_from_slice(&rsdt);
    tables.extend_from_slice(&fadt);
    tables.extend_from_slice(&facs);
    tables.extend_from_slice(&dsdt);
    tables.extend_from_slice(&madt);

    // Pre-fill pointer fields with intra-buffer offsets.
    // ADD_POINTER adds the allocated base address of src to the u32 at dest[offset].
    // RSDT -> FADT
    write_u32(&mut tables, (rsdt_off + 36) as usize, fadt_off);
    // RSDT -> MADT
    write_u32(&mut tables, (rsdt_off + 40) as usize, madt_off);
    // FADT -> FACS
    write_u32(&mut tables, (fadt_off + 36) as usize, facs_off);
    // FADT -> DSDT
    write_u32(&mut tables, (fadt_off + 40) as usize, dsdt_off);

    let mut loader = Vec::new();
    let mut emit = |cmd: [u8; 128]| loader.extend_from_slice(&cmd);

    // 1. Allocate tables (HIGH = zone 1) and rsdp (FSEG = zone 2, scannable by OS)
    emit(loader_allocate(&tables_file, 64, 1));
    emit(loader_allocate(&rsdp_file, 16, 2));

    // 2. RSDP -> RSDT pointer
    emit(loader_add_pointer(&rsdp_file, &tables_file, 16, 4));
    emit(loader_add_checksum(&rsdp_file, 8, 0, 20));

    // 3. FADT -> FACS, FADT -> DSDT
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 36, 4));
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 40, 4));
    emit(loader_add_checksum(&tables_file, fadt_off + 9, fadt_off, 116));

    // 4. RSDT -> FADT, RSDT -> MADT
    emit(loader_add_pointer(&tables_file, &tables_file, rsdt_off + 36, 4));
    emit(loader_add_pointer(&tables_file, &tables_file, rsdt_off + 40, 4));
    emit(loader_add_checksum(&tables_file, rsdt_off + 9, rsdt_off, 44));

    // 5. DSDT and MADT checksums
    emit(loader_add_checksum(&tables_file, dsdt_off + 9, dsdt_off, 36));
    emit(loader_add_checksum(&tables_file, madt_off + 9, madt_off, madt_len));

    (rsdp, tables, loader)
}
