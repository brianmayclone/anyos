//! Disassembly view using TextEditor with x86asm syntax highlighting.
//!
//! Highlights the current RIP line with a yellow background so the user
//! always sees which instruction will execute next.

use libanyui_client as ui;
use ui::Widget;
use alloc::string::String;
use crate::logic::disasm;
use crate::util::format::{hex64, hex_bytes};

/// Current-RIP highlight color: dark yellow/amber background.
const RIP_HIGHLIGHT: u32 = 0xFF3A3A00;

/// Disassembly view panel.
pub struct DisasmView {
    pub editor: ui::TextEditor,
}

impl DisasmView {
    /// Create the disassembly view with x86asm syntax highlighting.
    pub fn new(_parent: &impl Widget) -> Self {
        let editor = ui::TextEditor::new(800, 600);
        editor.set_dock(ui::DOCK_FILL);
        editor.set_read_only(true);
        editor.load_syntax("syntax/x86asm.syn");
        Self { editor }
    }

    /// Update the disassembly view with decoded instructions around RIP.
    ///
    /// `code` is the raw bytes read from the target's memory at `base_addr`.
    /// `current_rip` is highlighted with a colored background line.
    pub fn update(&self, code: &[u8], base_addr: u64, current_rip: u64) {
        let instrs = disasm::decode_block(code, base_addr, 64);
        let mut text = String::new();
        let mut rip_line: Option<u32> = None;

        for (i, (addr, instr)) in instrs.iter().enumerate() {
            if *addr == current_rip {
                rip_line = Some(i as u32);
            }
            let prefix = if *addr == current_rip { "\u{25B6} " } else { "  " };
            let addr_str = hex64(*addr);
            let bytes_str = hex_bytes(&instr.bytes[..instr.len as usize]);

            // Pad bytes to 24 chars for alignment
            let pad_len = if bytes_str.len() < 24 { 24 - bytes_str.len() } else { 0 };
            let padding: String = core::iter::repeat(' ').take(pad_len).collect();

            text.push_str(prefix);
            text.push_str(&addr_str);
            text.push_str("  ");
            text.push_str(&bytes_str);
            text.push_str(&padding);
            text.push_str("  ");
            text.push_str(instr.mnemonic_str());

            let operands = instr.operands_str();
            if !operands.is_empty() {
                let mnem_pad = if instr.mnemonic_len < 8 { 8 - instr.mnemonic_len as usize } else { 1 };
                let mp: String = core::iter::repeat(' ').take(mnem_pad).collect();
                text.push_str(&mp);
                text.push_str(operands);
            }
            text.push('\n');
        }

        self.editor.set_text(&text);

        // Highlight and scroll to the current RIP line
        self.editor.clear_highlights();
        if let Some(line) = rip_line {
            self.editor.highlight_line(line, RIP_HIGHLIGHT);
            self.editor.ensure_line_visible(line);
        }
    }

    /// Show a message when disassembly is not available.
    pub fn show_message(&self, msg: &str) {
        self.editor.clear_highlights();
        self.editor.set_text(msg);
    }
}
