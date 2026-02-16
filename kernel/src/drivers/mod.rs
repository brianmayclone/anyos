//! Device drivers for hardware peripherals.
//!
//! Includes serial, framebuffer, VGA text, input (keyboard/mouse), storage (ATA),
//! GPU (Bochs VGA, VMware SVGA II), networking (E1000), PCI bus, RTC, and the HAL registry.

pub mod audio;
pub mod boot_console;
pub mod framebuffer;
pub mod gpu;
pub mod hal;
pub mod input;
pub mod kdrv;
pub mod network;
pub mod pci;
pub mod rsod;
pub mod rtc;
pub mod serial;
pub mod storage;
pub mod usb;
pub mod vga_text;
pub mod virtio;
