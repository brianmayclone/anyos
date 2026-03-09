//! ATA/IDE disk controller emulation.
//!
//! Emulates a single primary IDE channel with two targets:
//! - master: ATA hard disk
//! - slave: ATAPI CD-ROM
//!
//! The ATA path is used for virtual disks, while ISO images are exposed as a
//! packet-based ATAPI device so SeaBIOS can boot them through its normal
//! El Torito code path.

use alloc::vec;
use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;
use crate::syscall;
#[cfg(feature = "host_test")]
use core::sync::atomic::{AtomicU32, Ordering};

macro_rules! ide_log {
    ($($arg:tt)*) => {{
        #[cfg(feature = "host_test")]
        {
            if IDE_LOG_BUDGET.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n > 0).then_some(n - 1)
            }).is_ok() {
                eprintln!("[ide] {}", format_args!($($arg)*));
            }
        }
        #[cfg(not(feature = "host_test"))]
        {
            let _ = format_args!($($arg)*);
        }
    }};
}

#[cfg(feature = "host_test")]
static IDE_LOG_BUDGET: AtomicU32 = AtomicU32::new(2048);

// ── ATA status register bits ──────────────────────────────────────────────

const SR_BSY: u8 = 0x80;
const SR_DRDY: u8 = 0x40;
const SR_DF: u8 = 0x20;
const SR_DSC: u8 = 0x10;
const SR_DRQ: u8 = 0x08;
const SR_ERR: u8 = 0x01;

// ── ATA error register bits ───────────────────────────────────────────────

const ER_ABRT: u8 = 0x04;

// ── ATA/ATAPI commands ────────────────────────────────────────────────────

const CMD_NOP: u8 = 0x00;
const CMD_DEVICE_RESET: u8 = 0x08;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_READ_SECTORS_EXT: u8 = 0x24;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_WRITE_SECTORS_EXT: u8 = 0x34;
const CMD_PACKET: u8 = 0xA0;
const CMD_IDENTIFY_PACKET: u8 = 0xA1;
const CMD_READ_MULTIPLE: u8 = 0xC4;
const CMD_WRITE_MULTIPLE: u8 = 0xC5;
const CMD_SET_MULTIPLE: u8 = 0xC6;
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;
const CMD_SET_FEATURES: u8 = 0xEF;
const CMD_INIT_DRIVE_PARAMS: u8 = 0x91;

// ── ATAPI packet opcodes ──────────────────────────────────────────────────

const PKT_TEST_UNIT_READY: u8 = 0x00;
const PKT_REQUEST_SENSE: u8 = 0x03;
const PKT_INQUIRY: u8 = 0x12;
const PKT_MODE_SENSE_6: u8 = 0x1A;
const PKT_START_STOP_UNIT: u8 = 0x1B;
const PKT_PREVENT_ALLOW_MEDIUM_REMOVAL: u8 = 0x1E;
const PKT_READ_FORMAT_CAPACITIES: u8 = 0x23;
const PKT_READ_CAPACITY_10: u8 = 0x25;
const PKT_READ_10: u8 = 0x28;
const PKT_READ_TOC_PMA_ATIP: u8 = 0x43;
const PKT_MODE_SENSE_10: u8 = 0x5A;
const PKT_READ_12: u8 = 0xA8;

const ATA_SECTOR_SIZE: usize = 512;
const ATAPI_SECTOR_SIZE: usize = 2048;
const ATAPI_PACKET_SIZE: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriveKind {
    AtaDisk,
    AtapiCdrom,
}

impl DriveKind {
    fn block_size(self) -> usize {
        match self {
            DriveKind::AtaDisk => ATA_SECTOR_SIZE,
            DriveKind::AtapiCdrom => ATAPI_SECTOR_SIZE,
        }
    }

    fn signature(self) -> (u8, u8, u8, u8) {
        match self {
            DriveKind::AtaDisk => (1, 1, 0x00, 0x00),
            DriveKind::AtapiCdrom => (1, 1, 0x14, 0xEB),
        }
    }
}

struct DriveState {
    disk: Vec<u8>,
    disk_fd: i32,
    total_blocks: u64,
    present: bool,
    kind: DriveKind,
}

impl DriveState {
    fn new(kind: DriveKind) -> Self {
        DriveState {
            disk: Vec::new(),
            disk_fd: -1,
            total_blocks: 0,
            present: false,
            kind,
        }
    }

    fn block_size(&self) -> usize {
        self.kind.block_size()
    }

    fn seek_to(&self, byte_offset: u64) {
        let fd = self.disk_fd as u32;
        syscall::lseek(fd, 0, 0);
        let mut remaining = byte_offset;
        const MAX_CHUNK: u64 = 0x7FFF_FFFF;
        while remaining > MAX_CHUNK {
            syscall::lseek(fd, MAX_CHUNK as i32, 1);
            remaining -= MAX_CHUNK;
        }
        if remaining > 0 {
            syscall::lseek(fd, remaining as i32, 1);
        }
    }

    fn read_at(&self, byte_offset: u64, buffer: &mut [u8]) {
        if self.disk_fd >= 0 {
            self.seek_to(byte_offset);
            let n = syscall::read(self.disk_fd as u32, buffer);
            let read_len = if n == u32::MAX { 0 } else { (n as usize).min(buffer.len()) };
            if read_len < buffer.len() {
                buffer[read_len..].fill(0);
            }
            return;
        }

        let start = byte_offset as usize;
        let end = start.saturating_add(buffer.len());
        if end <= self.disk.len() {
            buffer.copy_from_slice(&self.disk[start..end]);
        } else if start < self.disk.len() {
            let available = self.disk.len() - start;
            buffer[..available].copy_from_slice(&self.disk[start..]);
            buffer[available..].fill(0);
        } else {
            buffer.fill(0);
        }
    }

    fn write_at(&mut self, byte_offset: u64, buffer: &[u8]) {
        if self.kind != DriveKind::AtaDisk {
            return;
        }
        if self.disk_fd >= 0 {
            self.seek_to(byte_offset);
            syscall::write(self.disk_fd as u32, buffer);
            return;
        }

        let start = byte_offset as usize;
        let end = start.saturating_add(buffer.len());
        if end <= self.disk.len() {
            self.disk[start..end].copy_from_slice(buffer);
        }
    }

    fn read_block(&self, lba: u64, buffer: &mut [u8]) {
        if lba >= self.total_blocks || buffer.len() != self.block_size() {
            buffer.fill(0);
            return;
        }
        self.read_at(lba * self.block_size() as u64, buffer);
    }

    fn write_block(&mut self, lba: u64, buffer: &[u8]) {
        if lba >= self.total_blocks || buffer.len() != self.block_size() {
            return;
        }
        self.write_at(lba * self.block_size() as u64, buffer);
    }

    fn attach_image(&mut self, mut image: Vec<u8>, kind: DriveKind) {
        if self.disk_fd >= 0 {
            syscall::close(self.disk_fd as u32);
            self.disk_fd = -1;
        }

        let block_size = kind.block_size();
        let blocks = image.len() / block_size;
        image.truncate(blocks * block_size);
        self.disk = image;
        self.total_blocks = blocks as u64;
        self.present = true;
        self.kind = kind;
    }

    fn attach_fd(&mut self, fd: i32, size: u64, kind: DriveKind) {
        if self.disk_fd >= 0 {
            syscall::close(self.disk_fd as u32);
        }

        self.disk = Vec::new();
        self.disk_fd = fd;
        self.total_blocks = size / kind.block_size() as u64;
        self.present = true;
        self.kind = kind;
    }

    fn detach(&mut self) -> Vec<u8> {
        if self.disk_fd >= 0 {
            syscall::close(self.disk_fd as u32);
            self.disk_fd = -1;
        }
        self.total_blocks = 0;
        self.present = false;
        core::mem::take(&mut self.disk)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferMode {
    None,
    AtaRead,
    AtaWrite,
    AtapiPacket,
    AtapiRead,
}

pub struct Ide {
    drives: [DriveState; 2],

    error: u8,
    features: u8,
    sector_count: u8,
    sector_number: u8,
    cylinder_low: u8,
    cylinder_high: u8,
    drive_head: u8,
    status: u8,

    hob_sector_count: u8,
    hob_sector_number: u8,
    hob_cylinder_low: u8,
    hob_cylinder_high: u8,
    hob_toggle: bool,

    device_control: u8,

    buffer: Vec<u8>,
    buffer_offset: usize,
    buffer_limit: usize,
    transfer_mode: TransferMode,
    sectors_remaining: u32,
    irq_pending: bool,
    multiple_count: u8,
    atapi_byte_count_limit: u16,
    atapi_sense_key: u8,
    atapi_asc: u8,
    atapi_ascq: u8,
}

impl Ide {
    pub fn new() -> Self {
        let mut ide = Ide {
            drives: [
                DriveState::new(DriveKind::AtaDisk),
                DriveState::new(DriveKind::AtapiCdrom),
            ],
            error: 0x01,
            features: 0,
            sector_count: 1,
            sector_number: 1,
            cylinder_low: 0,
            cylinder_high: 0,
            drive_head: 0,
            status: SR_DRDY | SR_DSC,
            hob_sector_count: 0,
            hob_sector_number: 0,
            hob_cylinder_low: 0,
            hob_cylinder_high: 0,
            hob_toggle: false,
            device_control: 0,
            buffer: Vec::new(),
            buffer_offset: 0,
            buffer_limit: 0,
            transfer_mode: TransferMode::None,
            sectors_remaining: 0,
            irq_pending: false,
            multiple_count: 1,
            atapi_byte_count_limit: ATAPI_SECTOR_SIZE as u16,
            atapi_sense_key: 0,
            atapi_asc: 0,
            atapi_ascq: 0,
        };
        ide.apply_selected_drive_signature();
        ide
    }

    pub fn attach_disk(&mut self, image: Vec<u8>) {
        self.drives[0].attach_image(image, DriveKind::AtaDisk);
        if self.selected_drive() == 0 {
            self.apply_selected_drive_signature();
        }
        self.status = SR_DRDY | SR_DSC;
    }

    pub fn attach_slave(&mut self, image: Vec<u8>) {
        self.drives[1].attach_image(image, DriveKind::AtapiCdrom);
        if self.selected_drive() == 1 {
            self.apply_selected_drive_signature();
        }
    }

    pub fn attach_disk_fd(&mut self, fd: i32, size: u64) {
        ide_log!("attach master fd={} size={}", fd, size);
        self.drives[0].attach_fd(fd, size, DriveKind::AtaDisk);
        if self.selected_drive() == 0 {
            self.apply_selected_drive_signature();
        }
        self.status = SR_DRDY | SR_DSC;
    }

    pub fn attach_slave_fd(&mut self, fd: i32, size: u64) {
        ide_log!("attach slave cdrom fd={} size={}", fd, size);
        self.drives[1].attach_fd(fd, size, DriveKind::AtapiCdrom);
        if self.selected_drive() == 1 {
            self.apply_selected_drive_signature();
        }
    }

    pub fn detach_disk(&mut self) -> Vec<u8> {
        let disk = self.drives[0].detach();
        if self.selected_drive() == 0 {
            self.apply_selected_drive_signature();
        }
        disk
    }

    pub fn detach_slave(&mut self) -> Vec<u8> {
        let disk = self.drives[1].detach();
        if self.selected_drive() == 1 {
            self.apply_selected_drive_signature();
        }
        disk
    }

    pub fn irq_raised(&self) -> bool {
        self.irq_pending && (self.device_control & 0x02) == 0
    }

    pub fn clear_irq(&mut self) {
        self.irq_pending = false;
    }

    pub fn disk_size(&self) -> u64 {
        self.drives[0].total_blocks * ATA_SECTOR_SIZE as u64
    }

    fn selected_drive(&self) -> usize {
        if self.drive_head & 0x10 != 0 { 1 } else { 0 }
    }

    fn selected_drive_state(&self) -> &DriveState {
        &self.drives[self.selected_drive()]
    }

    fn selected_drive_state_mut(&mut self) -> &mut DriveState {
        &mut self.drives[self.selected_drive()]
    }

    fn ensure_buffer_len(&mut self, len: usize) {
        self.buffer.resize(len, 0);
        self.buffer_limit = len;
        self.buffer_offset = 0;
    }

    fn clear_transfer(&mut self) {
        self.transfer_mode = TransferMode::None;
        self.buffer_offset = 0;
        self.buffer_limit = 0;
    }

    fn apply_selected_drive_signature(&mut self) {
        let drv = self.selected_drive();
        self.error = 0x01;
        self.sector_count = 1;
        self.sector_number = 1;
        let (sc, sn, cl, ch) = if self.drives[drv].present {
            self.drives[drv].kind.signature()
        } else {
            (1, 1, 0, 0)
        };
        self.sector_count = sc;
        self.sector_number = sn;
        self.cylinder_low = cl;
        self.cylinder_high = ch;
        self.hob_sector_count = 0;
        self.hob_sector_number = 0;
        self.hob_cylinder_low = 0;
        self.hob_cylinder_high = 0;
        self.atapi_byte_count_limit = ATAPI_SECTOR_SIZE as u16;
        self.clear_transfer();
    }

    fn lba28(&self) -> u64 {
        (self.sector_number as u64)
            | ((self.cylinder_low as u64) << 8)
            | ((self.cylinder_high as u64) << 16)
            | (((self.drive_head & 0x0F) as u64) << 24)
    }

    fn lba48(&self) -> u64 {
        let lo = (self.sector_number as u64)
            | ((self.cylinder_low as u64) << 8)
            | ((self.cylinder_high as u64) << 16);
        let hi = (self.hob_sector_number as u64)
            | ((self.hob_cylinder_low as u64) << 8)
            | ((self.hob_cylinder_high as u64) << 16);
        lo | (hi << 24)
    }

    fn current_lba(&self) -> u64 {
        if self.drive_head & 0x40 != 0 {
            // LBA mode
            self.lba28()
        } else {
            // CHS mode: convert to LBA using standard geometry (16 heads, 63 sectors/track)
            let cylinder = (self.cylinder_high as u64) << 8 | self.cylinder_low as u64;
            let head = (self.drive_head & 0x0F) as u64;
            let sector = self.sector_number as u64;
            (cylinder * 16 + head) * 63 + sector.saturating_sub(1)
        }
    }

    fn advance_lba(&mut self) {
        let lba = self.current_lba() + 1;
        self.sector_number = (lba & 0xFF) as u8;
        self.cylinder_low = ((lba >> 8) & 0xFF) as u8;
        self.cylinder_high = ((lba >> 16) & 0xFF) as u8;
        self.drive_head = (self.drive_head & 0xF0) | (((lba >> 24) & 0x0F) as u8);
    }

    fn start_data_in(&mut self, data: &[u8], mode: TransferMode) {
        self.buffer.clear();
        self.buffer.extend_from_slice(data);
        self.buffer_limit = self.buffer.len();
        self.buffer_offset = 0;
        self.transfer_mode = mode;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.irq_pending = true;
    }

    fn expose_buffer_data_in(&mut self, len: usize, mode: TransferMode) {
        self.buffer_limit = len.min(self.buffer.len());
        self.buffer_offset = 0;
        self.transfer_mode = mode;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.irq_pending = true;
    }

    fn start_ata_read(&mut self, lba: u64, count: u32) {
        let drv = self.selected_drive();
        let drive = &self.drives[drv];
        if drive.kind != DriveKind::AtaDisk || lba >= drive.total_blocks {
            self.abort_ata();
            return;
        }

        self.ensure_buffer_len(ATA_SECTOR_SIZE);
        self.drives[drv].read_block(lba, &mut self.buffer[..ATA_SECTOR_SIZE]);
        self.transfer_mode = TransferMode::AtaRead;
        self.sectors_remaining = count.saturating_sub(1);
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.irq_pending = true;
    }

    fn start_ata_write(&mut self, count: u32) {
        if self.selected_drive_state().kind != DriveKind::AtaDisk {
            self.abort_ata();
            return;
        }
        self.ensure_buffer_len(ATA_SECTOR_SIZE);
        self.transfer_mode = TransferMode::AtaWrite;
        self.sectors_remaining = count;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
    }

    fn finish_command_ok(&mut self) {
        self.clear_transfer();
        self.status = SR_DRDY | SR_DSC;
        self.error = 0;
        self.irq_pending = true;
    }

    fn abort_ata(&mut self) {
        self.clear_transfer();
        self.status = SR_DRDY | SR_ERR;
        self.error = ER_ABRT;
        self.irq_pending = true;
    }

    fn set_atapi_sense(&mut self, key: u8, asc: u8, ascq: u8) {
        self.atapi_sense_key = key;
        self.atapi_asc = asc;
        self.atapi_ascq = ascq;
    }

    fn finish_atapi_command(&mut self) {
        self.clear_transfer();
        self.status = SR_DRDY | SR_DSC;
        self.error = 0;
        self.sector_count = 0x03;
        self.cylinder_low = 0;
        self.cylinder_high = 0;
        self.irq_pending = true;
    }

    fn abort_atapi(&mut self, key: u8, asc: u8, ascq: u8) {
        self.set_atapi_sense(key, asc, ascq);
        self.clear_transfer();
        self.status = SR_DRDY | SR_ERR;
        self.error = ER_ABRT;
        self.sector_count = 0x03;
        self.cylinder_low = 0;
        self.cylinder_high = 0;
        self.irq_pending = true;
    }

    fn start_atapi_packet_phase(&mut self) {
        self.atapi_byte_count_limit =
            u16::from(self.cylinder_low) | (u16::from(self.cylinder_high) << 8);
        if self.atapi_byte_count_limit == 0 {
            self.atapi_byte_count_limit = ATAPI_SECTOR_SIZE as u16;
        }
        self.ensure_buffer_len(ATAPI_PACKET_SIZE);
        self.transfer_mode = TransferMode::AtapiPacket;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.sector_count = 0x01;
        self.irq_pending = true;
    }

    fn start_atapi_data_in(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            self.finish_atapi_command();
            return;
        }
        self.buffer = data;
        self.buffer_offset = 0;
        self.transfer_mode = TransferMode::AtapiRead;
        self.prepare_atapi_data_chunk();
        ide_log!(
            "ATAPI data-in total_len={} byte_count_limit={}",
            self.buffer.len(),
            self.atapi_byte_count_limit
        );
    }

    fn prepare_atapi_data_chunk(&mut self) {
        let max_bytes = usize::from(self.atapi_byte_count_limit.max(1));
        let remaining = self.buffer.len().saturating_sub(self.buffer_offset);
        let chunk_len = remaining.min(max_bytes);
        self.buffer_limit = self.buffer_offset + chunk_len;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.sector_count = 0x02;
        self.cylinder_low = (chunk_len & 0xFF) as u8;
        self.cylinder_high = ((chunk_len >> 8) & 0xFF) as u8;
        self.irq_pending = true;
    }

    fn fill_identify_ata(&mut self) {
        let total_blocks = self.selected_drive_state().total_blocks;
        self.ensure_buffer_len(ATA_SECTOR_SIZE);
        self.buffer.fill(0);
        let w = |buf: &mut [u8], idx: usize, val: u16| {
            let off = idx * 2;
            buf[off] = val as u8;
            buf[off + 1] = (val >> 8) as u8;
        };

        w(&mut self.buffer, 0, 0x0040);
        let cyls = (total_blocks / (16 * 63)).min(16383) as u16;
        w(&mut self.buffer, 1, cyls);
        w(&mut self.buffer, 3, 16);
        w(&mut self.buffer, 6, 63);

        let serial = b"COREVM00000000000001";
        for i in 0..10 {
            let hi = serial[i * 2];
            let lo = serial[i * 2 + 1];
            w(&mut self.buffer, 10 + i, ((hi as u16) << 8) | lo as u16);
        }

        let fw = b"1.0     ";
        for i in 0..4 {
            let hi = fw[i * 2];
            let lo = fw[i * 2 + 1];
            w(&mut self.buffer, 23 + i, ((hi as u16) << 8) | lo as u16);
        }

        let model = b"CoreVM Virtual Disk                     ";
        for i in 0..20 {
            let hi = model[i * 2];
            let lo = model[i * 2 + 1];
            w(&mut self.buffer, 27 + i, ((hi as u16) << 8) | lo as u16);
        }

        w(&mut self.buffer, 47, 0x8010);
        w(&mut self.buffer, 49, 0x0200);
        w(&mut self.buffer, 53, 0x0007);
        w(&mut self.buffer, 54, cyls);
        w(&mut self.buffer, 55, 16);
        w(&mut self.buffer, 56, 63);

        let chs_sectors = (cyls as u32) * 16 * 63;
        w(&mut self.buffer, 57, chs_sectors as u16);
        w(&mut self.buffer, 58, (chs_sectors >> 16) as u16);

        let lba28_max = total_blocks.min(0x0FFF_FFFF) as u32;
        w(&mut self.buffer, 60, lba28_max as u16);
        w(&mut self.buffer, 61, (lba28_max >> 16) as u16);
        w(&mut self.buffer, 80, 0x0040);
        w(&mut self.buffer, 83, 0x0400);
        w(&mut self.buffer, 86, 0x0400);
        w(&mut self.buffer, 100, total_blocks as u16);
        w(&mut self.buffer, 101, (total_blocks >> 16) as u16);
        w(&mut self.buffer, 102, (total_blocks >> 32) as u16);
        w(&mut self.buffer, 103, (total_blocks >> 48) as u16);
    }

    fn fill_identify_packet(&mut self) {
        self.ensure_buffer_len(ATA_SECTOR_SIZE);
        self.buffer.fill(0);
        let w = |buf: &mut [u8], idx: usize, val: u16| {
            let off = idx * 2;
            buf[off] = val as u8;
            buf[off + 1] = (val >> 8) as u8;
        };

        w(&mut self.buffer, 0, 0x8580);
        w(&mut self.buffer, 49, 0x0200);
        w(&mut self.buffer, 80, 0x003E);
        w(&mut self.buffer, 82, 0x4000);
        w(&mut self.buffer, 83, 0x4000);

        let serial = b"COREVMCD000000000001";
        for i in 0..10 {
            let hi = serial[i * 2];
            let lo = serial[i * 2 + 1];
            w(&mut self.buffer, 10 + i, ((hi as u16) << 8) | lo as u16);
        }

        let fw = b"1.0     ";
        for i in 0..4 {
            let hi = fw[i * 2];
            let lo = fw[i * 2 + 1];
            w(&mut self.buffer, 23 + i, ((hi as u16) << 8) | lo as u16);
        }

        let model = b"CoreVM Virtual CD-ROM                   ";
        for i in 0..20 {
            let hi = model[i * 2];
            let lo = model[i * 2 + 1];
            w(&mut self.buffer, 27 + i, ((hi as u16) << 8) | lo as u16);
        }
    }

    fn fill_atapi_inquiry(&self) -> Vec<u8> {
        let mut data = vec![0u8; 36];
        data[0] = 0x05;
        data[1] = 0x80;
        data[2] = 0x00;
        data[3] = 0x21;
        data[4] = 31;
        data[8..16].copy_from_slice(b"COREVM  ");
        data[16..32].copy_from_slice(b"VIRTUAL CD-ROM  ");
        data[32..36].copy_from_slice(b"1.0 ");
        data
    }

    fn fill_atapi_request_sense(&mut self) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[0] = 0x70;
        data[2] = self.atapi_sense_key;
        data[7] = 10;
        data[12] = self.atapi_asc;
        data[13] = self.atapi_ascq;
        self.set_atapi_sense(0, 0, 0);
        data
    }

    fn fill_atapi_read_capacity(&self) -> Vec<u8> {
        let mut data = vec![0u8; 8];
        let blocks = self.drives[self.selected_drive()].total_blocks;
        let last_lba = if blocks == 0 { 0 } else { (blocks - 1).min(u32::MAX as u64) as u32 };
        data[0..4].copy_from_slice(&last_lba.to_be_bytes());
        data[4..8].copy_from_slice(&(ATAPI_SECTOR_SIZE as u32).to_be_bytes());
        data
    }

    fn fill_atapi_read_format_capacities(&self) -> Vec<u8> {
        let mut data = vec![0u8; 12];
        let blocks = self.drives[self.selected_drive()].total_blocks.min(u32::MAX as u64) as u32;
        data[3] = 8;
        data[4..8].copy_from_slice(&blocks.to_be_bytes());
        data[8] = 0x02;
        data[9..12].copy_from_slice(&(ATAPI_SECTOR_SIZE as u32).to_be_bytes()[1..4]);
        data
    }

    fn mode_data_6(&self) -> Vec<u8> {
        let mut data = vec![0u8; 4];
        data[0] = 3;
        data[2] = 0x80;
        data
    }

    fn mode_data_10(&self) -> Vec<u8> {
        let mut data = vec![0u8; 8];
        data[1] = 6;
        data[3] = 0x80;
        data
    }

    fn msf_from_lba(lba: u32) -> [u8; 4] {
        let mut abs = lba + 150;
        let minutes = abs / (75 * 60);
        abs %= 75 * 60;
        let seconds = abs / 75;
        let frames = abs % 75;
        [0, minutes as u8, seconds as u8, frames as u8]
    }

    fn fill_atapi_read_toc(&self, packet: &[u8]) -> Vec<u8> {
        let msf = packet[1] & 0x02 != 0;
        let mut data = vec![0u8; 20];
        data[1] = 18;
        data[2] = 1;
        data[3] = 1;
        data[5] = 0x14;
        data[6] = 1;
        data[13] = 0x16;
        data[14] = 0xAA;

        if msf {
            data[8..12].copy_from_slice(&Self::msf_from_lba(0));
            let lead_out = self.drives[self.selected_drive()].total_blocks.min(u32::MAX as u64) as u32;
            data[16..20].copy_from_slice(&Self::msf_from_lba(lead_out));
        } else {
            let lead_out = self.drives[self.selected_drive()].total_blocks.min(u32::MAX as u64) as u32;
            data[8..12].copy_from_slice(&0u32.to_be_bytes());
            data[16..20].copy_from_slice(&lead_out.to_be_bytes());
        }

        data
    }

    fn read_atapi_blocks(&mut self, lba: u32, blocks: u32) -> Option<Vec<u8>> {
        let drv = self.selected_drive();
        let drive = &self.drives[drv];
        let end = (lba as u64).checked_add(blocks as u64)?;
        if end > drive.total_blocks {
            return None;
        }

        let len = blocks as usize * ATAPI_SECTOR_SIZE;
        let mut data = vec![0u8; len];
        if len == 0 {
            return Some(data);
        }

        for block in 0..blocks {
            let start = block as usize * ATAPI_SECTOR_SIZE;
            let end = start + ATAPI_SECTOR_SIZE;
            self.drives[drv].read_block((lba + block) as u64, &mut data[start..end]);
        }
        Some(data)
    }

    fn allocation_len(packet: &[u8]) -> usize {
        match packet[0] {
            PKT_INQUIRY | PKT_REQUEST_SENSE | PKT_MODE_SENSE_6 => packet[4] as usize,
            PKT_MODE_SENSE_10 | PKT_READ_TOC_PMA_ATIP => {
                u16::from_be_bytes([packet[7], packet[8]]) as usize
            }
            _ => usize::MAX,
        }
    }

    fn start_atapi_response(&mut self, packet: &[u8], mut data: Vec<u8>) {
        let alloc_len = Self::allocation_len(packet);
        if alloc_len != usize::MAX && data.len() > alloc_len {
            data.truncate(alloc_len);
        }
        self.start_atapi_data_in(data);
    }

    fn execute_atapi_packet(&mut self) {
        let packet = self.buffer[..ATAPI_PACKET_SIZE].to_vec();
        ide_log!(
            "ATAPI packet op=0x{:02X} bytes={:02X?}",
            packet[0],
            &packet[..ATAPI_PACKET_SIZE]
        );

        match packet[0] {
            PKT_TEST_UNIT_READY
            | PKT_START_STOP_UNIT
            | PKT_PREVENT_ALLOW_MEDIUM_REMOVAL => self.finish_atapi_command(),
            PKT_REQUEST_SENSE => {
                let data = self.fill_atapi_request_sense();
                self.start_atapi_response(&packet, data);
            }
            PKT_INQUIRY => {
                let data = self.fill_atapi_inquiry();
                self.start_atapi_response(&packet, data);
            }
            PKT_MODE_SENSE_6 => {
                let data = self.mode_data_6();
                self.start_atapi_response(&packet, data);
            }
            PKT_MODE_SENSE_10 => {
                let data = self.mode_data_10();
                self.start_atapi_response(&packet, data);
            }
            PKT_READ_CAPACITY_10 => {
                let data = self.fill_atapi_read_capacity();
                self.start_atapi_response(&packet, data);
            }
            PKT_READ_FORMAT_CAPACITIES => {
                let data = self.fill_atapi_read_format_capacities();
                self.start_atapi_response(&packet, data);
            }
            PKT_READ_TOC_PMA_ATIP => {
                let data = self.fill_atapi_read_toc(&packet);
                self.start_atapi_response(&packet, data);
            }
            PKT_READ_10 => {
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let blocks = u16::from_be_bytes([packet[7], packet[8]]) as u32;
                ide_log!("READ(10) lba={} blocks={}", lba, blocks);
                match self.read_atapi_blocks(lba, blocks) {
                    Some(data) => self.start_atapi_data_in(data),
                    None => {
                        ide_log!("READ(10) beyond end: lba={} blocks={} total={}", lba, blocks, self.drives[self.selected_drive()].total_blocks);
                        self.abort_atapi(0x05, 0x21, 0x00)
                    }
                }
            }
            PKT_READ_12 => {
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let blocks = u32::from_be_bytes([packet[6], packet[7], packet[8], packet[9]]);
                ide_log!("READ(12) lba={} blocks={}", lba, blocks);
                match self.read_atapi_blocks(lba, blocks) {
                    Some(data) => self.start_atapi_data_in(data),
                    None => {
                        ide_log!("READ(12) beyond end: lba={} blocks={} total={}", lba, blocks, self.drives[self.selected_drive()].total_blocks);
                        self.abort_atapi(0x05, 0x21, 0x00)
                    }
                }
            }
            _ => self.abort_atapi(0x05, 0x20, 0x00),
        }
    }

    fn execute_ata_command(&mut self, cmd: u8) {
        match cmd {
            CMD_IDENTIFY => {
                self.fill_identify_ata();
                self.expose_buffer_data_in(ATA_SECTOR_SIZE, TransferMode::AtaRead);
            }
            CMD_READ_SECTORS | CMD_READ_MULTIPLE => {
                let lba = self.lba28();
                let count = if self.sector_count == 0 { 256 } else { self.sector_count as u32 };
                ide_log!("READ lba={} count={} dh=0x{:02X}", lba, count, self.drive_head);
                self.start_ata_read(lba, count);
            }
            CMD_READ_SECTORS_EXT => {
                let count = ((self.hob_sector_count as u32) << 8) | self.sector_count as u32;
                let count = if count == 0 { 65536 } else { count };
                self.start_ata_read(self.lba48(), count);
            }
            CMD_WRITE_SECTORS | CMD_WRITE_MULTIPLE => {
                let count = if self.sector_count == 0 { 256 } else { self.sector_count as u32 };
                self.start_ata_write(count);
            }
            CMD_WRITE_SECTORS_EXT => {
                let count = ((self.hob_sector_count as u32) << 8) | self.sector_count as u32;
                let count = if count == 0 { 65536 } else { count };
                self.start_ata_write(count);
            }
            CMD_SET_MULTIPLE => {
                if self.sector_count > 0 && self.sector_count <= 128 {
                    self.multiple_count = self.sector_count;
                    self.finish_command_ok();
                } else {
                    self.abort_ata();
                }
            }
            CMD_SET_FEATURES | CMD_INIT_DRIVE_PARAMS | CMD_NOP | CMD_FLUSH_CACHE => {
                self.finish_command_ok();
            }
            CMD_DEVICE_RESET => {
                self.apply_selected_drive_signature();
                self.finish_command_ok();
            }
            _ => self.abort_ata(),
        }
    }

    fn execute_atapi_command(&mut self, cmd: u8) {
        match cmd {
            CMD_IDENTIFY_PACKET => {
                self.fill_identify_packet();
                self.expose_buffer_data_in(ATA_SECTOR_SIZE, TransferMode::AtapiRead);
                self.sector_count = 0x02;
                self.cylinder_low = ATA_SECTOR_SIZE as u8;
                self.cylinder_high = (ATA_SECTOR_SIZE >> 8) as u8;
            }
            CMD_PACKET => self.start_atapi_packet_phase(),
            CMD_SET_FEATURES | CMD_NOP | CMD_FLUSH_CACHE => self.finish_atapi_command(),
            CMD_DEVICE_RESET => {
                self.apply_selected_drive_signature();
                self.finish_atapi_command();
            }
            CMD_IDENTIFY => self.abort_atapi(0x05, 0x20, 0x00),
            _ => self.abort_atapi(0x05, 0x20, 0x00),
        }
    }

    fn execute_command(&mut self, cmd: u8) {
        let drv = self.selected_drive();
        ide_log!("cmd=0x{:02X} drv={} present={} dh=0x{:02X}", cmd, drv, self.drives[drv].present, self.drive_head);

        if !self.drives[drv].present {
            self.abort_ata();
            return;
        }

        self.hob_toggle = false;
        match self.drives[drv].kind {
            DriveKind::AtaDisk => self.execute_ata_command(cmd),
            DriveKind::AtapiCdrom => self.execute_atapi_command(cmd),
        }
    }

    fn finish_data_read_byte(&mut self) {
        if self.buffer_offset < self.buffer_limit {
            return;
        }

        match self.transfer_mode {
            TransferMode::AtaRead => {
                if self.sectors_remaining > 0 {
                    self.advance_lba();
                    let lba = self.current_lba();
                    let drv = self.selected_drive();
                    self.drives[drv].read_block(lba, &mut self.buffer[..ATA_SECTOR_SIZE]);
                    self.buffer_offset = 0;
                    self.buffer_limit = ATA_SECTOR_SIZE;
                    self.sectors_remaining -= 1;
                    self.irq_pending = true;
                } else {
                    self.finish_command_ok();
                }
            }
            TransferMode::AtapiRead => {
                if self.buffer_offset < self.buffer.len() {
                    self.prepare_atapi_data_chunk();
                } else {
                    self.finish_atapi_command();
                }
            }
            _ => self.finish_command_ok(),
        }
    }

    fn read_data_byte(&mut self) -> u8 {
        if self.status & SR_DRQ == 0 || self.buffer_offset >= self.buffer_limit {
            return 0xFF;
        }
        let byte = self.buffer[self.buffer_offset];
        self.buffer_offset += 1;
        self.finish_data_read_byte();
        byte
    }

    fn read_data_word(&mut self) -> u16 {
        let lo = self.read_data_byte() as u16;
        let hi = self.read_data_byte() as u16;
        lo | (hi << 8)
    }

    fn read_data_dword(&mut self) -> u32 {
        let b0 = self.read_data_byte() as u32;
        let b1 = self.read_data_byte() as u32;
        let b2 = self.read_data_byte() as u32;
        let b3 = self.read_data_byte() as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    fn finish_data_write_byte(&mut self) {
        if self.buffer_offset < self.buffer_limit {
            return;
        }

        match self.transfer_mode {
            TransferMode::AtaWrite => {
                let lba = self.current_lba();
                let drv = self.selected_drive();
                let sector = &self.buffer[..ATA_SECTOR_SIZE];
                self.drives[drv].write_block(lba, sector);
                self.sectors_remaining = self.sectors_remaining.saturating_sub(1);
                if self.sectors_remaining > 0 {
                    self.advance_lba();
                    self.buffer[..ATA_SECTOR_SIZE].fill(0);
                    self.buffer_offset = 0;
                    self.buffer_limit = ATA_SECTOR_SIZE;
                    self.irq_pending = true;
                } else {
                    self.finish_command_ok();
                }
            }
            TransferMode::AtapiPacket => self.execute_atapi_packet(),
            _ => {}
        }
    }

    fn write_data_byte(&mut self, val: u8) {
        if self.status & SR_DRQ == 0 {
            return;
        }
        if !matches!(self.transfer_mode, TransferMode::AtaWrite | TransferMode::AtapiPacket) {
            return;
        }
        if self.buffer_offset < self.buffer_limit {
            self.buffer[self.buffer_offset] = val;
        }
        self.buffer_offset += 1;
        self.finish_data_write_byte();
    }

    fn write_data_word(&mut self, val: u16) {
        self.write_data_byte(val as u8);
        self.write_data_byte((val >> 8) as u8);
    }

    fn write_data_dword(&mut self, val: u32) {
        self.write_data_byte(val as u8);
        self.write_data_byte((val >> 8) as u8);
        self.write_data_byte((val >> 16) as u8);
        self.write_data_byte((val >> 24) as u8);
    }
}

impl IoHandler for Ide {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        match port {
            0x1F0 => match size {
                1 => Ok(self.read_data_byte() as u32),
                2 => Ok(self.read_data_word() as u32),
                _ => Ok(self.read_data_dword()),
            },
            0x1F1 => Ok(self.error as u32),
            0x1F2 => Ok(self.sector_count as u32),
            0x1F3 => Ok(self.sector_number as u32),
            0x1F4 => Ok(self.cylinder_low as u32),
            0x1F5 => Ok(self.cylinder_high as u32),
            0x1F6 => Ok(self.drive_head as u32),
            0x1F7 => {
                self.irq_pending = false;
                Ok(self.status as u32)
            }
            0x3F6 => Ok(self.status as u32),
            0x3F7 => Ok(0xFF),
            _ => Ok(0xFF),
        }
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        let v = val as u8;
        match port {
            0x1F0 => match size {
                1 => self.write_data_byte(val as u8),
                2 => self.write_data_word(val as u16),
                _ => self.write_data_dword(val),
            },
            0x1F1 => self.features = v,
            0x1F2 => {
                if self.hob_toggle {
                    self.hob_sector_count = v;
                } else {
                    self.sector_count = v;
                }
            }
            0x1F3 => {
                if self.hob_toggle {
                    self.hob_sector_number = v;
                } else {
                    self.sector_number = v;
                }
            }
            0x1F4 => {
                if self.hob_toggle {
                    self.hob_cylinder_low = v;
                } else {
                    self.cylinder_low = v;
                }
            }
            0x1F5 => {
                if self.hob_toggle {
                    self.hob_cylinder_high = v;
                } else {
                    self.cylinder_high = v;
                }
            }
            0x1F6 => {
                let prev_drive = self.selected_drive();
                self.drive_head = v;
                self.hob_toggle = false;
                // Device/head selects the target but must not reset the taskfile.
                // Native ATA/ATAPI drivers program 0x1F6 while building a
                // command; clobbering count/LBA/byte-count registers here
                // breaks packet commands after BIOS handoff.
                if prev_drive != self.selected_drive() && self.transfer_mode == TransferMode::None {
                    self.status = if self.selected_drive_state().present {
                        SR_DRDY | SR_DSC
                    } else {
                        SR_DSC
                    };
                }
            }
            0x1F7 => self.execute_command(v),
            0x3F6 => {
                let old = self.device_control;
                self.device_control = v;
                if v & 0x04 != 0 && old & 0x04 == 0 {
                    self.status = SR_BSY;
                }
                if v & 0x04 == 0 && old & 0x04 != 0 {
                    self.status = SR_DRDY | SR_DSC;
                    self.apply_selected_drive_signature();
                    self.irq_pending = false;
                }
                self.hob_toggle = v & 0x80 != 0;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_cd_image(blocks: u32) -> Vec<u8> {
        let mut image = vec![0u8; blocks as usize * ATAPI_SECTOR_SIZE];
        for block in 0..blocks as usize {
            for offset in 0..ATAPI_SECTOR_SIZE {
                image[block * ATAPI_SECTOR_SIZE + offset] =
                    ((block as u32).wrapping_mul(17).wrapping_add(offset as u32)) as u8;
            }
        }
        image
    }

    fn issue_atapi_read_10(ide: &mut Ide, lba: u32, blocks: u16, byte_count_limit: u16) {
        ide.attach_slave(build_cd_image(128));
        ide.write(0x1F6, 1, 0xB0).unwrap();
        ide.write(0x1F4, 1, (byte_count_limit & 0xFF) as u32).unwrap();
        ide.write(0x1F5, 1, (byte_count_limit >> 8) as u32).unwrap();
        ide.write(0x1F7, 1, CMD_PACKET as u32).unwrap();

        let packet = [
            PKT_READ_10,
            0,
            (lba >> 24) as u8,
            (lba >> 16) as u8,
            (lba >> 8) as u8,
            lba as u8,
            0,
            (blocks >> 8) as u8,
            blocks as u8,
            0,
            0,
            0,
        ];
        for chunk in packet.chunks_exact(2) {
            let word = u16::from_le_bytes([chunk[0], chunk[1]]);
            ide.write(0x1F0, 2, word as u32).unwrap();
        }
    }

    fn read_atapi_data_chunks(ide: &mut Ide) -> Vec<u8> {
        let mut data = Vec::new();
        loop {
            let status = ide.read(0x1F7, 1).unwrap() as u8;
            if status & SR_DRQ == 0 {
                break;
            }

            let chunk_len = ide.read(0x1F4, 1).unwrap() as usize
                | ((ide.read(0x1F5, 1).unwrap() as usize) << 8);
            assert!(chunk_len > 0);

            for _ in 0..(chunk_len / 2) {
                let word = ide.read(0x1F0, 2).unwrap() as u16;
                data.extend_from_slice(&word.to_le_bytes());
            }
            if (chunk_len & 1) != 0 {
                data.push(ide.read(0x1F0, 1).unwrap() as u8);
            }
        }
        data
    }

    fn expected_cd_bytes(lba: u32, blocks: u16) -> Vec<u8> {
        let image = build_cd_image(128);
        let start = lba as usize * ATAPI_SECTOR_SIZE;
        let end = start + blocks as usize * ATAPI_SECTOR_SIZE;
        image[start..end].to_vec()
    }

    #[test]
    fn atapi_read_10_preserves_multiblock_data_across_chunks() {
        let mut ide = Ide::new();
        issue_atapi_read_10(&mut ide, 47, 11, ATAPI_SECTOR_SIZE as u16);
        let data = read_atapi_data_chunks(&mut ide);
        assert_eq!(data, expected_cd_bytes(47, 11));
    }

    #[test]
    fn atapi_read_10_preserves_data_with_smaller_chunk_limit() {
        let mut ide = Ide::new();
        issue_atapi_read_10(&mut ide, 13, 4, 512);
        let data = read_atapi_data_chunks(&mut ide);
        assert_eq!(data, expected_cd_bytes(13, 4));
    }
}
