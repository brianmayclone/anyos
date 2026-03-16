//! Shared VM setup logic used by both vmctl and vmmanager.
//!
//! This module consolidates duplicated code for BIOS loading, CPU state
//! initialization, disk image attachment, and MAC address handling.

use std::path::{Path, PathBuf};

use crate::ffi::*;
use crate::backend::{VcpuRegs, VcpuSregs, SegmentReg, DescriptorTable};

// ── Guest OS and architecture types ─────────────────────────────────

/// Guest CPU architecture.
#[derive(Clone, Debug, PartialEq)]
pub enum GuestArch {
    /// 32-bit x86 (i386/i686)
    X86,
    /// 64-bit x86_64 / AMD64
    X64,
}

/// Guest operating system type.
///
/// Used by the VM setup logic to apply OS-specific tweaks (e.g. ACPI
/// table variants, CPUID feature masks, boot parameters).
#[derive(Clone, Debug, PartialEq)]
pub enum GuestOs {
    Other,
    // Windows desktop
    Windows7,
    Windows8,
    Windows10,
    Windows11,
    // Windows server
    WindowsServer2016,
    WindowsServer2019,
    WindowsServer2022,
    // Linux
    Ubuntu,
    Debian,
    Fedora,
    OpenSuse,
    RedHat,
    Arch,
    LinuxOther,
    // BSD / DOS
    FreeBsd,
    DosFreeDos,
}

impl GuestOs {
    /// All known guest OS variants, grouped by category.
    pub const ALL: &'static [GuestOs] = &[
        GuestOs::Other,
        GuestOs::Windows7,
        GuestOs::Windows8,
        GuestOs::Windows10,
        GuestOs::Windows11,
        GuestOs::WindowsServer2016,
        GuestOs::WindowsServer2019,
        GuestOs::WindowsServer2022,
        GuestOs::Ubuntu,
        GuestOs::Debian,
        GuestOs::Fedora,
        GuestOs::OpenSuse,
        GuestOs::RedHat,
        GuestOs::Arch,
        GuestOs::LinuxOther,
        GuestOs::FreeBsd,
        GuestOs::DosFreeDos,
    ];

    /// Human-readable display label.
    pub fn label(&self) -> &'static str {
        match self {
            GuestOs::Other              => "Other / Unknown",
            GuestOs::Windows7           => "Windows 7",
            GuestOs::Windows8           => "Windows 8 / 8.1",
            GuestOs::Windows10          => "Windows 10",
            GuestOs::Windows11          => "Windows 11",
            GuestOs::WindowsServer2016  => "Windows Server 2016",
            GuestOs::WindowsServer2019  => "Windows Server 2019",
            GuestOs::WindowsServer2022  => "Windows Server 2022",
            GuestOs::Ubuntu             => "Ubuntu Linux",
            GuestOs::Debian             => "Debian Linux",
            GuestOs::Fedora             => "Fedora Linux",
            GuestOs::OpenSuse           => "openSUSE Linux",
            GuestOs::RedHat             => "Red Hat Enterprise Linux",
            GuestOs::Arch               => "Arch Linux",
            GuestOs::LinuxOther         => "Linux (Other)",
            GuestOs::FreeBsd            => "FreeBSD",
            GuestOs::DosFreeDos         => "DOS / FreeDOS",
        }
    }

    /// Category grouping for UI display.
    pub fn category(&self) -> &'static str {
        match self {
            GuestOs::Other => "Other",
            GuestOs::Windows7 | GuestOs::Windows8 | GuestOs::Windows10
            | GuestOs::Windows11 | GuestOs::WindowsServer2016
            | GuestOs::WindowsServer2019 | GuestOs::WindowsServer2022 => "Windows",
            GuestOs::Ubuntu | GuestOs::Debian | GuestOs::Fedora
            | GuestOs::OpenSuse | GuestOs::RedHat | GuestOs::Arch
            | GuestOs::LinuxOther => "Linux",
            GuestOs::FreeBsd => "BSD",
            GuestOs::DosFreeDos => "DOS",
        }
    }

    /// Serialize to config file string.
    pub fn to_config_str(&self) -> &'static str {
        match self {
            GuestOs::Other              => "other",
            GuestOs::Windows7           => "win7",
            GuestOs::Windows8           => "win8",
            GuestOs::Windows10          => "win10",
            GuestOs::Windows11          => "win11",
            GuestOs::WindowsServer2016  => "winserv2016",
            GuestOs::WindowsServer2019  => "winserv2019",
            GuestOs::WindowsServer2022  => "winserv2022",
            GuestOs::Ubuntu             => "ubuntu",
            GuestOs::Debian             => "debian",
            GuestOs::Fedora             => "fedora",
            GuestOs::OpenSuse           => "opensuse",
            GuestOs::RedHat             => "rhel",
            GuestOs::Arch               => "arch",
            GuestOs::LinuxOther         => "linux",
            GuestOs::FreeBsd            => "freebsd",
            GuestOs::DosFreeDos         => "dos",
        }
    }

    /// Deserialize from config file string.
    pub fn from_config_str(s: &str) -> GuestOs {
        match s {
            "win7"        => GuestOs::Windows7,
            "win8"        => GuestOs::Windows8,
            "win10"       => GuestOs::Windows10,
            "win11"       => GuestOs::Windows11,
            "winserv2016" => GuestOs::WindowsServer2016,
            "winserv2019" => GuestOs::WindowsServer2019,
            "winserv2022" => GuestOs::WindowsServer2022,
            "ubuntu"      => GuestOs::Ubuntu,
            "debian"      => GuestOs::Debian,
            "fedora"      => GuestOs::Fedora,
            "opensuse"    => GuestOs::OpenSuse,
            "rhel"        => GuestOs::RedHat,
            "arch"        => GuestOs::Arch,
            "linux"       => GuestOs::LinuxOther,
            "freebsd"     => GuestOs::FreeBsd,
            "dos"         => GuestOs::DosFreeDos,
            _             => GuestOs::Other,
        }
    }

    /// Returns true if this is a Windows guest.
    pub fn is_windows(&self) -> bool {
        self.category() == "Windows"
    }

    /// Returns true if this is a Linux guest.
    pub fn is_linux(&self) -> bool {
        self.category() == "Linux"
    }

    /// Returns true if this OS has a built-in AHCI driver during installation.
    /// Windows 7 does NOT — it needs IDE for both disk and CDROM.
    /// Windows 8+ and all Linux distros have native AHCI support.
    pub fn has_native_ahci(&self) -> bool {
        match self {
            GuestOs::Windows7 | GuestOs::DosFreeDos => false,
            _ => true,
        }
    }
}

// ── Error helpers ────────────────────────────────────────────────────

/// Retrieve the last error message from libcorevm.
pub fn get_last_error() -> Option<String> {
    let len = corevm_last_error_len() as usize;
    if len == 0 { return None; }
    let ptr = corevm_last_error();
    if ptr.is_null() { return None; }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

// ── BIOS search paths ───────────────────────────────────────────────

/// Build a list of directories to search for BIOS files.
///
/// Accepts an optional list of extra search paths (e.g. application-specific
/// asset directories) that are prepended to the default system paths.
pub fn bios_search_paths(extra_paths: &[PathBuf]) -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let mut paths: Vec<PathBuf> = extra_paths.to_vec();

    if let Some(d) = &exe_dir {
        paths.push(d.join("bios"));
        paths.push(d.to_path_buf());
    }

    // Project source tree (works for both vmctl and vmmanager when run from cargo)
    if let Some(d) = &exe_dir {
        // Walk up from target/debug to project root
        let mut dir = d.as_path();
        for _ in 0..5 {
            if let Some(parent) = dir.parent() {
                dir = parent;
                let candidate = dir.join("libs/libcorevm/bios");
                if candidate.exists() {
                    paths.push(candidate);
                    paths.push(dir.join("build"));
                    break;
                }
            }
        }
    }

    // Common vmmanager asset paths
    if let Some(d) = &exe_dir {
        if let Some(parent) = d.parent() {
            paths.push(parent.join("vmmanager/assets/bios"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/share/corevm/bios"));
        paths.push(PathBuf::from("/usr/local/share/corevm/bios"));
        paths.push(PathBuf::from("/usr/share/seabios"));
        paths.push(PathBuf::from("/usr/share/qemu"));
        paths.push(PathBuf::from("/mnt/c/Program Files/qemu/share"));
    }

    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from("C:\\Program Files\\qemu\\share"));
    }

    paths
}

/// Find a BIOS file by name in the standard search paths.
pub fn find_bios(name: &str, extra_paths: &[PathBuf]) -> Option<PathBuf> {
    for dir in bios_search_paths(extra_paths) {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

// ── BIOS loading ────────────────────────────────────────────────────

/// Load SeaBIOS into the VM: main BIOS at 0xC0000, ROM overlay at top of
/// 4GB, and VGA BIOS via fw_cfg.
pub fn load_seabios(handle: u64, extra_bios_paths: &[PathBuf]) -> Result<(), String> {
    let bios_path = find_bios("bios.bin", extra_bios_paths)
        .ok_or("SeaBIOS bios.bin not found")?;
    let vgabios_path = find_bios("vgabios.bin", extra_bios_paths)
        .ok_or("VGA BIOS vgabios.bin not found")?;

    let bios = std::fs::read(&bios_path)
        .map_err(|e| format!("Failed to read BIOS: {}", e))?;
    let vgabios = std::fs::read(&vgabios_path)
        .map_err(|e| format!("Failed to read VGA BIOS: {}", e))?;

    // Load full BIOS at 0xC0000 (256KB SeaBIOS covers 0xC0000-0xFFFFF).
    corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

    // ROM overlay at top of 4GB address space.
    let rom_base = 0x1_0000_0000u64 - bios.len() as u64;
    let rom_size = bios.len();
    let rom_alloc = (rom_size + 0xFFF) & !0xFFF;
    let layout = std::alloc::Layout::from_size_align(rom_alloc, 4096)
        .map_err(|e| format!("ROM layout error: {}", e))?;
    let rom_ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if rom_ptr.is_null() {
        return Err("Failed to allocate ROM memory".into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bios.as_ptr(), rom_ptr, rom_size);
    }
    let ret = corevm_set_memory_region(handle, 1, rom_base, rom_alloc as u64, rom_ptr);
    if ret != 0 {
        return Err(format!("Failed to map ROM at 0x{:X}", rom_base));
    }
    // rom_ptr is intentionally leaked — it must remain valid for VM lifetime

    // VGA BIOS via fw_cfg so SeaBIOS loads it as option ROM at 0xC0000
    let name = b"vgaroms/vgabios.bin";
    let fw_rc = corevm_fw_cfg_add_file(
        handle,
        name.as_ptr(), name.len() as u32,
        vgabios.as_ptr(), vgabios.len() as u32,
    );
    if fw_rc != 0 {
        return Err(format!(
            "Failed to add VGA BIOS to fw_cfg (rc={}, vgabios_size={})",
            fw_rc, vgabios.len()
        ));
    }

    Ok(())
}

/// Load CoreVM custom BIOS into the VM at 0xF0000.
pub fn load_corevm_bios(handle: u64, extra_bios_paths: &[PathBuf]) -> Result<(), String> {
    let bios_path = find_bios("corevm-bios.bin", extra_bios_paths)
        .ok_or("CoreVM BIOS not found")?;
    let bios = std::fs::read(&bios_path)
        .map_err(|e| format!("Failed to read BIOS: {}", e))?;

    corevm_load_binary(handle, 0xF0000, bios.as_ptr(), bios.len() as u32);
    Ok(())
}

// ── CPU state initialization ────────────────────────────────────────

/// Set initial CPU state to real-mode reset vector (F000:FFF0).
pub fn set_initial_cpu_state(handle: u64) -> Result<(), String> {
    let data_seg = SegmentReg {
        base: 0,
        limit: 0xFFFF,
        selector: 0,
        type_: 0x03, // read/write, accessed
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 0,
        g: 0,
        avl: 0,
    };

    let sregs = VcpuSregs {
        cs: SegmentReg {
            base: 0xF0000,
            limit: 0xFFFF,
            selector: 0xF000,
            type_: 0x0B, // execute/read, accessed
            present: 1,
            dpl: 0,
            db: 0,
            s: 1,
            l: 0,
            g: 0,
            avl: 0,
        },
        ds: data_seg,
        es: data_seg,
        fs: data_seg,
        gs: data_seg,
        ss: data_seg,
        tr: SegmentReg {
            base: 0,
            limit: 0xFFFF,
            selector: 0,
            type_: 0x0B, // 16-bit busy TSS
            present: 1,
            dpl: 0,
            db: 0,
            s: 0, // system segment
            l: 0,
            g: 0,
            avl: 0,
        },
        ldt: SegmentReg {
            base: 0,
            limit: 0xFFFF,
            selector: 0,
            type_: 0x02, // LDT
            present: 1,
            dpl: 0,
            db: 0,
            s: 0, // system segment
            l: 0,
            g: 0,
            avl: 0,
        },
        gdt: DescriptorTable { base: 0, limit: 0xFFFF },
        idt: DescriptorTable { base: 0, limit: 0xFFFF },
        cr0: 0x10, // ET bit set (FPU extension type), PE=0 (real mode)
        cr2: 0,
        cr3: 0,
        cr4: 0,
        efer: 0,
    };

    let rc1 = corevm_set_vcpu_sregs(handle, 0, &sregs);
    if rc1 != 0 {
        let e = get_last_error().unwrap_or_else(|| "unknown".into());
        return Err(format!("set_vcpu_sregs failed (rc={}): {}", rc1, e));
    }

    let mut regs = VcpuRegs::default();
    regs.rip = 0xFFF0;
    regs.rflags = 0x02; // reserved bit
    let rc2 = corevm_set_vcpu_regs(handle, 0, &regs);
    if rc2 != 0 {
        let e = get_last_error().unwrap_or_else(|| "unknown".into());
        return Err(format!("set_vcpu_regs failed (rc={}): {}", rc2, e));
    }
    Ok(())
}

/// Write COM1 base address into BDA so SeaBIOS finds the serial port.
pub fn setup_bda_com1(handle: u64) {
    let com1_base: u16 = 0x03F8;
    corevm_write_phys(handle, 0x400, com1_base.to_le_bytes().as_ptr(), 2);
}

// ── Disk image attachment ───────────────────────────────────────────

/// Attach a disk or ISO image to an AHCI port via file descriptor.
#[cfg(unix)]
pub fn attach_image_to_ahci(handle: u64, path: &str, port: u32, is_cdrom: bool) -> Result<(), String> {
    use std::os::unix::io::IntoRawFd;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(!is_cdrom)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    let fd = file.into_raw_fd();

    let ret = if is_cdrom {
        corevm_ahci_attach_cdrom(handle, port, fd, size)
    } else {
        corevm_ahci_attach_disk(handle, port, fd, size)
    };

    if ret != 0 {
        // Close fd on failure by reclaiming ownership
        unsafe { drop(std::fs::File::from_raw_fd(fd)); }
        return Err(format!("Failed to attach {} to AHCI port {}", path, port));
    }
    // fd ownership transferred to AHCI, do NOT close it
    Ok(())
}

#[cfg(unix)]
use std::os::unix::io::FromRawFd;

/// Attach a disk or ISO image to an AHCI port via file handle (Windows).
#[cfg(windows)]
pub fn attach_image_to_ahci(handle: u64, path: &str, port: u32, is_cdrom: bool) -> Result<(), String> {
    use std::os::windows::io::IntoRawHandle;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(!is_cdrom)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    let handle_raw = file.into_raw_handle();
    let fd = handle_raw as isize as i32;

    let ret = if is_cdrom {
        corevm_ahci_attach_cdrom(handle, port, fd, size)
    } else {
        corevm_ahci_attach_disk(handle, port, fd, size)
    };

    if ret != 0 {
        return Err(format!("Failed to attach {} to AHCI port {}", path, port));
    }
    Ok(())
}

/// Attach a CDROM/ISO image to the IDE controller (legacy, Windows-compatible).
/// Uses IDE slave device — Windows has built-in ATAPI drivers for this.
#[cfg(unix)]
pub fn attach_cdrom_to_ide(handle: u64, path: &str) -> Result<(), String> {
    use std::os::unix::io::IntoRawFd;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    let fd = file.into_raw_fd();

    let ret = corevm_ide_attach_cdrom(handle, fd, size);
    if ret != 0 {
        unsafe { drop(std::fs::File::from_raw_fd(fd)); }
        return Err(format!("Failed to attach {} to IDE CDROM", path));
    }
    Ok(())
}

#[cfg(windows)]
pub fn attach_cdrom_to_ide(handle: u64, path: &str) -> Result<(), String> {
    use std::os::windows::io::IntoRawHandle;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    let handle_raw = file.into_raw_handle();
    let fd = handle_raw as isize as i32;

    let ret = corevm_ide_attach_cdrom(handle, fd, size);
    if ret != 0 {
        return Err(format!("Failed to attach {} to IDE CDROM", path));
    }
    Ok(())
}

// ── Display / VRAM ─────────────────────────────────────────────────

/// Set the VRAM size for the VM's virtual GPU.
/// Must be called BEFORE `corevm_setup_standard_devices()`.
/// `vram_mb`: size in MiB (8-256, 0 = default 16).
pub fn set_vram_mb(handle: u64, vram_mb: u32) {
    corevm_set_vram_mb(handle, vram_mb);
}

/// Configure disk I/O cache for an AHCI port.
/// `mode`: 0 = WriteBack, 1 = WriteThrough, 2 = None.
pub fn configure_disk_cache(handle: u64, port: u32, cache_mb: u32, mode: u32) {
    corevm_ahci_set_cache(handle, port, cache_mb, mode);
}

// ── Network ROM ─────────────────────────────────────────────────────

/// Load the E1000 PXE/iPXE option ROM via fw_cfg.
/// SeaBIOS discovers this as `genroms/pxe-e1000.rom` and loads it as
/// a PCI option ROM for the E1000 NIC. Without this ROM, the NIC may
/// not be fully initialized by the BIOS and guest drivers may fail.
pub fn load_e1000_rom(handle: u64, extra_paths: &[PathBuf]) -> Result<(), String> {
    let rom_path = find_bios("pxe-e1000.rom", extra_paths)
        .ok_or("E1000 PXE ROM (pxe-e1000.rom) not found")?;
    let rom_data = std::fs::read(&rom_path)
        .map_err(|e| format!("Failed to read E1000 ROM: {}", e))?;
    if rom_data.is_empty() {
        return Err("E1000 ROM is empty".into());
    }
    let name = b"genroms/pxe-e1000.rom";
    let rc = corevm_fw_cfg_add_file(
        handle,
        name.as_ptr(), name.len() as u32,
        rom_data.as_ptr(), rom_data.len() as u32,
    );
    if rc != 0 {
        return Err(format!("Failed to add E1000 ROM to fw_cfg (rc={})", rc));
    }
    Ok(())
}

// ── MAC address handling ────────────────────────────────────────────

/// Generate a deterministic locally-administered MAC from a VM UUID.
/// Uses prefix 52:54:00 (QEMU-style) + 3 bytes derived from UUID hash.
pub fn generate_mac(uuid: &str) -> [u8; 6] {
    let mut h: u32 = 5381;
    for b in uuid.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    [
        0x52, 0x54, 0x00,
        ((h >> 16) & 0xFF) as u8,
        ((h >> 8) & 0xFF) as u8,
        (h & 0xFF) as u8,
    ]
}

/// Parse a MAC address string like "52:54:00:AB:CD:EF" or "52-54-00-AB-CD-EF".
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = if s.contains(':') {
        s.split(':').collect()
    } else if s.contains('-') {
        s.split('-').collect()
    } else {
        return None;
    };
    if parts.len() != 6 { return None; }
    let mut mac = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

/// Resolve the MAC address: parse static or generate dynamic from UUID.
pub fn resolve_mac(uuid: &str, mac_static: bool, mac_address: &str) -> [u8; 6] {
    if mac_static && !mac_address.is_empty() {
        if let Some(mac) = parse_mac(mac_address) {
            return mac;
        }
    }
    generate_mac(uuid)
}
