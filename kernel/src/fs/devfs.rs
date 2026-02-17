//! Device filesystem (/dev) -- maps file operations to kernel device drivers.
//! Provides virtual files like /dev/null, /dev/zero, and /dev/console,
//! and bridges to HAL-registered hardware devices.

use crate::fs::file::{DirEntry, FileType};
use alloc::string::String;
use alloc::vec::Vec;

/// Device filesystem instance holding registered device entries.
pub struct DevFs {
    devices: Vec<DeviceEntry>,
}

struct DeviceEntry {
    name: String,
    backend: DeviceBackend,
}

enum DeviceBackend {
    /// Built-in virtual devices (null, zero, console)
    Callback {
        read_fn: Option<fn(&mut [u8]) -> usize>,
        write_fn: Option<fn(&[u8]) -> usize>,
    },
    /// HAL-registered hardware device — proxies to hal::device_read/write
    Hal { path: String },
}

impl DevFs {
    /// Create a new DevFs with the standard devices (null, zero, console).
    pub fn new() -> Self {
        let mut devfs = DevFs {
            devices: Vec::new(),
        };

        // Register standard virtual devices
        devfs.register_callback("null", Some(dev_null_read), Some(dev_null_write));
        devfs.register_callback("zero", Some(dev_zero_read), Some(dev_null_write));
        devfs.register_callback("console", None, Some(dev_console_write));

        devfs
    }

    /// Register a virtual device with direct read/write callbacks.
    fn register_callback(
        &mut self,
        name: &str,
        read_fn: Option<fn(&mut [u8]) -> usize>,
        write_fn: Option<fn(&[u8]) -> usize>,
    ) {
        self.devices.push(DeviceEntry {
            name: String::from(name),
            backend: DeviceBackend::Callback { read_fn, write_fn },
        });
    }

    /// Populate from HAL device registry. Each HAL device at "/dev/xyz" gets
    /// registered as "xyz" with a Hal backend. Skips names already registered.
    pub fn populate_from_hal(&mut self) {
        let hal_devices = crate::drivers::hal::list_devices();
        for (path, _name, _dtype) in hal_devices {
            // HAL paths are like "/dev/blk0", "/dev/ttyS0" — strip prefix
            let dev_name = if path.starts_with("/dev/") {
                &path[5..]
            } else {
                continue;
            };

            // Skip if already registered (e.g. "console" is both virtual and HAL)
            if self.devices.iter().any(|d| d.name == dev_name) {
                continue;
            }

            self.devices.push(DeviceEntry {
                name: String::from(dev_name),
                backend: DeviceBackend::Hal { path: path.clone() },
            });
        }
    }

    /// Look up a device by name. Returns the device index if found.
    pub fn lookup(&self, name: &str) -> Option<usize> {
        self.devices.iter().position(|d| d.name == name)
    }

    /// List all registered device entries.
    pub fn list(&self) -> Vec<DirEntry> {
        self.devices
            .iter()
            .map(|d| DirEntry {
                name: d.name.clone(),
                file_type: FileType::Device,
                size: 0,
                is_symlink: false,
            })
            .collect()
    }

    /// Read from a named device into `buf`. Returns `None` if not found or not readable.
    pub fn read(&self, name: &str, buf: &mut [u8]) -> Option<usize> {
        let dev = self.devices.iter().find(|d| d.name == name)?;
        match &dev.backend {
            DeviceBackend::Callback { read_fn, .. } => {
                read_fn.map(|f| f(buf))
            }
            DeviceBackend::Hal { path } => {
                crate::drivers::hal::device_read(path, 0, buf).ok()
            }
        }
    }

    /// Write `buf` to a named device. Returns `None` if not found or not writable.
    pub fn write(&self, name: &str, buf: &[u8]) -> Option<usize> {
        let dev = self.devices.iter().find(|d| d.name == name)?;
        match &dev.backend {
            DeviceBackend::Callback { write_fn, .. } => {
                write_fn.map(|f| f(buf))
            }
            DeviceBackend::Hal { path } => {
                crate::drivers::hal::device_write(path, 0, buf).ok()
            }
        }
    }
}

fn dev_null_read(_buf: &mut [u8]) -> usize {
    0 // EOF
}

fn dev_null_write(buf: &[u8]) -> usize {
    buf.len() // Discard all
}

fn dev_zero_read(buf: &mut [u8]) -> usize {
    for b in buf.iter_mut() {
        *b = 0;
    }
    buf.len()
}

fn dev_console_write(buf: &[u8]) -> usize {
    for &byte in buf {
        crate::drivers::serial::write_byte(byte);
    }
    buf.len()
}
