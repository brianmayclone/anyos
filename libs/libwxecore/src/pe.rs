use alloc::string::String;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20B;

#[derive(Clone, Copy)]
pub struct PeInfo {
    pub machine: u16,
    pub sections: u16,
    pub optional_magic: u16,
    pub subsystem: u16,
    pub entry_rva: u32,
    pub image_base: u64,
    pub size_of_image: u32,
    pub import_rva: u32,
    pub import_size: u32,
    pub reloc_rva: u32,
    pub reloc_size: u32,
    pub tls_rva: u32,
    pub tls_size: u32,
}

impl PeInfo {
    pub fn machine_name(&self) -> &'static str {
        match self.machine {
            IMAGE_FILE_MACHINE_AMD64 => "amd64",
            0x014c => "i386",
            0xaa64 => "arm64",
            _ => "unknown",
        }
    }

    pub fn subsystem_name(&self) -> &'static str {
        match self.subsystem {
            2 => "windows-gui",
            3 => "console",
            7 => "posix-cui",
            9 => "windows-ce-gui",
            10 => "efi-application",
            _ => "unknown",
        }
    }

    pub fn launchable_tier0(&self) -> bool {
        self.machine == IMAGE_FILE_MACHINE_AMD64
            && self.optional_magic == IMAGE_NT_OPTIONAL_HDR64_MAGIC
            && self.subsystem == 3
    }
}

pub fn inspect(data: &[u8]) -> Result<PeInfo, &'static str> {
    if data.len() < 0x40 {
        return Err("file too small for DOS header");
    }
    if read_u16(data, 0)? != IMAGE_DOS_SIGNATURE {
        return Err("missing MZ DOS signature");
    }

    let pe_offset = read_u32(data, 0x3c)? as usize;
    if pe_offset
        .checked_add(24)
        .map_or(true, |end| end > data.len())
    {
        return Err("PE header offset out of bounds");
    }
    if read_u32(data, pe_offset)? != IMAGE_NT_SIGNATURE {
        return Err("missing PE signature");
    }

    let coff = pe_offset + 4;
    let machine = read_u16(data, coff)?;
    let sections = read_u16(data, coff + 2)?;
    let optional_size = read_u16(data, coff + 16)? as usize;
    let opt = coff + 20;
    if opt
        .checked_add(optional_size)
        .map_or(true, |end| end > data.len())
    {
        return Err("optional header out of bounds");
    }
    if optional_size < 112 {
        return Err("optional header too small");
    }

    let optional_magic = read_u16(data, opt)?;
    if optional_magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
        return Err("not a PE32+ x86_64-compatible image");
    }

    let number_of_rva_and_sizes = read_u32(data, opt + 108)?;
    let data_dirs = opt + 112;
    let (import_rva, import_size) = read_data_dir(data, data_dirs, number_of_rva_and_sizes, 1)?;
    let (reloc_rva, reloc_size) = read_data_dir(data, data_dirs, number_of_rva_and_sizes, 5)?;
    let (tls_rva, tls_size) = read_data_dir(data, data_dirs, number_of_rva_and_sizes, 9)?;

    Ok(PeInfo {
        machine,
        sections,
        optional_magic,
        subsystem: read_u16(data, opt + 68)?,
        entry_rva: read_u32(data, opt + 16)?,
        image_base: read_u64(data, opt + 24)?,
        size_of_image: read_u32(data, opt + 56)?,
        import_rva,
        import_size,
        reloc_rva,
        reloc_size,
        tls_rva,
        tls_size,
    })
}

pub fn diagnose(data: &[u8]) -> String {
    match inspect(data) {
        Ok(info) => {
            let mut out = String::new();
            out.push_str("PE32+ image\n");
            out.push_str(&alloc::format!(
                "  machine: {} ({:#06x})\n",
                info.machine_name(),
                info.machine
            ));
            out.push_str(&alloc::format!("  sections: {}\n", info.sections));
            out.push_str(&alloc::format!(
                "  subsystem: {} ({})\n",
                info.subsystem_name(),
                info.subsystem
            ));
            out.push_str(&alloc::format!("  image-base: {:#x}\n", info.image_base));
            out.push_str(&alloc::format!("  entry-rva: {:#x}\n", info.entry_rva));
            out.push_str(&alloc::format!(
                "  size-of-image: {:#x}\n",
                info.size_of_image
            ));
            out.push_str(&alloc::format!(
                "  imports: rva={:#x} size={:#x}\n",
                info.import_rva,
                info.import_size
            ));
            out.push_str(&alloc::format!(
                "  relocations: rva={:#x} size={:#x}\n",
                info.reloc_rva,
                info.reloc_size
            ));
            out.push_str(&alloc::format!(
                "  tls: rva={:#x} size={:#x}\n",
                info.tls_rva,
                info.tls_size
            ));
            if info.launchable_tier0() {
                out.push_str("  wxe-tier0: candidate\n");
            } else {
                out.push_str("  wxe-tier0: unsupported\n");
            }
            out
        }
        Err(err) => alloc::format!("PE inspect failed: {}\n", err),
    }
}

fn read_data_dir(
    data: &[u8],
    data_dirs: usize,
    count: u32,
    index: usize,
) -> Result<(u32, u32), &'static str> {
    if count as usize <= index {
        return Ok((0, 0));
    }
    let off = data_dirs
        .checked_add(index * 8)
        .ok_or("data directory offset overflow")?;
    Ok((read_u32(data, off)?, read_u32(data, off + 4)?))
}

fn read_u16(data: &[u8], off: usize) -> Result<u16, &'static str> {
    let bytes = data.get(off..off + 2).ok_or("truncated u16")?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], off: usize) -> Result<u32, &'static str> {
    let bytes = data.get(off..off + 4).ok_or("truncated u32")?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], off: usize) -> Result<u64, &'static str> {
    let bytes = data.get(off..off + 8).ok_or("truncated u64")?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}
