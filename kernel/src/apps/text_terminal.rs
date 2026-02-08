/// Text-mode fallback terminal.
/// Uses VGA text mode (80x25) when no graphical framebuffer is available.

use alloc::string::ToString;
use crate::apps::shell::{Shell, ShellOutput};
use crate::drivers::input::keyboard::{self, Key};
use crate::drivers::vga_text;

/// VGA text output adapter for the shell
struct VgaShellOutput;

impl ShellOutput for VgaShellOutput {
    fn write_str(&mut self, s: &str) {
        // Use white for output
        vga_text::set_color(vga_text::Color::White, vga_text::Color::Black);
        for byte in s.bytes() {
            vga_text::put_char(byte);
        }
    }

    fn write_line(&mut self, s: &str) {
        self.write_str(s);
        self.write_str("\n");
    }

    fn clear(&mut self) {
        vga_text::clear();
    }
}

pub struct TextTerminal {
    shell: Shell,
}

impl TextTerminal {
    pub fn new() -> Self {
        TextTerminal {
            shell: Shell::new(),
        }
    }

    /// Run the text-mode terminal loop (never returns normally)
    pub fn run(&mut self) -> ! {
        vga_text::clear();
        vga_text::set_color(vga_text::Color::LightCyan, vga_text::Color::Black);
        vga_text::put_str(".anyOS Text Terminal v0.1\n");
        vga_text::set_color(vga_text::Color::LightGray, vga_text::Color::Black);
        vga_text::put_str("Type 'help' for available commands.\n\n");

        // Print initial prompt
        self.print_prompt();

        loop {
            if let Some(event) = keyboard::read_event() {
                if !event.pressed {
                    continue;
                }

                match event.key {
                    Key::Char(c) => {
                        self.shell.insert_char(c);
                        vga_text::set_color(vga_text::Color::White, vga_text::Color::Black);
                        vga_text::put_char(c as u8);
                    }
                    Key::Space => {
                        self.shell.insert_char(' ');
                        vga_text::put_char(b' ');
                    }
                    Key::Backspace => {
                        if self.shell.cursor() > 0 {
                            self.shell.backspace();
                            vga_text::backspace();
                        }
                    }
                    Key::Enter => {
                        let mut out = VgaShellOutput;
                        let should_continue = self.shell.submit(&mut out);
                        if should_continue {
                            self.print_prompt();
                        }
                        // If exit, just continue the loop (nowhere to exit to)
                    }
                    Key::Up => {
                        self.recall_history_up();
                    }
                    Key::Down => {
                        self.recall_history_down();
                    }
                    _ => {}
                }
            } else {
                // No input, wait for interrupt
                unsafe { core::arch::asm!("hlt"); }
            }
        }
    }

    fn print_prompt(&self) {
        vga_text::set_color(vga_text::Color::LightGreen, vga_text::Color::Black);
        vga_text::put_str(Shell::prompt());
        vga_text::set_color(vga_text::Color::White, vga_text::Color::Black);
    }

    fn recall_history_up(&mut self) {
        let old_len = self.shell.input().len();
        self.shell.history_up();
        self.redraw_input(old_len);
    }

    fn recall_history_down(&mut self) {
        let old_len = self.shell.input().len();
        self.shell.history_down();
        self.redraw_input(old_len);
    }

    fn redraw_input(&mut self, old_len: usize) {
        // Erase old input
        for _ in 0..old_len {
            vga_text::backspace();
        }
        // Clear to end of line
        vga_text::clear_to_eol();
        // Write new input
        let input = self.shell.input().to_string();
        for byte in input.bytes() {
            vga_text::put_char(byte);
        }
    }
}
