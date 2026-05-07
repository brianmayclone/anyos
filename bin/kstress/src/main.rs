#![no_std]
#![no_main]

use anyos_std::{fs, ipc, process, sys};
use anyos_std::{println, vec, String, Vec};
use core::sync::atomic::{AtomicU32, Ordering};

anyos_std::entry!(main);

const VERSION: &str = "1.1";
const TEST_DIR: &str = "/tmp/kstress";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Normal,
    Heavy,
    Brutal,
}

#[derive(Clone, Copy)]
struct Config {
    repeat: u32,
    seconds: u32,
    threads: u32,
    mem_mb: u32,
    profile: Profile,
}

impl Config {
    fn default() -> Self {
        Self {
            repeat: 1,
            seconds: 0,
            threads: 16,
            mem_mb: 32,
            profile: Profile::Normal,
        }
    }

    fn profile_name(&self) -> &'static str {
        match self.profile {
            Profile::Normal => "normal",
            Profile::Heavy => "heavy",
            Profile::Brutal => "brutal",
        }
    }

    fn scale(&self) -> u32 {
        match self.profile {
            Profile::Normal => 1,
            Profile::Heavy => 4,
            Profile::Brutal => 10,
        }
    }
}

// ── Global counters for thread tests ───────────────────────────────────────

static THREAD_COUNTER: AtomicU32 = AtomicU32::new(0);
static THREAD_ERRORS: AtomicU32 = AtomicU32::new(0);
static SCHED_YIELD_COUNT: AtomicU32 = AtomicU32::new(0);
static MEM_ALLOC_OK: AtomicU32 = AtomicU32::new(0);
static MEM_ALLOC_FAIL: AtomicU32 = AtomicU32::new(0);

static DEEP_OPS: AtomicU32 = AtomicU32::new(0);
static DEEP_ERRORS: AtomicU32 = AtomicU32::new(0);
static DEEP_ROUNDS: AtomicU32 = AtomicU32::new(0);
static DEEP_VM_ERRORS: AtomicU32 = AtomicU32::new(0);
static DEEP_VFS_ERRORS: AtomicU32 = AtomicU32::new(0);
static DEEP_IPC_ERRORS: AtomicU32 = AtomicU32::new(0);
static DEEP_SCHED_ERRORS: AtomicU32 = AtomicU32::new(0);

// ── Helpers ────────────────────────────────────────────────────────────────

fn elapsed_ms(start: u32) -> u32 {
    sys::uptime_ms().wrapping_sub(start)
}

fn print_header(name: &str) {
    println!();
    println!("--- {} ---", name);
}

fn print_result(name: &str, passed: bool, detail: &str) {
    if passed {
        println!("  [OK]   {}  {}", name, detail);
    } else {
        println!("  [FAIL] {}  {}", name, detail);
    }
}

fn print_metric(name: &str, value: u32, unit: &str) {
    println!("         {} = {} {}", name, value, unit);
}

fn print_usage() {
    println!("kstress - Kernel-Stresstest");
    println!();
    println!("Usage: kstress [options]");
    println!();
    println!("Options:");
    println!("  --repeat N       komplette Testsuite N-mal ausfuehren (default: 1)");
    println!("  --seconds N      mindestens N Sekunden lang wiederholen");
    println!("  --profile P      normal | heavy | brutal (default: normal)");
    println!("  --threads N      Thread-Fanout fuer Deep-Stress (default: 16)");
    println!("  --mem-mb N       Ziel fuer mmap-Speicherdruck in MiB (default: 32)");
    println!("  --help, -h       diese Hilfe anzeigen");
    println!();
    println!("Beispiele:");
    println!("  kstress --repeat 10 --profile heavy --threads 48 --mem-mb 128");
    println!("  kstress --seconds 300 --profile brutal --threads 96 --mem-mb 256");
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val = 0u32;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(val)
}

fn clamp_u32(v: u32, min: u32, max: u32) -> u32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

fn parse_config() -> Option<Config> {
    let mut buf = [0u8; 256];
    let raw = process::args(&mut buf);
    if raw.contains("--help") || raw.contains("-h") {
        print_usage();
        return None;
    }

    let mut cfg = Config::default();
    let args: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--repeat" | "-r" => {
                i += 1;
                if i >= args.len() {
                    println!("kstress: --repeat braucht eine Zahl");
                    return None;
                }
                cfg.repeat = clamp_u32(parse_u32(args[i]).unwrap_or(1), 1, 1_000_000);
            }
            "--seconds" | "-s" => {
                i += 1;
                if i >= args.len() {
                    println!("kstress: --seconds braucht eine Zahl");
                    return None;
                }
                cfg.seconds = parse_u32(args[i]).unwrap_or(0);
            }
            "--threads" | "-t" => {
                i += 1;
                if i >= args.len() {
                    println!("kstress: --threads braucht eine Zahl");
                    return None;
                }
                cfg.threads = clamp_u32(parse_u32(args[i]).unwrap_or(16), 1, 192);
            }
            "--mem-mb" | "-m" => {
                i += 1;
                if i >= args.len() {
                    println!("kstress: --mem-mb braucht eine Zahl");
                    return None;
                }
                cfg.mem_mb = clamp_u32(parse_u32(args[i]).unwrap_or(32), 1, 1024);
            }
            "--profile" | "-p" => {
                i += 1;
                if i >= args.len() {
                    println!("kstress: --profile braucht normal, heavy oder brutal");
                    return None;
                }
                cfg.profile = match args[i] {
                    "normal" => Profile::Normal,
                    "heavy" => Profile::Heavy,
                    "brutal" => Profile::Brutal,
                    _ => {
                        println!("kstress: unbekanntes Profil '{}'", args[i]);
                        return None;
                    }
                };
            }
            other => {
                println!("kstress: unbekannte Option '{}'", other);
                print_usage();
                return None;
            }
        }
        i += 1;
    }

    if cfg.profile == Profile::Heavy {
        cfg.threads = cfg.threads.max(32);
        cfg.mem_mb = cfg.mem_mb.max(96);
    } else if cfg.profile == Profile::Brutal {
        cfg.threads = cfg.threads.max(64);
        cfg.mem_mb = cfg.mem_mb.max(192);
    }

    Some(cfg)
}

fn get_free_pages() -> u32 {
    let mut buf = [0u8; 16];
    sys::sysinfo(0, &mut buf);
    u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]])
}

fn get_total_pages() -> u32 {
    let mut buf = [0u8; 16];
    sys::sysinfo(0, &mut buf);
    u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])
}

fn get_thread_count() -> u32 {
    let mut buf = [0u8; 4];
    sys::sysinfo(1, &mut buf)
}

fn get_cpu_count() -> u32 {
    let mut buf = [0u8; 4];
    sys::sysinfo(2, &mut buf)
}

// ── System info collection ─────────────────────────────────────────────────

fn print_sysinfo() {
    print_header("Systemuebersicht");

    let total = get_total_pages();
    let free = get_free_pages();
    let used = total - free;
    let threads = get_thread_count();
    let cpus = get_cpu_count();

    println!("  CPUs:       {}", cpus);
    println!("  Threads:    {}", threads);
    println!("  RAM total:  {} KB ({} Seiten)", total * 4, total);
    println!("  RAM frei:   {} KB ({} Seiten)", free * 4, free);
    println!("  RAM belegt: {} KB ({} Seiten)", used * 4, used);

    // Uptime
    let up_ms = sys::uptime_ms();
    let up_s = up_ms / 1000;
    println!("  Uptime:     {}s", up_s);

    // Tick rate
    let hz = sys::tick_hz();
    println!("  Timer:      {} Hz", hz);

    // Filesystem
    if let Some(st) = fs::statfs("/") {
        println!(
            "  Disk /:     {} KB total, {} KB frei",
            st.total_bytes / 1024,
            st.free_bytes / 1024
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 1: SCHEDULER
// ════════════════════════════════════════════════════════════════════════════

fn test_scheduler() -> (u32, u32) {
    print_header("Test 1: Scheduler");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // 1a: Thread erstellen und joinen
    {
        THREAD_COUNTER.store(0, Ordering::SeqCst);
        let t0 = sys::uptime_ms();
        let mut threads = Vec::new();
        let count = 16u32;
        for _ in 0..count {
            match process::Thread::spawn(sched_thread_fn, "kstress-sched") {
                Ok(t) => threads.push(t),
                Err(_) => {
                    THREAD_ERRORS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        for t in threads {
            t.join();
        }
        let dt = elapsed_ms(t0);
        let done = THREAD_COUNTER.load(Ordering::SeqCst);
        let errs = THREAD_ERRORS.load(Ordering::SeqCst);
        let ok = done == count && errs == 0;
        print_result(
            "Thread spawn/join (16x)",
            ok,
            if ok { "" } else { "Threads nicht komplett" },
        );
        print_metric("Dauer", dt, "ms");
        print_metric("Abgeschlossen", done, "");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 1b: Yield-Stress — viele schnelle Context-Switches
    {
        SCHED_YIELD_COUNT.store(0, Ordering::SeqCst);
        let t0 = sys::uptime_ms();
        let mut threads = Vec::new();
        for _ in 0..4 {
            match process::Thread::spawn(yield_stress_fn, "kstress-yield") {
                Ok(t) => threads.push(t),
                Err(_) => {}
            }
        }
        for t in threads {
            t.join();
        }
        let dt = elapsed_ms(t0);
        let yields = SCHED_YIELD_COUNT.load(Ordering::SeqCst);
        let ok = yields >= 4000;
        print_result("Yield-Stress (4 Threads, 1000x)", ok, "");
        print_metric("Yields gesamt", yields, "");
        print_metric("Dauer", dt, "ms");
        if dt > 0 {
            print_metric("Yields/ms", yields / dt.max(1), "");
        }
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 1c: Sleep-Genauigkeit
    {
        let t0 = sys::uptime_ms();
        process::sleep(100);
        let dt = elapsed_ms(t0);
        let ok = dt >= 90 && dt <= 200;
        print_result("Sleep 100ms Genauigkeit", ok, "");
        print_metric("Gemessen", dt, "ms (soll: 100)");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 1d: Viele Threads gleichzeitig
    {
        THREAD_COUNTER.store(0, Ordering::SeqCst);
        THREAD_ERRORS.store(0, Ordering::SeqCst);
        let t0 = sys::uptime_ms();
        let count = 64u32;
        let mut threads = Vec::new();
        for _ in 0..count {
            match process::Thread::spawn(sched_thread_fn, "kstress-mass") {
                Ok(t) => threads.push(t),
                Err(_) => {
                    THREAD_ERRORS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        for t in threads {
            t.join();
        }
        let dt = elapsed_ms(t0);
        let done = THREAD_COUNTER.load(Ordering::SeqCst);
        let errs = THREAD_ERRORS.load(Ordering::SeqCst);
        let ok = done + errs == count;
        print_result("Massen-Thread-Test (64x)", ok, "");
        print_metric("Erstellt", done, "");
        print_metric("Fehler", errs, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

fn sched_thread_fn() {
    THREAD_COUNTER.fetch_add(1, Ordering::SeqCst);
}

fn yield_stress_fn() {
    for _ in 0..1000 {
        process::yield_cpu();
        SCHED_YIELD_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 2: MEMORY
// ════════════════════════════════════════════════════════════════════════════

fn test_memory() -> (u32, u32) {
    print_header("Test 2: Speicher (Heap + mmap)");
    let mut pass = 0u32;
    let mut fail = 0u32;

    let free_before = get_free_pages();

    // 2a: Heap-Allokation — viele kleine Vecs
    {
        let t0 = sys::uptime_ms();
        let mut vecs: Vec<Vec<u8>> = Vec::new();
        let count = 1000u32;
        for i in 0..count {
            let mut v = Vec::with_capacity(64);
            for b in 0..64u8 {
                v.push(b.wrapping_add(i as u8));
            }
            vecs.push(v);
        }
        // Verify contents
        let mut errors = 0u32;
        for (i, v) in vecs.iter().enumerate() {
            for (b, val) in v.iter().enumerate() {
                if *val != (b as u8).wrapping_add(i as u8) {
                    errors += 1;
                    break;
                }
            }
        }
        drop(vecs);
        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result(
            "Kleine Alloc (1000x 64B)",
            ok,
            if ok { "" } else { "Datenkorruption!" },
        );
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 2b: Grosse Allokation
    {
        let t0 = sys::uptime_ms();
        let size = 1024 * 1024; // 1 MB
        let mut big = Vec::with_capacity(size);
        for i in 0..size {
            big.push((i & 0xFF) as u8);
        }
        // Verify
        let mut errors = 0u32;
        for (i, &val) in big.iter().enumerate() {
            if val != (i & 0xFF) as u8 {
                errors += 1;
                break;
            }
        }
        drop(big);
        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result(
            "Grosse Alloc (1 MB)",
            ok,
            if ok { "" } else { "Datenkorruption!" },
        );
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 2c: mmap/munmap Stress
    {
        let t0 = sys::uptime_ms();
        let page_size = 4096usize;
        let rounds = 50u32;
        let mut errors = 0u32;
        for r in 0..rounds {
            let ptr = process::mmap(page_size);
            if ptr.is_null() {
                errors += 1;
                continue;
            }
            // Write pattern
            unsafe {
                for i in 0..page_size {
                    *ptr.add(i) = ((r + i as u32) & 0xFF) as u8;
                }
            }
            // Verify
            unsafe {
                for i in 0..page_size {
                    if *ptr.add(i) != ((r + i as u32) & 0xFF) as u8 {
                        errors += 1;
                        break;
                    }
                }
            }
            process::munmap(ptr, page_size);
        }
        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result(
            "mmap/munmap (50 Seiten)",
            ok,
            if ok { "" } else { "Fehler bei mmap" },
        );
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 2d: Fragmentierungs-Stress
    {
        let t0 = sys::uptime_ms();
        let mut ptrs: Vec<*mut u8> = Vec::new();
        let count = 200u32;
        let mut alloc_fail = 0u32;
        // Allocate many
        for _ in 0..count {
            let ptr = process::mmap(4096);
            if ptr.is_null() {
                alloc_fail += 1;
            } else {
                ptrs.push(ptr);
            }
        }
        // Free every other
        let mut freed = 0u32;
        let len = ptrs.len();
        for i in (0..len).rev().step_by(2) {
            process::munmap(ptrs[i], 4096);
            ptrs.remove(i);
            freed += 1;
        }
        // Allocate again in the gaps
        let mut realloc = 0u32;
        for _ in 0..freed {
            let ptr = process::mmap(4096);
            if !ptr.is_null() {
                ptrs.push(ptr);
                realloc += 1;
            }
        }
        // Cleanup
        for ptr in ptrs.iter() {
            process::munmap(*ptr, 4096);
        }
        let dt = elapsed_ms(t0);
        let ok = alloc_fail == 0 && realloc == freed;
        print_result("Fragmentierung (200 Seiten)", ok, "");
        print_metric("Alloc-Fehler", alloc_fail, "");
        print_metric("Realloc", realloc, "von");
        print_metric("Erwartet", freed, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 2e: Memory-Leak-Check
    {
        let free_after = get_free_pages();
        let leaked = if free_before > free_after {
            free_before - free_after
        } else {
            0
        };
        let ok = leaked <= 2; // Toleranz: 2 Seiten fuer Heap-Overhead
        print_result("Speicher-Leak-Check", ok, "");
        print_metric("Vorher frei", free_before, "Seiten");
        print_metric("Nachher frei", free_after, "Seiten");
        print_metric("Differenz", leaked, "Seiten");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 2f: Multi-Thread Heap-Stress
    {
        MEM_ALLOC_OK.store(0, Ordering::SeqCst);
        MEM_ALLOC_FAIL.store(0, Ordering::SeqCst);
        let t0 = sys::uptime_ms();
        let mut threads = Vec::new();
        for _ in 0..8 {
            match process::Thread::spawn(mem_thread_fn, "kstress-mem") {
                Ok(t) => threads.push(t),
                Err(_) => {}
            }
        }
        for t in threads {
            t.join();
        }
        let dt = elapsed_ms(t0);
        let ok_count = MEM_ALLOC_OK.load(Ordering::SeqCst);
        let fail_count = MEM_ALLOC_FAIL.load(Ordering::SeqCst);
        let ok = fail_count == 0;
        print_result(
            "Multi-Thread Heap (8 Threads)",
            ok,
            if ok { "" } else { "Alloc-Fehler unter Last!" },
        );
        print_metric("Erfolgreiche Allocs", ok_count, "");
        print_metric("Fehlgeschlagen", fail_count, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

fn mem_thread_fn() {
    for _ in 0..100 {
        let mut v = Vec::with_capacity(256);
        for i in 0..256 {
            v.push(i as u8);
        }
        // Verify
        let mut ok = true;
        for (i, &val) in v.iter().enumerate() {
            if val != i as u8 {
                ok = false;
                break;
            }
        }
        if ok {
            MEM_ALLOC_OK.fetch_add(1, Ordering::Relaxed);
        } else {
            MEM_ALLOC_FAIL.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 3: VFS / DATEISYSTEM
// ════════════════════════════════════════════════════════════════════════════

fn test_vfs() -> (u32, u32) {
    print_header("Test 3: VFS / Dateisystem");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // Setup
    fs::mkdir("/tmp");
    fs::mkdir(TEST_DIR);

    // 3a: Datei erstellen, schreiben, lesen, vergleichen
    {
        let path = "/tmp/kstress/test_rw.bin";
        let t0 = sys::uptime_ms();

        // Write 64 KB pattern
        let size = 64 * 1024;
        let mut data = Vec::with_capacity(size);
        for i in 0..size {
            data.push(((i * 7 + 13) & 0xFF) as u8);
        }

        let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        let mut write_ok = false;
        if fd != u32::MAX {
            let written = fs::write(fd, &data);
            write_ok = written == size as u32;
            fs::close(fd);
        }

        // Read back
        let mut read_data = vec![0u8; size];
        let fd2 = fs::open(path, 0);
        let mut read_ok = false;
        if fd2 != u32::MAX {
            let n = fs::read(fd2, &mut read_data);
            read_ok = n == size as u32;
            fs::close(fd2);
        }

        // Compare
        let mut match_ok = true;
        if write_ok && read_ok {
            for i in 0..size {
                if data[i] != read_data[i] {
                    match_ok = false;
                    break;
                }
            }
        }

        let dt = elapsed_ms(t0);
        let ok = write_ok && read_ok && match_ok;
        print_result(
            "Datei schreiben/lesen (64 KB)",
            ok,
            if !write_ok {
                "Schreiben fehlgeschlagen"
            } else if !read_ok {
                "Lesen fehlgeschlagen"
            } else if !match_ok {
                "Daten stimmen nicht ueberein!"
            } else {
                ""
            },
        );
        print_metric("Dauer", dt, "ms");
        fs::unlink(path);
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 3b: Viele kleine Dateien
    {
        let t0 = sys::uptime_ms();
        let count = 50u32;
        let mut create_errors = 0u32;
        let mut verify_errors = 0u32;

        for i in 0..count {
            let mut path = String::from("/tmp/kstress/small_");
            push_u32(&mut path, i);
            path.push_str(".txt");

            let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
            if fd == u32::MAX {
                create_errors += 1;
                continue;
            }
            let data = [i as u8; 32];
            fs::write(fd, &data);
            fs::close(fd);
        }

        // Verify and delete
        for i in 0..count {
            let mut path = String::from("/tmp/kstress/small_");
            push_u32(&mut path, i);
            path.push_str(".txt");

            let fd = fs::open(&path, 0);
            if fd == u32::MAX {
                verify_errors += 1;
                continue;
            }
            let mut buf = [0u8; 32];
            let n = fs::read(fd, &mut buf);
            fs::close(fd);
            if n != 32 || buf[0] != i as u8 {
                verify_errors += 1;
            }
            fs::unlink(&path);
        }

        let dt = elapsed_ms(t0);
        let ok = create_errors == 0 && verify_errors == 0;
        print_result("Viele kleine Dateien (50x)", ok, "");
        print_metric("Erstell-Fehler", create_errors, "");
        print_metric("Verif.-Fehler", verify_errors, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 3c: Verzeichnis-Operationen
    {
        let t0 = sys::uptime_ms();
        let mut errors = 0u32;

        // Verschachtelte Verzeichnisse
        fs::mkdir("/tmp/kstress/d1");
        fs::mkdir("/tmp/kstress/d1/d2");
        fs::mkdir("/tmp/kstress/d1/d2/d3");

        // Datei im tiefen Verzeichnis
        let deep = "/tmp/kstress/d1/d2/d3/deep.txt";
        let fd = fs::open(deep, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            errors += 1;
        } else {
            fs::write(fd, b"deep test data");
            fs::close(fd);

            // Lesen
            let fd2 = fs::open(deep, 0);
            if fd2 == u32::MAX {
                errors += 1;
            } else {
                let mut buf = [0u8; 32];
                let n = fs::read(fd2, &mut buf);
                fs::close(fd2);
                if n != 14 {
                    errors += 1;
                }
            }
            fs::unlink(deep);
        }

        // Readdir
        let mut dir_buf = [0u8; 64 * 128];
        let entries = fs::readdir("/tmp/kstress", &mut dir_buf);
        if entries == 0 {
            errors += 1;
        }

        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result("Verzeichnis-Ops (mkdir/readdir)", ok, "");
        print_metric("Readdir-Eintraege", entries, "");
        print_metric("Fehler", errors, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 3d: Seek-Test
    {
        let path = "/tmp/kstress/seek_test.bin";
        let t0 = sys::uptime_ms();
        let mut errors = 0u32;

        // Schreibe 4 KB
        let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            errors += 1;
        } else {
            let mut data = [0u8; 4096];
            for i in 0..4096 {
                data[i] = (i & 0xFF) as u8;
            }
            fs::write(fd, &data);
            fs::close(fd);
        }

        // Lese an verschiedenen Positionen
        let fd2 = fs::open(path, 0);
        if fd2 == u32::MAX {
            errors += 1;
        } else {
            // Seek zur Mitte
            fs::lseek(fd2, 2048, fs::SEEK_SET);
            let mut buf = [0u8; 4];
            let n = fs::read(fd2, &mut buf);
            if n < 4 || buf[0] != 0x00 || buf[1] != 0x01 {
                // offset 2048 = 0x800, Byte 2048 & 0xFF = 0, Byte 2049 & 0xFF = 1
                errors += 1;
            }

            // Seek zum Anfang
            fs::lseek(fd2, 0, fs::SEEK_SET);
            let n2 = fs::read(fd2, &mut buf);
            if n2 < 4 || buf[0] != 0x00 || buf[1] != 0x01 {
                errors += 1;
            }

            fs::close(fd2);
        }

        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result("Seek-Test", ok, "");
        print_metric("Fehler", errors, "");
        print_metric("Dauer", dt, "ms");
        fs::unlink(path);
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 3e: Grosser sequentieller Write
    {
        let path = "/tmp/kstress/bigfile.bin";
        let t0 = sys::uptime_ms();

        let chunk = [0xABu8; 4096];
        let chunks = 256; // 1 MB
        let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        let mut write_ok = false;
        if fd != u32::MAX {
            write_ok = true;
            for _ in 0..chunks {
                let n = fs::write(fd, &chunk);
                if n != 4096 {
                    write_ok = false;
                    break;
                }
            }
            fs::close(fd);
        }

        let dt = elapsed_ms(t0);
        let speed = if dt > 0 {
            (chunks * 4) as u32 * 1000 / dt
        } else {
            0
        };
        let ok = write_ok;
        print_result("Seq. Write 1 MB", ok, "");
        print_metric("Dauer", dt, "ms");
        print_metric("Durchsatz", speed, "KB/s");
        fs::unlink(path);
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // Cleanup
    fs::unlink("/tmp/kstress/d1/d2/d3");
    fs::unlink("/tmp/kstress/d1/d2");
    fs::unlink("/tmp/kstress/d1");

    (pass, fail)
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 4: IPC (Pipes + Event Channels)
// ════════════════════════════════════════════════════════════════════════════

fn test_ipc() -> (u32, u32) {
    print_header("Test 4: IPC (Pipes / Events)");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // 4a: Pipe erstellen, schreiben, lesen
    {
        let t0 = sys::uptime_ms();
        let pipe = ipc::pipe_create("kstress_pipe1");
        let mut errors = 0u32;

        if pipe == 0 {
            errors += 1;
        } else {
            // Schreibe
            let data = b"Hello from kstress pipe test!";
            let written = ipc::pipe_write(pipe, data);
            if written != data.len() as u32 {
                errors += 1;
            }

            // Lese
            let mut buf = [0u8; 64];
            let n = ipc::pipe_read(pipe, &mut buf);
            if n != data.len() as u32 {
                errors += 1;
            } else if &buf[..n as usize] != data {
                errors += 1;
            }

            ipc::pipe_close(pipe);
        }

        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result("Pipe create/write/read", ok, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 4b: Pipe-Durchsatz
    {
        let t0 = sys::uptime_ms();
        let pipe = ipc::pipe_create("kstress_pipe2");
        let mut total_bytes = 0u32;
        let mut errors = 0u32;

        if pipe == 0 {
            errors += 1;
        } else {
            let chunk = [0x42u8; 256];
            let rounds = 500u32;
            for _ in 0..rounds {
                let written = ipc::pipe_write(pipe, &chunk);
                if written != 256 {
                    errors += 1;
                    break;
                }
                let mut buf = [0u8; 256];
                let n = ipc::pipe_read(pipe, &mut buf);
                if n != 256 {
                    errors += 1;
                    break;
                }
                total_bytes += 256;
            }
            ipc::pipe_close(pipe);
        }

        let dt = elapsed_ms(t0);
        let throughput = if dt > 0 {
            total_bytes * 1000 / dt / 1024
        } else {
            0
        };
        let ok = errors == 0 && total_bytes == 500 * 256;
        print_result("Pipe-Durchsatz (500x 256B)", ok, "");
        print_metric("Uebertragen", total_bytes / 1024, "KB");
        print_metric("Dauer", dt, "ms");
        print_metric("Durchsatz", throughput, "KB/s");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 4c: Event Channel
    {
        let t0 = sys::uptime_ms();
        let chan = ipc::evt_chan_create("kstress_evt");
        let mut errors = 0u32;

        if chan == 0 {
            errors += 1;
        } else {
            let sub = ipc::evt_chan_subscribe(chan, 0);
            if sub == 0 {
                errors += 1;
            } else {
                // Emit
                let event = [1u32, 42, 99, 0, 0];
                ipc::evt_chan_emit(chan, &event);

                // Poll
                let mut recv = [0u32; 5];
                if !ipc::evt_chan_poll(chan, sub, &mut recv) {
                    errors += 1;
                } else if recv[0] != 1 || recv[1] != 42 || recv[2] != 99 {
                    errors += 1;
                }

                ipc::evt_chan_unsubscribe(chan, sub);
            }
            ipc::evt_chan_destroy(chan);
        }

        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result("Event Channel (emit/poll)", ok, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 4d: Shared Memory
    {
        let t0 = sys::uptime_ms();
        let shm = ipc::shm_create(4096);
        let mut errors = 0u32;

        if shm == 0 {
            errors += 1;
        } else {
            let addr = ipc::shm_map(shm);
            if addr == 0 {
                errors += 1;
            } else {
                // Write pattern
                let ptr = addr as *mut u8;
                unsafe {
                    for i in 0..4096 {
                        *ptr.add(i) = (i & 0xFF) as u8;
                    }
                }
                // Verify
                unsafe {
                    for i in 0..4096 {
                        if *ptr.add(i) != (i & 0xFF) as u8 {
                            errors += 1;
                            break;
                        }
                    }
                }
                ipc::shm_unmap(shm);
            }
            ipc::shm_destroy(shm);
        }

        let dt = elapsed_ms(t0);
        let ok = errors == 0;
        print_result("Shared Memory (4 KB)", ok, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 5: TIMER / IRQ-STABILITÄT
// ════════════════════════════════════════════════════════════════════════════

fn test_timer() -> (u32, u32) {
    print_header("Test 5: Timer / IRQ-Stabilitaet");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // 5a: uptime_ms Monotonie — darf nie rueckwaerts gehen
    {
        let mut prev = sys::uptime_ms();
        let mut violations = 0u32;
        let samples = 10000u32;
        for _ in 0..samples {
            let now = sys::uptime_ms();
            // wrapping_sub: wenn now < prev (wrap), ist das ok bei 49-Tage-Wrap
            // Aber innerhalb eines kurzen Tests sollte es nie vorkommen
            if now < prev && prev - now < 0x80000000 {
                violations += 1;
            }
            prev = now;
        }
        let ok = violations == 0;
        print_result("uptime_ms Monotonie (10K)", ok, "");
        print_metric("Verletzungen", violations, "");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 5b: Timer-Aufloesung
    {
        let mut deltas: Vec<u32> = Vec::new();
        for _ in 0..100 {
            let t0 = sys::uptime_ms();
            // Busy-wait bis Tick aendert sich
            loop {
                let t1 = sys::uptime_ms();
                if t1 != t0 {
                    deltas.push(t1.wrapping_sub(t0));
                    break;
                }
            }
        }
        let mut min_d = u32::MAX;
        let mut max_d = 0u32;
        let mut sum = 0u32;
        let mut outliers = 0u32;
        for &d in deltas.iter() {
            if d < min_d {
                min_d = d;
            }
            if d > max_d {
                max_d = d;
            }
            if d > 20 {
                outliers += 1;
            }
            sum += d;
        }
        let avg = sum / deltas.len() as u32;
        // This loop measures timer granularity by busy-waiting until uptime_ms()
        // changes. Under kstress/desktop load the measuring thread can be
        // preempted, which shows up as one large delta even when the timer kept
        // ticking normally. Treat rare scheduling pauses as diagnostic outliers;
        // fail only when the clock is consistently coarse or stalls badly.
        let ok = min_d >= 1 && avg <= 5 && outliers <= 3 && max_d <= 100;
        print_result("Timer-Aufloesung", ok, "");
        print_metric("Min", min_d, "ms");
        print_metric("Max", max_d, "ms");
        print_metric("Avg", avg, "ms");
        print_metric("Ausreisser >20ms", outliers, "");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 5c: Sleep-Jitter — wiederholte kurze Sleeps
    {
        let mut jitter_sum = 0u32;
        let rounds = 20u32;
        let target = 50u32; // 50ms
        let mut max_jitter = 0u32;
        for _ in 0..rounds {
            let t0 = sys::uptime_ms();
            process::sleep(target);
            let actual = elapsed_ms(t0);
            let jitter = if actual > target {
                actual - target
            } else {
                target - actual
            };
            jitter_sum += jitter;
            if jitter > max_jitter {
                max_jitter = jitter;
            }
        }
        let avg_jitter = jitter_sum / rounds;
        let ok = avg_jitter <= 20 && max_jitter <= 50;
        print_result("Sleep-Jitter (20x 50ms)", ok, "");
        print_metric("Avg Jitter", avg_jitter, "ms");
        print_metric("Max Jitter", max_jitter, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 5d: Syscall-Latenz
    {
        let t0 = sys::uptime_ms();
        let rounds = 5000u32;
        for _ in 0..rounds {
            let _ = sys::uptime_ms();
        }
        let dt = elapsed_ms(t0);
        let per_call = if rounds > 0 && dt > 0 {
            dt * 1000 / rounds // in Mikrosekunden
        } else {
            0
        };
        let ok = true; // Informativ
        print_result("Syscall-Latenz (uptime_ms 5K)", ok, "");
        print_metric("Gesamt", dt, "ms");
        print_metric("Pro Aufruf", per_call, "us");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 6: PROCESS-MANAGEMENT
// ════════════════════════════════════════════════════════════════════════════

fn test_process() -> (u32, u32) {
    print_header("Test 6: Prozesse");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // 6a: Prozess spawnen und warten
    {
        let t0 = sys::uptime_ms();
        let tid = process::spawn("/System/bin/echo", "kstress-process-test");
        let ok = tid != 0 && tid != u32::MAX;
        if ok {
            let exit_code = process::waitpid(tid);
            let dt = elapsed_ms(t0);
            let ok2 = exit_code == 0;
            print_result("Prozess spawn/wait (echo)", ok2, "");
            print_metric("Exit-Code", exit_code, "");
            print_metric("Dauer", dt, "ms");
            if ok2 {
                pass += 1;
            } else {
                fail += 1;
            }
        } else {
            print_result("Prozess spawn/wait (echo)", false, "spawn fehlgeschlagen");
            fail += 1;
        }
    }

    // 6b: Mehrere Prozesse parallel
    {
        let t0 = sys::uptime_ms();
        let count = 8u32;
        let mut tids = Vec::new();
        let mut spawn_fail = 0u32;
        for _ in 0..count {
            let tid = process::spawn("/System/bin/echo", "parallel-test");
            if tid != 0 && tid != u32::MAX {
                tids.push(tid);
            } else {
                spawn_fail += 1;
            }
        }
        let mut exit_errors = 0u32;
        for &tid in tids.iter() {
            let code = process::waitpid(tid);
            if code != 0 {
                exit_errors += 1;
            }
        }
        let dt = elapsed_ms(t0);
        let ok = spawn_fail == 0 && exit_errors == 0;
        print_result("Parallele Prozesse (8x echo)", ok, "");
        print_metric("Spawn-Fehler", spawn_fail, "");
        print_metric("Exit-Fehler", exit_errors, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 6c: getpid
    {
        let pid = process::getpid();
        let ok = pid > 0;
        print_result("getpid() > 0", ok, "");
        print_metric("PID", pid, "");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 7: KOMBINIERTER STRESS
// ════════════════════════════════════════════════════════════════════════════

static STRESS_OPS: AtomicU32 = AtomicU32::new(0);
static STRESS_ERRORS: AtomicU32 = AtomicU32::new(0);

fn test_combined_stress() -> (u32, u32) {
    print_header("Test 7: Kombinierter Stresstest");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // Starte gleichzeitig: Heap-Stress + File-IO + Yield
    STRESS_OPS.store(0, Ordering::SeqCst);
    STRESS_ERRORS.store(0, Ordering::SeqCst);

    let free_before = get_free_pages();
    let t0 = sys::uptime_ms();

    let mut threads = Vec::new();

    // 4 Heap-Stress Threads
    for _ in 0..4 {
        if let Ok(t) = process::Thread::spawn(stress_heap_fn, "kstress-sh") {
            threads.push(t);
        }
    }
    // 2 File-IO Threads
    for _ in 0..2 {
        if let Ok(t) = process::Thread::spawn(stress_file_fn, "kstress-sf") {
            threads.push(t);
        }
    }
    // 2 Yield Threads
    for _ in 0..2 {
        if let Ok(t) = process::Thread::spawn(stress_yield_fn, "kstress-sy") {
            threads.push(t);
        }
    }

    let spawned = threads.len() as u32;
    for t in threads {
        t.join();
    }

    let dt = elapsed_ms(t0);
    let ops = STRESS_OPS.load(Ordering::SeqCst);
    let errors = STRESS_ERRORS.load(Ordering::SeqCst);
    let free_after = get_free_pages();
    let leaked = if free_before > free_after {
        free_before - free_after
    } else {
        0
    };

    let ok = errors == 0 && spawned == 8;
    print_result(
        "8 Threads (Heap+IO+Yield)",
        ok,
        if errors > 0 { "Fehler unter Last!" } else { "" },
    );
    print_metric("Threads", spawned, "");
    print_metric("Operationen", ops, "");
    print_metric("Fehler", errors, "");
    print_metric("Dauer", dt, "ms");
    print_metric("Ops/s", if dt > 0 { ops * 1000 / dt } else { 0 }, "");
    print_metric("Mem-Leak", leaked, "Seiten");
    if ok {
        pass += 1;
    } else {
        fail += 1;
    }

    (pass, fail)
}

fn stress_heap_fn() {
    for _ in 0..200 {
        let mut v = Vec::with_capacity(512);
        for i in 0..512 {
            v.push((i & 0xFF) as u8);
        }
        let ok = v.len() == 512 && v[0] == 0 && v[255] == 255;
        if !ok {
            STRESS_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        STRESS_OPS.fetch_add(1, Ordering::Relaxed);
    }
}

fn stress_file_fn() {
    let tid = process::getpid();
    let mut path = String::from("/tmp/kstress/stress_");
    push_u32(&mut path, tid);
    path.push_str(".tmp");

    for round in 0..50u32 {
        let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            STRESS_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let data = [round as u8; 128];
        fs::write(fd, &data);
        fs::close(fd);

        let fd2 = fs::open(&path, 0);
        if fd2 == u32::MAX {
            STRESS_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut buf = [0u8; 128];
        let n = fs::read(fd2, &mut buf);
        fs::close(fd2);

        if n != 128 || buf[0] != round as u8 {
            STRESS_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        STRESS_OPS.fetch_add(1, Ordering::Relaxed);
    }
    fs::unlink(&path);
}

fn stress_yield_fn() {
    for _ in 0..500 {
        process::yield_cpu();
        STRESS_OPS.fetch_add(1, Ordering::Relaxed);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 8: DEEP STRESS (VM + Scheduler + VFS + IPC parallel)
// ════════════════════════════════════════════════════════════════════════════

fn test_deep_stress(cfg: Config) -> (u32, u32) {
    print_header("Test 8: Deep Kernel Stress");
    let mut pass = 0u32;
    let mut fail = 0u32;

    fs::mkdir("/tmp");
    fs::mkdir(TEST_DIR);

    // 8a: Thread-Churn in Wellen. Das trifft Scheduler, Wait/Join und
    // Stack-mmap/munmap in schneller Folge.
    {
        THREAD_COUNTER.store(0, Ordering::SeqCst);
        THREAD_ERRORS.store(0, Ordering::SeqCst);
        let t0 = sys::uptime_ms();
        let waves = 2u32.saturating_mul(cfg.scale()).max(1);
        let per_wave = clamp_u32(cfg.threads, 4, 128);
        let mut spawned = 0u32;
        let mut spawn_fail = 0u32;

        for _ in 0..waves {
            let mut threads = Vec::new();
            for _ in 0..per_wave {
                match process::Thread::spawn(sched_thread_fn, "kstress-churn") {
                    Ok(t) => {
                        spawned += 1;
                        threads.push(t);
                    }
                    Err(_) => {
                        spawn_fail += 1;
                    }
                }
            }
            for t in threads {
                t.join();
            }
        }

        let dt = elapsed_ms(t0);
        let done = THREAD_COUNTER.load(Ordering::SeqCst);
        let ok = spawn_fail == 0 && done == spawned && spawned == waves * per_wave;
        print_result("Thread-Churn Wellen", ok, "");
        print_metric("Wellen", waves, "");
        print_metric("Threads", spawned, "");
        print_metric("Spawn-Fehler", spawn_fail, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 8b: Hoher mmap-Speicherdruck. kstress ist absichtlich aggressiv:
    // das angeforderte Ziel wird versucht, und jeder Alloc-/Verify-/Leak-Fehler
    // zaehlt als echter Kernel-Stressfund.
    {
        let free_before = get_free_pages();
        let target_mb = cfg.mem_mb.max(1);
        let chunk_size = 1024 * 1024usize;
        let t0 = sys::uptime_ms();
        let mut ptrs: Vec<*mut u8> = Vec::new();
        let mut alloc_fail = 0u32;
        let mut verify_errors = 0u32;

        for mb in 0..target_mb {
            let ptr = process::mmap(chunk_size);
            if ptr.is_null() {
                alloc_fail += 1;
                break;
            }
            unsafe {
                for off in (0..chunk_size).step_by(4096) {
                    *ptr.add(off) = (mb as u8).wrapping_add((off / 4096) as u8);
                }
                *ptr.add(chunk_size - 1) = (mb as u8) ^ 0xA5;
            }
            ptrs.push(ptr);
        }

        for (mb, &ptr) in ptrs.iter().enumerate() {
            unsafe {
                for off in (0..chunk_size).step_by(4096) {
                    let expected = (mb as u8).wrapping_add((off / 4096) as u8);
                    if *ptr.add(off) != expected {
                        verify_errors += 1;
                        break;
                    }
                }
                if *ptr.add(chunk_size - 1) != ((mb as u8) ^ 0xA5) {
                    verify_errors += 1;
                }
            }
        }

        for &ptr in ptrs.iter().rev() {
            if !process::munmap(ptr, chunk_size) {
                verify_errors += 1;
            }
        }

        let dt = elapsed_ms(t0);
        let free_after = get_free_pages();
        let leaked = if free_before > free_after {
            free_before - free_after
        } else {
            0
        };
        let ok = alloc_fail == 0 && verify_errors == 0 && leaked == 0;
        print_result("Hoher mmap-Druck", ok, "");
        print_metric("Ziel", target_mb, "MiB");
        print_metric("Allokiert", ptrs.len() as u32, "MiB");
        print_metric("Alloc-Fehler", alloc_fail, "");
        print_metric("Verify-Fehler", verify_errors, "");
        print_metric("Mem-Leak", leaked, "Seiten");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 8c: Prozesswellen. Das belastet Loader, Prozesslisten, Waitpid und
    // Parent/Child-Cleanup ohne viel Konsolenausgabe.
    {
        let t0 = sys::uptime_ms();
        let count = clamp_u32(cfg.threads / 2, 4, 48);
        let mut tids = Vec::new();
        let mut spawn_fail = 0u32;
        let mut exit_errors = 0u32;

        for _ in 0..count {
            let tid = process::spawn("/System/bin/true", "");
            if tid != 0 && tid != u32::MAX {
                tids.push(tid);
            } else {
                spawn_fail += 1;
            }
        }
        for &tid in tids.iter() {
            if process::waitpid(tid) != 0 {
                exit_errors += 1;
            }
        }

        let dt = elapsed_ms(t0);
        let ok = spawn_fail == 0 && exit_errors == 0 && tids.len() as u32 == count;
        print_result("Prozess-Welle (/System/bin/true)", ok, "");
        print_metric("Prozesse", tids.len() as u32, "");
        print_metric("Spawn-Fehler", spawn_fail, "");
        print_metric("Exit-Fehler", exit_errors, "");
        print_metric("Dauer", dt, "ms");
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    // 8d: Gemischter Parallelstress. Mehrere Worker-Typen laufen gleichzeitig
    // und reizen VM, VFS, IPC, Timer und Scheduler in derselben Zeitspanne.
    {
        DEEP_OPS.store(0, Ordering::SeqCst);
        DEEP_ERRORS.store(0, Ordering::SeqCst);
        DEEP_ROUNDS.store(60u32.saturating_mul(cfg.scale()), Ordering::SeqCst);
        DEEP_VM_ERRORS.store(0, Ordering::SeqCst);
        DEEP_VFS_ERRORS.store(0, Ordering::SeqCst);
        DEEP_IPC_ERRORS.store(0, Ordering::SeqCst);
        DEEP_SCHED_ERRORS.store(0, Ordering::SeqCst);

        let t0 = sys::uptime_ms();
        let mut threads = Vec::new();
        let count = clamp_u32(cfg.threads, 4, 128);
        let mut spawn_fail = 0u32;
        for i in 0..count {
            let entry = match i % 4 {
                0 => deep_vm_worker,
                1 => deep_vfs_worker,
                2 => deep_ipc_worker,
                _ => deep_sched_worker,
            };
            match process::Thread::spawn(entry, "kstress-deep") {
                Ok(t) => threads.push(t),
                Err(_) => spawn_fail += 1,
            }
        }
        for t in threads {
            t.join();
        }

        let dt = elapsed_ms(t0);
        let ops = DEEP_OPS.load(Ordering::SeqCst);
        let errors = DEEP_ERRORS.load(Ordering::SeqCst);
        let ok = spawn_fail == 0 && errors == 0 && ops > 0;
        print_result("Gemischter Parallelstress", ok, "");
        print_metric("Threads", count.saturating_sub(spawn_fail), "");
        print_metric("Spawn-Fehler", spawn_fail, "");
        print_metric("Operationen", ops, "");
        print_metric("Fehler", errors, "");
        print_metric("VM-Fehler", DEEP_VM_ERRORS.load(Ordering::SeqCst), "");
        print_metric("VFS-Fehler", DEEP_VFS_ERRORS.load(Ordering::SeqCst), "");
        print_metric("IPC-Fehler", DEEP_IPC_ERRORS.load(Ordering::SeqCst), "");
        print_metric("Sched-Fehler", DEEP_SCHED_ERRORS.load(Ordering::SeqCst), "");
        print_metric("Dauer", dt, "ms");
        print_metric(
            "Ops/s",
            if dt > 0 {
                ops.saturating_mul(1000) / dt
            } else {
                0
            },
            "",
        );
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }
    }

    (pass, fail)
}

fn deep_vm_worker() {
    let rounds = DEEP_ROUNDS.load(Ordering::Relaxed);
    for r in 0..rounds {
        let size = 16 * 4096usize;
        let ptr = process::mmap(size);
        if ptr.is_null() {
            DEEP_VM_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        unsafe {
            for off in (0..size).step_by(4096) {
                *ptr.add(off) = (r as u8).wrapping_add((off / 4096) as u8);
            }
            for off in (0..size).step_by(4096) {
                let expected = (r as u8).wrapping_add((off / 4096) as u8);
                if *ptr.add(off) != expected {
                    DEEP_VM_ERRORS.fetch_add(1, Ordering::Relaxed);
                    DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
                    break;
                }
            }
        }
        if !process::munmap(ptr, size) {
            DEEP_VM_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }

        let mut v = Vec::with_capacity(4096);
        for i in 0..4096 {
            v.push((i as u8).wrapping_add(r as u8));
        }
        if v[0] != (r as u8) || v[4095] != (255u8).wrapping_add(r as u8) {
            DEEP_VM_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        DEEP_OPS.fetch_add(2, Ordering::Relaxed);
    }
}

fn deep_vfs_worker() {
    let tid = process::getpid();
    let mut path = String::from("/tmp/kstress/deep_vfs_");
    push_u32(&mut path, tid);
    path.push_str(".bin");

    let rounds = DEEP_ROUNDS.load(Ordering::Relaxed);
    for r in 0..rounds {
        let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            DEEP_VFS_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut data = [0u8; 512];
        for i in 0..data.len() {
            data[i] = (r as u8).wrapping_add(i as u8);
        }
        if fs::write(fd, &data) != data.len() as u32 {
            DEEP_VFS_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        fs::close(fd);

        let fd2 = fs::open(&path, 0);
        if fd2 == u32::MAX {
            DEEP_VFS_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut buf = [0u8; 512];
        let n = fs::read(fd2, &mut buf);
        fs::close(fd2);
        if n != buf.len() as u32 || buf[0] != r as u8 || buf[511] != (r as u8).wrapping_add(255) {
            DEEP_VFS_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        DEEP_OPS.fetch_add(1, Ordering::Relaxed);
    }
    fs::unlink(&path);
}

fn deep_ipc_worker() {
    let tid = process::getpid();
    let mut name = String::from("ks_ipc_");
    push_u32(&mut name, tid);

    let rounds = DEEP_ROUNDS.load(Ordering::Relaxed);
    for r in 0..rounds {
        let pipe = ipc::pipe_create(&name);
        if pipe == 0 {
            DEEP_IPC_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut data = [0u8; 128];
        for i in 0..data.len() {
            data[i] = (r as u8) ^ (i as u8);
        }
        if ipc::pipe_write(pipe, &data) != data.len() as u32 {
            DEEP_IPC_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        let mut buf = [0u8; 128];
        let n = ipc::pipe_read(pipe, &mut buf);
        if n != buf.len() as u32 || buf[0] != (r as u8) || buf[127] != ((r as u8) ^ 127) {
            DEEP_IPC_ERRORS.fetch_add(1, Ordering::Relaxed);
            DEEP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        ipc::pipe_close(pipe);
        DEEP_OPS.fetch_add(1, Ordering::Relaxed);
    }
}

fn deep_sched_worker() {
    let rounds = DEEP_ROUNDS.load(Ordering::Relaxed);
    for i in 0..rounds.saturating_mul(20) {
        let _ = sys::uptime_ms();
        if i % 8 == 0 {
            process::yield_cpu();
        }
        if i % 257 == 0 {
            process::sleep_us(50);
        }
        DEEP_OPS.fetch_add(1, Ordering::Relaxed);
    }
}

// ── Utility ────────────────────────────────────────────────────────────────

fn push_u32(s: &mut String, val: u32) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut v = val;
    while v > 0 {
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        s.push(buf[i] as char);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// MAIN
// ════════════════════════════════════════════════════════════════════════════

fn main() {
    let cfg = match parse_config() {
        Some(cfg) => cfg,
        None => return,
    };

    println!();
    println!("========================================");
    println!(" kstress v{} — Kernel-Stresstest", VERSION);
    println!("========================================");
    println!(" Profil:     {}", cfg.profile_name());
    println!(" Repeat:     {}", cfg.repeat);
    if cfg.seconds > 0 {
        println!(" Mindestzeit: {}s", cfg.seconds);
    }
    println!(" Deep Threads: {}", cfg.threads);
    println!(" Memory-Ziel:  {} MiB", cfg.mem_mb);

    print_sysinfo();

    let mut total_pass = 0u32;
    let mut total_fail = 0u32;
    let mut passes_done = 0u32;

    let t_global = sys::uptime_ms();

    loop {
        passes_done += 1;
        println!();
        println!("========================================");
        println!(" DURCHLAUF {}", passes_done);
        println!("========================================");

        let (p, f) = test_scheduler();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_memory();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_vfs();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_ipc();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_timer();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_process();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_combined_stress();
        total_pass += p;
        total_fail += f;

        let (p, f) = test_deep_stress(cfg);
        total_pass += p;
        total_fail += f;

        let elapsed_s = elapsed_ms(t_global) / 1000;
        let repeat_done = passes_done >= cfg.repeat;
        let time_done = cfg.seconds == 0 || elapsed_s >= cfg.seconds;
        if repeat_done && time_done {
            break;
        }
        println!();
        println!(
            "Weiter: Durchlauf {}, elapsed {}s",
            passes_done + 1,
            elapsed_s
        );
    }

    let total_time = elapsed_ms(t_global);

    println!();
    println!("========================================");
    println!(" ERGEBNIS");
    println!("========================================");
    println!();
    println!("  Bestanden: {}", total_pass);
    println!("  Fehlgeschlagen: {}", total_fail);
    println!("  Gesamt: {}", total_pass + total_fail);
    println!("  Durchlaeufe: {}", passes_done);
    println!(
        "  Dauer: {}.{} s",
        total_time / 1000,
        (total_time % 1000) / 100
    );
    println!();

    if total_fail == 0 {
        println!("  Alle Tests bestanden.");
    } else {
        println!("  {} FEHLER GEFUNDEN!", total_fail);
    }
    println!();
}
