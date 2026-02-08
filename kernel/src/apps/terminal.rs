/// Graphical terminal emulator.
/// Renders a character grid into a window surface with scrollback.

use alloc::string::String;
use alloc::vec::Vec;
use crate::apps::shell::{Shell, ShellOutput};
use crate::drivers::keyboard::{Key, KeyEvent};
use crate::graphics::color::Color;
use crate::graphics::surface::Surface;

const CELL_W: u32 = 8;
const CELL_H: u32 = 16;
const MAX_SCROLLBACK: usize = 500;

const FG_DEFAULT: Color = Color::new(204, 204, 204);
const FG_PROMPT: Color = Color::new(100, 255, 100);
const BG_DEFAULT: Color = Color::new(30, 30, 40);

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: FG_DEFAULT, bg: BG_DEFAULT }
    }
}

pub struct TerminalBuffer {
    lines: Vec<Vec<Cell>>,
    cols: usize,
    visible_rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_offset: usize,
    current_fg: Color,
}

impl TerminalBuffer {
    fn new(cols: usize, visible_rows: usize) -> Self {
        let mut lines = Vec::new();
        lines.push(Vec::new());
        TerminalBuffer {
            lines,
            cols,
            visible_rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            current_fg: FG_DEFAULT,
        }
    }

    fn ensure_line(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    fn write_char(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.cursor_row += 1;
                self.cursor_col = 0;
                self.ensure_line(self.cursor_row);
                // Auto-scroll to keep cursor visible
                if self.cursor_row >= self.scroll_offset + self.visible_rows {
                    self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                }
                // Trim scrollback
                if self.lines.len() > MAX_SCROLLBACK {
                    let excess = self.lines.len() - MAX_SCROLLBACK;
                    self.lines.drain(0..excess);
                    if self.cursor_row >= excess {
                        self.cursor_row -= excess;
                    }
                    if self.scroll_offset >= excess {
                        self.scroll_offset -= excess;
                    } else {
                        self.scroll_offset = 0;
                    }
                }
            }
            '\r' => {
                self.cursor_col = 0;
            }
            _ => {
                self.ensure_line(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                while line.len() <= self.cursor_col {
                    line.push(Cell::default());
                }
                line[self.cursor_col] = Cell {
                    ch,
                    fg: self.current_fg,
                    bg: BG_DEFAULT,
                };
                self.cursor_col += 1;
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                    self.ensure_line(self.cursor_row);
                    if self.cursor_row >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                    }
                }
            }
        }
    }

    fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            if self.cursor_row < self.lines.len() {
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col < line.len() {
                    line.remove(self.cursor_col);
                }
            }
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
    }
}

/// Adapter to use TerminalBuffer as ShellOutput
struct TerminalOutputAdapter<'a> {
    buf: &'a mut TerminalBuffer,
}

impl<'a> ShellOutput for TerminalOutputAdapter<'a> {
    fn write_str(&mut self, s: &str) {
        self.buf.current_fg = FG_DEFAULT;
        self.buf.write_str(s);
    }
    fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Graphical terminal - owns a TerminalBuffer and Shell
pub struct GraphicalTerminal {
    buffer: TerminalBuffer,
    shell: Shell,
    dirty: bool,
    cols: usize,
    rows: usize,
}

impl GraphicalTerminal {
    pub fn new(width: u32, height: u32) -> Self {
        let cols = (width / CELL_W) as usize;
        let rows = (height / CELL_H) as usize;

        let mut term = GraphicalTerminal {
            buffer: TerminalBuffer::new(cols, rows),
            shell: Shell::new(),
            dirty: true,
            cols,
            rows,
        };

        // Print welcome message
        term.buffer.current_fg = Color::new(0, 200, 255);
        term.buffer.write_str(".anyOS Terminal v0.1\n");
        term.buffer.current_fg = Color::new(150, 150, 150);
        term.buffer.write_str("Type 'help' for available commands.\n\n");

        // Print initial prompt
        term.buffer.current_fg = FG_PROMPT;
        term.buffer.write_str(Shell::prompt());
        term.buffer.current_fg = FG_DEFAULT;

        term
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> bool {
        if !event.pressed {
            return true;
        }

        match event.key {
            Key::Char(c) => {
                self.shell.insert_char(c);
                self.buffer.write_char(c);
                self.dirty = true;
            }
            Key::Space => {
                self.shell.insert_char(' ');
                self.buffer.write_char(' ');
                self.dirty = true;
            }
            Key::Backspace => {
                if self.shell.cursor() > 0 {
                    self.shell.backspace();
                    self.buffer.backspace();
                    self.dirty = true;
                }
            }
            Key::Enter => {
                // Execute command
                let mut adapter = TerminalOutputAdapter { buf: &mut self.buffer };
                let should_continue = self.shell.submit(&mut adapter);
                if should_continue {
                    // Print new prompt
                    self.buffer.current_fg = FG_PROMPT;
                    self.buffer.write_str(Shell::prompt());
                    self.buffer.current_fg = FG_DEFAULT;
                } else {
                    return false;
                }
                self.dirty = true;
            }
            Key::Up => {
                self.recall_history_up();
                self.dirty = true;
            }
            Key::Down => {
                self.recall_history_down();
                self.dirty = true;
            }
            Key::Home => {
                self.shell.home();
            }
            Key::End => {
                self.shell.end();
            }
            _ => {}
        }
        true
    }

    fn recall_history_up(&mut self) {
        let old_len = self.shell.input().len();
        self.shell.history_up();
        // Clear old input from display and write new
        self.erase_input(old_len);
        self.buffer.write_str(self.shell.input());
    }

    fn recall_history_down(&mut self) {
        let old_len = self.shell.input().len();
        self.shell.history_down();
        self.erase_input(old_len);
        self.buffer.write_str(self.shell.input());
    }

    fn erase_input(&mut self, old_len: usize) {
        // Move cursor back to prompt end and clear
        for _ in 0..old_len {
            self.buffer.backspace();
        }
        // Clear remaining chars on line
        let row = self.buffer.cursor_row;
        let col = self.buffer.cursor_col;
        if row < self.buffer.lines.len() {
            self.buffer.lines[row].truncate(col);
        }
    }

    /// Render the terminal content into a surface
    pub fn render(&mut self, surface: &mut Surface) {
        if !self.dirty {
            return;
        }

        // Clear background
        for pixel in surface.pixels.iter_mut() {
            *pixel = BG_DEFAULT.to_u32();
        }

        let start_row = self.buffer.scroll_offset;
        let end_row = (start_row + self.rows).min(self.buffer.lines.len());

        for (screen_y, line_idx) in (start_row..end_row).enumerate() {
            let line = &self.buffer.lines[line_idx];
            for (col, cell) in line.iter().enumerate() {
                if col >= self.cols {
                    break;
                }
                let px = (col as u32) * CELL_W;
                let py = (screen_y as u32) * CELL_H;
                // Draw character using bitmap font (terminal needs fixed-width grid)
                crate::graphics::font::draw_char_bitmap(surface, px as i32, py as i32, cell.ch, cell.fg);
            }
        }

        // Draw cursor (block cursor)
        let cursor_screen_row = self.buffer.cursor_row as i32 - self.buffer.scroll_offset as i32;
        if cursor_screen_row >= 0 && (cursor_screen_row as usize) < self.rows {
            let cx = (self.buffer.cursor_col as u32) * CELL_W;
            let cy = (cursor_screen_row as u32) * CELL_H;
            // Draw a filled block cursor
            for dy in 0..CELL_H {
                for dx in 0..CELL_W {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px < surface.width && py < surface.height {
                        let idx = (py * surface.width + px) as usize;
                        // Invert colors for cursor
                        let existing = surface.pixels[idx];
                        surface.pixels[idx] = existing ^ 0x00FFFFFF;
                    }
                }
            }
        }

        self.dirty = false;
    }
}
