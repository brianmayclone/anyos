use alloc::vec::Vec;

#[cfg(not(target_os = "linux"))]
use crate::errors::AsldError;

use super::{exit_reason, VmExitInfo};

const PRIMARY_DATA: u16 = 0x1f0;
const PRIMARY_ERROR_FEATURES: u16 = 0x1f1;
const PRIMARY_SECTOR_COUNT: u16 = 0x1f2;
const PRIMARY_LBA_LOW: u16 = 0x1f3;
const PRIMARY_LBA_MID: u16 = 0x1f4;
const PRIMARY_LBA_HIGH: u16 = 0x1f5;
const PRIMARY_DRIVE_HEAD: u16 = 0x1f6;
const PRIMARY_STATUS_COMMAND: u16 = 0x1f7;
const PRIMARY_ALT_STATUS_CONTROL: u16 = 0x3f6;
const SECTOR_SIZE: usize = 512;
const STATUS_ERR: u8 = 0x01;
const STATUS_DRQ: u8 = 0x08;
const STATUS_DRDY: u8 = 0x40;

#[derive(Clone, Debug, PartialEq, Eq)]
enum IdeStorage {
    None,
    Memory(Vec<u8>),
    File {
        fd: u32,
        sectors: u32,
        writable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IdeController {
    storage: IdeStorage,
    slave_storage: IdeStorage,
    error: u8,
    sector_count: u8,
    lba_low: u8,
    lba_mid: u8,
    lba_high: u8,
    drive_head: u8,
    status: u8,
    data: Vec<u8>,
    data_offset: usize,
    write_target: Option<WriteTarget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct IdeIoAction {
    pub read_value: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WriteTarget {
    slave: bool,
    lba: u32,
    count: u32,
}

impl IdeController {
    pub(super) fn disabled() -> Self {
        Self {
            storage: IdeStorage::None,
            slave_storage: IdeStorage::None,
            error: 0,
            sector_count: 0,
            lba_low: 0,
            lba_mid: 0,
            lba_high: 0,
            drive_head: 0xe0,
            status: 0,
            data: Vec::new(),
            data_offset: 0,
            write_target: None,
        }
    }

    #[cfg(test)]
    fn with_memory_disk(bytes: Vec<u8>) -> Self {
        let mut controller = Self::disabled();
        controller.storage = IdeStorage::Memory(bytes);
        controller.status = STATUS_DRDY;
        controller
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn open_asl_disks(base_path: &str, seed_path: &str) -> Result<Self, AsldError> {
        let base = open_file_disk(base_path, true)
            .map_err(|_| AsldError::InvalidState("SeaBIOS boot disk is missing"))?;
        let seed = open_file_disk(seed_path, false).unwrap_or(IdeStorage::None);
        let mut controller = Self::disabled();
        controller.storage = base;
        controller.slave_storage = seed;
        controller.status = STATUS_DRDY;
        Ok(controller)
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn open_read_only(path: &str) -> Result<Self, AsldError> {
        let storage = open_file_disk(path, false)
            .map_err(|_| AsldError::InvalidState("SeaBIOS boot disk is missing"))?;
        let mut controller = Self::disabled();
        controller.storage = storage;
        controller.status = STATUS_DRDY;
        Ok(controller)
    }
}

#[cfg(not(target_os = "linux"))]
fn open_file_disk(path: &str, writable: bool) -> Result<IdeStorage, AsldError> {
    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(path, &mut stat_buf) != 0 || stat_buf[0] != 0 {
        return Err(AsldError::InvalidState("SeaBIOS boot disk is missing"));
    }
    let size = stat_buf[1] as usize;
    if size < SECTOR_SIZE {
        return Err(AsldError::InvalidState("SeaBIOS boot disk is too small"));
    }
    let fd = anyos_std::fs::open(path, if writable { anyos_std::fs::O_WRITE } else { 0 });
    if fd == 0 || fd == u32::MAX {
        return Err(AsldError::InvalidState("SeaBIOS boot disk is not readable"));
    }

    Ok(IdeStorage::File {
        fd,
        sectors: (size / SECTOR_SIZE) as u32,
        writable,
    })
}

impl IdeController {
    #[cfg(not(target_os = "linux"))]
    pub(super) fn close(&self) {
        for storage in [&self.storage, &self.slave_storage] {
            if let IdeStorage::File { fd, .. } = storage {
                let _ = anyos_std::fs::close(*fd);
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn close(&self) {}

    pub(super) fn io_action(&mut self, exit: &VmExitInfo) -> Option<IdeIoAction> {
        if exit.reason != exit_reason::IO_INSTRUCTION || !is_ide_port(exit.io_port) {
            return None;
        }

        if exit.is_read != 0 {
            return Some(IdeIoAction {
                read_value: Some(self.read_port(exit.io_port)),
            });
        }

        if exit.io_port == PRIMARY_DATA {
            self.write_data_port(exit.io_data as u32, exit.access_size as u32);
            return Some(IdeIoAction { read_value: None });
        }

        let value = (exit.io_data & 0xff) as u8;
        match exit.io_port {
            PRIMARY_ERROR_FEATURES => {}
            PRIMARY_SECTOR_COUNT => self.sector_count = value,
            PRIMARY_LBA_LOW => self.lba_low = value,
            PRIMARY_LBA_MID => self.lba_mid = value,
            PRIMARY_LBA_HIGH => self.lba_high = value,
            PRIMARY_DRIVE_HEAD => {
                self.drive_head = value;
                self.status = if self.has_disk() { STATUS_DRDY } else { 0 };
            }
            PRIMARY_STATUS_COMMAND => self.execute_command(value),
            PRIMARY_ALT_STATUS_CONTROL => {}
            _ => {}
        }
        Some(IdeIoAction { read_value: None })
    }

    pub(super) fn data_string_read_into(
        &mut self,
        exit: &VmExitInfo,
        buffer: &mut [u8],
    ) -> Option<usize> {
        if exit.reason != exit_reason::IO_INSTRUCTION
            || exit.io_port != PRIMARY_DATA
            || exit.is_read == 0
            || exit.access_size != 2
        {
            return None;
        }

        let words = buffer.len() / 2;
        for index in 0..words {
            let value = self.read_data_word().to_le_bytes();
            let offset = index * 2;
            buffer[offset] = value[0];
            buffer[offset + 1] = value[1];
        }
        Some(words * 2)
    }

    pub(super) fn data_string_write_from(
        &mut self,
        exit: &VmExitInfo,
        buffer: &[u8],
    ) -> Option<usize> {
        if exit.reason != exit_reason::IO_INSTRUCTION
            || exit.io_port != PRIMARY_DATA
            || exit.is_read != 0
            || exit.access_size != 2
        {
            return None;
        }
        self.write_data_bytes(buffer);
        Some(buffer.len())
    }

    fn read_port(&mut self, port: u16) -> u32 {
        match port {
            PRIMARY_DATA => self.read_data_word() as u32,
            PRIMARY_ERROR_FEATURES => self.error as u32,
            PRIMARY_SECTOR_COUNT => self.sector_count as u32,
            PRIMARY_LBA_LOW => self.lba_low as u32,
            PRIMARY_LBA_MID => self.lba_mid as u32,
            PRIMARY_LBA_HIGH => self.lba_high as u32,
            PRIMARY_DRIVE_HEAD => self.drive_head as u32,
            PRIMARY_STATUS_COMMAND | PRIMARY_ALT_STATUS_CONTROL => self.status as u32,
            _ => 0,
        }
    }

    fn sector_count(&self) -> u32 {
        match self.selected_storage() {
            IdeStorage::None => 0,
            IdeStorage::Memory(bytes) => (bytes.len() / SECTOR_SIZE) as u32,
            IdeStorage::File { sectors, .. } => *sectors,
        }
    }

    fn selected_lba(&self) -> u32 {
        ((self.drive_head as u32 & 0x0f) << 24)
            | ((self.lba_high as u32) << 16)
            | ((self.lba_mid as u32) << 8)
            | self.lba_low as u32
    }

    fn selected_sector_count(&self) -> u32 {
        if self.sector_count == 0 {
            256
        } else {
            self.sector_count as u32
        }
    }

    fn has_disk(&self) -> bool {
        !matches!(self.selected_storage(), IdeStorage::None)
    }

    fn selected_slave(&self) -> bool {
        (self.drive_head & 0x10) != 0
    }

    fn selected_storage(&self) -> &IdeStorage {
        if self.selected_slave() {
            &self.slave_storage
        } else {
            &self.storage
        }
    }

    fn read_sectors(&mut self) {
        let lba = self.selected_lba();
        let count = self.selected_sector_count();
        let byte_count = (count as usize).saturating_mul(SECTOR_SIZE);
        let offset = (lba as usize).saturating_mul(SECTOR_SIZE);
        let end = offset.saturating_add(byte_count);
        if !self.has_disk() || end > (self.sector_count() as usize * SECTOR_SIZE) {
            self.set_error(0x04);
            return;
        }

        self.data.resize(byte_count, 0);
        let ok = if self.selected_slave() {
            read_storage(&mut self.slave_storage, offset, end, &mut self.data)
        } else {
            read_storage(&mut self.storage, offset, end, &mut self.data)
        };
        if ok {
            self.data_offset = 0;
            self.error = 0;
            self.status = STATUS_DRDY | STATUS_DRQ;
        } else {
            self.set_error(0x04);
        }
    }

    fn prepare_write_sectors(&mut self) {
        let lba = self.selected_lba();
        let count = self.selected_sector_count();
        let byte_count = (count as usize).saturating_mul(SECTOR_SIZE);
        let offset = (lba as usize).saturating_mul(SECTOR_SIZE);
        let end = offset.saturating_add(byte_count);
        if !self.has_disk() || end > (self.sector_count() as usize * SECTOR_SIZE) {
            self.set_error(0x04);
            return;
        }
        self.data.clear();
        self.data.resize(byte_count, 0);
        self.data_offset = 0;
        self.write_target = Some(WriteTarget {
            slave: self.selected_slave(),
            lba,
            count,
        });
        self.error = 0;
        self.status = STATUS_DRDY | STATUS_DRQ;
    }

    fn identify(&mut self) {
        if !self.has_disk() {
            self.set_error(0x04);
            return;
        }

        self.data.clear();
        self.data.resize(SECTOR_SIZE, 0);
        write_ide_word(&mut self.data, 0, 0x0040);
        write_ide_ascii(&mut self.data, 10, 20, "ANYOS0000000000000000");
        write_ide_ascii(&mut self.data, 23, 8, "1.0");
        write_ide_ascii(&mut self.data, 27, 40, "anyOS ASL ATA disk");
        write_ide_word(&mut self.data, 49, 1 << 9);
        write_ide_word(&mut self.data, 53, 1);
        let sectors = self.sector_count();
        write_ide_word(&mut self.data, 60, (sectors & 0xffff) as u16);
        write_ide_word(&mut self.data, 61, (sectors >> 16) as u16);
        write_ide_word(&mut self.data, 80, 0x007e);
        write_ide_word(&mut self.data, 83, 1 << 10);
        write_ide_word(&mut self.data, 86, 1 << 10);
        self.data_offset = 0;
        self.error = 0;
        self.status = STATUS_DRDY | STATUS_DRQ;
    }

    fn finish_data_if_needed(&mut self) {
        if self.data_offset >= self.data.len() {
            if self.write_target.is_some() {
                self.commit_write();
                return;
            }
            self.data.clear();
            self.data_offset = 0;
            self.status = if self.has_disk() { STATUS_DRDY } else { 0 };
        }
    }

    fn read_data_word(&mut self) -> u16 {
        if self.data_offset + 1 >= self.data.len() {
            self.finish_data_if_needed();
            return 0;
        }
        let value =
            u16::from_le_bytes([self.data[self.data_offset], self.data[self.data_offset + 1]]);
        self.data_offset += 2;
        self.finish_data_if_needed();
        value
    }

    fn write_data_port(&mut self, value: u32, access_size: u32) {
        if access_size == 1 {
            let bytes = [(value & 0xff) as u8];
            self.write_data_bytes(&bytes);
        } else {
            let bytes = (value as u16).to_le_bytes();
            self.write_data_bytes(&bytes);
        }
    }

    fn write_data_bytes(&mut self, bytes: &[u8]) {
        if self.write_target.is_none() || self.data_offset >= self.data.len() {
            self.set_error(0x04);
            return;
        }
        let count = bytes.len().min(self.data.len() - self.data_offset);
        self.data[self.data_offset..self.data_offset + count].copy_from_slice(&bytes[..count]);
        self.data_offset += count;
        self.finish_data_if_needed();
    }

    fn commit_write(&mut self) {
        let Some(target) = self.write_target.take() else {
            return;
        };
        let offset = target.lba as usize * SECTOR_SIZE;
        let expected = target.count as usize * SECTOR_SIZE;
        let data = self.data.clone();
        let ok = if data.len() == expected {
            let storage = if target.slave {
                &mut self.slave_storage
            } else {
                &mut self.storage
            };
            match storage {
                IdeStorage::Memory(bytes) => {
                    let end = offset.saturating_add(data.len());
                    if end <= bytes.len() {
                        bytes[offset..end].copy_from_slice(&data);
                        true
                    } else {
                        false
                    }
                }
                IdeStorage::File { fd, writable, .. } => {
                    *writable && write_disk_file(*fd, offset, &data)
                }
                IdeStorage::None => false,
            }
        } else {
            false
        };
        self.data.clear();
        self.data_offset = 0;
        if ok {
            self.error = 0;
            self.status = if self.has_disk() { STATUS_DRDY } else { 0 };
        } else {
            self.set_error(0x04);
        }
    }

    fn execute_command(&mut self, command: u8) {
        match command {
            0x20 | 0x24 => self.read_sectors(),
            0x30 | 0x34 => self.prepare_write_sectors(),
            0x90 | 0x91 | 0xe7 => {
                self.error = 0;
                self.status = if self.has_disk() { STATUS_DRDY } else { 0 };
            }
            0xec => self.identify(),
            _ => self.set_error(0x04),
        }
    }

    fn set_error(&mut self, error: u8) {
        self.error = error;
        self.data.clear();
        self.data_offset = 0;
        self.status = if self.has_disk() {
            STATUS_DRDY | STATUS_ERR
        } else {
            STATUS_ERR
        };
    }
}

fn read_storage(storage: &mut IdeStorage, offset: usize, end: usize, data: &mut [u8]) -> bool {
    match storage {
        IdeStorage::Memory(bytes) => {
            data.copy_from_slice(&bytes[offset..end]);
            true
        }
        IdeStorage::File { fd, .. } => read_disk_file(*fd, offset, data),
        IdeStorage::None => false,
    }
}

#[cfg(not(target_os = "linux"))]
fn read_disk_file(fd: u32, offset: usize, data: &mut [u8]) -> bool {
    if offset > i32::MAX as usize {
        return false;
    }
    if anyos_std::fs::lseek(fd, offset as i32, anyos_std::fs::SEEK_SET) == u32::MAX {
        return false;
    }
    anyos_std::fs::read(fd, data) == data.len() as u32
}

#[cfg(target_os = "linux")]
fn read_disk_file(_fd: u32, _offset: usize, _data: &mut [u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
fn write_disk_file(fd: u32, offset: usize, data: &[u8]) -> bool {
    if offset > i32::MAX as usize {
        return false;
    }
    if anyos_std::fs::lseek(fd, offset as i32, anyos_std::fs::SEEK_SET) == u32::MAX {
        return false;
    }
    anyos_std::fs::write(fd, data) == data.len() as u32
}

#[cfg(target_os = "linux")]
fn write_disk_file(_fd: u32, _offset: usize, _data: &[u8]) -> bool {
    false
}

fn is_ide_port(port: u16) -> bool {
    matches!(
        port,
        PRIMARY_DATA
            | PRIMARY_ERROR_FEATURES
            | PRIMARY_SECTOR_COUNT
            | PRIMARY_LBA_LOW
            | PRIMARY_LBA_MID
            | PRIMARY_LBA_HIGH
            | PRIMARY_DRIVE_HEAD
            | PRIMARY_STATUS_COMMAND
            | PRIMARY_ALT_STATUS_CONTROL
    )
}

fn write_ide_word(buffer: &mut [u8], word_index: usize, value: u16) {
    let offset = word_index * 2;
    if offset + 1 < buffer.len() {
        buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
}

fn write_ide_ascii(buffer: &mut [u8], word_index: usize, word_count: usize, text: &str) {
    let offset = word_index * 2;
    let len = word_count * 2;
    if offset + len > buffer.len() {
        return;
    }
    let target = &mut buffer[offset..offset + len];
    target.fill(b' ');
    for (index, byte) in text.as_bytes().iter().copied().take(len).enumerate() {
        let word = index & !1;
        let swapped = word + (1 - (index & 1));
        target[swapped] = byte;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IdeController, IdeStorage, PRIMARY_DATA, PRIMARY_LBA_LOW, PRIMARY_SECTOR_COUNT,
        PRIMARY_STATUS_COMMAND, SECTOR_SIZE, STATUS_DRDY, STATUS_DRQ,
    };
    use crate::vm::{exit_reason, VmExitInfo};

    #[test]
    fn identifies_and_reads_boot_sector() {
        let mut disk = alloc::vec![0u8; 2 * SECTOR_SIZE];
        disk[0] = 0xeb;
        disk[1] = 0x3c;
        disk[510] = 0x55;
        disk[511] = 0xaa;
        disk[512] = 0x42;
        let mut state = IdeController::with_memory_disk(disk);

        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_STATUS_COMMAND,
            is_read: 0,
            io_data: 0xec,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(state.status, STATUS_DRDY | STATUS_DRQ);
        let identify_word0 = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_DATA,
            access_size: 2,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(identify_word0.unwrap().read_value, Some(0x0040));

        state.data.clear();
        state.data_offset = 0;
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_SECTOR_COUNT,
            is_read: 0,
            io_data: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_LBA_LOW,
            is_read: 0,
            io_data: 0,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_STATUS_COMMAND,
            is_read: 0,
            io_data: 0x20,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(state.status, STATUS_DRDY | STATUS_DRQ);
        let boot_word0 = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_DATA,
            access_size: 2,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(boot_word0.unwrap().read_value, Some(0x3ceb));

        let mut buffer = [0u8; 4];
        let copied = state.data_string_read_into(
            &VmExitInfo {
                reason: exit_reason::IO_INSTRUCTION,
                io_port: PRIMARY_DATA,
                access_size: 2,
                is_read: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
            &mut buffer,
        );
        assert_eq!(copied, Some(4));
        assert_eq!(buffer, [0, 0, 0, 0]);
    }

    #[test]
    fn writes_memory_disk_sector() {
        let disk = alloc::vec![0u8; 2 * SECTOR_SIZE];
        let mut state = IdeController::with_memory_disk(disk);

        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_SECTOR_COUNT,
            is_read: 0,
            io_data: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_LBA_LOW,
            is_read: 0,
            io_data: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_STATUS_COMMAND,
            is_read: 0,
            io_data: 0x30,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(state.status, STATUS_DRDY | STATUS_DRQ);
        for _ in 0..(SECTOR_SIZE / 2) {
            let _ = state.io_action(&VmExitInfo {
                reason: exit_reason::IO_INSTRUCTION,
                io_port: PRIMARY_DATA,
                access_size: 2,
                is_read: 0,
                io_data: 0x55aa,
                instruction_len: 1,
                ..VmExitInfo::default()
            });
        }
        assert_eq!(state.status, STATUS_DRDY);
        match &state.storage {
            IdeStorage::Memory(bytes) => {
                assert_eq!(bytes[SECTOR_SIZE], 0xaa);
                assert_eq!(bytes[SECTOR_SIZE + 1], 0x55);
            }
            _ => panic!("expected memory disk"),
        }
    }

    #[test]
    fn identifies_slave_seed_disk() {
        let mut state = IdeController::with_memory_disk(alloc::vec![0u8; 2 * SECTOR_SIZE]);
        state.slave_storage = IdeStorage::Memory(alloc::vec![0u8; 4 * SECTOR_SIZE]);
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: super::PRIMARY_DRIVE_HEAD,
            is_read: 0,
            io_data: 0xf0,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        let _ = state.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: PRIMARY_STATUS_COMMAND,
            is_read: 0,
            io_data: 0xec,
            instruction_len: 1,
            ..VmExitInfo::default()
        });
        assert_eq!(state.status, STATUS_DRDY | STATUS_DRQ);
        assert_eq!(state.sector_count(), 4);
    }
}
