use super::{exit_reason, VmExitInfo};

pub(super) const IO_PORT_POST_DELAY: u16 = 0x80;
const IO_PORT_PIC1_CMD: u16 = 0x20;
pub(super) const IO_PORT_PIC1_DATA: u16 = 0x21;
const IO_PORT_PIC2_CMD: u16 = 0xa0;
const IO_PORT_PIC2_DATA: u16 = 0xa1;
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
    pic1_cmd: u8,
    pic1_data: u8,
    pic2_cmd: u8,
    pic2_data: u8,
    pit_cmd: u8,
    pit_data: [u8; 3],
    cmos_index: u8,
}

impl Default for PlatformIoState {
    fn default() -> Self {
        Self {
            pic1_cmd: 0,
            pic1_data: 0xff,
            pic2_cmd: 0,
            pic2_data: 0xff,
            pit_cmd: 0,
            pit_data: [0; 3],
            cmos_index: 0,
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
        IO_PORT_PIC1_CMD => state.pic1_cmd = value,
        IO_PORT_PIC1_DATA => state.pic1_data = value,
        IO_PORT_PIC2_CMD => state.pic2_cmd = value,
        IO_PORT_PIC2_DATA => state.pic2_data = value,
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
        IO_PORT_PIC1_CMD => state.pic1_cmd as u32,
        IO_PORT_PIC1_DATA => state.pic1_data as u32,
        IO_PORT_PIC2_CMD => state.pic2_cmd as u32,
        IO_PORT_PIC2_DATA => state.pic2_data as u32,
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
