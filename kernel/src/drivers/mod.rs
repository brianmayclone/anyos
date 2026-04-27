//! Device drivers for hardware peripherals.
//!
//! Includes serial, framebuffer, VGA text, input (keyboard/mouse), storage (ATA),
//! GPU (Bochs VGA, VMware SVGA II), networking (E1000), PCI bus, RTC, and the HAL registry.

// x86-only hardware drivers
#[cfg(target_arch = "x86_64")]
pub mod audio;
#[cfg(target_arch = "aarch64")]
#[path = "audio_arm64.rs"]
pub mod audio;
#[cfg(target_arch = "x86_64")]
pub mod bluetooth;
pub mod boot_console;
pub mod framebuffer;
#[cfg(target_arch = "x86_64")]
pub mod gpu;
pub mod hal;
pub mod input;
/// Re-export input layout module so syscall handlers can use `crate::drivers::layout`.
pub use input::layout;
#[cfg(target_arch = "aarch64")]
pub mod arm;
#[cfg(target_arch = "x86_64")]
pub mod i2c;
#[cfg(target_arch = "x86_64")]
pub mod kdrv;
#[cfg(target_arch = "x86_64")]
pub mod monitor;
#[cfg(target_arch = "x86_64")]
pub mod network;
#[cfg(target_arch = "aarch64")]
#[path = "network_arm64.rs"]
pub mod network;
#[cfg(target_arch = "x86_64")]
pub mod pci;
#[cfg(target_arch = "x86_64")]
pub mod pci_drivers;
#[cfg(target_arch = "x86_64")]
pub mod pci_msi;
#[cfg(target_arch = "x86_64")]
pub mod rsod;
#[cfg(target_arch = "x86_64")]
pub mod rtc;
pub mod serial;
pub mod shutdown_screen;
#[cfg(target_arch = "x86_64")]
pub mod smbus;
#[cfg(target_arch = "x86_64")]
pub mod storage;
#[cfg(target_arch = "aarch64")]
#[path = "storage_arm64.rs"]
pub mod storage;
#[cfg(target_arch = "x86_64")]
pub mod textcon;
#[cfg(target_arch = "x86_64")]
pub mod thermal;
#[cfg(target_arch = "x86_64")]
pub mod usb;
#[cfg(target_arch = "x86_64")]
pub mod vga_text;
#[cfg(target_arch = "x86_64")]
pub mod virtio;
#[cfg(target_arch = "x86_64")]
pub mod vmmdev;
#[cfg(target_arch = "x86_64")]
pub mod watchdog;
