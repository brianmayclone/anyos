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
const PIT_INPUT_HZ: u64 = 1_193_182;
const PIT_DEFAULT_PERIOD_MS: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlatformIoState {
    pic1: PicChip,
    pic2: PicChip,
    pit: PitState,
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
            pit: PitState::default(),
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

    pub(super) fn pending_irq(&mut self) -> Option<u8> {
        self.pit.refresh(anyos_std::sys::uptime_ms());
        self.pit.pending_irq()
    }

    pub(super) fn ack_irq(&mut self, irq: u8) {
        if irq == 0 {
            self.pit.ack_irq();
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
        IO_PORT_PIT_CH0 => state.pit.write_channel0(value, anyos_std::sys::uptime_ms()),
        IO_PORT_PIT_CH1 => state.pit.write_unused_channel(1, value),
        IO_PORT_PIT_CH2 => state.pit.write_unused_channel(2, value),
        IO_PORT_PIT_CMD => state.pit.write_command(value, anyos_std::sys::uptime_ms()),
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
        IO_PORT_PIT_CH0 => state.pit.read_channel(0) as u32,
        IO_PORT_PIT_CH1 => state.pit.read_channel(1) as u32,
        IO_PORT_PIT_CH2 => state.pit.read_channel(2) as u32,
        IO_PORT_PIT_CMD => state.pit.command as u32,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PitState {
    command: u8,
    channel_data: [u8; 3],
    access_mode: u8,
    mode: u8,
    low_latch: u8,
    expecting_high: bool,
    reload: u16,
    period_ms: u32,
    next_irq_ms: u32,
    enabled: bool,
    pending_irq0: bool,
}

impl Default for PitState {
    fn default() -> Self {
        Self {
            command: 0,
            channel_data: [0; 3],
            access_mode: 3,
            mode: 3,
            low_latch: 0,
            expecting_high: false,
            reload: 0,
            period_ms: PIT_DEFAULT_PERIOD_MS,
            next_irq_ms: 0,
            enabled: false,
            pending_irq0: false,
        }
    }
}

impl PitState {
    fn write_command(&mut self, value: u8, now_ms: u32) {
        self.command = value;
        let channel = value >> 6;
        let access = (value >> 4) & 0x3;
        if channel != 0 || access == 0 {
            return;
        }
        self.access_mode = access;
        self.mode = (value >> 1) & 0x7;
        if self.mode >= 6 {
            self.mode &= 0x3;
        }
        self.expecting_high = false;
        if self.reload != 0 {
            self.arm(now_ms);
        }
    }

    fn write_channel0(&mut self, value: u8, now_ms: u32) {
        self.channel_data[0] = value;
        match self.access_mode {
            1 => self.program_reload((self.reload & 0xff00) | value as u16, now_ms),
            2 => self.program_reload((value as u16) << 8, now_ms),
            3 => {
                if self.expecting_high {
                    let reload = ((value as u16) << 8) | self.low_latch as u16;
                    self.expecting_high = false;
                    self.program_reload(reload, now_ms);
                } else {
                    self.low_latch = value;
                    self.expecting_high = true;
                }
            }
            _ => {}
        }
    }

    fn write_unused_channel(&mut self, channel: usize, value: u8) {
        if let Some(slot) = self.channel_data.get_mut(channel) {
            *slot = value;
        }
    }

    fn read_channel(&self, channel: usize) -> u8 {
        self.channel_data.get(channel).copied().unwrap_or(0)
    }

    fn pending_irq(&self) -> Option<u8> {
        if self.pending_irq0 {
            Some(0)
        } else {
            None
        }
    }

    fn ack_irq(&mut self) {
        self.pending_irq0 = false;
    }

    fn program_reload(&mut self, reload: u16, now_ms: u32) {
        self.reload = reload;
        self.period_ms = pit_period_ms(reload);
        self.arm(now_ms);
    }

    fn arm(&mut self, now_ms: u32) {
        self.enabled = true;
        self.pending_irq0 = false;
        self.next_irq_ms = now_ms.wrapping_add(self.period_ms);
    }

    fn refresh(&mut self, now_ms: u32) {
        if !self.enabled || self.pending_irq0 || !time_due(now_ms, self.next_irq_ms) {
            return;
        }

        self.pending_irq0 = true;
        let period = self.period_ms.max(1);
        self.next_irq_ms = self.next_irq_ms.wrapping_add(period);
        for _ in 0..16 {
            if !time_due(now_ms, self.next_irq_ms) {
                break;
            }
            self.next_irq_ms = self.next_irq_ms.wrapping_add(period);
        }
    }
}

fn pit_period_ms(reload: u16) -> u32 {
    let count = if reload == 0 { 65_536 } else { reload as u64 };
    let ms = (count * 1000 + PIT_INPUT_HZ - 1) / PIT_INPUT_HZ;
    (ms as u32).max(1)
}

fn time_due(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) <= 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::{
        pit_period_ms, platform_io_action, PlatformIoState, VmExitInfo, IO_PORT_PIC1_DATA,
    };
    use crate::vm::exit_reason;

    #[test]
    fn pit_reload_uses_legacy_18hz_period_for_zero_count() {
        assert_eq!(pit_period_ms(0), 55);
    }

    #[test]
    fn pit_channel0_raises_pending_irq_when_due() {
        let mut state = PlatformIoState::default();
        state.pit.write_command(0x36, 1000);
        state.pit.write_channel0(0, 1000);
        state.pit.write_channel0(0, 1000);

        assert_eq!(state.pit.pending_irq(), None);
        state.pit.refresh(1055);
        assert_eq!(state.pit.pending_irq(), Some(0));
        state.ack_irq(0);
        assert_eq!(state.pit.pending_irq(), None);
    }

    #[test]
    fn pic_remap_exposes_irq0_vector() {
        let mut state = PlatformIoState::default();
        write_port(&mut state, 0x20, 0x11);
        write_port(&mut state, 0x21, 0x20);
        write_port(&mut state, 0x21, 0x04);
        write_port(&mut state, 0x21, 0x01);
        write_port(&mut state, IO_PORT_PIC1_DATA, 0xfe);
        assert_eq!(state.irq_vector(0), Some(0x20));
    }

    fn write_port(state: &mut PlatformIoState, port: u16, value: u8) {
        let _ = platform_io_action(
            state,
            &VmExitInfo {
                reason: exit_reason::IO_INSTRUCTION,
                io_port: port,
                access_size: 1,
                is_read: 0,
                io_data: value as u64,
                ..Default::default()
            },
        );
    }
}
