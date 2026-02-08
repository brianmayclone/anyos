use core::fmt;

const VGA_BUFFER: u32 = 0xB8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

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

pub fn init() {
    clear();
}

pub fn set_color(fg: Color, bg: Color) {
    unsafe { ATTR = color_code(fg, bg); }
}

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

pub fn put_str(s: &str) {
    for byte in s.bytes() {
        put_char(byte);
    }
}

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
