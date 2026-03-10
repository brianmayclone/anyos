//! ARM64 boot information parsing.
//!
//! On ARM64, the bootloader (U-Boot or QEMU -kernel) passes the DTB physical
//! address in X0. This module provides helpers to extract boot information
//! from the Device Tree Blob (Flattened Device Tree / FDT format).

use core::sync::atomic::{AtomicU64, Ordering};

/// Physical address of the DTB, saved at boot entry.
static DTB_ADDR: AtomicU64 = AtomicU64::new(0);

/// Save the DTB address from X0 (called from boot.S entry).
pub fn save_dtb_addr(addr: u64) {
    DTB_ADDR.store(addr, Ordering::Relaxed);
}

/// Get the DTB physical address.
pub fn dtb_addr() -> u64 {
    DTB_ADDR.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// FDT (Flattened Device Tree) minimal parser
// ---------------------------------------------------------------------------
//
// FDT binary layout:
//   - Header (40 bytes): magic, totalsize, off_dt_struct, off_dt_strings,
//                        off_mem_rsvmap, version, ...
//   - Memory reservation map
//   - Structure block (tokens)
//   - Strings block
//
// Token types:
//   FDT_BEGIN_NODE = 0x01  — start of a node
//   FDT_END_NODE   = 0x02  — end of a node
//   FDT_PROP       = 0x03  — property (len, nameoff, data)
//   FDT_NOP        = 0x04  — no-op
//   FDT_END        = 0x09  — end of structure block

const FDT_MAGIC: u32 = 0xD00DFEED;

const FDT_BEGIN_NODE: u32 = 0x1;
const FDT_END_NODE: u32   = 0x2;
const FDT_PROP: u32       = 0x3;
const FDT_NOP: u32        = 0x4;
const FDT_END: u32        = 0x9;

/// FDT header (big-endian on disk, we byte-swap manually).
#[repr(C)]
struct FdtHeader {
    magic:           u32,
    totalsize:       u32,
    off_dt_struct:   u32,
    off_dt_strings:  u32,
    _off_mem_rsvmap: u32,
    version:         u32,
    _last_comp_ver:  u32,
    _boot_cpuid:     u32,
    _size_dt_strings:u32,
    _size_dt_struct: u32,
}

#[inline]
fn be32(p: *const u8) -> u32 {
    unsafe {
        let b = core::slice::from_raw_parts(p, 4);
        ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
    }
}

#[inline]
fn be64(p: *const u8) -> u64 {
    ((be32(p) as u64) << 32) | (be32(unsafe { p.add(4) }) as u64)
}

/// Parse the DTB to find the first `/memory` node and return (base, size).
///
/// The `reg` property of the memory node contains one or more (address, size)
/// pairs encoded as big-endian 64-bit values (address-cells=2, size-cells=2
/// is the QEMU virt default).
///
/// Returns `None` if parsing fails or the DTB address is zero.
fn parse_dtb_memory(dtb_phys: u64) -> Option<(u64, u64)> {
    if dtb_phys == 0 {
        return None;
    }

    // Map physical DTB address to virtual (TTBR1 device mapping).
    // QEMU virt DTB is placed just below 1 GiB (around 0x4800_0000 area),
    // which falls inside the RAM region. Since the MMU is already on and the
    // lower 1 GiB identity-map covers 0x4000_0000..0x8000_0000, we can
    // access it directly — the physical and virtual addresses are the same
    // in the identity window.
    let base = dtb_phys as *const u8;

    // Validate FDT magic (big-endian)
    if be32(base) != FDT_MAGIC {
        return None;
    }

    let off_struct  = be32(unsafe { base.add(8) })  as usize;
    let off_strings = be32(unsafe { base.add(12) }) as usize;

    let strings_base = unsafe { base.add(off_strings) };
    let mut ptr      = unsafe { base.add(off_struct) };

    // State machine over the structure block
    let mut depth: i32 = 0;
    let mut in_memory_node = false;

    loop {
        // Tokens are 4-byte aligned
        let align = ptr as usize % 4;
        if align != 0 {
            ptr = unsafe { ptr.add(4 - align) };
        }

        let token = be32(ptr);
        ptr = unsafe { ptr.add(4) };

        match token {
            FDT_BEGIN_NODE => {
                // Node name: null-terminated string
                let name_ptr = ptr;
                // Advance past the name (including null terminator, aligned to 4)
                let mut len = 0usize;
                while unsafe { *name_ptr.add(len) } != 0 {
                    len += 1;
                }
                let name = unsafe { core::slice::from_raw_parts(name_ptr, len) };
                let padded = (len + 1 + 3) & !3;
                ptr = unsafe { ptr.add(padded) };

                depth += 1;
                // Check if this is a `memory` or `memory@…` node at depth 1
                if depth == 1 {
                    in_memory_node = name.starts_with(b"memory");
                } else {
                    in_memory_node = false;
                }
            }
            FDT_END_NODE => {
                depth -= 1;
                in_memory_node = false;
            }
            FDT_PROP => {
                let prop_len    = be32(ptr) as usize; ptr = unsafe { ptr.add(4) };
                let nameoff     = be32(ptr) as usize; ptr = unsafe { ptr.add(4) };
                let data_ptr    = ptr;
                let padded_len  = (prop_len + 3) & !3;
                ptr = unsafe { ptr.add(padded_len) };

                if in_memory_node {
                    // Get property name from strings block
                    let name_ptr = unsafe { strings_base.add(nameoff) };
                    let mut nlen = 0usize;
                    while unsafe { *name_ptr.add(nlen) } != 0 { nlen += 1; }
                    let pname = unsafe { core::slice::from_raw_parts(name_ptr, nlen) };

                    if pname == b"reg" && prop_len >= 16 {
                        // First cell pair: (base_addr 8 bytes, size 8 bytes)
                        let mem_base = be64(data_ptr);
                        let mem_size = be64(unsafe { data_ptr.add(8) });
                        if mem_size > 0 {
                            return Some((mem_base, mem_size));
                        }
                    }
                }
            }
            FDT_NOP => {}
            FDT_END | _ => break,
        }
    }

    None
}

/// Detect available RAM by parsing the DTB.
///
/// Falls back to 512 MiB at 0x4000_0000 (QEMU virt default) if the DTB is
/// unavailable or parsing fails.
pub fn detect_memory() -> (u64, u64) {
    let dtb = dtb_addr();
    if let Some((base, size)) = parse_dtb_memory(dtb) {
        crate::serial_println!(
            "  DTB memory: {:#010x} - {:#010x} ({} MiB)",
            base, base + size, size / (1024 * 1024),
        );
        return (base, size);
    }

    // Fallback: QEMU virt default — 512 MiB at 1 GiB
    let base: u64 = 0x4000_0000;
    let size: u64 = 512 * 1024 * 1024;
    crate::serial_println!(
        "  Memory (fallback): {:#010x} - {:#010x} ({} MiB)",
        base, base + size, size / (1024 * 1024),
    );
    (base, size)
}
