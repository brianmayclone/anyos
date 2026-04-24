use super::{exit_reason, VmExitInfo};

pub(super) const IO_PORT_POST_DELAY: u16 = 0x80;
pub(super) const IO_PORT_PIC1_CMD: u16 = 0x20;
pub(super) const IO_PORT_PIC1_DATA: u16 = 0x21;
pub(super) const IO_PORT_PIC2_CMD: u16 = 0xa0;
pub(super) const IO_PORT_PIC2_DATA: u16 = 0xa1;
const IO_PORT_PIT_CH0: u16 = 0x40;
const IO_PORT_PIT_CH1: u16 = 0x41;
const IO_PORT_PIT_CH2: u16 = 0x42;
const IO_PORT_PIT_CMD: u16 = 0x43;
pub(super) const IO_PORT_CMOS_INDEX: u16 = 0x70;
pub(super) const IO_PORT_CMOS_DATA: u16 = 0x71;
const IO_PORT_KBD_DATA: u16 = 0x60;
pub(super) const IO_PORT_KBD_STATUS: u16 = 0x64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlatformIoState {
    pic1: PicChip,
    pic2: PicChip,
    pit_cmd: u8,
    pit_data: [u8; 3],
    cmos_index: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PicChip {
    command: u8,
    mask: u8,
    vector_offset: u8,
    init_step: u8,
    expect_icw4: bool,
}

impl PicChip {
    const fn new(vector_offset: u8) -> Self {
        Self {
            command: 0,
            mask: 0xff,
            vector_offset,
            init_step: 0,
            expect_icw4: false,
        }
    }

    fn command_write(&mut self, value: u8) {
        self.command = value;
        if value & 0x10 != 0 {
            self.init_step = 1;
            self.expect_icw4 = value & 0x01 != 0;
        }
    }

    fn data_write(&mut self, value: u8) {
        match self.init_step {
            1 => {
                self.vector_offset = value & 0xf8;
                self.init_step = 2;
            }
            2 => {
                self.init_step = if self.expect_icw4 { 3 } else { 0 };
            }
            3 => {
                self.init_step = 0;
            }
            _ => self.mask = value,
        }
    }

    fn data_read(&self) -> u8 {
        self.mask
    }
}

impl Default for PlatformIoState {
    fn default() -> Self {
        Self {
            pic1: PicChip::new(0x08),
            pic2: PicChip::new(0x70),
            pit_cmd: 0,
            pit_data: [0; 3],
            cmos_index: 0,
        }
    }
}

impl PlatformIoState {
    pub(super) fn irq_vector(&self, irq: u8) -> Option<u8> {
        match irq {
            0..=7 => {
                if self.pic1.mask & (1 << irq) == 0 {
                    Some(self.pic1.vector_offset.wrapping_add(irq))
                } else {
                    None
                }
            }
            8..=15 => {
                let slave_irq = irq - 8;
                let cascade_unmasked = self.pic1.mask & (1 << 2) == 0;
                let slave_unmasked = self.pic2.mask & (1 << slave_irq) == 0;
                if cascade_unmasked && slave_unmasked {
                    Some(self.pic2.vector_offset.wrapping_add(slave_irq))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PlatformIoAction {
    pub read_value: Option<u32>,
}

pub(super) fn platform_io_action(
    state: &mut PlatformIoState,
    exit: &VmExitInfo,
) -> Option<PlatformIoAction> {
    if exit.reason != exit_reason::IO_INSTRUCTION || !is_platform_io_port(exit.io_port) {
        return None;
    }

    if exit.is_read != 0 {
        return Some(PlatformIoAction {
            read_value: Some(platform_io_read(state, exit.io_port)),
        });
    }

    let value = (exit.io_data & 0xff) as u8;
    match exit.io_port {
        IO_PORT_PIC1_CMD => state.pic1.command_write(value),
        IO_PORT_PIC1_DATA => state.pic1.data_write(value),
        IO_PORT_PIC2_CMD => state.pic2.command_write(value),
        IO_PORT_PIC2_DATA => state.pic2.data_write(value),
        IO_PORT_PIT_CH0 => state.pit_data[0] = value,
        IO_PORT_PIT_CH1 => state.pit_data[1] = value,
        IO_PORT_PIT_CH2 => state.pit_data[2] = value,
        IO_PORT_PIT_CMD => state.pit_cmd = value,
        IO_PORT_CMOS_INDEX => state.cmos_index = value & 0x7f,
        IO_PORT_POST_DELAY | IO_PORT_CMOS_DATA | IO_PORT_KBD_DATA | IO_PORT_KBD_STATUS => {}
        _ => {}
    }
    Some(PlatformIoAction { read_value: None })
}

fn platform_io_read(state: &PlatformIoState, port: u16) -> u32 {
    match port {
        IO_PORT_PIC1_CMD => state.pic1.command as u32,
        IO_PORT_PIC1_DATA => state.pic1.data_read() as u32,
        IO_PORT_PIC2_CMD => state.pic2.command as u32,
        IO_PORT_PIC2_DATA => state.pic2.data_read() as u32,
        IO_PORT_PIT_CH0 => state.pit_data[0] as u32,
        IO_PORT_PIT_CH1 => state.pit_data[1] as u32,
        IO_PORT_PIT_CH2 => state.pit_data[2] as u32,
        IO_PORT_PIT_CMD => state.pit_cmd as u32,
        IO_PORT_CMOS_INDEX => state.cmos_index as u32,
        IO_PORT_CMOS_DATA => cmos_read(state.cmos_index),
        IO_PORT_KBD_DATA => 0,
        IO_PORT_KBD_STATUS => 0x10,
        IO_PORT_POST_DELAY => 0,
        _ => 0,
    }
}

fn cmos_read(index: u8) -> u32 {
    match index {
        0x0a => 0x26,
        0x0b => 0x02,
        0x0c => 0,
        0x0d => 0x80,
        0x15 => 0,
        0x16 => 0,
        0x17 => 0,
        0x18 => 0,
        _ => 0,
    }
}

fn is_platform_io_port(port: u16) -> bool {
    matches!(
        port,
        IO_PORT_POST_DELAY
            | IO_PORT_PIC1_CMD
            | IO_PORT_PIC1_DATA
            | IO_PORT_PIC2_CMD
            | IO_PORT_PIC2_DATA
            | IO_PORT_PIT_CH0
            | IO_PORT_PIT_CH1
            | IO_PORT_PIT_CH2
            | IO_PORT_PIT_CMD
            | IO_PORT_CMOS_INDEX
            | IO_PORT_CMOS_DATA
            | IO_PORT_KBD_DATA
            | IO_PORT_KBD_STATUS
    )
}
