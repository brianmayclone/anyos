//! Minimal user-space dynamic linker for anyOS.
//!
//! Provides `dl_open()` and `dl_sym()` for loading ELF64 ET_DYN shared objects
//! linked by anyld. Symbol lookup uses the ELF hash table directly from mapped
//! memory — no kernel syscall needed beyond the initial `SYS_DLL_LOAD`.
//!
//! # Usage
//! ```no_run
//! let handle = dynlink::dl_open("/system/lib/libanyui.so").unwrap();
//! let init: extern "C" fn() -> u32 = unsafe {
//!     core::mem::transmute(dynlink::dl_sym(&handle, "anyui_init").unwrap())
//! };
//! init();
//! ```

#![no_std]

mod elf;

use elf::{Elf64Dyn, Elf64Ehdr, Elf64Phdr, Elf64Sym};

/// Handle to a loaded shared library.
pub struct DlHandle {
    /// Base virtual address where the .so is mapped.
    pub base: u64,
    /// Pointer to .dynsym in mapped memory.
    symtab: *const Elf64Sym,
    /// Pointer to .dynstr in mapped memory.
    strtab: *const u8,
    /// Pointer to ELF hash table buckets (after the [nbuckets, nchain] header).
    buckets: *const u32,
    /// Pointer to ELF hash table chains.
    chains: *const u32,
    /// Number of hash buckets.
    nbuckets: u32,
}

// DlHandle contains raw pointers but we only use them for reads in the same address space.
unsafe impl Send for DlHandle {}
unsafe impl Sync for DlHandle {}

/// Load a shared library by path.
///
/// Calls `SYS_DLL_LOAD` to map the .so into the process, then parses the
/// ELF header and .dynamic section from mapped memory to set up symbol lookup.
pub fn dl_open(path: &str) -> Option<DlHandle> {
    let base = anyos_std::dll::dll_load(path) as u64;
    if base == 0 {
        return None;
    }

    // Parse ELF header at the base address (first page of RX segment)
    let ehdr = unsafe { &*(base as *const Elf64Ehdr) };

    // Validate magic
    if ehdr.e_ident[0] != 0x7F
        || ehdr.e_ident[1] != b'E'
        || ehdr.e_ident[2] != b'L'
        || ehdr.e_ident[3] != b'F'
    {
        return None;
    }
    if ehdr.e_ident[4] != 2 {
        return None; // Not ELF64
    }
    if ehdr.e_type != 3 {
        return None; // Not ET_DYN
    }

    // Walk program headers to find PT_DYNAMIC and compute load_bias.
    // For base-0 .so files (dynamically loaded), p_vaddr values are link-time
    // offsets. The kernel loads them at `base`, so we need load_bias to find
    // the actual VA of .dynamic. For fixed-base .so files, load_bias = 0.
    let phdr_base = (base + ehdr.e_phoff) as *const Elf64Phdr;
    let mut dynamic_va: u64 = 0;
    let mut link_base: u64 = u64::MAX;

    for i in 0..ehdr.e_phnum as usize {
        let ph = unsafe { &*phdr_base.add(i) };
        if ph.p_type == 1 {
            // PT_LOAD — track lowest p_vaddr to determine link base
            if ph.p_vaddr < link_base {
                link_base = ph.p_vaddr;
            }
        }
        if ph.p_type == 2 {
            // PT_DYNAMIC
            dynamic_va = ph.p_vaddr;
        }
    }

    if dynamic_va == 0 {
        return None; // No .dynamic section
    }

    // Compute load bias: difference between actual base and link-time base
    let load_bias = if link_base != u64::MAX {
        base - link_base
    } else {
        0
    };
    dynamic_va += load_bias;

    // Walk .dynamic entries to find DT_SYMTAB, DT_STRTAB, DT_HASH
    let mut symtab_va: u64 = 0;
    let mut strtab_va: u64 = 0;
    let mut hash_va: u64 = 0;

    let dyn_ptr = dynamic_va as *const Elf64Dyn;
    for i in 0..128 {
        let d = unsafe { &*dyn_ptr.add(i) };
        match d.d_tag {
            6 => symtab_va = d.d_val,  // DT_SYMTAB
            5 => strtab_va = d.d_val,  // DT_STRTAB
            4 => hash_va = d.d_val,    // DT_HASH
            0 => break,                // DT_NULL
            _ => {}
        }
    }

    if symtab_va == 0 || strtab_va == 0 || hash_va == 0 {
        return None;
    }

    // Parse hash table header: [nbuckets: u32, nchain: u32]
    let hash_ptr = hash_va as *const u32;
    let nbuckets = unsafe { *hash_ptr };
    let _nchain = unsafe { *hash_ptr.add(1) };
    let buckets = unsafe { hash_ptr.add(2) };
    let chains = unsafe { buckets.add(nbuckets as usize) };

    Some(DlHandle {
        base,
        symtab: symtab_va as *const Elf64Sym,
        strtab: strtab_va as *const u8,
        buckets,
        chains,
        nbuckets,
    })
}

/// Look up a symbol by name in a loaded shared library.
///
/// Returns the symbol's virtual address as a raw pointer, or `None` if not found.
/// The caller must cast to the appropriate function pointer type.
pub fn dl_sym(handle: &DlHandle, name: &str) -> Option<*const ()> {
    let h = elf_hash(name.as_bytes());
    let bucket_idx = h % handle.nbuckets;

    let mut idx = unsafe { *handle.buckets.add(bucket_idx as usize) };

    while idx != 0 {
        let sym = unsafe { &*handle.symtab.add(idx as usize) };
        if sym.st_value != 0 {
            let sym_name = unsafe { cstr_eq(handle.strtab.add(sym.st_name as usize), name.as_bytes()) };
            if sym_name {
                return Some(sym.st_value as *const ());
            }
        }
        idx = unsafe { *handle.chains.add(idx as usize) };
    }
    None
}

// ── dll_exports! macro ──────────────────────────────────────────────────────

/// Generate the boilerplate for a shared-library client crate.
///
/// Generates a private struct holding function pointers, a `static mut` global,
/// a `lib()` accessor, and a public `init() -> bool` that loads the library
/// and resolves all symbols.
///
/// # Example
/// ```ignore
/// dynlink::dll_exports! {
///     lib_path: "/Libraries/libfont.so",
///     lib_struct: FontLib,
///     init_call: "font_init",  // optional: symbol called after resolve
///     symbols: {
///         font_load(path_ptr: *const u8, len: u32) -> u32,
///         font_unload(id: u32) -> (),
///     }
/// }
///
/// // Now you can call:  init(), and lib().font_load, lib().font_unload, etc.
/// ```
#[macro_export]
macro_rules! dll_exports {
    (
        lib_path: $path:expr,
        lib_struct: $name:ident,
        init_call: $init_sym:expr,
        symbols: {
            $( $sym:ident ( $($pname:ident : $pty:ty),* $(,)? ) -> $ret:ty ),+ $(,)?
        }
    ) => {
        struct $name {
            _handle: $crate::DlHandle,
            $( $sym: extern "C" fn( $($pty),* ) -> $ret, )+
        }

        static mut LIB: Option<$name> = None;

        /// Get a reference to the loaded library. Panics if not initialized.
        pub(crate) fn lib() -> &'static $name {
            unsafe { LIB.as_ref().expect(concat!(stringify!($name), " not loaded")) }
        }

        /// Load the shared library and resolve all symbols. Call once at program start.
        pub fn init() -> bool {
            let handle = match $crate::dl_open($path) {
                Some(h) => h,
                None => return false,
            };

            unsafe {
                let lib = $name {
                    $(
                        $sym: {
                            let ptr = match $crate::dl_sym(&handle, stringify!($sym)) {
                                Some(p) => p,
                                None => return false,
                            };
                            core::mem::transmute_copy::<*const (), extern "C" fn( $($pty),* ) -> $ret>(&ptr)
                        },
                    )+
                    _handle: handle,
                };
                // Call init symbol
                let init_ptr = match $crate::dl_sym(&lib._handle, $init_sym) {
                    Some(p) => p,
                    None => {
                        LIB = Some(lib);
                        return true;
                    }
                };
                let init_fn: extern "C" fn() = core::mem::transmute_copy::<*const (), extern "C" fn()>(&init_ptr);
                (init_fn)();
                LIB = Some(lib);
            }
            true
        }
    };
    // Variant without init_call
    (
        lib_path: $path:expr,
        lib_struct: $name:ident,
        symbols: {
            $( $sym:ident ( $($pname:ident : $pty:ty),* $(,)? ) -> $ret:ty ),+ $(,)?
        }
    ) => {
        struct $name {
            _handle: $crate::DlHandle,
            $( $sym: extern "C" fn( $($pty),* ) -> $ret, )+
        }

        static mut LIB: Option<$name> = None;

        /// Get a reference to the loaded library. Panics if not initialized.
        pub(crate) fn lib() -> &'static $name {
            unsafe { LIB.as_ref().expect(concat!(stringify!($name), " not loaded")) }
        }

        /// Load the shared library and resolve all symbols. Call once at program start.
        pub fn init() -> bool {
            let handle = match $crate::dl_open($path) {
                Some(h) => h,
                None => return false,
            };

            unsafe {
                let lib = $name {
                    $(
                        $sym: {
                            let ptr = match $crate::dl_sym(&handle, stringify!($sym)) {
                                Some(p) => p,
                                None => return false,
                            };
                            core::mem::transmute_copy::<*const (), extern "C" fn( $($pty),* ) -> $ret>(&ptr)
                        },
                    )+
                    _handle: handle,
                };
                LIB = Some(lib);
            }
            true
        }
    };
}

/// ELF hash function (SysV ABI).
fn elf_hash(name: &[u8]) -> u32 {
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

/// Compare a NUL-terminated C string with a Rust byte slice.
unsafe fn cstr_eq(cstr: *const u8, name: &[u8]) -> bool {
    for (i, &b) in name.iter().enumerate() {
        let c = unsafe { *cstr.add(i) };
        if c != b {
            return false;
        }
    }
    // Check that the C string is also terminated here
    unsafe { *cstr.add(name.len()) == 0 }
}
