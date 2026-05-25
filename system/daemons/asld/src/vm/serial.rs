use alloc::vec::Vec;

use super::{exit_reason, VmExitInfo};

const COM1_BASE: u16 = 0x3f8;
pub(super) const UART_RBR_THR_DLL: u16 = COM1_BASE;
pub(super) const UART_IER_DLM: u16 = COM1_BASE + 1;
pub(super) const UART_IIR_FCR: u16 = COM1_BASE + 2;
pub(super) const UART_LCR: u16 = COM1_BASE + 3;
const UART_MCR: u16 = COM1_BASE + 4;
pub(super) const UART_LSR: u16 = COM1_BASE + 5;
const UART_MSR: u16 = COM1_BASE + 6;
const UART_SCR: u16 = COM1_BASE + 7;
pub(super) const UART_LCR_DLAB: u8 = 0x80;
const UART_IER_RX_AVAILABLE: u8 = 0x01;
const UART_IIR_NO_INTERRUPT: u32 = 0x01;
const UART_IIR_RX_AVAILABLE: u32 = 0x04;
const UART_LSR_DATA_READY: u32 = 0x01;
const UART_LSR_THR_EMPTY: u32 = 0x20;
const UART_LSR_TRANSMITTER_EMPTY: u32 = 0x40;
const RX_QUEUE_LIMIT: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SerialPortState {
    lcr: u8,
    ier: u8,
    mcr: u8,
    scratch: u8,
    rx: Vec<u8>,
    tx_count: u64,
}

impl SerialPortState {
    pub(super) fn push_input(&mut self, bytes: &[u8]) {
        let room = RX_QUEUE_LIMIT.saturating_sub(self.rx.len());
        self.rx.extend(bytes.iter().copied().take(room));
    }

    pub(super) fn pending_irq(&self) -> bool {
        self.ier & UART_IER_RX_AVAILABLE != 0 && !self.rx.is_empty()
    }

    pub(super) fn output_bytes(&self) -> u64 {
        self.tx_count
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SerialIoAction {
    pub output: Vec<u8>,
    pub read_value: Option<u32>,
}

pub(super) fn serial_io_action(
    state: &mut SerialPortState,
    exit: &VmExitInfo,
) -> Option<SerialIoAction> {
    if exit.reason != exit_reason::IO_INSTRUCTION || !is_com1_port(exit.io_port) {
        return None;
    }

    if exit.is_read != 0 {
        return Some(SerialIoAction {
            output: Vec::new(),
            read_value: Some(serial_read(state, exit.io_port)),
        });
    }

    let value = (exit.io_data & 0xff) as u8;
    let mut output = Vec::new();
    match exit.io_port {
        UART_RBR_THR_DLL => {
            if state.lcr & UART_LCR_DLAB == 0 {
                output.push(value);
                state.tx_count = state.tx_count.wrapping_add(1);
            }
        }
        UART_IER_DLM => {
            if state.lcr & UART_LCR_DLAB == 0 {
                state.ier = value;
            }
        }
        UART_LCR => state.lcr = value,
        UART_MCR => state.mcr = value,
        UART_SCR => state.scratch = value,
        UART_IIR_FCR => {
            if value & 0x02 != 0 {
                state.rx.clear();
            }
        }
        UART_LSR | UART_MSR => {}
        _ => {}
    }
    Some(SerialIoAction {
        output,
        read_value: None,
    })
}

fn serial_read(state: &mut SerialPortState, port: u16) -> u32 {
    match port {
        UART_RBR_THR_DLL => {
            if state.lcr & UART_LCR_DLAB == 0 {
                if state.rx.is_empty() {
                    0
                } else {
                    state.rx.remove(0) as u32
                }
            } else {
                0
            }
        }
        UART_IER_DLM => {
            if state.lcr & UART_LCR_DLAB == 0 {
                state.ier as u32
            } else {
                0
            }
        }
        UART_IIR_FCR => {
            if state.pending_irq() {
                UART_IIR_RX_AVAILABLE
            } else {
                UART_IIR_NO_INTERRUPT
            }
        }
        UART_LCR => state.lcr as u32,
        UART_MCR => state.mcr as u32,
        UART_LSR => {
            let data_ready = if state.rx.is_empty() {
                0
            } else {
                UART_LSR_DATA_READY
            };
            UART_LSR_THR_EMPTY | UART_LSR_TRANSMITTER_EMPTY | data_ready
        }
        UART_MSR => 0xb0,
        UART_SCR => state.scratch as u32,
        _ => 0,
    }
}

fn is_com1_port(port: u16) -> bool {
    (COM1_BASE..=UART_SCR).contains(&port)
}
