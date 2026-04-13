#![no_std]
#![no_main]

anyos_std::entry!(main);

const PAGE_LINES: u32 = 24;

/// Open the terminal keyboard pipe (TERM_KBD_PIPE env var).
/// Returns 0 if not available (no interactive paging).
fn open_kbd_pipe() -> u32 {
    let mut buf = [0u8; 64];
    let len = anyos_std::env::get("TERM_KBD_PIPE", &mut buf);
    if len == 0 || len == u32::MAX { return 0; }
    let name = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    if name.is_empty() { return 0; }
    anyos_std::ipc::pipe_open(name)
}

/// Print a prompt and wait for a keypress on the given keyboard pipe.
/// Returns false if the user pressed 'q' / ESC / Ctrl+C to quit.
fn prompt_continue(kbd_pipe: u32) -> bool {
    anyos_std::print!("--More-- (Space/Enter=next page, q=quit) ");
    let mut key_buf = [0u8; 4];
    loop {
        let n = anyos_std::ipc::pipe_read(kbd_pipe, &mut key_buf);
        if n == u32::MAX {
            // Pipe closed/destroyed — stop
            anyos_std::print!("\r\x1B[K");
            return false;
        }
        if n == 0 {
            // No data yet — yield CPU and retry
            anyos_std::process::sleep(5);
            continue;
        }
        let ch = key_buf[0];
        anyos_std::print!("\r\x1B[K"); // clear the prompt line
        match ch {
            b'q' | b'Q' | 0x1B | 3 => return false, // q, ESC, Ctrl+C
            b'\r' | b'\n' | b' ' => return true,
            _ => {}
        }
    }
}

/// Page through a readable fd.
/// `kbd_pipe`: non-zero means interactive paging is available.
/// Returns false if the user quit early.
fn page_fd(fd: u32, kbd_pipe: u32) -> bool {
    let mut line_count: u32 = 0;
    let mut buf = [0u8; 512];

    loop {
        let n = anyos_std::fs::read(fd, &mut buf) as usize;
        if n == 0 || n == u32::MAX as usize { break; }

        let mut i = 0;
        while i < n {
            let b = buf[i];
            anyos_std::print!("{}", b as char);
            i += 1;
            if b == b'\n' {
                line_count += 1;
                if kbd_pipe != 0 && line_count >= PAGE_LINES {
                    line_count = 0;
                    if !prompt_continue(kbd_pipe) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("more - Display file contents one screen at a time\n\nUsage: more [FILE...]\n\nKeys:\n  Space / Enter  Next page\n  q / ESC        Quit");
        return;
    }

    // Try to get keyboard pipe (set by the terminal for interactive paging).
    let kbd_pipe = open_kbd_pipe();

    if args.pos_count == 0 {
        // Pipeline mode: read from stdin, page if keyboard pipe available.
        page_fd(0, kbd_pipe);
    } else {
        for i in 0..args.pos_count {
            let path = args.positional[i as usize];
            let fd = anyos_std::fs::open(path, 0);
            if fd == u32::MAX {
                anyos_std::println!("more: cannot open '{}'", path);
                continue;
            }
            if args.pos_count > 1 {
                anyos_std::println!("==> {} <==", path);
            }
            let cont = page_fd(fd, kbd_pipe);
            anyos_std::fs::close(fd);
            if !cont { break; }
        }
    }

    if kbd_pipe != 0 {
        anyos_std::ipc::pipe_close(kbd_pipe);
    }
}
