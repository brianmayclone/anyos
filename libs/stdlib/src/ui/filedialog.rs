//! Legacy compatibility wrapper for file dialogs.
//!
//! The only real implementation lives in `libanyui`. This module just loads
//! the shared library and forwards calls to its exported dialog functions.

use alloc::string::String;

/// Result of a file dialog interaction.
pub enum FileDialogResult {
    Selected(String),
    Cancelled,
}

type AnyuiInitFn = extern "C" fn() -> u32;
type OpenFolderFn = extern "C" fn(*mut u8, u32) -> u32;
type OpenFileFn = extern "C" fn(*mut u8, u32) -> u32;
type SaveFileFn = extern "C" fn(*mut u8, u32, *const u8, u32) -> u32;
type CreateFolderFn = extern "C" fn(*mut u8, u32) -> u32;

struct AnyuiDialogApi {
    init: AnyuiInitFn,
    open_folder: OpenFolderFn,
    open_file: OpenFileFn,
    save_file: SaveFileFn,
    create_folder: CreateFolderFn,
}

static mut ANYUI_DIALOG_API: Option<AnyuiDialogApi> = None;

/// Open a file browser to select a file.
///
/// `starting_path` is kept for API compatibility and currently ignored.
pub fn open_file(_starting_path: &str) -> FileDialogResult {
    let Some(api) = ensure_anyui_dialog_api() else {
        return FileDialogResult::Cancelled;
    };
    let mut buf = [0u8; 257];
    let len = (api.open_file)(buf.as_mut_ptr(), buf.len() as u32);
    decode_result(&buf, len)
}

/// Open a file browser to select a folder.
///
/// `starting_path` is kept for API compatibility and currently ignored.
pub fn open_folder(_starting_path: &str) -> FileDialogResult {
    let Some(api) = ensure_anyui_dialog_api() else {
        return FileDialogResult::Cancelled;
    };
    let mut buf = [0u8; 257];
    let len = (api.open_folder)(buf.as_mut_ptr(), buf.len() as u32);
    decode_result(&buf, len)
}

/// Open a save dialog.
///
/// `starting_path` is kept for API compatibility and currently ignored.
pub fn save_file(_starting_path: &str, default_name: &str) -> FileDialogResult {
    let Some(api) = ensure_anyui_dialog_api() else {
        return FileDialogResult::Cancelled;
    };
    let mut buf = [0u8; 257];
    let len = (api.save_file)(
        buf.as_mut_ptr(),
        buf.len() as u32,
        default_name.as_ptr(),
        default_name.len() as u32,
    );
    decode_result(&buf, len)
}

/// Open a dialog to create a new folder.
///
/// `parent_path` is kept for API compatibility and currently ignored.
pub fn create_folder(_parent_path: &str) -> FileDialogResult {
    let Some(api) = ensure_anyui_dialog_api() else {
        return FileDialogResult::Cancelled;
    };
    let mut buf = [0u8; 257];
    let len = (api.create_folder)(buf.as_mut_ptr(), buf.len() as u32);
    decode_result(&buf, len)
}

fn decode_result(buf: &[u8; 257], len: u32) -> FileDialogResult {
    if len == 0 {
        return FileDialogResult::Cancelled;
    }
    let n = (len as usize).min(buf.len());
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("");
    FileDialogResult::Selected(String::from(s))
}

fn ensure_anyui_dialog_api() -> Option<&'static AnyuiDialogApi> {
    unsafe {
        if let Some(api) = ANYUI_DIALOG_API.as_ref() {
            return Some(api);
        }

        let base = crate::dll::dll_load("/Libraries/libanyui.so") as u64;
        if base == 0 {
            return None;
        }

        let init: AnyuiInitFn = resolve_sym(base, b"anyui_init")?;
        let open_folder: OpenFolderFn = resolve_sym(base, b"anyui_open_folder")?;
        let open_file: OpenFileFn = resolve_sym(base, b"anyui_open_file")?;
        let save_file: SaveFileFn = resolve_sym(base, b"anyui_save_file")?;
        let create_folder: CreateFolderFn = resolve_sym(base, b"anyui_create_folder")?;

        let api = AnyuiDialogApi {
            init,
            open_folder,
            open_file,
            save_file,
            create_folder,
        };
        ANYUI_DIALOG_API = Some(api);
        let api_ref = ANYUI_DIALOG_API.as_ref()?;

        if (api_ref.init)() == 0 {
            ANYUI_DIALOG_API = None;
            return None;
        }

        ANYUI_DIALOG_API.as_ref()
    }
}

/// Mini ELF64 symbol resolver copied from the stdlib UI dynamic-loading path.
unsafe fn resolve_sym<T: Copy>(base: u64, name: &[u8]) -> Option<T> {
    let ehdr = base as *const u8;
    if *ehdr != 0x7F || *ehdr.add(1) != b'E' || *ehdr.add(2) != b'L' || *ehdr.add(3) != b'F' {
        return None;
    }
    let e_phoff = *(ehdr.add(32) as *const u64);
    let e_phnum = *(ehdr.add(56) as *const u16);

    let mut dynamic_va: u64 = 0;
    let mut link_base: u64 = u64::MAX;
    for i in 0..e_phnum as usize {
        let ph = (base + e_phoff + (i as u64) * 56) as *const u8;
        let p_type = *(ph as *const u32);
        if p_type == 1 {
            let p_vaddr = *(ph.add(16) as *const u64);
            if p_vaddr < link_base {
                link_base = p_vaddr;
            }
        }
        if p_type == 2 {
            dynamic_va = *(ph.add(16) as *const u64);
        }
    }
    if dynamic_va == 0 {
        return None;
    }

    let load_bias = if link_base != u64::MAX { base - link_base } else { 0 };
    dynamic_va += load_bias;

    let mut symtab: u64 = 0;
    let mut strtab: u64 = 0;
    let mut hash: u64 = 0;
    let dyn_ptr = dynamic_va as *const u8;
    for i in 0..128 {
        let entry = dyn_ptr.add(i * 16);
        let d_tag = *(entry as *const i64);
        let d_val = *(entry.add(8) as *const u64);
        match d_tag {
            6 => symtab = d_val,
            5 => strtab = d_val,
            4 => hash = d_val,
            0 => break,
            _ => {}
        }
    }
    if symtab == 0 || strtab == 0 || hash == 0 {
        return None;
    }

    let nbuckets = *(hash as *const u32);
    let buckets = (hash as *const u32).add(2);
    let chains = buckets.add(nbuckets as usize);

    let h = elf_hash_sym(name);
    let mut idx = *buckets.add((h % nbuckets) as usize);
    while idx != 0 {
        let sym = (symtab + idx as u64 * 24) as *const u8;
        let st_name = *(sym as *const u32);
        let st_value = *(sym.add(8) as *const u64);
        if st_value != 0 && cstr_eq_sym(strtab as *const u8, st_name as usize, name) {
            return Some(core::mem::transmute_copy::<u64, T>(&st_value));
        }
        idx = *chains.add(idx as usize);
    }
    None
}

fn elf_hash_sym(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xF000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

unsafe fn cstr_eq_sym(strtab: *const u8, offset: usize, name: &[u8]) -> bool {
    let s = strtab.add(offset);
    for (i, &b) in name.iter().enumerate() {
        if *s.add(i) != b {
            return false;
        }
    }
    *s.add(name.len()) == 0
}
