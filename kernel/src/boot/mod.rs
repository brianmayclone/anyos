//! Kernel boot orchestration and boot-time state.
//!
//! Keeps the crate root thin by moving the staged boot sequence into dedicated
//! modules. Public boot flags remain exported so syscall handlers and other
//! subsystems can query boot/runtime mode without depending on the old
//! monolithic `main.rs`.

#[cfg(target_arch = "aarch64")]
mod arm64;
#[cfg(target_arch = "x86_64")]
mod x86;

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use alloc::string::String;

use crate::fs::file::FileType;
use crate::fs::vfs::{self, FsError};

/// Boot mode: 0 = Legacy BIOS, 1 = UEFI.
static BOOT_MODE: AtomicU8 = AtomicU8::new(0);

/// GPU 2D acceleration available (queried by SYS_GPU_HAS_ACCEL).
pub static GPU_ACCEL: AtomicBool = AtomicBool::new(false);

/// GPU hardware cursor available (queried by SYS_GPU_HAS_HW_CURSOR).
pub static GPU_HW_CURSOR: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "nogui" parameter.
/// Skips compositor and init; starts textmode_console instead.
pub static NOGUI: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "setup" parameter (ISO installer mode).
/// Starts compositor + installer directly, skipping login/dock/init.
pub static SETUP_MODE: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "schedstress" parameter. Spawns the
/// concurrent SMP scheduler stress test after userspace init — the safety net
/// for the Phase 4b run-queue lock split. Opt-in only; never runs by default.
pub static SCHED_STRESS: AtomicBool = AtomicBool::new(false);

/// Get the boot mode (0 = BIOS, 1 = UEFI).
pub fn boot_mode() -> u8 {
    BOOT_MODE.load(Ordering::Relaxed)
}

pub(crate) fn set_boot_mode(mode: u8) {
    BOOT_MODE.store(mode, Ordering::Relaxed);
}

pub(crate) fn reset_root_tmp_dir_or_halt() {
    crate::serial_println!("[BOOT] Resetting rootfs /tmp");

    match reset_root_tmp_dir() {
        Ok(removed) => {
            crate::serial_println!(
                "[OK] rootfs /tmp is clean (removed {} stale path(s))",
                removed
            );
        }
        Err(err) => {
            crate::serial_println!(
                "FATAL: Failed to reset rootfs /tmp before userspace: {:?}",
                err
            );
            loop {
                crate::arch::hal::halt();
            }
        }
    }
}

fn reset_root_tmp_dir() -> Result<usize, FsError> {
    let removed = match vfs::lstat("/tmp") {
        Ok(stat) if stat.file_type == FileType::Directory => delete_tree("/tmp")?,
        Ok(_) => {
            vfs::delete("/tmp")?;
            1
        }
        Err(FsError::NotFound) => 0,
        Err(err) => return Err(err),
    };

    match vfs::mkdir("/tmp") {
        Ok(()) => Ok(removed),
        Err(FsError::AlreadyExists) => match vfs::lstat("/tmp") {
            Ok(stat) if stat.file_type == FileType::Directory => Ok(removed),
            Ok(_) => Err(FsError::AlreadyExists),
            Err(err) => Err(err),
        },
        Err(err) => Err(err),
    }
}

fn delete_tree(path: &str) -> Result<usize, FsError> {
    let entries = vfs::read_dir(path)?;
    let mut removed = 0usize;

    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }

        let child = join_path(path, &entry.name);
        if entry.file_type == FileType::Directory && !entry.is_symlink {
            removed += delete_tree(&child)?;
        } else {
            vfs::delete(&child)?;
            removed += 1;
        }
    }

    vfs::delete(path)?;
    Ok(removed + 1)
}

fn join_path(parent: &str, name: &str) -> String {
    let mut path = String::from(parent.trim_end_matches('/'));
    path.push('/');
    path.push_str(name);
    path
}

/// Shared boot entry used by the crate-root wrapper.
pub fn kernel_main(boot_info_addr: u64) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        x86::kernel_main(boot_info_addr)
    }

    #[cfg(target_arch = "aarch64")]
    {
        arm64::kernel_main(boot_info_addr)
    }
}
