//! ELF symbol table loader and address-to-symbol lookup.

use alloc::string::String;
use alloc::vec::Vec;

/// A symbol from the ELF symbol table.
#[derive(Clone)]
pub struct Symbol {
    /// Virtual address of the symbol.
    pub addr: u64,
    /// Size of the symbol in bytes.
    pub size: u64,
    /// Symbol name.
    pub name: String,
}

/// Parsed symbol table from an ELF binary.
pub struct SymbolTable {
    entries: Vec<Symbol>,
    pub module_name: String,
}

// ELF64 constants
const EI_MAG: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const STT_FUNC: u8 = 2;

impl SymbolTable {
    /// Load symbols from an ELF file on disk.
    pub fn load(path: &str) -> Option<Self> {
        let data = anyos_std::fs::read_to_vec(path).ok()?;
        if data.len() < 64 {
            return None;
        }

        // Verify ELF magic
        if data[0..4] != EI_MAG {
            return None;
        }
        // Must be ELF64 (class = 2)
        if data[4] != 2 {
            return None;
        }

        // Parse ELF64 header
        let e_shoff = u64::from_le_bytes(data[40..48].try_into().ok()?) as usize;
        let e_shentsize = u16::from_le_bytes(data[58..60].try_into().ok()?) as usize;
        let e_shnum = u16::from_le_bytes(data[60..62].try_into().ok()?) as usize;
        let e_shstrndx = u16::from_le_bytes(data[62..64].try_into().ok()?) as usize;

        if e_shoff == 0 || e_shnum == 0 {
            return None;
        }

        // Find .symtab and .strtab sections
        let mut symtab_off = 0usize;
        let mut symtab_size = 0usize;
        let mut symtab_link = 0usize; // Index of associated .strtab
        let mut strtab_off = 0usize;
        let mut strtab_size = 0usize;

        for i in 0..e_shnum {
            let sh_off = e_shoff + i * e_shentsize;
            if sh_off + e_shentsize > data.len() {
                break;
            }
            let sh_type = u32::from_le_bytes(data[sh_off + 4..sh_off + 8].try_into().ok()?);
            let sh_offset = u64::from_le_bytes(data[sh_off + 24..sh_off + 32].try_into().ok()?) as usize;
            let sh_size = u64::from_le_bytes(data[sh_off + 32..sh_off + 40].try_into().ok()?) as usize;
            let sh_link = u32::from_le_bytes(data[sh_off + 40..sh_off + 44].try_into().ok()?) as usize;

            if sh_type == SHT_SYMTAB && symtab_off == 0 {
                symtab_off = sh_offset;
                symtab_size = sh_size;
                symtab_link = sh_link;
            }
        }

        if symtab_off == 0 {
            return None;
        }

        // Get the linked string table
        let strtab_sh = e_shoff + symtab_link * e_shentsize;
        if strtab_sh + e_shentsize <= data.len() {
            strtab_off = u64::from_le_bytes(data[strtab_sh + 24..strtab_sh + 32].try_into().ok()?) as usize;
            strtab_size = u64::from_le_bytes(data[strtab_sh + 32..strtab_sh + 40].try_into().ok()?) as usize;
        }

        if strtab_off == 0 {
            return None;
        }

        // Parse symbol entries (ELF64_Sym = 24 bytes each)
        let sym_entry_size = 24;
        let sym_count = symtab_size / sym_entry_size;
        let mut entries = Vec::new();

        for i in 0..sym_count {
            let s_off = symtab_off + i * sym_entry_size;
            if s_off + sym_entry_size > data.len() {
                break;
            }

            let st_name = u32::from_le_bytes(data[s_off..s_off + 4].try_into().ok()?) as usize;
            let st_info = data[s_off + 4];
            let st_value = u64::from_le_bytes(data[s_off + 8..s_off + 16].try_into().ok()?);
            let st_size = u64::from_le_bytes(data[s_off + 16..s_off + 24].try_into().ok()?);

            // Only include function symbols
            if (st_info & 0xF) != STT_FUNC {
                continue;
            }
            if st_value == 0 {
                continue;
            }

            // Read name from string table
            let name_off = strtab_off + st_name;
            if name_off >= data.len() {
                continue;
            }
            let name_end = data[name_off..].iter().position(|&b| b == 0)
                .map(|p| name_off + p)
                .unwrap_or(data.len().min(name_off + 256));
            let name = String::from(
                core::str::from_utf8(&data[name_off..name_end]).unwrap_or("?")
            );

            entries.push(Symbol {
                addr: st_value,
                size: st_size,
                name,
            });
        }

        // Sort by address for binary search
        entries.sort_by_key(|s| s.addr);

        // Extract module name from path
        let module_name = path.rsplit('/').next().unwrap_or(path);

        Some(Self {
            entries,
            module_name: String::from(module_name),
        })
    }

    /// Look up the symbol containing the given address.
    ///
    /// Returns `(symbol_name, offset_within_symbol)` or None.
    pub fn lookup(&self, addr: u64) -> Option<(&str, u64)> {
        // Binary search for the last symbol with addr <= target
        let idx = match self.entries.binary_search_by_key(&addr, |s| s.addr) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };

        let sym = &self.entries[idx];
        let offset = addr - sym.addr;

        // Check if within symbol bounds (or within 64 KB if size is 0)
        if sym.size > 0 && offset >= sym.size {
            return None;
        }
        if sym.size == 0 && offset > 0x10000 {
            return None;
        }

        Some((&sym.name, offset))
    }
}
