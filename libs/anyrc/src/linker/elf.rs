// ELF64 constants
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const ELFOSABI_NONE: u8 = 0;
const ET_REL: u16 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;

const SHT_NULL: u32 = 0;
const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_RELA: u32 = 4;
#[allow(dead_code)]
const SHT_NOBITS: u32 = 8;

const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;

const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;

const STT_NOTYPE: u8 = 0;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;

#[allow(dead_code)]
const R_X86_64_64: u32 = 1;
const R_X86_64_PC32: u32 = 2;

const PT_LOAD: u32 = 1;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

pub struct ObjectFile {
    pub sections: Vec<Section>,
    pub symbols: Vec<ElfSymbol>,
    pub relocations: Vec<ElfRelocation>,
}

pub struct Section {
    pub name: String,
    pub data: Vec<u8>,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addralign: u64,
}

pub struct ElfSymbol {
    pub name: String,
    pub section: Option<usize>,
    pub offset: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
}

pub struct ElfRelocation {
    pub offset: u64,
    pub symbol: usize,
    pub rela_type: u32,
    pub addend: i64,
}

impl ElfSymbol {
    pub fn global_func(name: &str, section: usize, offset: u64, size: u64) -> Self {
        ElfSymbol {
            name: name.to_string(),
            section: Some(section),
            offset,
            size,
            binding: STB_GLOBAL,
            sym_type: STT_FUNC,
        }
    }
}

impl ElfRelocation {
    pub fn pc32(offset: u64, symbol: usize, addend: i64) -> Self {
        ElfRelocation { offset, symbol, rela_type: R_X86_64_PC32, addend }
    }
}

struct StringTable {
    data: Vec<u8>,
}

impl StringTable {
    fn new() -> Self {
        let mut data = Vec::new();
        data.push(0); // null string at index 0
        StringTable { data }
    }

    fn add(&mut self, s: &str) -> u32 {
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        offset
    }
}

fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write an ELF64 relocatable object file.
pub fn write_object(obj: &ObjectFile) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut shstrtab = StringTable::new();
    let mut strtab = StringTable::new();

    // Reserve shstrtab names for user sections + fixed sections
    // We'll build section headers at the end once we know offsets.

    // Layout:
    // 1. ELF header (64 bytes)
    // 2. User section data
    // 3. .symtab data
    // 4. .strtab data
    // 5. .rela.text data
    // 6. .shstrtab data
    // 7. Section header table

    // -- ELF header placeholder (64 bytes) --
    buf.resize(64, 0);

    // -- User section data --
    let mut section_offsets = Vec::new();
    for sec in &obj.sections {
        // Align
        let align = sec.sh_addralign.max(1) as usize;
        let padding = (align - (buf.len() % align)) % align;
        buf.extend(core::iter::repeat(0u8).take(padding));
        section_offsets.push(buf.len());
        buf.extend_from_slice(&sec.data);
    }

    // -- Build symbol table --
    // Symbol 0: null
    // Then local symbols (section symbols)
    // Then global symbols
    let mut sym_entries: Vec<(u32, u8, u8, u16, u64, u64)> = Vec::new();
    // Null symbol
    sym_entries.push((0, 0, 0, 0, 0, 0));

    // Section symbols (local) - one per user section
    for i in 0..obj.sections.len() {
        let name_off = strtab.add("");
        sym_entries.push((name_off, (STB_LOCAL << 4) | STT_SECTION, 0, (i + 1) as u16, 0, 0));
    }

    let first_global = sym_entries.len() as u32;

    // Map from obj.symbols index to actual symtab index
    let mut sym_index_map: Vec<usize> = Vec::new();
    for sym in &obj.symbols {
        let idx = sym_entries.len();
        sym_index_map.push(idx);
        let name_off = strtab.add(&sym.name);
        let shndx = sym.section.map(|s| (s + 1) as u16).unwrap_or(0);
        let info = (sym.binding << 4) | sym.sym_type;
        sym_entries.push((name_off, info, 0, shndx, sym.offset, sym.size));
    }

    // Write symtab
    let symtab_align_pad = (8 - (buf.len() % 8)) % 8;
    buf.extend(core::iter::repeat(0u8).take(symtab_align_pad));
    let symtab_offset = buf.len();
    for &(st_name, st_info, st_other, st_shndx, st_value, st_size) in &sym_entries {
        write_u32(&mut buf, st_name);
        buf.push(st_info);
        buf.push(st_other);
        write_u16(&mut buf, st_shndx);
        write_u64(&mut buf, st_value);
        write_u64(&mut buf, st_size);
    }
    let symtab_size = buf.len() - symtab_offset;

    // Write strtab
    let strtab_offset = buf.len();
    buf.extend_from_slice(&strtab.data);
    let strtab_size = strtab.data.len();

    // Write .rela.text
    let rela_align_pad = (8 - (buf.len() % 8)) % 8;
    buf.extend(core::iter::repeat(0u8).take(rela_align_pad));
    let rela_offset = buf.len();
    for rel in &obj.relocations {
        let sym_idx = sym_index_map[rel.symbol] as u64;
        write_u64(&mut buf, rel.offset);
        write_u64(&mut buf, (sym_idx << 32) | (rel.rela_type as u64));
        write_i64(&mut buf, rel.addend);
    }
    let rela_size = buf.len() - rela_offset;

    // Write shstrtab
    let shstrtab_offset = buf.len();
    // We need to add section names to shstrtab now
    let mut shstrtab_names = Vec::new();
    // null section
    shstrtab_names.push(shstrtab.add(""));
    // user sections
    for sec in &obj.sections {
        shstrtab_names.push(shstrtab.add(&sec.name));
    }
    let symtab_name = shstrtab.add(".symtab");
    let strtab_name = shstrtab.add(".strtab");
    let rela_name = shstrtab.add(".rela.text");
    let shstrtab_name = shstrtab.add(".shstrtab");
    buf.extend_from_slice(&shstrtab.data);
    let shstrtab_size = shstrtab.data.len();

    // Align section header table to 8 bytes
    let shdr_align = (8 - (buf.len() % 8)) % 8;
    buf.extend(core::iter::repeat(0u8).take(shdr_align));
    let shoff = buf.len();

    // Section headers:
    // 0: null
    // 1..N: user sections
    // N+1: .symtab
    // N+2: .strtab
    // N+3: .rela.text
    // N+4: .shstrtab

    let num_user = obj.sections.len();
    let symtab_idx = num_user + 1;
    let strtab_idx = num_user + 2;
    let rela_idx = num_user + 3;
    let shstrtab_idx = num_user + 4;
    let shnum = shstrtab_idx + 1;

    // 0: null section header
    write_section_header(&mut buf, 0, SHT_NULL, 0, 0, 0, 0, 0, 0, 0, 0);

    // user sections
    for (i, sec) in obj.sections.iter().enumerate() {
        write_section_header(
            &mut buf,
            shstrtab_names[i + 1],
            sec.sh_type,
            sec.sh_flags,
            0, // sh_addr
            section_offsets[i] as u64,
            sec.data.len() as u64,
            0, // sh_link
            0, // sh_info
            sec.sh_addralign,
            0, // sh_entsize
        );
    }

    // .symtab
    write_section_header(
        &mut buf,
        symtab_name,
        SHT_SYMTAB,
        0,
        0,
        symtab_offset as u64,
        symtab_size as u64,
        strtab_idx as u32, // sh_link = .strtab index
        first_global,       // sh_info = first global symbol index
        8,
        24, // sizeof(Elf64_Sym)
    );

    // .strtab
    write_section_header(
        &mut buf,
        strtab_name,
        SHT_STRTAB,
        0,
        0,
        strtab_offset as u64,
        strtab_size as u64,
        0, 0, 1, 0,
    );

    // .rela.text
    // sh_link = .symtab, sh_info = .text section index (1 if .text is first user section)
    let text_section_idx = obj.sections.iter().position(|s| s.name == ".text").map(|i| i + 1).unwrap_or(1);
    write_section_header(
        &mut buf,
        rela_name,
        SHT_RELA,
        0,
        0,
        rela_offset as u64,
        rela_size as u64,
        symtab_idx as u32,
        text_section_idx as u32,
        8,
        24, // sizeof(Elf64_Rela)
    );

    // .shstrtab
    write_section_header(
        &mut buf,
        shstrtab_name,
        SHT_STRTAB,
        0,
        0,
        shstrtab_offset as u64,
        shstrtab_size as u64,
        0, 0, 1, 0,
    );

    // Now go back and fill in the ELF header
    // e_ident
    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = ELFCLASS64;
    buf[5] = ELFDATA2LSB;
    buf[6] = EV_CURRENT;
    buf[7] = ELFOSABI_NONE;
    // [8..16] already zero

    // e_type
    buf[16..18].copy_from_slice(&ET_REL.to_le_bytes());
    // e_machine
    buf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
    // e_version
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    // e_entry = 0
    // e_phoff = 0
    // e_shoff
    buf[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
    // e_flags = 0
    // e_ehsize = 64
    buf[52..54].copy_from_slice(&64u16.to_le_bytes());
    // e_phentsize = 0, e_phnum = 0
    // e_shentsize = 64
    buf[58..60].copy_from_slice(&64u16.to_le_bytes());
    // e_shnum
    buf[60..62].copy_from_slice(&(shnum as u16).to_le_bytes());
    // e_shstrndx
    buf[62..64].copy_from_slice(&(shstrtab_idx as u16).to_le_bytes());

    buf
}

fn write_section_header(
    buf: &mut Vec<u8>,
    sh_name: u32,
    sh_type: u32,
    sh_flags: u64,
    sh_addr: u64,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u64,
    sh_entsize: u64,
) {
    write_u32(buf, sh_name);
    write_u32(buf, sh_type);
    write_u64(buf, sh_flags);
    write_u64(buf, sh_addr);
    write_u64(buf, sh_offset);
    write_u64(buf, sh_size);
    write_u32(buf, sh_link);
    write_u32(buf, sh_info);
    write_u64(buf, sh_addralign);
    write_u64(buf, sh_entsize);
}

/// Write an ELF64 executable from merged code with a given entry point.
pub fn write_executable(code: &[u8], data: &[u8], entry_offset: u64) -> Vec<u8> {
    let base_addr: u64 = 0x400000;
    // Layout:
    // ELF header (64 bytes)
    // Program header (56 bytes)
    // Code + data
    let phdr_offset: u64 = 64;
    let phdr_size: u64 = 56;
    let content_offset = (phdr_offset + phdr_size + 15) & !15; // align to 16
    let total_content = code.len() + data.len();

    let entry = base_addr + content_offset + entry_offset;

    let mut buf = Vec::new();
    buf.resize(content_offset as usize, 0);

    // ELF header
    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = ELFCLASS64;
    buf[5] = ELFDATA2LSB;
    buf[6] = EV_CURRENT;
    buf[7] = ELFOSABI_NONE;
    buf[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    buf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    buf[24..32].copy_from_slice(&entry.to_le_bytes());
    buf[32..40].copy_from_slice(&phdr_offset.to_le_bytes());
    // e_shoff = 0 (no section headers in exe for simplicity)
    buf[52..54].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize
    buf[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
    buf[56..58].copy_from_slice(&1u16.to_le_bytes());  // e_phnum

    // Program header (single PT_LOAD, RWX)
    let ph_start = phdr_offset as usize;
    let file_size = content_offset as u64 + total_content as u64;
    let mem_size = file_size; // no BSS for now

    // p_type
    buf[ph_start..ph_start + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
    // p_flags
    buf[ph_start + 4..ph_start + 8].copy_from_slice(&(PF_R | PF_W | PF_X).to_le_bytes());
    // p_offset = 0 (load from start of file)
    buf[ph_start + 8..ph_start + 16].copy_from_slice(&0u64.to_le_bytes());
    // p_vaddr
    buf[ph_start + 16..ph_start + 24].copy_from_slice(&base_addr.to_le_bytes());
    // p_paddr
    buf[ph_start + 24..ph_start + 32].copy_from_slice(&base_addr.to_le_bytes());
    // p_filesz
    buf[ph_start + 32..ph_start + 40].copy_from_slice(&file_size.to_le_bytes());
    // p_memsz
    buf[ph_start + 40..ph_start + 48].copy_from_slice(&mem_size.to_le_bytes());
    // p_align
    buf[ph_start + 48..ph_start + 56].copy_from_slice(&0x200000u64.to_le_bytes());

    // Content
    buf.extend_from_slice(code);
    buf.extend_from_slice(data);

    buf
}

/// Parse an ELF64 object file. Returns sections, symbols, and relocations.
pub fn parse_object(data: &[u8]) -> Option<ObjectFile> {
    if data.len() < 64 || &data[0..4] != &ELF_MAGIC {
        return None;
    }
    if data[4] != ELFCLASS64 || data[5] != ELFDATA2LSB {
        return None;
    }

    let e_type = u16::from_le_bytes(data[16..18].try_into().ok()?);
    if e_type != ET_REL {
        return None;
    }

    let e_shoff = u64::from_le_bytes(data[40..48].try_into().ok()?) as usize;
    let e_shentsize = u16::from_le_bytes(data[58..60].try_into().ok()?) as usize;
    let e_shnum = u16::from_le_bytes(data[60..62].try_into().ok()?) as usize;
    let e_shstrndx = u16::from_le_bytes(data[62..64].try_into().ok()?) as usize;

    // Read section headers
    struct RawShdr {
        sh_name: u32,
        sh_type: u32,
        sh_flags: u64,
        sh_offset: u64,
        sh_size: u64,
        sh_link: u32,
        sh_info: u32,
        sh_addralign: u64,
        #[allow(dead_code)]
        sh_entsize: u64,
    }

    let mut shdrs = Vec::new();
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        let sh_name = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        let sh_type = u32::from_le_bytes(data[off + 4..off + 8].try_into().ok()?);
        let sh_flags = u64::from_le_bytes(data[off + 8..off + 16].try_into().ok()?);
        let sh_offset = u64::from_le_bytes(data[off + 24..off + 32].try_into().ok()?);
        let sh_size = u64::from_le_bytes(data[off + 32..off + 40].try_into().ok()?);
        let sh_link = u32::from_le_bytes(data[off + 40..off + 44].try_into().ok()?);
        let sh_info = u32::from_le_bytes(data[off + 44..off + 48].try_into().ok()?);
        let sh_addralign = u64::from_le_bytes(data[off + 48..off + 56].try_into().ok()?);
        let sh_entsize = u64::from_le_bytes(data[off + 56..off + 64].try_into().ok()?);
        shdrs.push(RawShdr { sh_name, sh_type, sh_flags, sh_offset, sh_size, sh_link, sh_info, sh_addralign, sh_entsize });
    }

    // Get shstrtab
    let shstrtab_off = shdrs[e_shstrndx].sh_offset as usize;
    let shstrtab_size = shdrs[e_shstrndx].sh_size as usize;
    let shstrtab = &data[shstrtab_off..shstrtab_off + shstrtab_size];

    let read_str = |strtab: &[u8], offset: u32| -> String {
        let start = offset as usize;
        let end = strtab[start..].iter().position(|&b| b == 0).unwrap_or(strtab.len() - start) + start;
        String::from_utf8_lossy(&strtab[start..end]).to_string()
    };

    // Collect user sections (PROGBITS, NOBITS)
    let mut sections = Vec::new();
    // Map from raw section index to our section index
    let mut sec_index_map = std::collections::HashMap::new();
    for (i, shdr) in shdrs.iter().enumerate() {
        if shdr.sh_type == SHT_PROGBITS || shdr.sh_type == SHT_NOBITS {
            let name = read_str(shstrtab, shdr.sh_name);
            let sec_data = if shdr.sh_type == SHT_NOBITS {
                vec![0u8; shdr.sh_size as usize]
            } else {
                data[shdr.sh_offset as usize..(shdr.sh_offset + shdr.sh_size) as usize].to_vec()
            };
            sec_index_map.insert(i, sections.len());
            sections.push(Section {
                name,
                data: sec_data,
                sh_type: shdr.sh_type,
                sh_flags: shdr.sh_flags,
                sh_addralign: shdr.sh_addralign,
            });
        }
    }

    // Find symtab
    let mut symbols = Vec::new();
    let mut raw_sym_to_our: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut raw_syms: Vec<(String, Option<usize>, u64, u64, u8, u8)> = Vec::new();

    for shdr in &shdrs {
        if shdr.sh_type == SHT_SYMTAB {
            let strtab_shdr = &shdrs[shdr.sh_link as usize];
            let sym_strtab = &data[strtab_shdr.sh_offset as usize..(strtab_shdr.sh_offset + strtab_shdr.sh_size) as usize];
            let count = shdr.sh_size as usize / 24;
            let base = shdr.sh_offset as usize;
            for j in 0..count {
                let off = base + j * 24;
                let st_name = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
                let st_info = data[off + 4];
                let st_shndx = u16::from_le_bytes(data[off + 6..off + 8].try_into().ok()?);
                let st_value = u64::from_le_bytes(data[off + 8..off + 16].try_into().ok()?);
                let st_size = u64::from_le_bytes(data[off + 16..off + 24].try_into().ok()?);
                let binding = st_info >> 4;
                let sym_type = st_info & 0xf;
                let name = read_str(sym_strtab, st_name);
                let section = if st_shndx > 0 && (st_shndx as usize) < shdrs.len() {
                    sec_index_map.get(&(st_shndx as usize)).copied()
                } else {
                    None
                };
                raw_syms.push((name, section, st_value, st_size, binding, sym_type));
            }

            // Skip null + section symbols, export only named globals
            for (j, (name, section, offset, size, binding, sym_type)) in raw_syms.iter().enumerate() {
                if j == 0 { continue; } // null
                if *sym_type == STT_SECTION { continue; }
                raw_sym_to_our.insert(j, symbols.len());
                symbols.push(ElfSymbol {
                    name: name.clone(),
                    section: *section,
                    offset: *offset,
                    size: *size,
                    binding: *binding,
                    sym_type: *sym_type,
                });
            }
            break;
        }
    }

    // Find relocations
    let mut relocations = Vec::new();
    for shdr in &shdrs {
        if shdr.sh_type == SHT_RELA {
            let count = shdr.sh_size as usize / 24;
            let base = shdr.sh_offset as usize;
            for j in 0..count {
                let off = base + j * 24;
                let r_offset = u64::from_le_bytes(data[off..off + 8].try_into().ok()?);
                let r_info = u64::from_le_bytes(data[off + 8..off + 16].try_into().ok()?);
                let r_addend = i64::from_le_bytes(data[off + 16..off + 24].try_into().ok()?);
                let sym_idx = (r_info >> 32) as usize;
                let rela_type = (r_info & 0xffffffff) as u32;
                // Map raw sym index to our index
                let our_sym = raw_sym_to_our.get(&sym_idx).copied().unwrap_or(0);
                relocations.push(ElfRelocation {
                    offset: r_offset,
                    symbol: our_sym,
                    rela_type,
                    addend: r_addend,
                });
            }
        }
    }

    Some(ObjectFile { sections, symbols, relocations })
}

/// Parse section names from an ELF file (for testing).
pub fn parse_section_names(data: &[u8]) -> Vec<String> {
    if data.len() < 64 { return vec![]; }
    let e_shoff = u64::from_le_bytes(data[40..48].try_into().unwrap()) as usize;
    let e_shentsize = u16::from_le_bytes(data[58..60].try_into().unwrap()) as usize;
    let e_shnum = u16::from_le_bytes(data[60..62].try_into().unwrap()) as usize;
    let e_shstrndx = u16::from_le_bytes(data[62..64].try_into().unwrap()) as usize;

    if e_shnum == 0 || e_shoff == 0 { return vec![]; }

    // Read shstrtab
    let shstr_off = e_shoff + e_shstrndx * e_shentsize;
    let shstrtab_offset = u64::from_le_bytes(data[shstr_off + 24..shstr_off + 32].try_into().unwrap()) as usize;
    let shstrtab_size = u64::from_le_bytes(data[shstr_off + 32..shstr_off + 40].try_into().unwrap()) as usize;
    let shstrtab = &data[shstrtab_offset..shstrtab_offset + shstrtab_size];

    let mut names = Vec::new();
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        let sh_name = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        let end = shstrtab[sh_name..].iter().position(|&b| b == 0).unwrap_or(shstrtab.len() - sh_name) + sh_name;
        let name = String::from_utf8_lossy(&shstrtab[sh_name..end]).to_string();
        if !name.is_empty() {
            names.push(name);
        }
    }
    names
}

/// Parse symbol names from an ELF object file (for testing).
pub fn parse_symbol_names(data: &[u8]) -> Vec<String> {
    if let Some(obj) = parse_object(data) {
        obj.symbols.iter().map(|s| s.name.clone()).collect()
    } else {
        vec![]
    }
}
