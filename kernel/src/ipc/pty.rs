//! Minimal pseudo-terminal line discipline.
//!
//! The terminal application remains the master side via its existing named
//! stdin/stdout pipes. Processes see fd 0/1/2 as a PTY slave with termios state,
//! canonical input, echo, and basic output post-processing.

use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

const ICRNL: u32 = 0x0100;
const OPOST: u32 = 0x0001;
const ONLCR: u32 = 0x0004;
const ICANON: u32 = 0x0002;
const ECHO: u32 = 0x0008;

const EAGAIN_SENTINEL: u32 = u32::MAX - 10;

#[derive(Clone, Copy)]
pub struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; 19],
}

impl Termios {
    pub const fn default() -> Self {
        Self {
            iflag: 0x0500, // ICRNL | IXON
            oflag: 0x0005, // OPOST | ONLCR
            cflag: 0x00bf, // B38400 | CS8 | CREAD
            lflag: 0x8a3b, // ISIG | ICANON | ECHO | ECHOE | ECHOK | IEXTEN ...
            line: 0,
            cc: [
                3, 28, 127, 21, 4, 0, 1, 0, 17, 19, 26, 0, 18, 15, 23, 22, 0, 0, 0,
            ],
        }
    }
}

struct Pty {
    id: u32,
    input_pipe: u32,
    output_pipe: u32,
    termios: Termios,
    read_buf: VecDeque<u8>,
    line_buf: Vec<u8>,
}

static PTYS: Spinlock<Vec<Pty>> = Spinlock::new(Vec::new());
static NEXT_PTY_ID: AtomicU32 = AtomicU32::new(1);

pub fn create(input_pipe: u32, output_pipe: u32) -> u32 {
    if input_pipe == 0 || output_pipe == 0 {
        return 0;
    }
    let id = NEXT_PTY_ID.fetch_add(1, Ordering::Relaxed);
    PTYS.lock().push(Pty {
        id,
        input_pipe,
        output_pipe,
        termios: Termios::default(),
        read_buf: VecDeque::new(),
        line_buf: Vec::new(),
    });
    id
}

pub fn get_termios(id: u32) -> Option<Termios> {
    PTYS.lock()
        .iter()
        .find(|pty| pty.id == id)
        .map(|pty| pty.termios)
}

pub fn set_termios(id: u32, termios: Termios) -> bool {
    let mut ptys = PTYS.lock();
    if let Some(pty) = ptys.iter_mut().find(|pty| pty.id == id) {
        pty.termios = termios;
        true
    } else {
        false
    }
}

/// Called by the named-pipe layer when a terminal master writes input bytes.
///
/// This makes PTY echo and canonical editing happen at input time, just like a
/// real terminal driver.  Previously the input pipe was only pumped when the
/// slave process called `read()`, so interactive echo could be delayed until the
/// next completed line.
pub fn notify_input_pipe_written(input_pipe: u32) {
    if input_pipe == 0 {
        return;
    }

    let Some(mut ptys) = PTYS.try_lock() else {
        return;
    };

    for pty in ptys.iter_mut().filter(|pty| pty.input_pipe == input_pipe) {
        pty.pump_master_input();
    }
}

pub fn read_slave(id: u32, buf: &mut [u8], blocking: bool) -> u32 {
    if buf.is_empty() {
        return 0;
    }

    loop {
        let n = {
            let mut ptys = PTYS.lock();
            let Some(pty) = ptys.iter_mut().find(|pty| pty.id == id) else {
                return 0;
            };
            pty.pump_master_input();
            pty.copy_ready(buf)
        };

        if n != 0 {
            return n;
        }
        if !blocking {
            return EAGAIN_SENTINEL;
        }

        let wake_at = crate::arch::hal::timer_current_ticks().wrapping_add(1);
        crate::task::scheduler::sleep_until(wake_at);
    }
}

pub fn write_slave(id: u32, data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }

    let output_pipe = {
        let ptys = PTYS.lock();
        let Some(pty) = ptys.iter().find(|pty| pty.id == id) else {
            return u32::MAX;
        };
        pty.output_pipe
    };

    let termios = get_termios(id).unwrap_or_else(Termios::default);
    let mut out = Vec::with_capacity(data.len().saturating_mul(2).min(4096));
    for &byte in data {
        if byte == b'\n' && (termios.oflag & (OPOST | ONLCR)) == (OPOST | ONLCR) {
            out.push(b'\r');
            out.push(b'\n');
        } else {
            out.push(byte);
        }
        if out.len() >= 4096 {
            let _ = crate::ipc::pipe::write(output_pipe, &out);
            out.clear();
        }
    }
    if !out.is_empty() {
        let _ = crate::ipc::pipe::write(output_pipe, &out);
    }
    data.len() as u32
}

impl Pty {
    fn copy_ready(&mut self, buf: &mut [u8]) -> u32 {
        let n = self.read_buf.len().min(buf.len());
        for dst in buf.iter_mut().take(n) {
            *dst = self.read_buf.pop_front().unwrap_or(0);
        }
        n as u32
    }

    fn pump_master_input(&mut self) {
        let mut tmp = [0u8; 256];
        loop {
            let n = crate::ipc::pipe::read(self.input_pipe, &mut tmp);
            if n == 0 || n == u32::MAX {
                break;
            }
            for &byte in &tmp[..n as usize] {
                self.accept_input_byte(byte);
            }
        }
    }

    fn accept_input_byte(&mut self, mut byte: u8) {
        if byte == b'\r' && (self.termios.iflag & ICRNL) != 0 {
            byte = b'\n';
        }

        if (self.termios.lflag & ICANON) != 0 {
            match byte {
                0x08 | 0x7f => {
                    if self.line_buf.pop().is_some() && (self.termios.lflag & ECHO) != 0 {
                        self.echo(b"\x08 \x08");
                    }
                }
                b'\n' => {
                    self.line_buf.push(b'\n');
                    if (self.termios.lflag & ECHO) != 0 {
                        self.echo(b"\r\n");
                    }
                    self.read_buf.extend(self.line_buf.drain(..));
                }
                byte => {
                    self.line_buf.push(byte);
                    if (self.termios.lflag & ECHO) != 0 {
                        self.echo(&[byte]);
                    }
                }
            }
        } else {
            self.read_buf.push_back(byte);
            if (self.termios.lflag & ECHO) != 0 {
                self.echo(&[byte]);
            }
        }
    }

    fn echo(&self, bytes: &[u8]) {
        let _ = crate::ipc::pipe::write(self.output_pipe, bytes);
    }
}
