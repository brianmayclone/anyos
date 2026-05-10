#![no_std]
#![no_main]

use alloc::format;
use anyos_std::{fs, println, process, sys, String, Vec};

anyos_std::entry!(main);

const VERSION: &str = "0.1";
const DEFAULT_DIR: &str = "/tmp/vfsstress";
const MAX_BLOCK: usize = 256 * 1024;
const STACK_WRITE_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Quick,
    Normal,
    Heavy,
}

struct Config {
    dir: String,
    repeat: u32,
    total_bytes: u32,
    profile: Profile,
    keep: bool,
    sync_each: bool,
}

impl Config {
    fn default() -> Self {
        Self {
            dir: String::from(DEFAULT_DIR),
            repeat: 3,
            total_bytes: 8 * 1024 * 1024,
            profile: Profile::Normal,
            keep: false,
            sync_each: false,
        }
    }

    fn profile_name(&self) -> &'static str {
        match self.profile {
            Profile::Quick => "quick",
            Profile::Normal => "normal",
            Profile::Heavy => "heavy",
        }
    }
}

struct Summary {
    tests: u32,
    failures: u32,
    warnings: u32,
}

impl Summary {
    fn new() -> Self {
        Self {
            tests: 0,
            failures: 0,
            warnings: 0,
        }
    }

    fn ok(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        println!("  [OK]   {:<22} {}", name, detail);
    }

    fn fail(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.failures += 1;
        println!("  [FAIL] {:<22} {}", name, detail);
    }
}

struct CaseResult {
    block_size: usize,
    total_bytes: u32,
    write_ms: u32,
    read_ms: u32,
    ok: bool,
    phase: &'static str,
    first_bad: Option<u32>,
    path: String,
}

fn main() {
    let Some(mut cfg) = parse_config() else {
        return;
    };
    apply_profile(&mut cfg);

    println!();
    println!("vfsstress {} - VFS Diagnose", VERSION);
    println!("============================");
    println!("  profile:   {}", cfg.profile_name());
    println!("  repeat:    {}", cfg.repeat);
    println!("  total:     {} KB", cfg.total_bytes / 1024);
    println!("  dir:       {}", cfg.dir);
    println!("  sync-each: {}", if cfg.sync_each { "yes" } else { "no" });
    println!();

    let started = sys::uptime_ms();
    let mut summary = Summary::new();
    if prepare_dir(&cfg.dir) {
        summary.ok("work-dir", &format!("{} writable", cfg.dir));
    } else {
        summary.fail("work-dir", &format!("{} nicht beschreibbar", cfg.dir));
        print_summary(&summary, started);
        return;
    }

    let blocks = [4096usize, 16 * 1024, 64 * 1024, 256 * 1024];
    let mut results = Vec::new();

    println!();
    println!("--- Schreib-/Readback-Runden ---");
    for round in 1..=cfg.repeat {
        for &block in &blocks {
            let seed = round.wrapping_mul(131).wrapping_add(block as u32);
            let path = format!("{}/r{}-b{}.bin", cfg.dir, round, block);
            let result = run_case(&cfg, &path, block, seed);
            print_case(round, &result);
            if result.ok {
                summary.ok("case", &format!("round={} block={} ok", round, block));
            } else {
                summary.fail(
                    "case",
                    &format!(
                        "round={} block={} first_bad={}",
                        round,
                        block,
                        result.first_bad.unwrap_or(u32::MAX)
                    ),
                );
            }
            results.push(result);
        }
    }

    run_full_suite(&cfg, &mut summary);

    print_protocol(&results);
    if !cfg.keep {
        for r in &results {
            let _ = fs::unlink(&r.path);
        }
    } else {
        println!();
        println!("Artefakte bleiben erhalten in {}", cfg.dir);
    }
    print_summary(&summary, started);
}

fn parse_config() -> Option<Config> {
    let mut buf = [0u8; 512];
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
                    println!("vfsstress: --repeat braucht eine Zahl");
                    return None;
                }
                cfg.repeat = clamp(parse_u32(args[i]).unwrap_or(cfg.repeat), 1, 1000);
            }
            "--total-kb" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --total-kb braucht eine Zahl");
                    return None;
                }
                let kb = clamp(
                    parse_u32(args[i]).unwrap_or(cfg.total_bytes / 1024),
                    64,
                    1024 * 1024,
                );
                cfg.total_bytes = kb.saturating_mul(1024);
            }
            "--dir" | "-d" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --dir braucht einen Pfad");
                    return None;
                }
                cfg.dir = String::from(args[i]);
            }
            "--profile" | "-p" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --profile braucht quick|normal|heavy");
                    return None;
                }
                cfg.profile = match args[i] {
                    "quick" => Profile::Quick,
                    "normal" => Profile::Normal,
                    "heavy" => Profile::Heavy,
                    other => {
                        println!("vfsstress: unbekanntes Profil '{}'", other);
                        return None;
                    }
                };
            }
            "--sync-each" => cfg.sync_each = true,
            "--keep" => cfg.keep = true,
            other => {
                println!("vfsstress: unbekannte Option '{}'", other);
                return None;
            }
        }
        i += 1;
    }
    Some(cfg)
}

fn apply_profile(cfg: &mut Config) {
    match cfg.profile {
        Profile::Quick => {
            if cfg.repeat == Config::default().repeat {
                cfg.repeat = 1;
            }
            if cfg.total_bytes == Config::default().total_bytes {
                cfg.total_bytes = 1024 * 1024;
            }
        }
        Profile::Normal => {}
        Profile::Heavy => {
            if cfg.repeat == Config::default().repeat {
                cfg.repeat = 10;
            }
            if cfg.total_bytes == Config::default().total_bytes {
                cfg.total_bytes = 32 * 1024 * 1024;
            }
        }
    }
}

fn print_usage() {
    println!("vfsstress - VFS Schreib-/Readback-Stresstest");
    println!();
    println!("Usage: vfsstress [options]");
    println!("  --profile P       quick | normal | heavy (default: normal)");
    println!("  --repeat N        Wiederholungen pro Blockgroesse");
    println!("  --total-kb N      Dateigroesse pro Testfall in KB");
    println!("  --dir PATH        Testverzeichnis (default: /tmp/vfsstress)");
    println!("  --sync-each       fsync nach jedem Write-Chunk");
    println!("  --keep            Testdateien behalten");
    println!("  --help, -h        diese Hilfe anzeigen");
}

fn run_full_suite(cfg: &Config, summary: &mut Summary) {
    println!();
    println!("--- Volltest-Suite ---");

    run_named(summary, "small io", small_io_case(cfg));
    run_named(summary, "overwrite in-block", overwrite_in_block_case(cfg));
    run_named(
        summary,
        "overwrite block-edge",
        overwrite_block_edge_case(cfg),
    );
    run_named(
        summary,
        "overwrite full-block",
        overwrite_full_block_case(cfg),
    );
    run_named(
        summary,
        "overwrite multi-block",
        overwrite_multi_block_case(cfg),
    );
    run_named(summary, "seek overwrite", seek_overwrite_case(cfg));
    run_named(summary, "append truncate", append_truncate_case(cfg));
    run_named(summary, "metadata rename", metadata_rename_case(cfg));
    run_named(summary, "directory churn", directory_churn_case(cfg));
    run_named(summary, "fd offsets", fd_offsets_case(cfg));
    run_named(summary, "stack write 64k", stack_write_64k_case(cfg));

    fs::sync();
}

fn run_named(summary: &mut Summary, name: &str, result: Result<String, &'static str>) {
    match result {
        Ok(detail) => summary.ok(name, &detail),
        Err(err) => summary.fail(name, err),
    }
}

fn small_io_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-small-io.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC | fs::O_SYNC);
    if fd == u32::MAX {
        return Err("open-write");
    }

    let total = 32 * 1024u32;
    let seed = 0x5151_0101;
    let mut pos = 0u32;
    let sizes = [1usize, 2, 3, 5, 7, 11, 17, 31, 64, 127, 251];
    let mut buf = [0u8; 251];
    while pos < total {
        let want = sizes[(pos as usize / 17) % sizes.len()].min((total - pos) as usize);
        fill_pattern(&mut buf[..want], pos, seed);
        if fs::write(fd, &buf[..want]) != want as u32 {
            fs::close(fd);
            return Err("write");
        }
        pos += want as u32;
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);

    verify_file_pattern(&path, total, seed, &sizes)?;
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!("{} bytes kleine Writes/Reads ok", total))
}

fn overwrite_in_block_case(cfg: &Config) -> Result<String, &'static str> {
    overwrite_probe_case(
        cfg,
        "overwrite-in-block",
        16 * 1024,
        &[(1024, 257)],
        &[4096],
    )
}

fn overwrite_block_edge_case(cfg: &Config) -> Result<String, &'static str> {
    overwrite_probe_case(
        cfg,
        "overwrite-block-edge",
        16 * 1024,
        &[(4093, 64)],
        &[4096],
    )
}

fn overwrite_full_block_case(cfg: &Config) -> Result<String, &'static str> {
    overwrite_probe_case(
        cfg,
        "overwrite-full-block",
        16 * 1024,
        &[(4096, 4096)],
        &[4096],
    )
}

fn overwrite_multi_block_case(cfg: &Config) -> Result<String, &'static str> {
    overwrite_probe_case(
        cfg,
        "overwrite-multi-block",
        96 * 1024,
        &[(4093, 9000), (48 * 1024, 17 * 1024)],
        &[4096, 16 * 1024],
    )
}

fn overwrite_probe_case(
    cfg: &Config,
    name: &str,
    total: u32,
    patches: &[(u32, usize)],
    read_sizes: &[usize],
) -> Result<String, &'static str> {
    let path = format!("{}/full-{}.bin", cfg.dir, name);
    let _ = fs::unlink(&path);
    let base_seed = 0x2A2A_0001;
    let patch_seed = 0x3B3B_1001;
    write_pattern_file(&path, total, 4096, base_seed)?;

    let fd = fs::open(&path, fs::O_WRITE);
    if fd == u32::MAX {
        return Err("open-overwrite");
    }
    let mut buf = Vec::new();
    buf.resize(MAX_BLOCK.min(32 * 1024), 0);
    for &(off, len) in patches {
        if off.saturating_add(len as u32) > total {
            fs::close(fd);
            return Err("bad-patch");
        }
        if len > buf.len() {
            fs::close(fd);
            return Err("patch-too-large");
        }
        if fs::lseek(fd, off as i32, fs::SEEK_SET) != off {
            fs::close(fd);
            return Err("seek");
        }
        fill_pattern(&mut buf[..len], off, patch_seed);
        if fs::write(fd, &buf[..len]) != len as u32 {
            fs::close(fd);
            return Err("overwrite");
        }
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);

    if stat_size(&path)? != total {
        return Err("stat-size");
    }
    verify_seek_overwrite_with_reads(&path, total, base_seed, patch_seed, patches, read_sizes)?;
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!(
        "{} bytes, {} Overwrite-Patches + fsync ok",
        total,
        patches.len()
    ))
}

fn seek_overwrite_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-seek-overwrite.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let total = 96 * 1024u32;
    let base_seed = 0x2222_0001;
    let patch_seed = 0x3333_1001;
    write_pattern_file(&path, total, 4096, base_seed)?;

    let fd = fs::open(&path, fs::O_WRITE);
    if fd == u32::MAX {
        return Err("open-overwrite");
    }
    let patches = [
        (0u32, 513usize),
        (4093, 3000),
        (32768, 8192),
        (total - 777, 777),
    ];
    let mut buf = [0u8; 8192];
    for &(off, len) in &patches {
        if fs::lseek(fd, off as i32, fs::SEEK_SET) != off {
            fs::close(fd);
            return Err("seek");
        }
        fill_pattern(&mut buf[..len], off, patch_seed);
        if fs::write(fd, &buf[..len]) != len as u32 {
            fs::close(fd);
            return Err("overwrite");
        }
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);

    let mut stat = [0u32; 7];
    if fs::stat(&path, &mut stat) != 0 || stat[1] != total {
        return Err("stat-size");
    }
    verify_seek_overwrite(&path, total, base_seed, patch_seed, &patches)?;
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!("{} bytes mit {} Patches ok", total, patches.len()))
}

fn append_truncate_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-append-truncate.bin", cfg.dir);
    let _ = fs::unlink(&path);
    write_pattern_file(&path, 4096, 512, 0x4444_0001)?;

    let fd = fs::open(&path, fs::O_WRITE | fs::O_APPEND);
    if fd == u32::MAX {
        return Err("open-append");
    }
    let mut buf = [0u8; 2048];
    fill_pattern(&mut buf, 4096, 0x4444_0001);
    if fs::write(fd, &buf) != buf.len() as u32 {
        fs::close(fd);
        return Err("append-write");
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("append-fsync");
    }
    fs::close(fd);

    let size = stat_size(&path)?;
    if size != 6144 {
        return Err("append-size");
    }
    verify_file_pattern(&path, 6144, 0x4444_0001, &[4096, 2048])?;

    if fs::truncate(&path) != 0 {
        return Err("truncate");
    }
    if stat_size(&path)? != 0 {
        return Err("truncate-size");
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok("append und truncate ok".into())
}

fn metadata_rename_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-meta", cfg.dir);
    let _ = fs::mkdir(&dir);
    let a = format!("{}/alpha.txt", dir);
    let b = format!("{}/beta.txt", dir);
    let _ = fs::unlink(&a);
    let _ = fs::unlink(&b);

    write_bytes_file(&a, b"metadata-check")?;
    if stat_size(&a)? != 14 {
        return Err("stat-before");
    }
    if fs::rename(&a, &b) != 0 {
        return Err("rename");
    }
    let mut stat = [0u32; 7];
    if fs::stat(&a, &mut stat) == 0 {
        return Err("old-still-exists");
    }
    if stat_size(&b)? != 14 {
        return Err("stat-after");
    }
    let data = read_exact_file(&b, 14)?;
    if data.as_slice() != b"metadata-check" {
        return Err("read-after-rename");
    }
    if !dir_contains(&dir, "beta.txt") {
        return Err("readdir");
    }
    if !cfg.keep {
        let _ = fs::unlink(&b);
    }
    Ok("stat/readdir/rename ok".into())
}

fn directory_churn_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-churn", cfg.dir);
    let _ = fs::mkdir(&dir);
    for i in 0..32u32 {
        let p = format!("{}/f{:02}.dat", dir, i);
        let q = format!("{}/g{:02}.dat", dir, i);
        let _ = fs::unlink(&p);
        let _ = fs::unlink(&q);
    }

    for i in 0..32u32 {
        let p = format!("{}/f{:02}.dat", dir, i);
        let body = [(i & 0xff) as u8, ((i * 3) & 0xff) as u8, 0x5a];
        write_bytes_file(&p, &body)?;
    }
    for i in (0..32u32).step_by(2) {
        let p = format!("{}/f{:02}.dat", dir, i);
        let q = format!("{}/g{:02}.dat", dir, i);
        if fs::rename(&p, &q) != 0 {
            return Err("rename");
        }
    }
    let mut found = 0u32;
    for i in 0..32u32 {
        let name = if i % 2 == 0 {
            format!("g{:02}.dat", i)
        } else {
            format!("f{:02}.dat", i)
        };
        if dir_contains(&dir, &name) {
            found += 1;
        }
    }
    if found != 32 {
        return Err("readdir-count");
    }
    if !cfg.keep {
        for i in 0..32u32 {
            let p = if i % 2 == 0 {
                format!("{}/g{:02}.dat", dir, i)
            } else {
                format!("{}/f{:02}.dat", dir, i)
            };
            let _ = fs::unlink(&p);
        }
    }
    Ok("32 create/rename/readdir ok".into())
}

fn fd_offsets_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-fd-offsets.bin", cfg.dir);
    let _ = fs::unlink(&path);
    write_pattern_file(&path, 8192, 1024, 0x7777_0001)?;

    let fd1 = fs::open(&path, 0);
    let fd2 = fs::open(&path, 0);
    if fd1 == u32::MAX || fd2 == u32::MAX {
        if fd1 != u32::MAX {
            fs::close(fd1);
        }
        if fd2 != u32::MAX {
            fs::close(fd2);
        }
        return Err("open");
    }
    let mut a = [0u8; 37];
    let mut b = [0u8; 37];
    if fs::read(fd1, &mut a) != a.len() as u32 || fs::read(fd2, &mut b) != b.len() as u32 {
        fs::close(fd1);
        fs::close(fd2);
        return Err("read");
    }
    if a != b {
        fs::close(fd1);
        fs::close(fd2);
        return Err("independent-offset");
    }
    if fs::lseek(fd1, 4096, fs::SEEK_SET) != 4096 {
        fs::close(fd1);
        fs::close(fd2);
        return Err("seek");
    }
    let mut c = [0u8; 64];
    if fs::read(fd1, &mut c) != c.len() as u32 {
        fs::close(fd1);
        fs::close(fd2);
        return Err("read-seek");
    }
    if verify_pattern(&c, 4096, 0x7777_0001).is_some() {
        fs::close(fd1);
        fs::close(fd2);
        return Err("verify-seek");
    }
    fs::close(fd1);
    fs::close(fd2);
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok("zwei FDs mit eigenen Offsets ok".into())
}

fn stack_write_64k_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-stack-write-64k.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let total = cfg.total_bytes.max(STACK_WRITE_SIZE as u32);
    let seed = 0x6464_0001;

    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }

    let mut stack_buf = [0u8; STACK_WRITE_SIZE];
    let mut offset = 0u32;
    while offset < total {
        let len = (total - offset).min(STACK_WRITE_SIZE as u32) as usize;
        fill_pattern(&mut stack_buf[..len], offset, seed);
        let n = fs::write(fd, &stack_buf[..len]);
        if n != len as u32 {
            fs::close(fd);
            return Err("write");
        }
        offset += len as u32;
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);

    verify_file_pattern(&path, total, seed, &[STACK_WRITE_SIZE])?;
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!("{} bytes via direkte 64K Stack-Slices ok", total))
}

fn run_case(cfg: &Config, path: &str, block_size: usize, seed: u32) -> CaseResult {
    let _ = fs::unlink(path);
    let mut write_buf = Vec::new();
    write_buf.resize(block_size.min(MAX_BLOCK), 0);
    let mut read_buf = Vec::new();
    read_buf.resize(block_size.min(MAX_BLOCK), 0);

    let write_start = sys::uptime_ms();
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return failed_case(
            path,
            block_size,
            cfg.total_bytes,
            0,
            0,
            "open-write",
            Some(0),
        );
    }

    let mut offset = 0u32;
    let mut fail_phase = "write";
    while offset < cfg.total_bytes {
        let len = (cfg.total_bytes - offset).min(block_size as u32) as usize;
        fill_pattern(&mut write_buf[..len], offset, seed);
        let n = fs::write(fd, &write_buf[..len]);
        if n != len as u32 {
            fail_phase = "write";
            break;
        }
        if cfg.sync_each && !fs::fsync(fd as i32) {
            fail_phase = "fsync-each";
            break;
        }
        offset += len as u32;
    }
    let final_sync_ok = fs::fsync(fd as i32);
    fs::close(fd);
    let write_ms = elapsed_ms(write_start);

    if offset < cfg.total_bytes {
        return failed_case(
            path,
            block_size,
            cfg.total_bytes,
            write_ms,
            0,
            fail_phase,
            Some(offset),
        );
    }
    if !final_sync_ok {
        return failed_case(
            path,
            block_size,
            cfg.total_bytes,
            write_ms,
            0,
            "fsync-final",
            Some(offset),
        );
    }

    let read_start = sys::uptime_ms();
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return failed_case(
            path,
            block_size,
            cfg.total_bytes,
            write_ms,
            0,
            "open-read",
            Some(0),
        );
    }

    let mut first_bad = None;
    let mut fail_phase = "verify";
    let mut read_offset = 0u32;
    while read_offset < cfg.total_bytes {
        let len = (cfg.total_bytes - read_offset).min(block_size as u32) as usize;
        let n = fs::read(fd, &mut read_buf[..len]);
        if n != len as u32 {
            fail_phase = "read-short";
            first_bad = Some(read_offset.saturating_add(n.min(len as u32)));
            break;
        }
        if let Some(bad) = verify_pattern(&read_buf[..len], read_offset, seed) {
            fail_phase = "verify";
            first_bad = Some(read_offset + bad as u32);
            break;
        }
        read_offset += len as u32;
    }
    fs::close(fd);
    let read_ms = elapsed_ms(read_start);

    CaseResult {
        block_size,
        total_bytes: cfg.total_bytes,
        write_ms,
        read_ms,
        ok: first_bad.is_none(),
        phase: if first_bad.is_none() {
            "ok"
        } else {
            fail_phase
        },
        first_bad,
        path: String::from(path),
    }
}

fn failed_case(
    path: &str,
    block_size: usize,
    total_bytes: u32,
    write_ms: u32,
    read_ms: u32,
    phase: &'static str,
    first_bad: Option<u32>,
) -> CaseResult {
    CaseResult {
        block_size,
        total_bytes,
        write_ms,
        read_ms,
        ok: false,
        phase,
        first_bad,
        path: String::from(path),
    }
}

fn fill_pattern(buf: &mut [u8], start: u32, seed: u32) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = pattern_byte(start + i as u32, seed);
    }
}

fn verify_pattern(buf: &[u8], start: u32, seed: u32) -> Option<usize> {
    for (i, &b) in buf.iter().enumerate() {
        if b != pattern_byte(start + i as u32, seed) {
            return Some(i);
        }
    }
    None
}

fn pattern_byte(pos: u32, seed: u32) -> u8 {
    let x = pos
        .wrapping_mul(1_103_515_245)
        .wrapping_add(seed.rotate_left((pos & 15) + 1))
        ^ (pos >> 7)
        ^ (pos >> 19);
    (x ^ (x >> 8) ^ (x >> 16) ^ (x >> 24)) as u8
}

fn write_pattern_file(
    path: &str,
    total_bytes: u32,
    block_size: usize,
    seed: u32,
) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }
    let mut buf = Vec::new();
    buf.resize(block_size.min(MAX_BLOCK), 0);
    let mut offset = 0u32;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(buf.len() as u32) as usize;
        fill_pattern(&mut buf[..len], offset, seed);
        if fs::write(fd, &buf[..len]) != len as u32 {
            fs::close(fd);
            return Err("write");
        }
        offset += len as u32;
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);
    Ok(())
}

fn write_bytes_file(path: &str, bytes: &[u8]) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }
    if fs::write(fd, bytes) != bytes.len() as u32 {
        fs::close(fd);
        return Err("write");
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);
    Ok(())
}

fn verify_file_pattern(
    path: &str,
    total_bytes: u32,
    seed: u32,
    read_sizes: &[usize],
) -> Result<(), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = [0u8; 4096];
    let mut offset = 0u32;
    let mut idx = 0usize;
    while offset < total_bytes {
        let want = read_sizes[idx % read_sizes.len()]
            .min(buf.len())
            .min((total_bytes - offset) as usize);
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("read-short");
        }
        if verify_pattern(&buf[..want], offset, seed).is_some() {
            fs::close(fd);
            return Err("verify");
        }
        offset += want as u32;
        idx += 1;
    }
    fs::close(fd);
    Ok(())
}

fn verify_seek_overwrite(
    path: &str,
    total_bytes: u32,
    base_seed: u32,
    patch_seed: u32,
    patches: &[(u32, usize)],
) -> Result<(), &'static str> {
    verify_seek_overwrite_with_reads(path, total_bytes, base_seed, patch_seed, patches, &[4096])
}

fn verify_seek_overwrite_with_reads(
    path: &str,
    total_bytes: u32,
    base_seed: u32,
    patch_seed: u32,
    patches: &[(u32, usize)],
    read_sizes: &[usize],
) -> Result<(), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = [0u8; 4096];
    let mut offset = 0u32;
    let mut read_index = 0usize;
    while offset < total_bytes {
        let want = read_sizes[read_index % read_sizes.len()]
            .min(buf.len())
            .min((total_bytes - offset) as usize);
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("read-short");
        }
        for (i, &actual) in buf[..want].iter().enumerate() {
            let pos = offset + i as u32;
            let seed = if pos_in_patches(pos, patches) {
                patch_seed
            } else {
                base_seed
            };
            if actual != pattern_byte(pos, seed) {
                fs::close(fd);
                return Err("verify");
            }
        }
        offset += want as u32;
        read_index += 1;
    }
    fs::close(fd);
    Ok(())
}

fn pos_in_patches(pos: u32, patches: &[(u32, usize)]) -> bool {
    for &(start, len) in patches {
        let end = start.saturating_add(len as u32);
        if pos >= start && pos < end {
            return true;
        }
    }
    false
}

fn read_exact_file(path: &str, len: usize) -> Result<Vec<u8>, &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut out = Vec::new();
    out.resize(len, 0);
    let mut done = 0usize;
    while done < len {
        let n = fs::read(fd, &mut out[done..]);
        if n == 0 || n == u32::MAX {
            fs::close(fd);
            return Err("read-short");
        }
        done += (n as usize).min(len - done);
    }
    fs::close(fd);
    Ok(out)
}

fn stat_size(path: &str) -> Result<u32, &'static str> {
    let mut stat = [0u32; 7];
    if fs::stat(path, &mut stat) != 0 {
        return Err("stat");
    }
    Ok(stat[1])
}

fn dir_contains(path: &str, needle: &str) -> bool {
    let mut buf = [0u8; 64 * 96];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return false;
    }
    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > buf.len() {
            break;
        }
        let name_len = buf[off + 1] as usize;
        if name_len > 56 {
            continue;
        }
        let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len]).unwrap_or("");
        if name == needle {
            return true;
        }
    }
    false
}

fn print_case(round: u32, result: &CaseResult) {
    let w = kb_per_s(result.total_bytes, result.write_ms);
    let r = kb_per_s(result.total_bytes, result.read_ms);
    if result.ok {
        println!(
            "  r={} block={:>6} total={} KB write={} ms ({} KB/s) read={} ms ({} KB/s) ok",
            round,
            result.block_size,
            result.total_bytes / 1024,
            result.write_ms,
            w,
            result.read_ms,
            r
        );
    } else {
        println!(
            "  r={} block={:>6} total={} KB FAIL phase={} first_bad={}",
            round,
            result.block_size,
            result.total_bytes / 1024,
            result.phase,
            result.first_bad.unwrap_or(u32::MAX)
        );
    }
}

fn print_protocol(results: &[CaseResult]) {
    println!();
    println!("--- Protokoll ---");
    println!("  block | bytes | write_ms | write_kb_s | read_ms | read_kb_s | result | phase | first_bad | path");
    for r in results {
        println!(
            "  {} | {} | {} | {} | {} | {} | {} | {} | {} | {}",
            r.block_size,
            r.total_bytes,
            r.write_ms,
            kb_per_s(r.total_bytes, r.write_ms),
            r.read_ms,
            kb_per_s(r.total_bytes, r.read_ms),
            if r.ok { "ok" } else { "FAIL" },
            r.phase,
            r.first_bad.unwrap_or(u32::MAX),
            r.path
        );
    }
}

fn prepare_dir(path: &str) -> bool {
    mkdir_parents(path);
    let probe = format!("{}/.vfsstress-write-test", path);
    let fd = fs::open(&probe, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return false;
    }
    let ok = fs::write(fd, b"ok") == 2;
    let _ = fs::fsync(fd as i32);
    fs::close(fd);
    let _ = fs::unlink(&probe);
    ok
}

fn mkdir_parents(path: &str) {
    let bytes = path.as_bytes();
    let mut i = 0usize;
    if !bytes.is_empty() && bytes[0] == b'/' {
        i = 1;
    }
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'/' {
            if i > 0 {
                let component = core::str::from_utf8(&bytes[..i]).unwrap_or("");
                if !component.is_empty() {
                    let _ = fs::mkdir(component);
                }
            }
        }
        i += 1;
    }
}

fn print_summary(summary: &Summary, started: u32) {
    println!();
    println!("=== Zusammenfassung ===");
    println!("  Tests:    {}", summary.tests);
    println!("  Warns:    {}", summary.warnings);
    println!("  Fehler:   {}", summary.failures);
    println!("  Laufzeit: {} ms", elapsed_ms(started));
    if summary.failures == 0 {
        println!("  Ergebnis: PASS");
    } else {
        println!("  Ergebnis: FAIL");
    }
}

fn kb_per_s(bytes: u32, ms: u32) -> u32 {
    if ms == 0 {
        return 0;
    }
    ((bytes as u64 * 1000) / (ms as u64 * 1024)) as u32
}

fn elapsed_ms(start: u32) -> u32 {
    sys::uptime_ms().wrapping_sub(start)
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(value)
}

fn clamp(value: u32, min: u32, max: u32) -> u32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}
