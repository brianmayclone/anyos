//! VGA text mode driver for the 80x25 color console.
//!
//! Writes directly to the VGA text buffer at 0xB8000 with support for
//! scrolling, color attributes, cursor positioning, and `fmt::Write` output.

use core::fmt;

/// Physical address of the VGA text mode framebuffer.
const VGA_BUFFER: u32 = 0xB8000;
/// Number of character columns on screen.
const VGA_WIDTH: usize = 80;
/// Number of character rows on screen.
const VGA_HEIGHT: usize = 25;

/// Standard 16-color VGA text mode palette.
#[repr(u8)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

fn color_code(fg: Color, bg: Color) -> u8 {
    (bg as u8) << 4 | (fg as u8)
}

static mut COL: usize = 0;
static mut ROW: usize = 0;
static mut ATTR: u8 = 0x0F; // White on black

/// Initialize the VGA text console by clearing the screen.
pub fn init() {
    clear();
}

/// Set the foreground and background color for subsequent text output.
pub fn set_color(fg: Color, bg: Color) {
    unsafe { ATTR = color_code(fg, bg); }
}

/// Clear the entire VGA text screen and reset cursor to top-left.
pub fn clear() {
    let buffer = VGA_BUFFER as *mut u16;
    let blank = 0x0F00 | b' ' as u16; // White on black, space
    for i in 0..(VGA_WIDTH * VGA_HEIGHT) {
        unsafe { buffer.add(i).write_volatile(blank); }
    }
    unsafe {
        COL = 0;
        ROW = 0;
    }
}

fn scroll() {
    let buffer = VGA_BUFFER as *mut u16;

    // Move all rows up by one
    for row in 1..VGA_HEIGHT {
        for col in 0..VGA_WIDTH {
            let src = row * VGA_WIDTH + col;
            let dst = (row - 1) * VGA_WIDTH + col;
            unsafe {
                let ch = buffer.add(src).read_volatile();
                buffer.add(dst).write_volatile(ch);
            }
        }
    }

    // Clear last row
    let blank = (unsafe { ATTR } as u16) << 8 | b' ' as u16;
    for col in 0..VGA_WIDTH {
        let offset = (VGA_HEIGHT - 1) * VGA_WIDTH + col;
        unsafe { buffer.add(offset).write_volatile(blank); }
    }
}

/// Write a single character to the VGA console, handling newlines, tabs, and scrolling.
pub fn put_char(c: u8) {
    unsafe {
        match c {
            b'\n' => {
                COL = 0;
                ROW += 1;
            }
            b'\r' => {
                COL = 0;
            }
            b'\t' => {
                COL = (COL + 8) & !7;
            }
            _ => {
                let offset = ROW * VGA_WIDTH + COL;
                let entry = (ATTR as u16) << 8 | c as u16;
                let buffer = VGA_BUFFER as *mut u16;
                buffer.add(offset).write_volatile(entry);
                COL += 1;
            }
        }

        if COL >= VGA_WIDTH {
            COL = 0;
            ROW += 1;
        }

        if ROW >= VGA_HEIGHT {
            scroll();
            ROW = VGA_HEIGHT - 1;
        }
    }
}

/// Erase the character before the cursor position.
pub fn backspace() {
    unsafe {
        if COL > 0 {
            COL -= 1;
            let offset = ROW * VGA_WIDTH + COL;
            let blank = (ATTR as u16) << 8 | b' ' as u16;
            let buffer = VGA_BUFFER as *mut u16;
            buffer.add(offset).write_volatile(blank);
        }
    }
}

/// Clear from the current cursor position to the end of the line.
pub fn clear_to_eol() {
    unsafe {
        let blank = (ATTR as u16) << 8 | b' ' as u16;
        let buffer = VGA_BUFFER as *mut u16;
        for col in COL..VGA_WIDTH {
            let offset = ROW * VGA_WIDTH + col;
            buffer.add(offset).write_volatile(blank);
        }
    }
}

/// Write a string to the VGA console.
pub fn put_str(s: &str) {
    for byte in s.bytes() {
        put_char(byte);
    }
}

/// Zero-sized type implementing `fmt::Write` for VGA text output.
pub struct VgaWriter;

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            put_char(byte);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::drivers::vga_text::VgaWriter, $($arg)*);
    }};
}

#[macro_export]
macro_rules! vga_println {
    () => { $crate::vga_print!("\n") };
    ($($arg:tt)*) => { $crate::vga_print!("{}\n", format_args!($($arg)*)) };
}
