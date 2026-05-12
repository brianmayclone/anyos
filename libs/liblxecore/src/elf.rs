use crate::config::LxeConfig;
use crate::model::Elf64Diag;
use crate::rootfs::{
    linux_path_in_rootfs, path_exists, path_exists_no_follow, print_path_probe,
    resolve_rootfs_symlink_path, rootfs_for_path,
};
use alloc::string::String;
use anyos_std::{fs, println};

const EI_CLASS: usize = 4;
const ELFCLASS64: u8 = 2;
const EI_DATA: usize = 5;
const ELFDATA2LSB: u8 = 1;
const ET_DYN: u16 = 3;
const PT_INTERP: u32 = 3;
const ELF64_E_TYPE: usize = 16;
const ELF64_E_ENTRY: usize = 24;
const ELF64_E_PHOFF: usize = 32;
const ELF64_E_PHENTSIZE: usize = 54;
const ELF64_E_PHNUM: usize = 56;
const ELF64_PH_TYPE: usize = 0;
const ELF64_PH_OFFSET: usize = 8;
const ELF64_PH_FILESZ: usize = 32;
const ELF64_PH_SIZE: usize = 56;

pub(crate) fn diagnose_linux_binary(config: &LxeConfig, label: &str, path: &str) {
    let read_path =
        resolve_rootfs_symlink_path(&config.rootfs, path).unwrap_or_else(|| String::from(path));
    let data = match fs::read_to_vec(&read_path) {
        Ok(data) => data,
        Err(_) => {
            println!("[ERROR]\t{}: cannot read '{}'", label, path);
            return;
        }
    };
    let Some(info) = inspect_elf64(&data) else {
        println!(
            "[ERROR]\t{}: '{}' is not a supported little-endian ELF64 binary",
            label, path
        );
        return;
    };
    if info.entry == 0 {
        println!(
            "[WARN]\t{}: ELF64 entry point is 0; loader may reject this binary",
            label
        );
    }
    if let Some(interp) = info.interp_path {
        let rootfs = rootfs_for_path(config, path);
        let resolved = linux_path_in_rootfs(&rootfs, &interp);
        let final_path =
            resolve_rootfs_symlink_path(&rootfs, &resolved).unwrap_or_else(|| resolved.clone());
        if !(path_exists_no_follow(&resolved) || path_exists(&final_path)) {
            println!("[ERROR]\t{}: missing PT_INTERP {}", label, interp);
            println!("[ERROR]\t{}: expected interpreter at {}", label, resolved);
            print_path_probe(label, &resolved);
            print_path_probe(
                label,
                &linux_path_in_rootfs(&rootfs, "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"),
            );
        }
    }
    if fs::isatty(0) != 1 {
        println!(
            "[WARN]\t{}: stdin is not a tty; interactive Linux tools may fail",
            label
        );
    }
    if fs::isatty(1) != 1 {
        println!(
            "[WARN]\t{}: stdout is not a tty; terminal-oriented Linux tools may fail",
            label
        );
    }
}

fn inspect_elf64(data: &[u8]) -> Option<Elf64Diag> {
    if data.len() < 64 || &data[..4] != b"\x7FELF" {
        return None;
    }
    if data[EI_CLASS] != ELFCLASS64 || data[EI_DATA] != ELFDATA2LSB {
        return None;
    }
    let phoff = read_u64(data, ELF64_E_PHOFF)? as usize;
    let phentsize = read_u16(data, ELF64_E_PHENTSIZE)? as usize;
    let phnum = read_u16(data, ELF64_E_PHNUM)? as usize;
    if phentsize < ELF64_PH_SIZE {
        return None;
    }

    let mut interp_path = None;
    for idx in 0..phnum {
        let off = phoff.checked_add(idx.checked_mul(phentsize)?)?;
        if off.checked_add(ELF64_PH_SIZE)? > data.len() {
            return None;
        }
        if read_u32(data, off + ELF64_PH_TYPE)? != PT_INTERP {
            continue;
        }
        let interp_off = read_u64(data, off + ELF64_PH_OFFSET)? as usize;
        let interp_size = read_u64(data, off + ELF64_PH_FILESZ)? as usize;
        if interp_size == 0 || interp_size > 512 {
            continue;
        }
        let end = interp_off.checked_add(interp_size)?;
        if end > data.len() {
            continue;
        }
        let raw = &data[interp_off..end];
        let nul = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        if let Ok(path) = core::str::from_utf8(&raw[..nul]) {
            if !path.is_empty() {
                interp_path = Some(String::from(path));
            }
        }
    }

    Some(Elf64Diag {
        entry: read_u64(data, ELF64_E_ENTRY)?,
        is_dyn: read_u16(data, ELF64_E_TYPE)? == ET_DYN,
        interp_path,
    })
}

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    if off + 2 > data.len() {
        return None;
    }
    Some(u16::from_le_bytes([data[off], data[off + 1]]))
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    if off + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn read_u64(data: &[u8], off: usize) -> Option<u64> {
    if off + 8 > data.len() {
        return None;
    }
    Some(u64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ]))
}
