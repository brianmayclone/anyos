use alloc::vec::Vec;

use super::{exit_reason, VmExitInfo};

pub(super) const E1000_MMIO_BASE: u64 = 0xfebc_0000;
pub(super) const E1000_MMIO_SIZE: u64 = 0x0002_0000;

const REG_CTRL: u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_EERD: u32 = 0x0014;
const REG_ICR: u32 = 0x00c0;
const REG_IMS: u32 = 0x00d0;
const REG_IMC: u32 = 0x00d8;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_RDTR: u32 = 0x2820;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const REG_MTA: u32 = 0x5200;
const REG_RAL0: u32 = 0x5400;
const REG_RAH0: u32 = 0x5404;

const CTRL_RST: u32 = 1 << 26;
const CTRL_SLU: u32 = 1 << 6;
const STATUS_LINK_UP: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 1;
const TCTL_EN: u32 = 1 << 1;
const ICR_TXDW: u32 = 1 << 0;
const ICR_LSC: u32 = 1 << 2;
const ICR_RXT0: u32 = 1 << 7;

const TDESC_STATUS_DD: u8 = 1 << 0;
const RDESC_STATUS_DD: u8 = 1 << 0;
const RDESC_STATUS_EOP: u8 = 1 << 1;

const DESC_SIZE: usize = 16;
const MAX_FRAME_BYTES: usize = 1518;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct E1000Device {
    ctrl: u32,
    status: u32,
    icr: u32,
    ims: u32,
    rctl: u32,
    tctl: u32,
    tipg: u32,
    rdtr: u32,
    rdbal: u32,
    rdbah: u32,
    rdlen: u32,
    rdh: u32,
    rdt: u32,
    tdbal: u32,
    tdbah: u32,
    tdlen: u32,
    tdh: u32,
    tdt: u32,
    ral0: u32,
    rah0: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct E1000MmioAction {
    pub read_value: Option<u32>,
    pub tx_frames: Vec<Vec<u8>>,
    pub rx_poll: bool,
    pub interrupt: bool,
}

impl Default for E1000Device {
    fn default() -> Self {
        Self {
            ctrl: CTRL_SLU,
            status: STATUS_LINK_UP,
            icr: ICR_LSC,
            ims: 0,
            rctl: 0,
            tctl: 0,
            tipg: 0,
            rdtr: 0,
            rdbal: 0,
            rdbah: 0,
            rdlen: 0,
            rdh: 0,
            rdt: 0,
            tdbal: 0,
            tdbah: 0,
            tdlen: 0,
            tdh: 0,
            tdt: 0,
            ral0: u32::from_le_bytes([0x02, 0x41, 0x53, 0x4c]),
            rah0: 0x8000_0100,
        }
    }
}

impl E1000Device {
    pub(super) fn mmio_action<R, W>(
        &mut self,
        exit: &VmExitInfo,
        mut read_guest: R,
        mut write_guest: W,
    ) -> Option<E1000MmioAction>
    where
        R: FnMut(u64, &mut [u8]) -> bool,
        W: FnMut(u64, &[u8]) -> bool,
    {
        if exit.reason != exit_reason::EPT_VIOLATION || !is_e1000_mmio(exit.guest_phys_addr) {
            return None;
        }

        let reg = (exit.guest_phys_addr - E1000_MMIO_BASE) as u32;
        if exit.is_read != 0 {
            let read_value = self.read_reg(reg);
            return Some(E1000MmioAction {
                read_value: Some(read_value),
                tx_frames: Vec::new(),
                rx_poll: self.rx_enabled(),
                interrupt: self.interrupt_pending(),
            });
        }

        let mut action = E1000MmioAction::default();
        self.write_reg(
            reg,
            exit.io_data as u32,
            &mut read_guest,
            &mut write_guest,
            &mut action,
        );
        action.interrupt = self.interrupt_pending();
        Some(action)
    }

    pub(super) fn inject_rx_frame<R, W>(
        &mut self,
        frame: &[u8],
        mut read_guest: R,
        mut write_guest: W,
    ) -> bool
    where
        R: FnMut(u64, &mut [u8]) -> bool,
        W: FnMut(u64, &[u8]) -> bool,
    {
        if !self.rx_enabled() || frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
            return false;
        }
        let desc_count = self.rx_desc_count();
        if desc_count == 0 {
            return false;
        }

        let index = (self.rdh as usize) % desc_count;
        let desc_addr = self.rx_desc_base().wrapping_add((index * DESC_SIZE) as u64);
        let mut desc = [0u8; DESC_SIZE];
        if !read_guest(desc_addr, &mut desc) {
            return false;
        }

        let buffer_addr = read_u64(&desc, 0);
        if buffer_addr == 0 || !write_guest(buffer_addr, frame) {
            return false;
        }

        write_u16(&mut desc, 8, frame.len() as u16);
        desc[10] = 0;
        desc[11] = 0;
        desc[12] = RDESC_STATUS_DD | RDESC_STATUS_EOP;
        desc[13] = 0;
        write_u16(&mut desc, 14, 0);
        if !write_guest(desc_addr, &desc) {
            return false;
        }

        self.rdh = ((index + 1) % desc_count) as u32;
        self.icr |= ICR_RXT0;
        true
    }

    pub(super) fn interrupt_pending(&self) -> bool {
        self.icr & self.ims != 0
    }

    pub(super) fn wants_rx_poll(&self) -> bool {
        self.rx_enabled()
    }

    fn read_reg(&mut self, reg: u32) -> u32 {
        match reg {
            REG_CTRL => self.ctrl,
            REG_STATUS => self.status,
            REG_EERD => 0,
            REG_ICR => {
                let value = self.icr;
                self.icr = 0;
                value
            }
            REG_IMS => self.ims,
            REG_RCTL => self.rctl,
            REG_TCTL => self.tctl,
            REG_TIPG => self.tipg,
            REG_RDBAL => self.rdbal,
            REG_RDBAH => self.rdbah,
            REG_RDLEN => self.rdlen,
            REG_RDH => self.rdh,
            REG_RDT => self.rdt,
            REG_RDTR => self.rdtr,
            REG_TDBAL => self.tdbal,
            REG_TDBAH => self.tdbah,
            REG_TDLEN => self.tdlen,
            REG_TDH => self.tdh,
            REG_TDT => self.tdt,
            REG_RAL0 => self.ral0,
            REG_RAH0 => self.rah0,
            reg if (REG_MTA..REG_MTA + 128 * 4).contains(&reg) => 0,
            _ => 0,
        }
    }

    fn write_reg<R, W>(
        &mut self,
        reg: u32,
        value: u32,
        read_guest: &mut R,
        write_guest: &mut W,
        action: &mut E1000MmioAction,
    ) where
        R: FnMut(u64, &mut [u8]) -> bool,
        W: FnMut(u64, &[u8]) -> bool,
    {
        match reg {
            REG_CTRL => {
                if value & CTRL_RST != 0 {
                    self.reset_runtime();
                } else {
                    self.ctrl = value | CTRL_SLU;
                }
            }
            REG_IMS => self.ims |= value,
            REG_IMC => self.ims &= !value,
            REG_ICR => self.icr &= !value,
            REG_RCTL => self.rctl = value,
            REG_TCTL => self.tctl = value,
            REG_TIPG => self.tipg = value,
            REG_RDTR => self.rdtr = value,
            REG_RDBAL => self.rdbal = value,
            REG_RDBAH => self.rdbah = value,
            REG_RDLEN => self.rdlen = value,
            REG_RDH => self.rdh = value,
            REG_RDT => {
                self.rdt = value;
                action.rx_poll = self.rx_enabled();
            }
            REG_TDBAL => self.tdbal = value,
            REG_TDBAH => self.tdbah = value,
            REG_TDLEN => self.tdlen = value,
            REG_TDH => self.tdh = value,
            REG_TDT => {
                self.tdt = value;
                self.collect_tx_frames(read_guest, write_guest, &mut action.tx_frames);
            }
            REG_RAL0 => self.ral0 = value,
            REG_RAH0 => self.rah0 = value,
            reg if (REG_MTA..REG_MTA + 128 * 4).contains(&reg) => {}
            _ => {}
        }
    }

    fn collect_tx_frames<R, W>(
        &mut self,
        read_guest: &mut R,
        write_guest: &mut W,
        out: &mut Vec<Vec<u8>>,
    ) where
        R: FnMut(u64, &mut [u8]) -> bool,
        W: FnMut(u64, &[u8]) -> bool,
    {
        if !self.tx_enabled() {
            return;
        }
        let desc_count = self.tx_desc_count();
        if desc_count == 0 {
            return;
        }

        let target_tail = (self.tdt as usize) % desc_count;
        let mut index = (self.tdh as usize) % desc_count;
        while index != target_tail {
            let desc_addr = self.tx_desc_base().wrapping_add((index * DESC_SIZE) as u64);
            let mut desc = [0u8; DESC_SIZE];
            if !read_guest(desc_addr, &mut desc) {
                break;
            }

            let buffer_addr = read_u64(&desc, 0);
            let len = read_u16(&desc, 8) as usize;
            if buffer_addr != 0 && (1..=MAX_FRAME_BYTES).contains(&len) {
                let mut frame = Vec::new();
                frame.resize(len, 0);
                if read_guest(buffer_addr, &mut frame) {
                    out.push(frame);
                }
            }

            desc[12] |= TDESC_STATUS_DD;
            let _ = write_guest(desc_addr, &desc);
            index = (index + 1) % desc_count;
        }

        self.tdh = target_tail as u32;
        if !out.is_empty() {
            self.icr |= ICR_TXDW;
        }
    }

    fn reset_runtime(&mut self) {
        self.ctrl = CTRL_SLU;
        self.status = STATUS_LINK_UP;
        self.icr = ICR_LSC;
        self.ims = 0;
        self.rctl = 0;
        self.tctl = 0;
        self.rdh = 0;
        self.rdt = 0;
        self.tdh = 0;
        self.tdt = 0;
    }

    fn rx_enabled(&self) -> bool {
        self.rctl & RCTL_EN != 0
    }

    fn tx_enabled(&self) -> bool {
        self.tctl & TCTL_EN != 0
    }

    fn rx_desc_base(&self) -> u64 {
        ((self.rdbah as u64) << 32) | self.rdbal as u64
    }

    fn tx_desc_base(&self) -> u64 {
        ((self.tdbah as u64) << 32) | self.tdbal as u64
    }

    fn rx_desc_count(&self) -> usize {
        (self.rdlen as usize / DESC_SIZE).min(4096)
    }

    fn tx_desc_count(&self) -> usize {
        (self.tdlen as usize / DESC_SIZE).min(4096)
    }
}

fn is_e1000_mmio(gpa: u64) -> bool {
    (E1000_MMIO_BASE..E1000_MMIO_BASE + E1000_MMIO_SIZE).contains(&gpa)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        E1000Device, E1000_MMIO_BASE, RCTL_EN, REG_RCTL, REG_RDBAL, REG_RDLEN, REG_RDT, REG_STATUS,
        REG_TCTL, REG_TDBAL, REG_TDLEN, REG_TDT, STATUS_LINK_UP, TCTL_EN,
    };
    use crate::vm::{exit_reason, VmExitInfo};

    fn mmio_read(dev: &mut E1000Device, reg: u32) -> u32 {
        dev.mmio_action(
            &VmExitInfo {
                reason: exit_reason::EPT_VIOLATION,
                guest_phys_addr: E1000_MMIO_BASE + reg as u64,
                access_size: 4,
                is_read: 1,
                ..VmExitInfo::default()
            },
            |_addr, _buf| false,
            |_addr, _buf| false,
        )
        .unwrap()
        .read_value
        .unwrap()
    }

    fn mmio_write(
        dev: &mut E1000Device,
        reg: u32,
        value: u32,
        memory: &RefCell<alloc::vec::Vec<u8>>,
    ) -> super::E1000MmioAction {
        dev.mmio_action(
            &VmExitInfo {
                reason: exit_reason::EPT_VIOLATION,
                guest_phys_addr: E1000_MMIO_BASE + reg as u64,
                access_size: 4,
                is_read: 0,
                io_data: value as u64,
                ..VmExitInfo::default()
            },
            |addr, dest| {
                let memory = memory.borrow();
                let start = addr as usize;
                let end = start + dest.len();
                if end > memory.len() {
                    return false;
                }
                dest.copy_from_slice(&memory[start..end]);
                true
            },
            |addr, bytes| {
                let mut memory = memory.borrow_mut();
                let start = addr as usize;
                let end = start + bytes.len();
                if end > memory.len() {
                    return false;
                }
                memory[start..end].copy_from_slice(bytes);
                true
            },
        )
        .unwrap()
    }

    #[test]
    fn status_register_reports_link_up() {
        let mut dev = E1000Device::default();
        assert_eq!(mmio_read(&mut dev, REG_STATUS), STATUS_LINK_UP);
    }

    #[test]
    fn tx_tail_write_dma_reads_frame_and_writebacks_descriptor() {
        let memory = RefCell::new(alloc::vec![0u8; 0x8000]);
        let mut dev = E1000Device::default();
        let desc = 0x1000usize;
        let buf = 0x2000usize;
        let frame = [0xaa, 0xbb, 0xcc, 0xdd];

        {
            let mut memory = memory.borrow_mut();
            memory[desc..desc + 8].copy_from_slice(&(buf as u64).to_le_bytes());
            memory[desc + 8..desc + 10].copy_from_slice(&(frame.len() as u16).to_le_bytes());
            memory[buf..buf + frame.len()].copy_from_slice(&frame);
        }

        let _ = mmio_write(&mut dev, REG_TDBAL, desc as u32, &memory);
        let _ = mmio_write(&mut dev, REG_TDLEN, 32, &memory);
        let _ = mmio_write(&mut dev, REG_TCTL, TCTL_EN, &memory);
        let action = mmio_write(&mut dev, REG_TDT, 1, &memory);

        assert_eq!(action.tx_frames, alloc::vec![frame.to_vec()]);
        assert_eq!(memory.borrow()[desc + 12] & 1, 1);
    }

    #[test]
    fn inject_rx_frame_dma_writes_buffer_and_descriptor() {
        let memory = RefCell::new(alloc::vec![0u8; 0x8000]);
        let mut dev = E1000Device::default();
        let desc = 0x3000usize;
        let buf = 0x4000usize;
        let frame = [1, 2, 3, 4, 5, 6];

        memory.borrow_mut()[desc..desc + 8].copy_from_slice(&(buf as u64).to_le_bytes());
        let _ = mmio_write(&mut dev, REG_RDBAL, desc as u32, &memory);
        let _ = mmio_write(&mut dev, REG_RDLEN, 16, &memory);
        let _ = mmio_write(&mut dev, REG_RCTL, RCTL_EN, &memory);
        let _ = mmio_write(&mut dev, REG_RDT, 0, &memory);

        let injected = dev.inject_rx_frame(
            &frame,
            |addr, dest| {
                let memory = memory.borrow();
                let start = addr as usize;
                let end = start + dest.len();
                if end > memory.len() {
                    return false;
                }
                dest.copy_from_slice(&memory[start..end]);
                true
            },
            |addr, bytes| {
                let mut memory = memory.borrow_mut();
                let start = addr as usize;
                let end = start + bytes.len();
                if end > memory.len() {
                    return false;
                }
                memory[start..end].copy_from_slice(bytes);
                true
            },
        );

        let memory = memory.borrow();
        assert!(injected);
        assert_eq!(&memory[buf..buf + frame.len()], &frame);
        assert_eq!(
            u16::from_le_bytes([memory[desc + 8], memory[desc + 9]]),
            frame.len() as u16
        );
        assert_eq!(memory[desc + 12] & 0x3, 0x3);
    }
}
