use crate::fs::file::{DirEntry, FileType};
use alloc::string::String;
use alloc::vec::Vec;

/// Device filesystem - provides /dev/* entries
/// Maps file operations to device drivers

pub struct DevFs {
    devices: Vec<DeviceEntry>,
}

struct DeviceEntry {
    name: String,
    read_fn: Option<fn(&mut [u8]) -> usize>,
    write_fn: Option<fn(&[u8]) -> usize>,
}

impl DevFs {
    pub fn new() -> Self {
        let mut devfs = DevFs {
            devices: Vec::new(),
        };

        // Register standard devices
        devfs.register("null", Some(dev_null_read), Some(dev_null_write));
        devfs.register("zero", Some(dev_zero_read), Some(dev_null_write));
        devfs.register("console", None, Some(dev_console_write));

        devfs
    }

    pub fn register(
        &mut self,
        name: &str,
        read_fn: Option<fn(&mut [u8]) -> usize>,
        write_fn: Option<fn(&[u8]) -> usize>,
    ) {
        self.devices.push(DeviceEntry {
            name: String::from(name),
            read_fn,
            write_fn,
        });
    }

    pub fn list(&self) -> Vec<DirEntry> {
        self.devices
            .iter()
            .map(|d| DirEntry {
                name: d.name.clone(),
                file_type: FileType::Device,
                size: 0,
            })
            .collect()
    }

    pub fn read(&self, name: &str, buf: &mut [u8]) -> Option<usize> {
        self.devices
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| d.read_fn.map(|f| f(buf)))
    }

    pub fn write(&self, name: &str, buf: &[u8]) -> Option<usize> {
        self.devices
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| d.write_fn.map(|f| f(buf)))
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
