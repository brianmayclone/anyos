use super::elf;
use crate::prelude::*;
use anyos_std::collections::HashMap;

/// Extended link options for kernel-level linking.
#[derive(Debug, Clone)]
pub struct LinkOptions {
    /// Linker script path (parsed for ENTRY point and section layout).
    pub linker_script: Option<String>,
    /// Additional pre-assembled object files to link in.
    pub extra_objects: Vec<Vec<u8>>,
    /// Base address override (from linker script or flag).
    pub base_address: Option<u64>,
    /// Physical load address override (from linker script LMA).
    pub physical_base_address: Option<u64>,
    /// Custom entry point symbol name (default: "_start").
    pub entry_symbol: Option<String>,
    /// Startup/exit syscall ABI for the generated `_start` stub.
    pub target_abi: TargetAbi,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            linker_script: None,
            extra_objects: Vec::new(),
            base_address: None,
            physical_base_address: None,
            entry_symbol: None,
            target_abi: TargetAbi::AnyOs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAbi {
    AnyOs,
    Linux,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub kind: LinkErrorKind,
    pub symbol: String,
    pub offset: u64,
    pub rela_type: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkErrorKind {
    UnresolvedSymbol,
    RelocationOutOfBounds,
    RelocationOutOfRange,
    UnsupportedRelocation,
}

impl LinkError {
    fn new(kind: LinkErrorKind, symbol: &str, offset: u64, rela_type: u32) -> Self {
        Self {
            kind,
            symbol: symbol.to_string(),
            offset,
            rela_type,
        }
    }
}

const LINUX_USER_BASE: u64 = 0x400000;
const ANYOS_USER_BASE: u64 = 0x08000000;

fn default_base_address(target_abi: TargetAbi) -> u64 {
    match target_abi {
        TargetAbi::AnyOs => ANYOS_USER_BASE,
        TargetAbi::Linux => LINUX_USER_BASE,
    }
}

fn is_text_section(name: &str) -> bool {
    name == ".text" || name.starts_with(".text.")
}

fn is_data_section(name: &str) -> bool {
    name == ".data"
        || name.starts_with(".data.")
        || name == ".rodata"
        || name.starts_with(".rodata.")
        || name == ".bss"
        || name.starts_with(".bss.")
}

fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        value
    } else {
        (value + align - 1) & !(align - 1)
    }
}

/// Link one or more ELF object files into an executable (extended version).
pub fn link_ext(
    objects: &[Vec<u8>],
    _output_name: &str,
    no_main: bool,
    opts: &LinkOptions,
) -> Vec<u8> {
    link_ext_checked(objects, _output_name, no_main, opts).expect("anyrc linker failed")
}

pub fn link_ext_checked(
    objects: &[Vec<u8>],
    _output_name: &str,
    no_main: bool,
    opts: &LinkOptions,
) -> Result<Vec<u8>, Vec<LinkError>> {
    // Merge extra objects into the object list
    let mut all_objects = objects.to_vec();
    for extra in &opts.extra_objects {
        all_objects.push(extra.clone());
    }

    // Parse linker script for base address and entry point
    let mut base_addr = opts
        .base_address
        .unwrap_or_else(|| default_base_address(opts.target_abi));
    let mut physical_base_addr = opts.physical_base_address;
    let mut code_starts_at_base = false;
    let mut entry_name = opts
        .entry_symbol
        .clone()
        .unwrap_or_else(|| "_start".to_string());

    if let Some(ref script_path) = opts.linker_script {
        if let Some(parsed) = parse_linker_script_minimal(script_path) {
            if let Some(addr) = parsed.base_address {
                base_addr = addr;
            }
            if let Some(addr) = parsed.physical_base_address {
                physical_base_addr = Some(addr);
            }
            code_starts_at_base = parsed.code_starts_at_base
                && (physical_base_addr.is_some() || base_addr >= 0xffff_8000_0000_0000);
            if let Some(ref entry) = parsed.entry_point {
                entry_name = entry.clone();
            }
        }
    }

    link_impl(
        &all_objects,
        no_main,
        base_addr,
        &entry_name,
        opts.target_abi,
        physical_base_addr,
        code_starts_at_base,
    )
}

/// Link one or more ELF object files into an executable.
/// Returns the raw bytes of the ELF executable.
pub fn link(objects: &[Vec<u8>], _output_name: &str, no_main: bool) -> Vec<u8> {
    link_checked(objects, _output_name, no_main).expect("anyrc linker failed")
}

pub fn link_checked(
    objects: &[Vec<u8>],
    _output_name: &str,
    no_main: bool,
) -> Result<Vec<u8>, Vec<LinkError>> {
    link_impl(
        objects,
        no_main,
        ANYOS_USER_BASE,
        "_start",
        TargetAbi::AnyOs,
        None,
        false,
    )
}

pub fn link_for_target(
    objects: &[Vec<u8>],
    _output_name: &str,
    no_main: bool,
    target_abi: TargetAbi,
) -> Vec<u8> {
    link_for_target_checked(objects, _output_name, no_main, target_abi)
        .expect("anyrc linker failed")
}

pub fn link_for_target_checked(
    objects: &[Vec<u8>],
    _output_name: &str,
    no_main: bool,
    target_abi: TargetAbi,
) -> Result<Vec<u8>, Vec<LinkError>> {
    link_impl(
        objects,
        no_main,
        default_base_address(target_abi),
        "_start",
        target_abi,
        None,
        false,
    )
}

fn link_impl(
    objects: &[Vec<u8>],
    no_main: bool,
    base_addr: u64,
    entry_name: &str,
    target_abi: TargetAbi,
    physical_base_addr: Option<u64>,
    code_starts_at_base: bool,
) -> Result<Vec<u8>, Vec<LinkError>> {
    let mut merged_code = Vec::new();
    let mut merged_data = Vec::new();

    // Symbol table: name -> offset in merged_code
    let mut global_symbols: HashMap<String, u64> = HashMap::new();

    // Pending relocations: (offset_in_merged, symbol_name, rela_type, addend)
    let mut pending_relocs: Vec<(u64, String, u32, i64)> = Vec::new();

    // Emit _start stub (unless no_main is set)
    if !no_main {
        // call main
        // mov rdi, rax  (exit code = return value of main)
        let start_offset = merged_code.len() as u64;
        // CALL rel32 (placeholder, will be patched)
        merged_code.push(0xE8);
        merged_code.extend_from_slice(&[0, 0, 0, 0]); // rel32 placeholder
                                                      // mov rdi, rax
        merged_code.extend_from_slice(&[0x48, 0x89, 0xC7]);
        match target_abi {
            TargetAbi::AnyOs => {
                // anyOS native syscall ABI: RAX=SYS_EXIT, RBX=exit code
                merged_code.extend_from_slice(&[0x48, 0x89, 0xFB]); // mov rbx, rdi
                merged_code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]);
                merged_code.extend_from_slice(&[0x0F, 0x05]);
            }
            TargetAbi::Linux => {
                // mov rax, 60 (SYS_exit), syscall
                merged_code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x3C, 0x00, 0x00, 0x00]);
                merged_code.extend_from_slice(&[0x0F, 0x05]);
            }
        }

        // Add _start->main relocation
        pending_relocs.push((
            start_offset + 1,
            "main".to_string(),
            2, /* R_X86_64_PC32 */
            -4,
        ));

        global_symbols.insert("_start".to_string(), start_offset);
    }

    // Process each object
    for obj_data in objects {
        let obj = match elf::parse_object(obj_data) {
            Some(o) => o,
            None => continue,
        };

        let mut text_section_offsets: HashMap<usize, u64> = HashMap::new();
        let mut data_section_offsets: HashMap<usize, u64> = HashMap::new();

        for (sec_idx, section) in obj.sections.iter().enumerate() {
            if is_text_section(&section.name) {
                let offset = merged_code.len() as u64;
                text_section_offsets.insert(sec_idx, offset);
                merged_code.extend_from_slice(&section.data);
            } else if is_data_section(&section.name) {
                let offset = merged_data.len() as u64;
                data_section_offsets.insert(sec_idx, offset);
                merged_data.extend_from_slice(&section.data);
            }
        }

        // Register symbols
        for sym in &obj.symbols {
            if !sym.name.is_empty() && sym.section.is_some() {
                let sec_idx = sym.section.unwrap();
                if let Some(section_offset) = data_section_offsets.get(&sec_idx) {
                    // Data symbol: offset relative to start of data in merged output
                    // Data follows code in the flat layout, so offset = code_total + data_offset
                    // We'll use a special marker to handle this at relocation time
                    // For now, store as negative offset to distinguish
                    // Actually, let's store (data_offset, true) meaning it's in data
                    // We need the final code length to compute this, so we defer.
                    // Instead, use a prefix convention:
                    let final_offset = section_offset + sym.offset;
                    global_symbols.insert(format!("\x01{}", sym.name), final_offset);
                } else if let Some(section_offset) = text_section_offsets.get(&sec_idx) {
                    let final_offset = section_offset + sym.offset;
                    global_symbols.insert(sym.name.clone(), final_offset);
                }
            }
        }

        // Collect relocations
        for rel in &obj.relocations {
            let sym_name = if rel.symbol < obj.symbols.len() {
                obj.symbols[rel.symbol].name.clone()
            } else {
                continue;
            };
            let reloc_section = rel.section.unwrap_or(0);
            let offset_in_merged = text_section_offsets
                .get(&reloc_section)
                .copied()
                .unwrap_or(0)
                + rel.offset;
            pending_relocs.push((offset_in_merged, sym_name, rel.rela_type, rel.addend));
        }
    }

    // Resolve relocations
    // Content starts after the ELF/program header page. Keep this in sync
    // with the executable writer so absolute and PC-relative relocations point
    // at the same virtual addresses the loader will map.
    let content_file_offset: u64 = elf::EXECUTABLE_CONTENT_OFFSET;
    let code_vaddr = if code_starts_at_base || physical_base_addr.is_some() {
        base_addr
    } else {
        base_addr + content_file_offset
    };

    // Data section starts after code in the flat layout
    let code_size = merged_code.len() as u64;
    let data_relative_offset = align_up(
        content_file_offset + code_size,
        elf::EXECUTABLE_SEGMENT_ALIGN,
    ) - content_file_offset;
    let bss_start = data_relative_offset + merged_data.len() as u64;
    let kernel_symbol_base = if code_starts_at_base || physical_base_addr.is_some() {
        code_vaddr
    } else {
        0
    };
    global_symbols
        .entry("_bss_start".to_string())
        .or_insert(kernel_symbol_base + bss_start);
    global_symbols
        .entry("_bss_end".to_string())
        .or_insert(kernel_symbol_base + bss_start);
    global_symbols
        .entry("_boot_stack_bottom".to_string())
        .or_insert(kernel_symbol_base + bss_start);
    global_symbols
        .entry("_boot_stack_top".to_string())
        .or_insert(kernel_symbol_base + bss_start + 0x80000);
    global_symbols
        .entry("_kernel_end".to_string())
        .or_insert(kernel_symbol_base + bss_start + 0x80000);
    let mut errors = Vec::new();

    for (offset, sym_name, rela_type, addend) in &pending_relocs {
        // Check if this is a data symbol (prefixed with \x01) or a code symbol
        let short_sym_name = sym_name.rsplit("::").next().unwrap_or(sym_name.as_str());
        let short_data_name = format!("\x01{}", short_sym_name);

        let (sym_offset, is_data) =
            if let Some(&o) = global_symbols.get(&format!("\x01{}", sym_name)) {
                (o, true)
            } else if let Some(&o) = global_symbols.get(sym_name) {
                (o, false)
            } else if short_sym_name != sym_name {
                if let Some(&o) = global_symbols.get(&short_data_name) {
                    (o, true)
                } else if let Some(&o) = global_symbols.get(short_sym_name) {
                    (o, false)
                } else {
                    errors.push(LinkError::new(
                        LinkErrorKind::UnresolvedSymbol,
                        sym_name,
                        *offset,
                        *rela_type,
                    ));
                    continue;
                }
            } else {
                errors.push(LinkError::new(
                    LinkErrorKind::UnresolvedSymbol,
                    sym_name,
                    *offset,
                    *rela_type,
                ));
                continue;
            };

        // For data symbols, the actual offset is the aligned RW segment start
        // plus the symbol's data-section offset.
        let sym_addr = if is_data {
            let sym_pos = data_relative_offset + sym_offset;
            if sym_offset >= code_vaddr {
                sym_offset
            } else {
                code_vaddr + sym_pos
            }
        } else if sym_offset >= code_vaddr {
            sym_offset
        } else {
            code_vaddr + sym_offset
        };

        match *rela_type {
            2 /* R_X86_64_PC32 */ => {
                // S + A - P where S = sym_addr, P = reloc_addr
                let s = sym_addr;
                let p = code_vaddr + offset;
                let value = (s as i64) + addend - (p as i64);
                if value < i32::MIN as i64 || value > i32::MAX as i64 {
                    errors.push(LinkError::new(
                        LinkErrorKind::RelocationOutOfRange,
                        sym_name,
                        *offset,
                        *rela_type,
                    ));
                    continue;
                }
                let bytes = (value as i32).to_le_bytes();
                let off = *offset as usize;
                if off + 4 <= merged_code.len() {
                    merged_code[off..off + 4].copy_from_slice(&bytes);
                } else {
                    errors.push(LinkError::new(
                        LinkErrorKind::RelocationOutOfBounds,
                        sym_name,
                        *offset,
                        *rela_type,
                    ));
                }
            }
            1 /* R_X86_64_64 */ => {
                let s = sym_addr;
                let value = (s as i64 + addend) as u64;
                let bytes = value.to_le_bytes();
                let off = *offset as usize;
                if off + 8 <= merged_code.len() {
                    merged_code[off..off + 8].copy_from_slice(&bytes);
                } else {
                    errors.push(LinkError::new(
                        LinkErrorKind::RelocationOutOfBounds,
                        sym_name,
                        *offset,
                        *rela_type,
                    ));
                }
            }
            _ => {
                errors.push(LinkError::new(
                    LinkErrorKind::UnsupportedRelocation,
                    sym_name,
                    *offset,
                    *rela_type,
                ));
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // Entry point
    let entry_offset = *global_symbols.get(entry_name).unwrap_or(&0);

    if code_starts_at_base || physical_base_addr.is_some() {
        let entry_offset = if entry_offset >= code_vaddr {
            entry_offset - code_vaddr
        } else {
            entry_offset
        };
        Ok(elf::write_executable_image_at(
            &merged_code,
            &merged_data,
            entry_offset,
            code_vaddr,
            physical_base_addr.unwrap_or(code_vaddr),
        ))
    } else {
        Ok(elf::write_executable_at(
            &merged_code,
            &merged_data,
            entry_offset,
            base_addr,
        ))
    }
}

/// Minimal linker script info extracted from a `.ld` file.
struct LinkerScriptInfo {
    base_address: Option<u64>,
    physical_base_address: Option<u64>,
    entry_point: Option<String>,
    code_starts_at_base: bool,
}

/// Parse a linker script for ENTRY() and base address.
/// Only extracts minimal info needed for linking — not a full LD script parser.
fn parse_linker_script_minimal(path: &str) -> Option<LinkerScriptInfo> {
    let data = crate::loader::OsFileLoader::read_bytes(path)?;
    let text = core::str::from_utf8(&data).ok()?;

    let mut info = LinkerScriptInfo {
        base_address: None,
        physical_base_address: None,
        entry_point: None,
        code_starts_at_base: false,
    };
    let mut constants: HashMap<String, u64> = HashMap::new();

    for line in text.lines() {
        let line = strip_line_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        // ENTRY(symbol)
        if line.starts_with("ENTRY(") {
            if let Some(end) = line.find(')') {
                let sym = line[6..end].trim();
                info.entry_point = Some(sym.to_string());
            }
        }

        if let Some((name, value)) = parse_script_assignment(line, &constants) {
            if name == "KERNEL_VMA" {
                info.base_address = Some(value);
            } else if name == "KERNEL_LMA" {
                info.physical_base_address = Some(value);
            }
            constants.insert(name, value);
            continue;
        }

        // . = 0xFFFFFFFF80000000; or . = 0x400000;
        if line.starts_with(". =") || line.starts_with(".=") {
            let after_eq = if line.starts_with(". =") {
                &line[3..]
            } else {
                &line[2..]
            };
            let after_eq = after_eq.trim().trim_end_matches(';').trim();
            if let Some(addr) = parse_script_value(after_eq, &constants) {
                if info.base_address.is_none() {
                    info.base_address = Some(addr);
                }
                info.code_starts_at_base = true;
            }
        }
    }

    Some(info)
}

fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn parse_script_assignment(line: &str, constants: &HashMap<String, u64>) -> Option<(String, u64)> {
    if line.starts_with('.') || line.starts_with('/') || line.starts_with('*') {
        return None;
    }
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    let value = value.trim().trim_end_matches(';').trim();
    parse_script_value(value, constants).map(|v| (name.to_string(), v))
}

fn parse_script_value(s: &str, constants: &HashMap<String, u64>) -> Option<u64> {
    let s = s.trim();
    if let Some(value) = parse_hex_or_dec(s) {
        return Some(value);
    }
    constants.get(s).copied()
}

fn parse_hex_or_dec(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        u64::from_str_radix(&s[2..], 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}
