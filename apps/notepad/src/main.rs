//! Notepad — Simple text editor for anyOS using libanyui.
//!
//! Layout: Toolbar (DOCK_TOP) | TextEditor (DOCK_FILL) | StatusBar (DOCK_BOTTOM)

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec;
use libanyui_client as anyui;
use anyui::IconType;

anyos_std::entry!(main);

// ════════════════════════════════════════════════════════════════
//  Global application state
// ════════════════════════════════════════════════════════════════

struct AppState {
    editor: anyui::TextEditor,
    status_file: anyui::Label,
    status_cursor: anyui::Label,
    status_encoding: anyui::Label,
    file_path: String,
    modified: bool,
    win: anyui::Window,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("app not initialized") }
}

// ════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════

fn main() {
    if !anyui::init() {
        anyos_std::println!("notepad: failed to load libanyui.so");
        return;
    }

    // ── Parse command-line args ──
    let mut args_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut args_buf);
    let arg_path = raw_args.trim();

    // Determine initial file path and content
    let (initial_path, initial_content) = if !arg_path.is_empty() {
        let content = read_file(arg_path);
        (String::from(arg_path), content)
    } else {
        (String::new(), None)
    };

    let title = make_title(&initial_path, false);

    // ── Create window ──
    let win = anyui::Window::new(&title, -1, -1, 700, 500);

    // ── Toolbar (DOCK_TOP) ──
    let tc = anyui::theme::colors();
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(700, 42);
    toolbar.set_color(tc.sidebar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_system_icon("file-plus", IconType::Outline, tc.text, 24);
    btn_new.set_tooltip("New");

    let btn_open = toolbar.add_icon_button("");
    btn_open.set_size(34, 34);
    btn_open.set_system_icon("folder-open", IconType::Outline, tc.text, 24);
    btn_open.set_tooltip("Open");

    let btn_save = toolbar.add_icon_button("");
    btn_save.set_size(34, 34);
    btn_save.set_system_icon("device-floppy", IconType::Outline, tc.text, 24);
    btn_save.set_tooltip("Save");

    let btn_save_as = toolbar.add_icon_button("");
    btn_save_as.set_size(34, 34);
    btn_save_as.set_system_icon("file-export", IconType::Outline, tc.text, 24);
    btn_save_as.set_tooltip("Save As");

    // Separator
    let sep = toolbar.add_icon_button("");
    sep.set_size(2, 28);
    sep.set_color(0xFF555555);

    let btn_cut = toolbar.add_icon_button("");
    btn_cut.set_size(34, 34);
    btn_cut.set_system_icon("cut", IconType::Outline, tc.text, 24);
    btn_cut.set_tooltip("Cut (Ctrl+X)");

    let btn_copy = toolbar.add_icon_button("");
    btn_copy.set_size(34, 34);
    btn_copy.set_system_icon("copy", IconType::Outline, tc.text, 24);
    btn_copy.set_tooltip("Copy (Ctrl+C)");

    let btn_paste = toolbar.add_icon_button("");
    btn_paste.set_size(34, 34);
    btn_paste.set_system_icon("clipboard", IconType::Outline, tc.text, 24);
    btn_paste.set_tooltip("Paste (Ctrl+V)");

    win.add(&toolbar);

    // ── Status bar (DOCK_BOTTOM) ──
    let status_panel = anyui::View::new();
    status_panel.set_color(0xFF007ACC);
    status_panel.set_size(700, 22);
    status_panel.set_dock(anyui::DOCK_BOTTOM);

    let status_file = anyui::Label::new(&display_filename(&initial_path));
    status_file.set_position(8, 3);
    status_file.set_font_size(11);
    status_file.set_text_color(0xFFFFFFFF);
    status_panel.add(&status_file);

    let status_cursor = anyui::Label::new("Ln 1, Col 1");
    status_cursor.set_position(350, 3);
    status_cursor.set_font_size(11);
    status_cursor.set_text_color(0xFFFFFFFF);
    status_panel.add(&status_cursor);

    let status_encoding = anyui::Label::new("UTF-8");
    status_encoding.set_position(550, 3);
    status_encoding.set_font_size(11);
    status_encoding.set_text_color(0xFFFFFFFF);
    status_panel.add(&status_encoding);

    win.add(&status_panel);

    // ── TextEditor (DOCK_FILL) ──
    let editor = anyui::TextEditor::new(700, 400);
    editor.set_dock(anyui::DOCK_FILL);
    editor.set_editor_font(0, 13);
    editor.set_tab_width(4);
    editor.set_show_line_numbers(true);

    if let Some(ref data) = initial_content {
        editor.set_text_bytes(data);
    }

    win.add(&editor);

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            editor,
            status_file,
            status_cursor,
            status_encoding,
            file_path: initial_path,
            modified: false,
            win,
        });
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring
    // ════════════════════════════════════════════════════════════════

    // ── New ──
    btn_new.on_click(|_| {
        let s = app();
        s.editor.set_text_bytes(b"");
        s.file_path = String::new();
        s.modified = false;
        update_title(s);
        s.status_file.set_text("Untitled");
    });

    // ── Open ──
    btn_open.on_click(|_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            let s = app();
            if let Some(data) = read_file(&path) {
                s.editor.set_text_bytes(&data);
            } else {
                s.editor.set_text_bytes(b"");
            }
            s.file_path = path;
            s.modified = false;
            update_title(s);
            s.status_file.set_text(&display_filename(&s.file_path));
        }
    });

    // ── Save ──
    btn_save.on_click(|_| {
        save_current();
    });

    // ── Save As ──
    btn_save_as.on_click(|_| {
        save_as();
    });

    // ── Clipboard toolbar buttons ──
    btn_copy.on_click(|_| {
        app().editor.copy();
    });

    btn_cut.on_click(|_| {
        if app().editor.cut() {
            let s = app();
            if !s.modified {
                s.modified = true;
                update_title(s);
            }
        }
    });

    btn_paste.on_click(|_| {
        if app().editor.paste() > 0 {
            let s = app();
            if !s.modified {
                s.modified = true;
                update_title(s);
            }
        }
    });

    // ── Keyboard shortcuts ──
    // Note: Ctrl+C/V/X/A are handled internally by the TextEditor widget.
    // The window on_key_down only receives keys NOT consumed by the focused control.
    app().win.on_key_down(|ke| {
        if ke.ctrl() {
            match ke.char_code {
                0x73 => save_current(),                   // Ctrl+S
                0x6F => {                                 // Ctrl+O
                    if let Some(path) = anyui::FileDialog::open_file() {
                        let s = app();
                        if let Some(data) = read_file(&path) {
                            s.editor.set_text_bytes(&data);
                        } else {
                            s.editor.set_text_bytes(b"");
                        }
                        s.file_path = path;
                        s.modified = false;
                        update_title(s);
                        s.status_file.set_text(&display_filename(&s.file_path));
                    }
                }
                0x6E => {                                 // Ctrl+N
                    let s = app();
                    s.editor.set_text_bytes(b"");
                    s.file_path = String::new();
                    s.modified = false;
                    update_title(s);
                    s.status_file.set_text("Untitled");
                }
                _ => {}
            }
        }
    });

    // ── Text change tracking ──
    app().editor.on_text_changed(|_| {
        let s = app();
        if !s.modified {
            s.modified = true;
            update_title(s);
        }
    });

    // ── Cursor position timer (500ms) ──
    anyui::set_timer(500, || {
        let s = app();
        let (row, col) = s.editor.cursor();
        let text = format!("Ln {}, Col {}", row + 1, col + 1);
        s.status_cursor.set_text(&text);
    });

    // ════════════════════════════════════════════════════════════════
    //  Run event loop
    // ════════════════════════════════════════════════════════════════

    anyui::run();
}

// ════════════════════════════════════════════════════════════════
//  Helper functions
// ════════════════════════════════════════════════════════════════

fn save_current() {
    let s = app();
    if s.file_path.is_empty() {
        // No path yet — do Save As
        save_as();
        return;
    }
    let mut buf = vec![0u8; 128 * 1024];
    let len = s.editor.get_text(&mut buf);
    if write_file(&s.file_path, &buf[..len as usize]) {
        s.modified = false;
        update_title(s);
    }
}

fn save_as() {
    let default = if app().file_path.is_empty() {
        "untitled.txt"
    } else {
        basename(&app().file_path)
    };
    if let Some(path) = anyui::FileDialog::save_file(default) {
        let s = app();
        let mut buf = vec![0u8; 128 * 1024];
        let len = s.editor.get_text(&mut buf);
        if write_file(&path, &buf[..len as usize]) {
            s.file_path = path;
            s.modified = false;
            update_title(s);
            s.status_file.set_text(&display_filename(&s.file_path));
        }
    }
}

fn update_title(s: &AppState) {
    let title = make_title(&s.file_path, s.modified);
    s.win.set_title(&title);
}

fn make_title(path: &str, modified: bool) -> String {
    let name = if path.is_empty() {
        "Untitled"
    } else {
        basename(path)
    };
    let mut t = String::new();
    if modified {
        t.push_str("* ");
    }
    t.push_str(name);
    t.push_str(" - Notepad");
    t
}

fn display_filename(path: &str) -> String {
    if path.is_empty() {
        String::from("Untitled")
    } else {
        String::from(basename(path))
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn read_file(path: &str) -> Option<alloc::vec::Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut content = alloc::vec::Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

fn write_file(path: &str, data: &[u8]) -> bool {
    use anyos_std::fs;
    fs::truncate(path);
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return false;
    }
    fs::write(fd, data);
    fs::close(fd);
    true
}
