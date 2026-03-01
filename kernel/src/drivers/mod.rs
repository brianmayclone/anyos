//! Device drivers for hardware peripherals.
//!
//! Includes serial, framebuffer, VGA text, input (keyboard/mouse), storage (ATA),
//! GPU (Bochs VGA, VMware SVGA II), networking (E1000), PCI bus, RTC, and the HAL registry.

// x86-only hardware drivers
#[cfg(target_arch = "x86_64")]
pub mod audio;
#[cfg(target_arch = "x86_64")]
pub mod boot_console;
pub mod framebuffer;
#[cfg(target_arch = "x86_64")]
pub mod gpu;
pub mod hal;
#[cfg(target_arch = "x86_64")]
pub mod input;
#[cfg(target_arch = "x86_64")]
pub mod kdrv;
#[cfg(target_arch = "x86_64")]
pub mod network;
#[cfg(target_arch = "x86_64")]
pub mod pci;
#[cfg(target_arch = "x86_64")]
pub mod pci_drivers;
#[cfg(target_arch = "x86_64")]
pub mod rsod;
#[cfg(target_arch = "x86_64")]
pub mod rtc;
pub mod serial;
#[cfg(target_arch = "x86_64")]
pub mod storage;
#[cfg(target_arch = "x86_64")]
pub mod usb;
#[cfg(target_arch = "x86_64")]
pub mod vga_text;
#[cfg(target_arch = "x86_64")]
pub mod vmmdev;
#[cfg(target_arch = "x86_64")]
pub mod virtio;
