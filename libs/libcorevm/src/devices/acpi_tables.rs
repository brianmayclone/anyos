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

fn write_u64(buf: &mut Vec<u8>, offset: usize, val: u64) {
    let b = val.to_le_bytes();
    buf[offset..offset + 8].copy_from_slice(&b);
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

/// Build ACPI 2.0 RSDP (36 bytes).
fn build_rsdp() -> Vec<u8> {
    let mut r = vec![0u8; 36];
    r[0..8].copy_from_slice(b"RSD PTR ");
    // [8] checksum (bytes 0..20), patched by loader
    r[9..15].copy_from_slice(b"ANYOS\0");
    r[15] = 2; // revision 2 = ACPI 2.0+
    // [16..20] RSDT address = 0, patched by loader
    // [20..24] length = 36
    r[20..24].copy_from_slice(&36u32.to_le_bytes());
    // [24..32] XSDT address = 0, patched by loader
    // [32] extended checksum (bytes 0..36), patched by loader
    // [33..36] reserved
    r
}

/// Build RSDT with 32-bit pointers (36 + N*4 bytes).
fn build_rsdt(num_tables: u32) -> Vec<u8> {
    let len = 36 + num_tables * 4;
    let mut t = vec![0u8; len as usize];
    write_acpi_header(&mut t, 0, b"RSDT", len, 1);
    t
}

/// Build XSDT with 64-bit pointers (36 + N*8 bytes).
fn build_xsdt(num_tables: u32) -> Vec<u8> {
    let len = 36 + num_tables * 8;
    let mut t = vec![0u8; len as usize];
    write_acpi_header(&mut t, 0, b"XSDT", len, 1);
    t
}

/// Write a Generic Address Structure (GAS, 12 bytes) for system I/O space.
fn write_gas_io(buf: &mut Vec<u8>, offset: usize, addr: u64, bit_width: u8) {
    buf[offset] = 1; // address_space_id = System I/O
    buf[offset + 1] = bit_width;
    buf[offset + 2] = 0; // register_bit_offset
    buf[offset + 3] = if bit_width == 32 { 3 } else if bit_width == 16 { 2 } else { 1 }; // access_size
    write_u64(buf, offset + 4, addr);
}

/// Build FADT revision 3 (ACPI 2.0), 244 bytes.
fn build_fadt() -> Vec<u8> {
    let mut t = vec![0u8; 244];
    write_acpi_header(&mut t, 0, b"FACP", 244, 3);
    // [36] FIRMWARE_CTRL (u32), patched by loader
    // [40] DSDT (u32), patched by loader
    t[45] = 0; // Preferred_PM_Profile = unspecified
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
    // IAPC_BOOT_ARCH (8042, legacy devices)
    write_u16(&mut t, 109, 0x0003);
    // FLAGS: WBINVD(0) | PROC_C1(2) | SLP_BUTTON(5) | RTC_S4(7) | TMR_VAL_EXT(8)
    write_u32(&mut t, 112, 0x000001A5);

    // ACPI 2.0 extended fields (offset 132+)
    // X_FIRMWARE_CTRL (u64 at offset 132), patched by loader
    // X_DSDT (u64 at offset 140), patched by loader

    // Extended GAS fields for PM registers
    // X_PM1a_EVT_BLK (offset 148, 12 bytes)
    write_gas_io(&mut t, 148, 0xB000, 32);
    // X_PM1a_CNT_BLK (offset 172, 12 bytes)
    write_gas_io(&mut t, 172, 0xB004, 16);
    // X_PM_TMR_BLK (offset 208, 12 bytes)
    write_gas_io(&mut t, 208, 0xB008, 32);
    // X_GPE0_BLK (offset 220, 12 bytes)
    write_gas_io(&mut t, 220, 0xB020, 32);

    t
}

fn build_facs() -> Vec<u8> {
    let mut t = vec![0u8; 64];
    t[0..4].copy_from_slice(b"FACS");
    write_u32(&mut t, 4, 64);
    t
}

/// Build AML for a simple ISA device: Name(_HID, EisaId(id))
/// Plus _CRS resource template with I/O range and IRQ.
fn aml_isa_device(name: &[u8; 4], eisa_id: [u8; 4], io_min: u16, io_len: u8, irq: Option<u8>) -> Vec<u8> {
    let mut dev = Vec::new();
    // Name(_HID, EisaId(...))
    dev.extend_from_slice(&[0x08]); // NameOp
    dev.extend_from_slice(b"_HID");
    dev.extend_from_slice(&[0x0C]); // DWordPrefix
    dev.extend_from_slice(&eisa_id);

    // _CRS resource template
    let mut crs_body = Vec::new();
    // I/O descriptor (small): tag=0x47, length=7
    crs_body.extend_from_slice(&[0x47, 0x01]); // I/O port descriptor, decode 16-bit
    crs_body.extend_from_slice(&io_min.to_le_bytes()); // _MIN
    crs_body.extend_from_slice(&io_min.to_le_bytes()); // _MAX
    crs_body.push(0x01); // _ALN (alignment)
    crs_body.push(io_len); // _LEN
    // IRQ descriptor (small): tag=0x22/0x23, length=2
    if let Some(irq_num) = irq {
        let irq_mask: u16 = 1 << irq_num;
        crs_body.push(0x22); // IRQ descriptor
        crs_body.extend_from_slice(&irq_mask.to_le_bytes());
    }
    // End tag
    crs_body.extend_from_slice(&[0x79, 0x00]);

    // Name(_CRS, ResourceTemplate() { ... })
    dev.extend_from_slice(&[0x08]); // NameOp
    dev.extend_from_slice(b"_CRS");
    // Buffer: BufferOp(0x11) PkgLength BufferSize data
    // body after PkgLength = BytePrefix(1) + size(1) + crs_body
    let buf_body_len = 2 + crs_body.len();
    dev.push(0x11); // BufferOp
    aml_push_pkg_len(&mut dev, buf_body_len);
    dev.push(0x0A); // BytePrefix
    dev.push(crs_body.len() as u8);
    dev.extend_from_slice(&crs_body);

    // Wrap in Device(name): DeviceOp(5B 82) PkgLength NameSeg body
    // body after PkgLength = name(4) + dev contents
    let mut out = Vec::new();
    out.extend_from_slice(&[0x5B, 0x82]); // DeviceOp
    let dev_body_len = 4 + dev.len();
    aml_push_pkg_len(&mut out, dev_body_len);
    out.extend_from_slice(name);
    out.extend_from_slice(&dev);
    out
}

/// Compute the number of bytes needed for a PkgLength field that encodes
/// a total length (= PkgLength bytes + body bytes).  `body_len` is the size
/// of everything that comes *after* the PkgLength field.
fn aml_pkg_len_size(body_len: usize) -> usize {
    // Try each encoding width; the total includes the PkgLength field itself.
    if body_len + 1 < 0x40 { return 1; }
    if body_len + 2 < 0x1000 { return 2; }
    if body_len + 3 < 0x100000 { return 3; }
    4
}

/// Push an AML PkgLength encoding for a block whose body (everything after
/// PkgLength) is `body_len` bytes.
fn aml_push_pkg_len(buf: &mut Vec<u8>, body_len: usize) {
    let n = aml_pkg_len_size(body_len);
    let total = body_len + n;
    match n {
        1 => {
            buf.push(total as u8);
        }
        2 => {
            buf.push(0x40 | (total & 0x0F) as u8);
            buf.push((total >> 4) as u8);
        }
        3 => {
            buf.push(0x80 | (total & 0x0F) as u8);
            buf.push((total >> 4) as u8);
            buf.push((total >> 12) as u8);
        }
        _ => {
            buf.push(0xC0 | (total & 0x0F) as u8);
            buf.push((total >> 4) as u8);
            buf.push((total >> 12) as u8);
            buf.push((total >> 20) as u8);
        }
    }
}

/// EISA ID encoding: 3-char compressed + 4-hex product ID
/// E.g., "PNP0A03" → [0x41, 0xD0, 0x0A, 0x03]
fn eisa_id(s: &str) -> [u8; 4] {
    let b = s.as_bytes();
    let c1 = (b[0] - b'@') & 0x1F;
    let c2 = (b[1] - b'@') & 0x1F;
    let c3 = (b[2] - b'@') & 0x1F;
    let prod = u16::from_str_radix(&s[3..7], 16).unwrap_or(0);
    [
        (c1 << 2) | (c2 >> 3),
        (c2 << 5) | c3,
        (prod >> 8) as u8,
        prod as u8,
    ]
}

fn build_dsdt_with_cpus(num_cpus: u32, devices: &AcpiDeviceConfig) -> Vec<u8> {
    let num_cpus = num_cpus.max(1).min(32);
    // Build comprehensive DSDT with ISA devices and PCI interrupt routing.
    // Devices:
    //   \_SB_.PCI0       PNP0A03  PCI root bridge
    //   \_SB_.PCI0.ISA_  PNP0A00  ISA bridge (under PCI bus)
    //   \_SB_.PCI0.ISA_.KBD  PNP0303  PS/2 keyboard (IRQ 1)
    //   \_SB_.PCI0.ISA_.MOU  PNP0F13  PS/2 mouse (IRQ 12)
    //   \_SB_.PCI0.ISA_.COM1 PNP0501  Serial port COM1 (IRQ 4)
    //   \_SB_.PCI0.ISA_.RTC0 PNP0B00  RTC/CMOS (IRQ 8)
    //   \_SB_.PCI0.ISA_.TIMR PNP0100  System timer (IRQ 0)
    //   \_SB_.PCI0.ISA_.SPKR PNP0800  Speaker
    //   \_SB_.PCI0._PRT  PCI interrupt routing table
    //   \_PIC method     Interrupt mode switching (PIC/APIC/SAPIC)
    //   \_S5_ sleep object  Soft-off power state
    //   \_PR_.CPU0       Processor object

    let mut aml = Vec::new();

    // ── Global \_PIC method ──
    // Windows calls \_PIC(n) during HAL init to switch interrupt mode:
    //   0 = PIC mode, 1 = APIC mode, 2 = SAPIC mode
    // Method(\_PIC, 1) { Store(Arg0, PICM) }
    // We also need: Name(PICM, 0)
    {
        // Name(PICM, Zero) — global variable for interrupt mode
        aml.push(0x08); // NameOp
        aml.extend_from_slice(b"PICM");
        aml.push(0x00); // Zero (initial value)

        // Method(\_PIC, 1, NotSerialized) { Store(Arg0, PICM) }
        let mut method_body = Vec::new();
        // Store(Arg0, PICM)
        method_body.push(0x70); // StoreOp
        method_body.push(0x68); // Arg0
        method_body.extend_from_slice(b"PICM"); // target = PICM
        // Wrap: MethodOp(0x14) PkgLength NameSeg MethodFlags body
        aml.push(0x14); // MethodOp
        let m_body_len = 4 + 1 + method_body.len(); // name(4) + flags(1) + body
        aml_push_pkg_len(&mut aml, m_body_len);
        aml.extend_from_slice(b"_PIC");
        aml.push(0x01); // MethodFlags: ArgCount=1, NotSerialized
        aml.extend_from_slice(&method_body);
    }

    // ── \_S5_ (Soft Off) sleep object ──
    // Name(\_S5_, Package(4) { 0, 0, 0, 0 })
    // Windows needs this for shutdown support.
    {
        aml.push(0x08); // NameOp
        aml.extend_from_slice(b"_S5_");
        aml.push(0x12); // PackageOp
        // Body: NumElements(1) + BytePrefix+5(2) + BytePrefix+5(2) + Zero(1) + Zero(1) = 7
        aml_push_pkg_len(&mut aml, 7);
        aml.push(4);    // NumElements
        aml.push(0x0A); aml.push(0x05); // BytePrefix + SLP_TYP_S5 = 5
        aml.push(0x0A); aml.push(0x05); // BytePrefix + SLP_TYP_S5 = 5
        aml.push(0x00); // Zero
        aml.push(0x00); // Zero
    }

    // ── Build \_SB_ scope contents ──

    // Build ISA device children
    let kbd  = aml_isa_device(b"KBD_", eisa_id("PNP0303"), 0x0060, 1, Some(1));
    let mou  = aml_isa_device(b"MOU_", eisa_id("PNP0F13"), 0x0060, 1, Some(12));
    let com1 = aml_isa_device(b"COM1", eisa_id("PNP0501"), 0x03F8, 8, Some(4));
    let rtc  = aml_isa_device(b"RTC0", eisa_id("PNP0B00"), 0x0070, 2, Some(8));
    let timr = aml_isa_device(b"TIMR", eisa_id("PNP0100"), 0x0040, 4, Some(0));
    let spkr = aml_isa_device(b"SPKR", eisa_id("PNP0800"), 0x0061, 1, None);

    // ISA bridge device (contains all ISA children)
    let mut isa_inner = Vec::new();
    // Name(_HID, EisaId("PNP0A00"))
    isa_inner.extend_from_slice(&[0x08]); // NameOp
    isa_inner.extend_from_slice(b"_HID");
    isa_inner.extend_from_slice(&[0x0C]); // DWordPrefix
    isa_inner.extend_from_slice(&eisa_id("PNP0A00"));
    isa_inner.extend_from_slice(&kbd);
    isa_inner.extend_from_slice(&mou);
    isa_inner.extend_from_slice(&com1);
    isa_inner.extend_from_slice(&rtc);
    isa_inner.extend_from_slice(&timr);
    isa_inner.extend_from_slice(&spkr);

    let mut isa_dev = Vec::new();
    isa_dev.extend_from_slice(&[0x5B, 0x82]); // DeviceOp
    let isa_body_len = 4 + isa_inner.len();
    aml_push_pkg_len(&mut isa_dev, isa_body_len);
    isa_dev.extend_from_slice(b"ISA_");
    isa_dev.extend_from_slice(&isa_inner);

    // PCI0 interrupt routing table _PRT
    // Maps PCI device pins to IOAPIC GSI numbers.
    // Our PCI topology:
    //   00:00.0 = i440FX host bridge
    //   00:01.0 = PIIX3 ISA bridge
    //   00:02.0 = VGA (doesn't use IRQ)
    //   00:03.0 = AHCI (IRQ 11)
    //
    // _PRT format: Package { Package { address, pin, source, source_index } ... }
    // With IOAPIC, source=0 and source_index=GSI number.
    let mut prt = Vec::new();

    // Helper: one _PRT entry: Package(4) { DWord addr, Byte pin, Zero, Byte gsi }
    fn prt_entry(buf: &mut Vec<u8>, dev: u32, pin: u8, gsi: u8) {
        buf.push(0x12); // PackageOp
        // Body after PkgLength: NumElements(1) + DWordPrefix(1) + DWord(4) +
        //   BytePrefix(1) + pin(1) + Zero(1) + BytePrefix(1) + gsi(1) = 11
        buf.push(12);   // PkgLength (= 1 + 11 body bytes)
        buf.push(4);    // NumElements
        // Address (u64 encoded as DWord high + DWord low... actually just 32-bit)
        buf.push(0x0C); // DWordPrefix
        buf.extend_from_slice(&((dev << 16) as u32 | 0xFFFF).to_le_bytes());
        buf.push(0x0A); buf.push(pin);  // BytePrefix + pin
        buf.push(0x00); // Zero (source name = none, use GSI)
        buf.push(0x0A); buf.push(gsi);  // BytePrefix + GSI
    }

    // PCI interrupt routing — must match PIIX3 PIRQ register values.
    // ACPI _PRT pin is 0-based (0=INTA,1=INTB,2=INTC,3=INTD).
    // PIIX3 swizzle: PIRQ index = (device_slot + pin - 1) % 4
    //   PIRQA(0x60)=10, PIRQB(0x61)=5, PIRQC(0x62)=11, PIRQD(0x63)=11
    let mut num_prt_entries = 0u8;

    // Dev 2 (VGA): always present, INTA → PIRQC → GSI 11
    prt_entry(&mut prt, 2, 0, 11);
    num_prt_entries += 1;

    // Dev 3 (AHCI): always present, INTA → PIRQD → GSI 11
    prt_entry(&mut prt, 3, 0, 11);
    num_prt_entries += 1;

    // Dev 4 (E1000): INTA → GSI 11 (shared with AHCI)
    if devices.has_e1000 {
        prt_entry(&mut prt, 4, 0, 11);
        num_prt_entries += 1;
    }

    // Dev 5 (AC97): INTA → PIRQB → GSI 5
    if devices.has_ac97 {
        prt_entry(&mut prt, 5, 0, 5);
        num_prt_entries += 1;
    }

    // Dev 6 (UHCI): INTD → PIRQB → GSI 5
    if devices.has_uhci {
        prt_entry(&mut prt, 6, 3, 5);
        num_prt_entries += 1;
    }

    // Wrap in Package: PackageOp PkgLength NumElements entries...
    let mut prt_pkg = Vec::new();
    prt_pkg.push(0x12); // PackageOp
    let prt_body_len = 1 + prt.len(); // numElements + entries
    aml_push_pkg_len(&mut prt_pkg, prt_body_len);
    prt_pkg.push(num_prt_entries); // NumElements
    prt_pkg.extend_from_slice(&prt);

    // Name(_PRT, Package() { ... })
    let mut prt_name = Vec::new();
    prt_name.push(0x08); // NameOp
    prt_name.extend_from_slice(b"_PRT");
    prt_name.extend_from_slice(&prt_pkg);

    // PCI0 device
    let mut pci0_inner = Vec::new();
    // Name(_HID, EisaId("PNP0A03"))
    pci0_inner.extend_from_slice(&[0x08]); // NameOp
    pci0_inner.extend_from_slice(b"_HID");
    pci0_inner.extend_from_slice(&[0x0C, 0x41, 0xD0, 0x0A, 0x03]);
    // Name(_UID, 0)
    pci0_inner.extend_from_slice(&[0x08]); // NameOp
    pci0_inner.extend_from_slice(b"_UID");
    pci0_inner.push(0x00);
    // Name(_BBN, 0)
    pci0_inner.extend_from_slice(&[0x08]); // NameOp
    pci0_inner.extend_from_slice(b"_BBN");
    pci0_inner.push(0x00);

    // _CRS — PCI root bridge resource descriptors (bus range, I/O, memory)
    // Windows needs this to enumerate PCI devices.
    {
        let mut crs_body = Vec::new();

        // Word Bus Number descriptor: bus 0-255
        // WordBusNumber(ResourceProducer, MinFixed, MaxFixed, , 0, 0, 0xFF, 0, 0x100)
        // Large resource: type=0x88, length=0x0D (13 = restype+genflags+typeflags+5*u16)
        crs_body.extend_from_slice(&[0x88, 0x0D, 0x00]); // tag + length(13) LE
        crs_body.push(0x02); // ResourceType=2 (bus range)
        crs_body.push(0x0C); // General Flags: MinFixed | MaxFixed
        crs_body.push(0x00); // Type Specific Flags (none for bus number)
        crs_body.extend_from_slice(&0u16.to_le_bytes()); // _GRA (granularity)
        crs_body.extend_from_slice(&0u16.to_le_bytes()); // _MIN (bus 0)
        crs_body.extend_from_slice(&0xFFu16.to_le_bytes()); // _MAX (bus 255)
        crs_body.extend_from_slice(&0u16.to_le_bytes()); // _TRA (translation)
        crs_body.extend_from_slice(&0x100u16.to_le_bytes()); // _LEN (256 buses)

        // DWord I/O range: 0x0000 - 0x0CF7
        // DWordIO(ResourceProducer, MinFixed, MaxFixed, PosDecode, EntireRange, 0, 0, 0x0CF7, 0, 0x0CF8)
        crs_body.extend_from_slice(&[0x87, 0x17, 0x00]); // tag + length(23) LE
        crs_body.push(0x01); // ResourceType=1 (I/O)
        crs_body.push(0x0C); // MinFixed | MaxFixed, PosDecode
        crs_body.push(0x03); // ISA+non-ISA ranges
        crs_body.extend_from_slice(&0u32.to_le_bytes()); // _GRA
        crs_body.extend_from_slice(&0u32.to_le_bytes()); // _MIN
        crs_body.extend_from_slice(&0x0CF7u32.to_le_bytes()); // _MAX
        crs_body.extend_from_slice(&0u32.to_le_bytes()); // _TRA
        crs_body.extend_from_slice(&0x0CF8u32.to_le_bytes()); // _LEN

        // DWord I/O range: 0x0D00 - 0xFFFF (above PCI config)
        crs_body.extend_from_slice(&[0x87, 0x17, 0x00]);
        crs_body.push(0x01);
        crs_body.push(0x0C);
        crs_body.push(0x03);
        crs_body.extend_from_slice(&0u32.to_le_bytes());
        crs_body.extend_from_slice(&0x0D00u32.to_le_bytes());
        crs_body.extend_from_slice(&0xFFFFu32.to_le_bytes());
        crs_body.extend_from_slice(&0u32.to_le_bytes());
        crs_body.extend_from_slice(&0xF300u32.to_le_bytes());

        // DWord Memory range: 0xE0000000 - 0xFEBFFFFF (PCI MMIO)
        // DWordMemory(ResourceProducer, PosDecode, MinFixed, MaxFixed, NonCacheable, ReadWrite,
        //   0, 0xE0000000, 0xFEBFFFFF, 0, 0x0EC00000)
        crs_body.extend_from_slice(&[0x87, 0x17, 0x00]); // tag + length(23) LE
        crs_body.push(0x00); // ResourceType=0 (Memory)
        crs_body.push(0x0C); // MinFixed | MaxFixed
        crs_body.push(0x01); // ReadWrite
        crs_body.extend_from_slice(&0u32.to_le_bytes()); // _GRA
        crs_body.extend_from_slice(&0xE000_0000u32.to_le_bytes()); // _MIN
        crs_body.extend_from_slice(&0xFEBF_FFFFu32.to_le_bytes()); // _MAX
        crs_body.extend_from_slice(&0u32.to_le_bytes()); // _TRA
        crs_body.extend_from_slice(&0x0EC0_0000u32.to_le_bytes()); // _LEN

        // End tag
        crs_body.extend_from_slice(&[0x79, 0x00]);

        // Name(_CRS, ResourceTemplate() { ... })
        // Buffer: BufferOp(0x11) PkgLength BufferSize data
        // BufferSize prefix: BytePrefix(0x0A, 2 bytes) or WordPrefix(0x0B, 3 bytes)
        pci0_inner.extend_from_slice(&[0x08]); // NameOp
        pci0_inner.extend_from_slice(b"_CRS");
        pci0_inner.push(0x11); // BufferOp
        let size_prefix_len = if crs_body.len() < 64 { 2 } else { 3 };
        let buf_body_len = size_prefix_len + crs_body.len();
        aml_push_pkg_len(&mut pci0_inner, buf_body_len);
        if crs_body.len() < 64 {
            pci0_inner.push(0x0A); // BytePrefix
            pci0_inner.push(crs_body.len() as u8);
        } else {
            pci0_inner.push(0x0B); // WordPrefix
            pci0_inner.extend_from_slice(&(crs_body.len() as u16).to_le_bytes());
        }
        pci0_inner.extend_from_slice(&crs_body);
    }

    // _PRT
    pci0_inner.extend_from_slice(&prt_name);
    // ISA bridge
    pci0_inner.extend_from_slice(&isa_dev);

    let mut pci0_dev = Vec::new();
    pci0_dev.extend_from_slice(&[0x5B, 0x82]); // DeviceOp
    let pci0_body_len = 4 + pci0_inner.len();
    aml_push_pkg_len(&mut pci0_dev, pci0_body_len);
    pci0_dev.extend_from_slice(b"PCI0");
    pci0_dev.extend_from_slice(&pci0_inner);

    // Scope(\_SB_) { PCI0 device }
    let mut scope = Vec::new();
    scope.push(0x10); // ScopeOp
    let scope_body_len = 5 + pci0_dev.len(); // \_SB_ name (5 bytes: \ _SB_) + devices
    aml_push_pkg_len(&mut scope, scope_body_len);
    scope.extend_from_slice(&[0x5C, 0x5F, 0x53, 0x42, 0x5F]); // \_SB_
    scope.extend_from_slice(&pci0_dev);

    aml.extend_from_slice(&scope);

    // ── Processor objects: Scope(\_PR_) { Processor(CPU0..CPUN, ...) } ──
    // Windows/Linux need processor objects matching the MADT LAPIC entries.
    {
        let mut pr_inner = Vec::new();
        for cpu_id in 0..num_cpus {
            // Processor(CPUn, n, 0x00000000, 0x00)
            // ProcessorOp = 0x5B 0x83, PkgLength, NameSeg, ProcID(1), PblkAddr(4), PblkLen(1)
            pr_inner.extend_from_slice(&[0x5B, 0x83]); // ProcessorOp
            let proc_body_len = 4 + 1 + 4 + 1; // name + procID + PblkAddr + PblkLen
            aml_push_pkg_len(&mut pr_inner, proc_body_len);
            // Name: CPU0, CPU1, ... CPU9, CPUA, ...
            let digit = if cpu_id < 10 { b'0' + cpu_id as u8 } else { b'A' + (cpu_id - 10) as u8 };
            pr_inner.extend_from_slice(&[b'C', b'P', b'U', digit]);
            pr_inner.push(cpu_id as u8); // ProcID
            pr_inner.extend_from_slice(&0u32.to_le_bytes()); // PblkAddr = 0
            pr_inner.push(0x00); // PblkLen = 0
        }

        // Scope(\_PR_) { ... }
        let mut pr_scope = Vec::new();
        pr_scope.push(0x10); // ScopeOp
        let pr_scope_body_len = 5 + pr_inner.len();
        aml_push_pkg_len(&mut pr_scope, pr_scope_body_len);
        pr_scope.extend_from_slice(&[0x5C, 0x5F, 0x50, 0x52, 0x5F]); // \_PR_
        pr_scope.extend_from_slice(&pr_inner);
        aml.extend_from_slice(&pr_scope);
    }

    let total_len = 36 + aml.len();
    let mut t = vec![0u8; total_len];
    write_acpi_header(&mut t, 0, b"DSDT", total_len as u32, 2);
    t[36..].copy_from_slice(&aml);
    t
}

/// Build HPET ACPI table (38 bytes).
/// Describes the HPET at 0xFED00000 with hardware revision 1.
fn build_hpet_table() -> Vec<u8> {
    let len: u32 = 56; // ACPI header (36) + HPET-specific (20)
    let mut t = vec![0u8; len as usize];
    write_acpi_header(&mut t, 0, b"HPET", len, 1);

    // [36] Hardware Rev ID (from HPET General Capabilities register)
    t[36] = 0x01;
    // [37] Byte 1 of Event Timer Block ID:
    //   Bits [4:0] = NUM_COMPARATORS (bits 12:8 of dword) = 2 (3 timers)
    //   Bit 5 = COUNT_SIZE_CAP (bit 13 of dword) = 1 (64-bit)
    //   Bit 6 = reserved (bit 14)
    //   Bit 7 = LEG_RT_CAP (bit 15 of dword) = 1
    //   NOTE: This table is only included when --hpet is passed (Windows guests).
    //   Linux guests must NOT have HPET in ACPI tables because HPET Legacy
    //   Replacement mode disables PIT before our polled HPET timer can deliver
    //   interrupts, causing "IO-APIC + timer doesn't work!" kernel panic.
    t[37] = 2 | (1 << 5) | (1 << 7);
    // PCI vendor ID = 0x8086 (Intel)
    write_u16(&mut t, 38, 0x8086);

    // [40..52] Base Address Structure (GAS, 12 bytes) — Memory-mapped
    t[40] = 0;  // address_space_id = System Memory
    t[41] = 64; // register_bit_width
    t[42] = 0;  // register_bit_offset
    t[43] = 0;  // access_size (undefined for memory)
    write_u64(&mut t, 44, 0xFED0_0000); // HPET base address

    // [52] HPET number (sequence number for this HPET block)
    t[52] = 0;
    // [54..56] Minimum clock tick (in periodic mode, without losing interrupts)
    write_u16(&mut t, 54, 0x0080); // 128 ticks minimum

    t
}

fn build_madt_with_cpus(num_cpus: u32) -> Vec<u8> {
    let num_cpus = num_cpus.max(1).min(32);
    let len: u32 = 44 + (num_cpus * 8) + 12 + 40; // header + N*LAPIC + IOAPIC + 4*override
    let mut t = vec![0u8; len as usize];
    write_acpi_header(&mut t, 0, b"APIC", len, 3);
    // Local APIC address
    write_u32(&mut t, 36, 0xFEE0_0000);
    // Flags (PCAT_COMPAT)
    write_u32(&mut t, 40, 1);

    let mut off = 44;
    // Local APIC entries (type=0, len=8) — one per CPU
    for cpu_id in 0..num_cpus {
        t[off] = 0;      // type = Local APIC
        t[off + 1] = 8;  // length
        t[off + 2] = cpu_id as u8; // ACPI processor ID
        t[off + 3] = cpu_id as u8; // APIC ID
        write_u32(&mut t, off + 4, 1); // flags: enabled
        off += 8;
    }

    // IOAPIC entry (type=1, len=12)
    t[off] = 1;
    t[off + 1] = 12;
    t[off + 2] = 0; // IOAPIC ID
    write_u32(&mut t, off + 4, 0xFEC0_0000);
    write_u32(&mut t, off + 8, 0); // GSI base
    off += 12;

    // Interrupt Source Overrides (type=2, len=10)
    // Note: IRQ 0 → GSI 2 override deliberately omitted.
    // KVM's in-kernel PIT delivers timer interrupts at GSI 0 (IRQ 0).
    // With the override, Linux would configure IOAPIC pin 2 for timer
    // but KVM delivers on pin 0, causing timer interrupts to be lost.
    let overrides: &[(u8, u32, u16)] = &[
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

/// Device presence flags for dynamic ACPI table generation.
#[derive(Clone, Debug, Default)]
pub struct AcpiDeviceConfig {
    pub has_e1000: bool,
    pub has_ac97: bool,
    pub has_uhci: bool,
}

/// Generate ACPI 2.0 tables for SeaBIOS fw_cfg.
/// Returns (rsdp_data, tables_data, loader_data).
pub fn generate_acpi_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    generate_acpi_tables_full(false, 1, &AcpiDeviceConfig::default())
}

pub fn generate_acpi_tables_with_hpet() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    generate_acpi_tables_full(true, 1, &AcpiDeviceConfig::default())
}

pub fn generate_acpi_tables_smp(num_cpus: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    generate_acpi_tables_full(false, num_cpus, &AcpiDeviceConfig::default())
}

pub fn generate_acpi_tables_smp_hpet(num_cpus: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    generate_acpi_tables_full(true, num_cpus, &AcpiDeviceConfig::default())
}

/// Full ACPI table generation with device config.
pub fn generate_acpi_tables_configured(enable_hpet: bool, num_cpus: u32, devices: &AcpiDeviceConfig) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    generate_acpi_tables_full(enable_hpet, num_cpus, devices)
}

fn generate_acpi_tables_full(enable_hpet: bool, num_cpus: u32, devices: &AcpiDeviceConfig) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let num_cpus = num_cpus.max(1).min(32);
    let tables_file = make_name(TABLES_NAME);
    let rsdp_file = make_name(RSDP_NAME);

    let mut rsdp = build_rsdp();

    let num_tables: u32 = if enable_hpet { 3 } else { 2 };
    let rsdt = build_rsdt(num_tables);
    let xsdt = build_xsdt(num_tables);
    let fadt = build_fadt();
    let facs = build_facs();
    let dsdt = build_dsdt_with_cpus(num_cpus, devices);
    let madt = build_madt_with_cpus(num_cpus);

    // Layout: RSDT | XSDT | FADT | FACS | DSDT | MADT [| HPET]
    let rsdt_off: u32 = 0;
    let rsdt_len = rsdt.len() as u32;
    let xsdt_off = rsdt_len;
    let xsdt_len = xsdt.len() as u32;

    // Pre-fill RSDP pointer fields with intra-buffer offsets.
    rsdp[16..20].copy_from_slice(&rsdt_off.to_le_bytes());
    rsdp[24..32].copy_from_slice(&(xsdt_off as u64).to_le_bytes());
    let fadt_off = xsdt_off + xsdt_len;
    let facs_off = fadt_off + fadt.len() as u32;
    let dsdt_off = facs_off + facs.len() as u32;
    let madt_off = dsdt_off + dsdt.len() as u32;
    let madt_len = madt.len() as u32;

    let mut tables = Vec::new();
    tables.extend_from_slice(&rsdt);
    tables.extend_from_slice(&xsdt);
    tables.extend_from_slice(&fadt);
    tables.extend_from_slice(&facs);
    tables.extend_from_slice(&dsdt);
    tables.extend_from_slice(&madt);

    // RSDT -> FADT (u32 at offset 36), MADT (40)
    write_u32(&mut tables, (rsdt_off + 36) as usize, fadt_off);
    write_u32(&mut tables, (rsdt_off + 40) as usize, madt_off);
    // XSDT -> FADT (u64 at offset 36), MADT (44)
    write_u64(&mut tables, (xsdt_off + 36) as usize, fadt_off as u64);
    write_u64(&mut tables, (xsdt_off + 44) as usize, madt_off as u64);

    let hpet_off;
    let hpet_len;
    if enable_hpet {
        let hpet = build_hpet_table();
        hpet_off = madt_off + madt_len;
        hpet_len = hpet.len() as u32;
        tables.extend_from_slice(&hpet);
        // RSDT -> HPET (u32 at offset 44)
        write_u32(&mut tables, (rsdt_off + 44) as usize, hpet_off);
        // XSDT -> HPET (u64 at offset 52)
        write_u64(&mut tables, (xsdt_off + 52) as usize, hpet_off as u64);
    } else {
        hpet_off = 0;
        hpet_len = 0;
    }

    // FADT -> FACS (u32 at offset 36)
    write_u32(&mut tables, (fadt_off + 36) as usize, facs_off);
    // FADT -> DSDT (u32 at offset 40)
    write_u32(&mut tables, (fadt_off + 40) as usize, dsdt_off);
    // FADT -> X_FIRMWARE_CTRL (u64 at offset 132)
    write_u64(&mut tables, (fadt_off + 132) as usize, facs_off as u64);
    // FADT -> X_DSDT (u64 at offset 140)
    write_u64(&mut tables, (fadt_off + 140) as usize, dsdt_off as u64);

    let mut loader = Vec::new();
    let mut emit = |cmd: [u8; 128]| loader.extend_from_slice(&cmd);

    // 1. Allocate tables and RSDP
    emit(loader_allocate(&tables_file, 64, 1));
    emit(loader_allocate(&rsdp_file, 16, 2));

    // 2. RSDP pointers + checksums
    emit(loader_add_pointer(&rsdp_file, &tables_file, 16, 4));
    emit(loader_add_pointer(&rsdp_file, &tables_file, 24, 8));
    emit(loader_add_checksum(&rsdp_file, 8, 0, 20));
    emit(loader_add_checksum(&rsdp_file, 32, 0, 36));

    // 3. FADT pointers + checksum
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 36, 4));
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 40, 4));
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 132, 8));
    emit(loader_add_pointer(&tables_file, &tables_file, fadt_off + 140, 8));
    emit(loader_add_checksum(&tables_file, fadt_off + 9, fadt_off, 244));

    // 4. RSDT pointers + checksum
    emit(loader_add_pointer(&tables_file, &tables_file, rsdt_off + 36, 4));
    emit(loader_add_pointer(&tables_file, &tables_file, rsdt_off + 40, 4));
    if enable_hpet {
        emit(loader_add_pointer(&tables_file, &tables_file, rsdt_off + 44, 4));
    }
    emit(loader_add_checksum(&tables_file, rsdt_off + 9, rsdt_off, rsdt_len));

    // 5. XSDT pointers + checksum
    emit(loader_add_pointer(&tables_file, &tables_file, xsdt_off + 36, 8));
    emit(loader_add_pointer(&tables_file, &tables_file, xsdt_off + 44, 8));
    if enable_hpet {
        emit(loader_add_pointer(&tables_file, &tables_file, xsdt_off + 52, 8));
    }
    emit(loader_add_checksum(&tables_file, xsdt_off + 9, xsdt_off, xsdt_len));

    // 6. Table checksums
    let dsdt_len = dsdt.len() as u32;
    emit(loader_add_checksum(&tables_file, dsdt_off + 9, dsdt_off, dsdt_len));
    emit(loader_add_checksum(&tables_file, madt_off + 9, madt_off, madt_len));
    if enable_hpet {
        emit(loader_add_checksum(&tables_file, hpet_off + 9, hpet_off, hpet_len));
    }

    (rsdp, tables, loader)
}
