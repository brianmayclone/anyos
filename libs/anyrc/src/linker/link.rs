use crate::prelude::*;
use anyos_std::collections::HashMap;
use super::elf;

/// Link one or more ELF object files into an executable.
/// Returns the raw bytes of the ELF executable.
pub fn link(objects: &[Vec<u8>], _output_name: &str, no_main: bool) -> Vec<u8> {
    let mut merged_code = Vec::new();
    let mut merged_data = Vec::new();

    // Symbol table: name -> offset in merged_code
    let mut global_symbols: HashMap<String, u64> = HashMap::new();

    // Pending relocations: (offset_in_merged, symbol_name, rela_type, addend)
    let mut pending_relocs: Vec<(u64, String, u32, i64)> = Vec::new();

    // Emit _start stub (unless no_main is set)
    if !no_main {
        // call main
        // mov rdi, rax
        // mov rax, 60
        // syscall
        let start_offset = merged_code.len() as u64;
        // CALL rel32 (placeholder, will be patched)
        merged_code.push(0xE8);
        merged_code.extend_from_slice(&[0, 0, 0, 0]); // rel32 placeholder
        // mov rdi, rax  =>  REX.W 89 C7  (mov rdi, rax = 48 89 c7)
        merged_code.extend_from_slice(&[0x48, 0x89, 0xC7]);
        // mov rax, 60  =>  REX.W B8 imm64
        merged_code.extend_from_slice(&[0x48, 0xB8]);
        merged_code.extend_from_slice(&60u64.to_le_bytes());
        // syscall
        merged_code.extend_from_slice(&[0x0F, 0x05]);

        // Add _start->main relocation
        pending_relocs.push((start_offset + 1, "main".to_string(), 2 /* R_X86_64_PC32 */, -4));

        global_symbols.insert("_start".to_string(), start_offset);
    }

    // Process each object
    for obj_data in objects {
        let obj = match elf::parse_object(obj_data) {
            Some(o) => o,
            None => continue,
        };

        // Find .text section
        let text_idx = obj.sections.iter().position(|s| s.name == ".text");
        let text_offset_in_merged = merged_code.len() as u64;

        if let Some(ti) = text_idx {
            merged_code.extend_from_slice(&obj.sections[ti].data);
        }

        // Find .data section
        let data_idx = obj.sections.iter().position(|s| s.name == ".data");
        let data_offset_in_merged = merged_data.len() as u64;

        if let Some(di) = data_idx {
            merged_data.extend_from_slice(&obj.sections[di].data);
        }

        // Register symbols
        for sym in &obj.symbols {
            if !sym.name.is_empty() && sym.section.is_some() {
                let sec_idx = sym.section.unwrap();
                let sec_name = if sec_idx < obj.sections.len() {
                    &obj.sections[sec_idx].name
                } else {
                    ""
                };
                if sec_name == ".data" {
                    // Data symbol: offset relative to start of data in merged output
                    // Data follows code in the flat layout, so offset = code_total + data_offset
                    // We'll use a special marker to handle this at relocation time
                    // For now, store as negative offset to distinguish
                    // Actually, let's store (data_offset, true) meaning it's in data
                    // We need the final code length to compute this, so we defer.
                    // Instead, use a prefix convention:
                    let final_offset = data_offset_in_merged + sym.offset;
                    global_symbols.insert(format!("\x01{}", sym.name), final_offset);
                } else {
                    let final_offset = text_offset_in_merged + sym.offset;
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
            let offset_in_merged = text_offset_in_merged + rel.offset;
            pending_relocs.push((offset_in_merged, sym_name, rel.rela_type, rel.addend));
        }
    }

    // Resolve relocations
    let base_addr: u64 = 0x400000;
    // Content starts at offset 128 (64 ehdr + 56 phdr, aligned to 16 = 128)
    let content_file_offset: u64 = 128;

    // Data section starts after code in the flat layout
    let code_size = merged_code.len() as u64;

    for (offset, sym_name, rela_type, addend) in &pending_relocs {
        // Check if this is a data symbol (prefixed with \x01) or a code symbol
        let (sym_offset, is_data) = if let Some(&o) = global_symbols.get(&format!("\x01{}", sym_name)) {
            (o, true)
        } else if let Some(&o) = global_symbols.get(sym_name) {
            (o, false)
        } else {
            continue; // unresolved, skip
        };

        // For data symbols, the actual offset in the flat file is code_size + sym_offset
        let file_sym_offset = if is_data { code_size + sym_offset } else { sym_offset };

        match *rela_type {
            2 /* R_X86_64_PC32 */ => {
                // S + A - P where S = sym_addr, P = reloc_addr
                let s = base_addr + content_file_offset + file_sym_offset;
                let p = base_addr + content_file_offset + offset;
                let value = (s as i64) + addend - (p as i64);
                let bytes = (value as i32).to_le_bytes();
                let off = *offset as usize;
                if off + 4 <= merged_code.len() {
                    merged_code[off..off + 4].copy_from_slice(&bytes);
                }
            }
            1 /* R_X86_64_64 */ => {
                let s = base_addr + content_file_offset + file_sym_offset;
                let value = (s as i64 + addend) as u64;
                let bytes = value.to_le_bytes();
                let off = *offset as usize;
                if off + 8 <= merged_code.len() {
                    merged_code[off..off + 8].copy_from_slice(&bytes);
                }
            }
            _ => {}
        }
    }

    // Entry point is _start
    let entry_offset = *global_symbols.get("_start").unwrap_or(&0);

    elf::write_executable(&merged_code, &merged_data, entry_offset)
}
