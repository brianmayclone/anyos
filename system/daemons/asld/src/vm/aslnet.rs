use alloc::vec::Vec;

use super::{exit_reason, VmExitInfo};

const ASL_NET_MAGIC_PORT: u16 = 0x5650;
const ASL_NET_STATUS_PORT: u16 = 0x5651;
const ASL_NET_COMMAND_PORT: u16 = 0x5652;
const ASL_NET_LENGTH_PORT: u16 = 0x5653;
const ASL_NET_DATA_PORT: u16 = 0x5654;

const ASL_NET_MAGIC: u32 = 0x4e4c_5341; // "ASLN" little-endian
const ASL_NET_STATUS_LINK_UP: u32 = 1 << 0;
const ASL_NET_STATUS_RX_READY: u32 = 1 << 1;
const ASL_NET_STATUS_TX_READY: u32 = 1 << 2;
const ASL_NET_STATUS_TX_PENDING: u32 = 1 << 3;

const ASL_NET_COMMAND_TX_FLUSH: u32 = 1;
const ASL_NET_COMMAND_RX_POLL: u32 = 2;
const ASL_NET_COMMAND_TX_CLEAR: u32 = 3;

const MAX_FRAME_BYTES: usize = 1518;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct AslNetDevice {
    tx: Vec<u8>,
    tx_target_len: usize,
    rx: Vec<u8>,
    rx_offset: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct AslNetIoAction {
    pub read_value: Option<u32>,
    pub tx_frame: Option<Vec<u8>>,
    pub rx_poll: bool,
}

impl AslNetDevice {
    pub(super) fn io_action(&mut self, exit: &VmExitInfo) -> Option<AslNetIoAction> {
        if exit.reason != exit_reason::IO_INSTRUCTION || !is_asl_net_port(exit.io_port) {
            return None;
        }

        if exit.is_read != 0 {
            return Some(AslNetIoAction {
                read_value: Some(self.read_port(exit.io_port, exit.access_size)),
                tx_frame: None,
                rx_poll: false,
            });
        }

        let value = exit.io_data as u32;
        match exit.io_port {
            ASL_NET_COMMAND_PORT => return Some(self.write_command(value)),
            ASL_NET_LENGTH_PORT => self.set_tx_target_len(value as usize),
            ASL_NET_DATA_PORT => self.write_data(value, exit.access_size),
            _ => {}
        }
        Some(AslNetIoAction::default())
    }

    pub(super) fn load_rx_frame(&mut self, bytes: Vec<u8>) {
        self.rx = bytes;
        self.rx_offset = 0;
    }

    fn write_command(&mut self, value: u32) -> AslNetIoAction {
        match value {
            ASL_NET_COMMAND_TX_FLUSH => {
                let frame = if self.tx.is_empty() {
                    None
                } else {
                    let mut frame = Vec::new();
                    core::mem::swap(&mut frame, &mut self.tx);
                    self.tx_target_len = 0;
                    Some(frame)
                };
                AslNetIoAction {
                    read_value: None,
                    tx_frame: frame,
                    rx_poll: false,
                }
            }
            ASL_NET_COMMAND_RX_POLL => AslNetIoAction {
                read_value: None,
                tx_frame: None,
                rx_poll: true,
            },
            ASL_NET_COMMAND_TX_CLEAR => {
                self.tx.clear();
                self.tx_target_len = 0;
                AslNetIoAction::default()
            }
            _ => AslNetIoAction::default(),
        }
    }

    fn set_tx_target_len(&mut self, len: usize) {
        self.tx.clear();
        self.tx_target_len = len.min(MAX_FRAME_BYTES);
        if self.tx_target_len > 0 {
            self.tx.reserve(self.tx_target_len);
        }
    }

    fn write_data(&mut self, value: u32, access_size: u8) {
        let bytes = value.to_le_bytes();
        let count = access_size.min(4) as usize;
        let target = if self.tx_target_len == 0 {
            MAX_FRAME_BYTES
        } else {
            self.tx_target_len
        };
        let remaining = target.saturating_sub(self.tx.len());
        self.tx.extend_from_slice(&bytes[..count.min(remaining)]);
    }

    fn read_port(&mut self, port: u16, access_size: u8) -> u32 {
        match port {
            ASL_NET_MAGIC_PORT => ASL_NET_MAGIC,
            ASL_NET_STATUS_PORT => self.status_bits(),
            ASL_NET_LENGTH_PORT => self.rx_remaining() as u32,
            ASL_NET_DATA_PORT => self.read_data(access_size),
            _ => 0,
        }
    }

    fn status_bits(&self) -> u32 {
        let mut status = ASL_NET_STATUS_LINK_UP | ASL_NET_STATUS_TX_READY;
        if self.rx_remaining() > 0 {
            status |= ASL_NET_STATUS_RX_READY;
        }
        if !self.tx.is_empty() {
            status |= ASL_NET_STATUS_TX_PENDING;
        }
        status
    }

    fn rx_remaining(&self) -> usize {
        self.rx.len().saturating_sub(self.rx_offset)
    }

    fn read_data(&mut self, access_size: u8) -> u32 {
        let count = access_size.min(4) as usize;
        let mut bytes = [0u8; 4];
        let available = self.rx_remaining().min(count);
        if available > 0 {
            bytes[..available]
                .copy_from_slice(&self.rx[self.rx_offset..self.rx_offset + available]);
            self.rx_offset += available;
            if self.rx_offset >= self.rx.len() {
                self.rx.clear();
                self.rx_offset = 0;
            }
        }
        u32::from_le_bytes(bytes)
    }
}

fn is_asl_net_port(port: u16) -> bool {
    matches!(
        port,
        ASL_NET_MAGIC_PORT
            | ASL_NET_STATUS_PORT
            | ASL_NET_COMMAND_PORT
            | ASL_NET_LENGTH_PORT
            | ASL_NET_DATA_PORT
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AslNetDevice, ASL_NET_COMMAND_PORT, ASL_NET_COMMAND_RX_POLL, ASL_NET_COMMAND_TX_FLUSH,
        ASL_NET_DATA_PORT, ASL_NET_LENGTH_PORT, ASL_NET_MAGIC, ASL_NET_MAGIC_PORT,
    };
    use crate::vm::{exit_reason, VmExitInfo};

    #[test]
    fn buffers_tx_frame_until_flush() {
        let mut device = AslNetDevice::default();
        let _ = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_LENGTH_PORT,
            access_size: 2,
            io_data: 4,
            ..VmExitInfo::default()
        });
        let _ = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_DATA_PORT,
            access_size: 4,
            io_data: 0xddccbbaa,
            ..VmExitInfo::default()
        });
        let action = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_COMMAND_PORT,
            access_size: 4,
            io_data: ASL_NET_COMMAND_TX_FLUSH as u64,
            ..VmExitInfo::default()
        });
        assert_eq!(
            action.unwrap().tx_frame,
            Some(alloc::vec![0xaa, 0xbb, 0xcc, 0xdd])
        );
    }

    #[test]
    fn exposes_rx_frame_to_guest_reads() {
        let mut device = AslNetDevice::default();
        let magic = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_MAGIC_PORT,
            access_size: 4,
            is_read: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(magic.unwrap().read_value, Some(ASL_NET_MAGIC));

        let poll = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_COMMAND_PORT,
            access_size: 4,
            io_data: ASL_NET_COMMAND_RX_POLL as u64,
            ..VmExitInfo::default()
        });
        assert!(poll.unwrap().rx_poll);

        device.load_rx_frame(alloc::vec![1, 2, 3, 4]);
        let len = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_LENGTH_PORT,
            access_size: 4,
            is_read: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(len.unwrap().read_value, Some(4));

        let data = device.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: ASL_NET_DATA_PORT,
            access_size: 4,
            is_read: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(data.unwrap().read_value, Some(0x0403_0201));
    }
}
