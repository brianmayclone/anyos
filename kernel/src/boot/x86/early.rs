//! Early x86_64 boot parsing and boot-parameter handling.

use crate::boot::{set_boot_mode, NOGUI, SETUP_MODE};
use crate::boot_info::{self, BootInfo};
use crate::serial_println;
use core::sync::atomic::Ordering;

pub(super) fn early_output() {
    let version = option_env!("ANYOS_VERSION").unwrap_or("dev");
    serial_println!("");
    serial_println!("  .anyOS Kernel (x86_64) v{}", version);
    crate::drivers::vga_text::init();
}

pub(super) fn read_boot_info(boot_info_addr: u64) -> &'static BootInfo {
    let boot_info = unsafe { &*(boot_info_addr as *const BootInfo) };
    let magic = unsafe { core::ptr::addr_of!((*boot_info).magic).read_unaligned() };
    if magic != boot_info::BOOT_INFO_MAGIC {
        serial_println!("WARNING: BootInfo magic mismatch (got {:#010x})", magic);
    } else {
        serial_println!("BootInfo validated (magic OK)");
    }

    let boot_mode = unsafe { core::ptr::addr_of!((*boot_info).boot_mode).read_unaligned() };
    set_boot_mode(boot_mode);
    serial_println!("Boot mode: {}", if boot_mode == 1 { "UEFI" } else { "BIOS" });

    let kernel_phys_start =
        unsafe { core::ptr::addr_of!((*boot_info).kernel_phys_start).read_unaligned() };
    let kernel_phys_end =
        unsafe { core::ptr::addr_of!((*boot_info).kernel_phys_end).read_unaligned() };
    serial_println!(
        "Kernel loaded at {:#010x} - {:#010x}",
        kernel_phys_start,
        kernel_phys_end
    );

    parse_boot_params(boot_info);
    store_bootloader_edid(boot_info);
    boot_info
}

fn parse_boot_params(boot_info: &BootInfo) {
    let params = unsafe { core::ptr::addr_of!((*boot_info).boot_params).read_unaligned() };
    let len = params.iter().position(|&b| b == 0).unwrap_or(params.len());
    if len == 0 {
        return;
    }

    if let Ok(params_str) = core::str::from_utf8(&params[..len]) {
        serial_println!("Boot params: \"{}\"", params_str);
        for token in params_str.split_ascii_whitespace() {
            match token {
                "verbose" => {
                    crate::drivers::serial::set_verbose(true);
                    serial_println!("Verbose logging enabled via boot params");
                }
                "nogui" => {
                    NOGUI.store(true, Ordering::Relaxed);
                    serial_println!("No-GUI mode enabled via boot params (textmode_console)");
                }
                "setup" => {
                    SETUP_MODE.store(true, Ordering::Relaxed);
                    serial_println!("Setup mode enabled (ISO installer)");
                }
                _ => parse_resolution_override(token),
            }
        }
    }
}

fn parse_resolution_override(token: &str) {
    let Some(resolution) = token.strip_prefix("res=") else {
        return;
    };
    let Some((width, height)) = resolution.split_once('x') else {
        return;
    };
    if let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) {
        if width >= 640 && height >= 480 {
            crate::drivers::gpu::set_preferred_resolution(width, height);
            serial_println!("Preferred resolution: {}x{}", width, height);
        }
    }
}

fn store_bootloader_edid(boot_info: &BootInfo) {
    let edid_valid = unsafe { core::ptr::addr_of!((*boot_info).edid_valid).read_unaligned() };
    if edid_valid == 1 {
        let edid_data = unsafe { core::ptr::addr_of!((*boot_info).edid_data).read_unaligned() };
        crate::drivers::monitor::store_bootloader_edid(edid_data);
        serial_println!("[OK] Bootloader EDID: 128 bytes from VBE DDC");
    }
}
