#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;

mod app;
mod config;
mod diagnostics;
mod dialog_create_vm;
mod dialog_create_disk;
mod dialog_add_disk;
mod dialog_disk_pool;
mod dialog_about;
mod dialog_snapshots;
mod dialogs;
mod display;
mod filebrowser;
mod input;
mod platform;
mod sidebar;
mod statusbar;
mod theme;
mod settings;
mod toolbar;
mod vm;

/// Show a native error message box. On Windows this uses MessageBoxW,
/// on other platforms it prints to stderr.
fn show_fatal_error(title: &str, message: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::iter;

        extern "system" {
            fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }

        let title_wide: Vec<u16> = OsStr::new(title).encode_wide().chain(iter::once(0)).collect();
        let msg_wide: Vec<u16> = OsStr::new(message).encode_wide().chain(iter::once(0)).collect();
        const MB_OK: u32 = 0x00000000;
        const MB_ICONERROR: u32 = 0x00000010;
        unsafe {
            MessageBoxW(core::ptr::null_mut(), msg_wide.as_ptr(), title_wide.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("{}: {}", title, message);
    }
}

fn check_hardware_support() -> Result<(), String> {
    use libcorevm::ffi::corevm_has_hw_support;

    if corevm_has_hw_support() == 1 {
        return Ok(());
    }

    let mut diag = String::from("Hardware virtualization is not available.\n\n");

    #[cfg(target_os = "linux")]
    {
        diag.push_str("Diagnostics:\n");
        // Check if /dev/kvm exists
        if std::path::Path::new("/dev/kvm").exists() {
            diag.push_str("  - /dev/kvm exists but could not be opened.\n");
            diag.push_str("  - Check permissions: is your user in the 'kvm' group?\n");
            diag.push_str("    Run: sudo usermod -aG kvm $USER\n");
        } else {
            diag.push_str("  - /dev/kvm not found.\n");
            diag.push_str("  - KVM kernel module may not be loaded.\n");
            diag.push_str("    Run: sudo modprobe kvm_intel  (or kvm_amd for AMD CPUs)\n");
            diag.push_str("  - If running in a VM, enable nested virtualization.\n");
        }
    }

    #[cfg(target_os = "windows")]
    {
        diag.push_str("Diagnostics:\n");
        diag.push_str("  - Windows Hypervisor Platform (WHP) is not available.\n\n");
        diag.push_str("Required Windows features:\n");
        diag.push_str("  1. Hyper-V (must be enabled in Windows Features)\n");
        diag.push_str("  2. Windows Hypervisor Platform (must be enabled in Windows Features)\n\n");
        diag.push_str("To enable, run in an elevated PowerShell:\n");
        diag.push_str("  Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform\n");
        diag.push_str("  Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All\n\n");
        diag.push_str("Then restart your computer.\n\n");
        diag.push_str("If running in WSL, ensure Hyper-V is enabled on the Windows host.\n");

        // Try to get the specific error from libcorevm
        let err = vm::get_last_error_public();
        if let Some(e) = err {
            diag.push_str(&format!("\nBackend error: {}\n", e));
        }
    }

    Err(diag)
}

fn main() -> eframe::Result {
    // Set up a panic hook that shows a message box on Windows
    #[cfg(target_os = "windows")]
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            let location = info.location().map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
            show_fatal_error("CoreVM Manager - Fatal Error", &format!("{}{}", msg, location));
            default_hook(info);
        }));
    }

    // Check hardware support before launching UI
    if let Err(diag) = check_hardware_support() {
        show_fatal_error("CoreVM Manager - Hardware Support", &diag);
        std::process::exit(1);
    }

    // Force glow (OpenGL) renderer — wgpu/Vulkan fails under WSLg
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("CoreVM Manager"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "CoreVM Manager",
        options,
        Box::new(|_cc| Ok(Box::new(app::CoreVmApp::new()))),
    )
}
