#![no_std]
#![no_main]

anyos_std::entry!(main);

// ANSI escape codes
const CYAN: &[u8] = b"\x1b[36m";
const BOLD_CYAN: &[u8] = b"\x1b[1;36m";
const RESET: &[u8] = b"\x1b[0m";

/// ASCII art logo for .anyOS — 18 lines, each padded to 34 chars visible width.
const LOGO: [&[u8]; 18] = [
    b"                                  ",
    b"             .oOOOOo.             ",
    b"          .oO'      'Oo.          ",
    b"        oO'            'Oo        ",
    b"      oO'                'Oo      ",
    b"     oO'                  'Oo     ",
    b"    oO'    .oOOo.          'Oo    ",
    b"    Oo    oO    Oo   .anyOS Oo    ",
    b"    Oo    Oo    oO          Oo    ",
    b"    Oo     'oOOo'           Oo    ",
    b"     Oo                    oO     ",
    b"      Oo.                .oO      ",
    b"        'Oo.          .oO'        ",
    b"          'Oo..    ..oO'          ",
    b"             'oOOOOo'             ",
    b"                                  ",
    b"                                  ",
    b"                                  ",
];

const LOGO_PAD: &[u8] = b"                                  ";
const MAX_INFO_LINES: usize = 18;

/// A pre-formatted info line stored as bytes (may include ANSI escapes).
struct InfoLine {
    buf: [u8; 160],
    len: usize,
}

impl InfoLine {
    fn new() -> Self {
        InfoLine { buf: [0u8; 160], len: 0 }
    }

    fn push(&mut self, data: &[u8]) {
        for &b in data {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn push_u32(&mut self, n: u32) {
        if n == 0 {
            self.push(b"0");
            return;
        }
        let mut digits = [0u8; 10];
        let mut val = n;
        let mut dlen = 0;
        while val > 0 {
            digits[dlen] = b'0' + (val % 10) as u8;
            val /= 10;
            dlen += 1;
        }
        for i in (0..dlen).rev() {
            self.push(&[digits[i]]);
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// Extract basename from path (e.g., "/System/bin/ash" -> "ash")
fn basename(path: &[u8]) -> &[u8] {
    let mut last_slash = 0;
    for i in 0..path.len() {
        if path[i] == b'/' {
            last_slash = i + 1;
        }
    }
    if last_slash < path.len() {
        &path[last_slash..]
    } else {
        path
    }
}

/// Get an environment variable as a byte slice.
fn get_env<'a>(key: &str, buf: &'a mut [u8]) -> &'a [u8] {
    let len = anyos_std::env::get(key, buf);
    if len == u32::MAX || len == 0 {
        return b"";
    }
    let end = (len as usize).min(buf.len());
    &buf[..end]
}

fn main() {
    // ── Gather system info ──────────────────────────

    // User
    let mut user_buf = [0u8; 64];
    let user = get_env("USER", &mut user_buf);

    let hostname = b".anyOS";

    // Shell
    let mut shell_buf = [0u8; 64];
    let shell_raw = get_env("SHELL", &mut shell_buf);
    let shell = basename(shell_raw);

    // Terminal
    let mut term_buf = [0u8; 64];
    let term = get_env("TERM", &mut term_buf);

    // Uptime
    let ticks = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz();
    let total_secs = if hz > 0 { ticks / hz } else { 0 };
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;

    // Memory (cmd=0): [total_frames:u32, free_frames:u32, heap_used:u32, heap_total:u32]
    let mut mem_buf = [0u8; 16];
    let mut total_mb: u32 = 0;
    let mut used_mb: u32 = 0;
    let mut mem_percent: u32 = 0;
    if anyos_std::sys::sysinfo(0, &mut mem_buf) == 0 {
        let total_frames = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free_frames = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        total_mb = total_frames * 4 / 1024;
        used_mb = (total_frames - free_frames) * 4 / 1024;
        if total_mb > 0 {
            mem_percent = used_mb * 100 / total_mb;
        }
    }

    // CPU count (cmd=2)
    let mut cpu_buf = [0u8; 4];
    let mut cpus: u32 = 0;
    if anyos_std::sys::sysinfo(2, &mut cpu_buf) == 0 {
        cpus = u32::from_le_bytes([cpu_buf[0], cpu_buf[1], cpu_buf[2], cpu_buf[3]]);
    }

    // Process/thread count (cmd=1)
    let mut thread_buf = [0u8; 36 * 64];
    let thread_ret = anyos_std::sys::sysinfo(1, &mut thread_buf);
    let threads = if thread_ret != u32::MAX { thread_ret } else { 0 };

    // Resolution
    let resolutions = anyos_std::ui::window::list_resolutions();
    let (res_w, res_h) = if !resolutions.is_empty() {
        resolutions[0]
    } else {
        (0, 0)
    };

    // GPU name
    let gpu = anyos_std::ui::window::gpu_name();

    // Local IP
    let mut net_buf = [0u8; 24];
    let has_net = anyos_std::net::get_config(&mut net_buf) == 0;
    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];

    // ── Build info lines ────────────────────────────

    let mut lines: [InfoLine; MAX_INFO_LINES] = core::array::from_fn(|_| InfoLine::new());
    let mut lc: usize = 0;

    // Title: user@hostname
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN);
        l.push(user);
        l.push(RESET);
        l.push(b"@");
        l.push(BOLD_CYAN);
        l.push(hostname);
        l.push(RESET);
        lc += 1;
    }

    // Underline
    {
        let l = &mut lines[lc];
        let title_len = user.len() + 1 + hostname.len();
        for _ in 0..title_len {
            l.push(b"-");
        }
        lc += 1;
    }

    // OS
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"OS"); l.push(RESET);
        l.push(b": .anyOS ");
        l.push(env!("ANYOS_VERSION").as_bytes());
        l.push(b" x86_64");
        lc += 1;
    }

    // Host
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Host"); l.push(RESET);
        l.push(b": anyOS Virtual Machine");
        lc += 1;
    }

    // Kernel
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Kernel"); l.push(RESET);
        l.push(b": anyOS x86_64");
        lc += 1;
    }

    // Uptime
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Uptime"); l.push(RESET);
        l.push(b": ");
        if days > 0 {
            l.push_u32(days);
            if days == 1 { l.push(b" day, "); } else { l.push(b" days, "); }
        }
        if hours > 0 || days > 0 {
            l.push_u32(hours);
            if hours == 1 { l.push(b" hour, "); } else { l.push(b" hours, "); }
        }
        l.push_u32(mins);
        if mins == 1 { l.push(b" min"); } else { l.push(b" mins"); }
        lc += 1;
    }

    // Shell
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Shell"); l.push(RESET);
        l.push(b": ");
        if shell.is_empty() { l.push(b"unknown"); } else { l.push(shell); }
        lc += 1;
    }

    // Resolution
    if res_w > 0 && res_h > 0 {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Resolution"); l.push(RESET);
        l.push(b": ");
        l.push_u32(res_w);
        l.push(b"x");
        l.push_u32(res_h);
        lc += 1;
    }

    // WM
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"WM"); l.push(RESET);
        l.push(b": anyOS Compositor");
        lc += 1;
    }

    // Terminal
    if !term.is_empty() {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Terminal"); l.push(RESET);
        l.push(b": ");
        l.push(term);
        lc += 1;
    }

    // CPU
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"CPU"); l.push(RESET);
        l.push(b": ");
        l.push_u32(cpus);
        l.push(b" x x86_64 (");
        l.push_u32(threads);
        l.push(b" threads)");
        lc += 1;
    }

    // GPU
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"GPU"); l.push(RESET);
        l.push(b": ");
        l.push(gpu.as_bytes());
        lc += 1;
    }

    // Memory
    {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Memory"); l.push(RESET);
        l.push(b": ");
        l.push_u32(used_mb);
        l.push(b" MiB / ");
        l.push_u32(total_mb);
        l.push(b" MiB (");
        l.push_u32(mem_percent);
        l.push(b"%)");
        lc += 1;
    }

    // Local IP
    if has_net && (ip[0] != 0 || ip[1] != 0 || ip[2] != 0 || ip[3] != 0) {
        let l = &mut lines[lc];
        l.push(BOLD_CYAN); l.push(b"Local IP"); l.push(RESET);
        l.push(b": ");
        l.push_u32(ip[0] as u32); l.push(b".");
        l.push_u32(ip[1] as u32); l.push(b".");
        l.push_u32(ip[2] as u32); l.push(b".");
        l.push_u32(ip[3] as u32);
        lc += 1;
    }

    // Empty line before color blocks
    lc += 1;

    // Color blocks row 1 (colors 0-7)
    {
        let l = &mut lines[lc];
        for c in 0u8..8 {
            l.push(b"\x1b[4");
            l.push(&[b'0' + c]);
            l.push(b"m   ");
        }
        l.push(RESET);
        lc += 1;
    }

    // Color blocks row 2 (colors 8-15)
    {
        let l = &mut lines[lc];
        for c in 8u8..16 {
            l.push(b"\x1b[48;5;");
            if c >= 10 {
                l.push(&[b'1', b'0' + c - 10]);
            } else {
                l.push(&[b'0' + c]);
            }
            l.push(b"m   ");
        }
        l.push(RESET);
        lc += 1;
    }

    // ── Render: logo + info side by side ────────────

    let total_lines = if lc > LOGO.len() { lc } else { LOGO.len() };

    for i in 0..total_lines {
        // Logo (cyan)
        if i < LOGO.len() {
            anyos_std::fs::write(1, CYAN);
            anyos_std::fs::write(1, LOGO[i]);
            anyos_std::fs::write(1, RESET);
        } else {
            anyos_std::fs::write(1, LOGO_PAD);
        }

        // Gap between logo and info
        anyos_std::fs::write(1, b"   ");

        // Info line
        if i < lc {
            anyos_std::fs::write(1, lines[i].as_bytes());
        }

        anyos_std::fs::write(1, b"\n");
    }
}
