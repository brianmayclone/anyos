use alloc::vec::Vec;

use super::{exit_reason, VmExitInfo};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA_START: u16 = 0x0cfc;
const PCI_CONFIG_DATA_END: u16 = 0x0cff;

const PCI_ADDRESS_ENABLE: u32 = 0x8000_0000;
const PCI_VENDOR_INVALID: u32 = 0xffff_ffff;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PciBus {
    config_address: u32,
    devices: Vec<PciDevice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PciIoAction {
    pub read_value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PciDevice {
    bus: u8,
    device: u8,
    function: u8,
    config: [u8; 256],
    writable_mask: [u8; 256],
    bars: [PciBar; 6],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PciBar {
    offset: u8,
    value: u32,
    size_mask: u32,
    probe: bool,
}

impl Default for PciBus {
    fn default() -> Self {
        let mut bus = Self {
            config_address: 0,
            devices: Vec::new(),
        };
        bus.devices.push(PciDevice::host_bridge());
        bus.devices.push(PciDevice::isa_bridge());
        bus.devices.push(PciDevice::e1000());
        bus
    }
}

impl PciBus {
    pub(super) fn io_action(&mut self, exit: &VmExitInfo) -> Option<PciIoAction> {
        if exit.reason != exit_reason::IO_INSTRUCTION || !is_pci_config_port(exit.io_port) {
            return None;
        }

        if exit.is_read != 0 {
            return Some(PciIoAction {
                read_value: Some(self.read_port(exit.io_port, exit.access_size)),
            });
        }

        self.write_port(exit.io_port, exit.access_size, exit.io_data as u32);
        Some(PciIoAction { read_value: None })
    }

    fn read_port(&self, port: u16, access_size: u8) -> u32 {
        if port == PCI_CONFIG_ADDRESS {
            return mask_width(self.config_address, access_size);
        }

        let Some((bus, device, function, offset)) = self.selected_offset(port) else {
            return mask_width(PCI_VENDOR_INVALID, access_size);
        };
        let value = self
            .find_device(bus, device, function)
            .map(|dev| dev.read_config(offset))
            .unwrap_or(PCI_VENDOR_INVALID);
        mask_width(value, access_size)
    }

    fn write_port(&mut self, port: u16, access_size: u8, value: u32) {
        if port == PCI_CONFIG_ADDRESS {
            self.config_address = merge_width(self.config_address, 0, access_size, value);
            return;
        }

        let Some((bus, device, function, offset)) = self.selected_offset(port) else {
            return;
        };
        if let Some(dev) = self.find_device_mut(bus, device, function) {
            dev.write_config(offset, access_size, value);
        }
    }

    fn selected_offset(&self, port: u16) -> Option<(u8, u8, u8, u8)> {
        if (self.config_address & PCI_ADDRESS_ENABLE) == 0 {
            return None;
        }
        if !(PCI_CONFIG_DATA_START..=PCI_CONFIG_DATA_END).contains(&port) {
            return None;
        }

        let bus = ((self.config_address >> 16) & 0xff) as u8;
        let device = ((self.config_address >> 11) & 0x1f) as u8;
        let function = ((self.config_address >> 8) & 0x7) as u8;
        let register = (self.config_address & 0xfc) as u8;
        let port_offset = (port - PCI_CONFIG_DATA_START) as u8;
        Some((bus, device, function, register.wrapping_add(port_offset)))
    }

    fn find_device(&self, bus: u8, device: u8, function: u8) -> Option<&PciDevice> {
        self.devices
            .iter()
            .find(|dev| dev.bus == bus && dev.device == device && dev.function == function)
    }

    fn find_device_mut(&mut self, bus: u8, device: u8, function: u8) -> Option<&mut PciDevice> {
        self.devices
            .iter_mut()
            .find(|dev| dev.bus == bus && dev.device == device && dev.function == function)
    }
}

impl PciDevice {
    fn host_bridge() -> Self {
        let mut dev = Self::new(0, 0, 0);
        dev.write_ro16(0x00, 0x8086);
        dev.write_ro16(0x02, 0x1237);
        dev.write_ro8(0x08, 0x02);
        dev.write_ro8(0x0a, 0x00);
        dev.write_ro8(0x0b, 0x06);
        dev
    }

    fn isa_bridge() -> Self {
        let mut dev = Self::new(0, 1, 0);
        dev.write_ro16(0x00, 0x8086);
        dev.write_ro16(0x02, 0x7000);
        dev.write_ro8(0x08, 0x00);
        dev.write_ro8(0x0a, 0x01);
        dev.write_ro8(0x0b, 0x06);
        dev
    }

    fn e1000() -> Self {
        let mut dev = Self::new(0, 3, 0);
        dev.write_ro16(0x00, 0x8086);
        dev.write_ro16(0x02, 0x100e);
        dev.write_rw16(0x04, 0x0000, 0x0007);
        dev.write_ro16(0x06, 0x0010);
        dev.write_ro8(0x08, 0x02);
        dev.write_ro8(0x0a, 0x00);
        dev.write_ro8(0x0b, 0x02);
        dev.write_ro8(0x0e, 0x00);
        dev.write_rw32(0x10, 0xfebc_0000, 0xfffe_0000);
        dev.write_ro16(0x2c, 0x8086);
        dev.write_ro16(0x2e, 0x100e);
        dev.write_ro8(0x3c, 11);
        dev.write_ro8(0x3d, 1);
        dev.bars[0] = PciBar {
            offset: 0x10,
            value: 0xfebc_0000,
            size_mask: 0xfffe_0000,
            probe: false,
        };
        dev
    }

    fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
            config: [0; 256],
            writable_mask: [0; 256],
            bars: [PciBar::default(); 6],
        }
    }

    fn read_config(&self, offset: u8) -> u32 {
        let base = (offset & 0xfc) as usize;
        if let Some(bar) = self.probed_bar(base as u8) {
            return bar.size_mask;
        }
        u32::from_le_bytes([
            self.config[base],
            self.config[base + 1],
            self.config[base + 2],
            self.config[base + 3],
        ])
    }

    fn write_config(&mut self, offset: u8, access_size: u8, value: u32) {
        let base = (offset & 0xfc) as usize;
        let shift = ((offset & 3) * 8) as u32;
        let bytes = value.to_le_bytes();
        let count = access_size.min(4) as usize;

        if self.write_bar_probe(offset, access_size, value) {
            return;
        }

        for index in 0..count {
            let cfg_index = base + ((shift as usize / 8) + index);
            if cfg_index >= self.config.len() {
                break;
            }
            let mask = self.writable_mask[cfg_index];
            self.config[cfg_index] = (self.config[cfg_index] & !mask) | (bytes[index] & mask);
        }

        self.sync_bar_values();
    }

    fn write_bar_probe(&mut self, offset: u8, access_size: u8, value: u32) -> bool {
        if access_size != 4 {
            return false;
        }
        let Some(bar) = self
            .bars
            .iter_mut()
            .find(|bar| bar.offset == offset && bar.size_mask != 0)
        else {
            return false;
        };

        if value == 0xffff_ffff {
            bar.probe = true;
            return true;
        }

        bar.probe = false;
        bar.value = value & bar.size_mask;
        write_u32(&mut self.config, offset, bar.value);
        true
    }

    fn probed_bar(&self, offset: u8) -> Option<PciBar> {
        self.bars
            .iter()
            .copied()
            .find(|bar| bar.offset == offset && bar.probe)
    }

    fn sync_bar_values(&mut self) {
        let mut updates = [(0u8, 0u32); 6];
        let mut count = 0usize;
        for bar in self.bars.iter_mut().filter(|bar| bar.size_mask != 0) {
            let raw = read_u32(&self.config, bar.offset);
            bar.value = raw & bar.size_mask;
            updates[count] = (bar.offset, bar.value);
            count += 1;
        }
        for (offset, value) in updates.iter().copied().take(count) {
            write_u32(&mut self.config, offset, value);
        }
    }

    fn write_ro8(&mut self, offset: u8, value: u8) {
        self.config[offset as usize] = value;
    }

    fn write_ro16(&mut self, offset: u8, value: u16) {
        let bytes = value.to_le_bytes();
        self.write_ro8(offset, bytes[0]);
        self.write_ro8(offset + 1, bytes[1]);
    }

    fn write_rw16(&mut self, offset: u8, value: u16, mask: u16) {
        self.write_ro16(offset, value);
        let bytes = mask.to_le_bytes();
        self.writable_mask[offset as usize] = bytes[0];
        self.writable_mask[offset as usize + 1] = bytes[1];
    }

    fn write_rw32(&mut self, offset: u8, value: u32, mask: u32) {
        write_u32(&mut self.config, offset, value);
        let bytes = mask.to_le_bytes();
        self.writable_mask[offset as usize..offset as usize + 4].copy_from_slice(&bytes);
    }
}

fn is_pci_config_port(port: u16) -> bool {
    port == PCI_CONFIG_ADDRESS || (PCI_CONFIG_DATA_START..=PCI_CONFIG_DATA_END).contains(&port)
}

fn mask_width(value: u32, access_size: u8) -> u32 {
    match access_size {
        1 => value & 0xff,
        2 => value & 0xffff,
        _ => value,
    }
}

fn merge_width(original: u32, offset: u8, access_size: u8, value: u32) -> u32 {
    let shift = ((offset & 3) * 8) as u32;
    let mask = match access_size {
        1 => 0xffu32,
        2 => 0xffffu32,
        _ => 0xffff_ffffu32,
    } << shift;
    (original & !mask) | ((value << shift) & mask)
}

fn read_u32(buffer: &[u8; 256], offset: u8) -> u32 {
    let base = offset as usize;
    u32::from_le_bytes([
        buffer[base],
        buffer[base + 1],
        buffer[base + 2],
        buffer[base + 3],
    ])
}

fn write_u32(buffer: &mut [u8; 256], offset: u8, value: u32) {
    let base = offset as usize;
    buffer[base..base + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::{PciBus, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA_START};
    use crate::vm::{exit_reason, VmExitInfo};

    fn outl(bus: &mut PciBus, port: u16, value: u32) {
        let _ = bus.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: port,
            access_size: 4,
            io_data: value as u64,
            ..VmExitInfo::default()
        });
    }

    fn inl(bus: &mut PciBus, port: u16) -> u32 {
        bus.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: port,
            access_size: 4,
            is_read: 1,
            ..VmExitInfo::default()
        })
        .unwrap()
        .read_value
        .unwrap()
    }

    #[test]
    fn enumerates_e1000_config_space() {
        let mut bus = PciBus::default();
        outl(&mut bus, PCI_CONFIG_ADDRESS, 0x8000_1800);
        assert_eq!(inl(&mut bus, PCI_CONFIG_DATA_START), 0x100e_8086);

        outl(&mut bus, PCI_CONFIG_ADDRESS, 0x8000_1808);
        let class = inl(&mut bus, PCI_CONFIG_DATA_START);
        assert_eq!((class >> 24) & 0xff, 0x02);
        assert_eq!((class >> 16) & 0xff, 0x00);
    }

    #[test]
    fn supports_bar_size_probe_and_restore() {
        let mut bus = PciBus::default();
        outl(&mut bus, PCI_CONFIG_ADDRESS, 0x8000_1810);
        assert_eq!(inl(&mut bus, PCI_CONFIG_DATA_START), 0xfebc_0000);

        outl(&mut bus, PCI_CONFIG_DATA_START, 0xffff_ffff);
        assert_eq!(inl(&mut bus, PCI_CONFIG_DATA_START), 0xfffe_0000);

        outl(&mut bus, PCI_CONFIG_DATA_START, 0xfebe_0000);
        assert_eq!(inl(&mut bus, PCI_CONFIG_DATA_START), 0xfebe_0000);
    }
}
