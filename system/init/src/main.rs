#![no_std]
#![no_main]

use anyos_std::process;
use anyos_std::sys;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::println;

anyos_std::entry!(main);

// ─── CPU Benchmark ──────────────────────────────────────────────────────────

/// Simple integer benchmark: counts how many iterations of a mixed
/// arithmetic workload complete in the given number of PIT ticks.
fn benchmark_cpu(duration_ticks: u32) -> u32 {
    let start = sys::uptime();
    let mut iterations: u32 = 0;
    let mut acc: u32 = 0x12345678;

    while sys::uptime().wrapping_sub(start) < duration_ticks {
        // Mixed integer operations (add, xor, shift, multiply)
        for _ in 0..1000 {
            acc = acc.wrapping_mul(1103515245).wrapping_add(12345);
            acc ^= acc >> 16;
            acc = acc.wrapping_add(acc << 5);
        }
        iterations += 1;
    }

    // Prevent optimizer from eliminating the loop
    if acc == 0 { iterations += 1; }
    iterations
}

/// Memory bandwidth benchmark: read/write 64 KiB buffer repeatedly.
fn benchmark_memory(duration_ticks: u32) -> u32 {
    let start = sys::uptime();
    let mut iterations: u32 = 0;
    let mut buf = [0u32; 4096]; // 16 KiB

    while sys::uptime().wrapping_sub(start) < duration_ticks {
        // Write pass
        for i in 0..buf.len() {
            buf[i] = (i as u32).wrapping_mul(0xDEADBEEF);
        }
        // Read + accumulate pass
        let mut sum: u32 = 0;
        for i in 0..buf.len() {
            sum = sum.wrapping_add(buf[i]);
        }
        if sum == 0 { buf[0] = 1; } // prevent optimization
        iterations += 1;
    }

    iterations
}

// ─── Init Config Parser ─────────────────────────────────────────────────────

/// Read /System/etc/init/init.conf and spawn each program listed (one path per line).
/// Lines starting with '#' are comments. Empty lines are skipped.
fn run_init_conf() {
    // Read config file
    let fd = fs::open("/System/etc/init/init.conf", 0); // read-only
    if fd == u32::MAX {
        println!("init: /System/etc/init/init.conf not found, skipping");
        return;
    }

    let mut buf = [0u8; 1024];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 {
        println!("init: /System/etc/init/init.conf is empty");
        return;
    }

    // Parse line by line
    let data = &buf[..n];
    let mut line_start = 0;
    for i in 0..=n {
        let at_end = i == n;
        let is_newline = !at_end && data[i] == b'\n';

        if is_newline || at_end {
            let line_end = if !at_end && i > 0 && data[i.saturating_sub(1)] == b'\r' {
                i - 1
            } else {
                i
            };
            let line = &data[line_start..line_end];
            line_start = i + 1;

            // Skip empty lines and comments
            if line.is_empty() || line[0] == b'#' {
                continue;
            }

            // Trim leading whitespace
            let trimmed = trim_bytes(line);
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(entry) = core::str::from_utf8(trimmed) {
                // Suffix '&' means background (don't wait)
                let (path, background) = if entry.ends_with('&') {
                    (entry[..entry.len() - 1].trim_end(), true)
                } else {
                    (entry, false)
                };
                println!("init: spawning '{}'{}", path, if background { " [bg]" } else { "" });
                let tid = process::spawn(path, "");
                if tid == u32::MAX {
                    println!("init: FAILED to spawn '{}'", path);
                } else if !background {
                    let code = process::waitpid(tid);
                    println!("init: '{}' exited (code={})", path, code);
                }
            }
        }
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < b.len() && (b[start] == b' ' || b[start] == b'\t') {
        start += 1;
    }
    let mut end = b.len();
    while end > start && (b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    &b[start..end]
}

// ─── Formatting ─────────────────────────────────────────────────────────────

fn fmt_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut v = val;
    let mut tmp = [0u8; 10];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {

    let hz = sys::tick_hz();

    // ── Phase 1: CPU Benchmark (2 seconds) ──
    println!("Running CPU benchmark (2s)...");
    let cpu_score = benchmark_cpu(hz * 2);

    // ── Phase 2: Memory Benchmark (1 second) ──
    println!("Running memory benchmark (1s)...");
    let mem_score = benchmark_memory(hz);

    // ── Report results ──

    let mut line = [0u8; 80];
    let mut p: usize;

    // CPU score
    p = 0;
    let s = b"  CPU score : ";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    p += fmt_u32(&mut line[p..], cpu_score);
    let s = b" Kops/2s";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    if let Ok(s) = core::str::from_utf8(&line[..p]) { println!("{}", s); }

    // Memory score
    p = 0;
    let s = b"  Mem score : ";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    p += fmt_u32(&mut line[p..], mem_score);
    let s = b" passes/1s (16K buf)";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    if let Ok(s) = core::str::from_utf8(&line[..p]) { println!("{}", s); }

    // Store results in a named pipe for other programs to read
    let pipe_id = ipc::pipe_create("sys:startup_info");
    if pipe_id > 0 {
        let mut info = [0u8; 128];
        let mut ip = 0;
        let s = b"cpu_score=";
        info[ip..ip + s.len()].copy_from_slice(s); ip += s.len();
        ip += fmt_u32(&mut info[ip..], cpu_score);
        info[ip] = b'\n'; ip += 1;
        let s = b"mem_score=";
        info[ip..ip + s.len()].copy_from_slice(s); ip += s.len();
        ip += fmt_u32(&mut info[ip..], mem_score);
        info[ip] = b'\n'; ip += 1;
        ipc::pipe_write(pipe_id, &info[..ip]);
    }

    // ── Signal boot ready ──
    sys::boot_ready();

    // ── Phase 3: Run init config ──
    run_init_conf();
}


