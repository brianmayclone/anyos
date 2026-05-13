#![no_std]
#![no_main]

use alloc::format;
use anyos_std::{env, fs, println, process, sys, String, Vec};

anyos_std::entry!(main);

const VERSION: &str = "0.1";
const DEFAULT_DIR: &str = "/tmp/vfsstress";
const MAX_BLOCK: usize = 256 * 1024;
const STACK_WRITE_SIZE: usize = 64 * 1024;
const WRITEBACK_STREAM_SIZE: u32 = 12_085_491;
const WRITEBACK_STREAM_CHUNK_16K: usize = 16 * 1024;
const WRITEBACK_STREAM_CHUNK_32K: usize = 32 * 1024;
const USER_COPY_BOUNDARY_TOTAL: u32 = 384 * 1024;
const FSX_QUICK_OPS: u32 = 96;
const FSX_NORMAL_OPS: u32 = 384;
const FSX_HEAVY_OPS: u32 = 1536;
const FS_TYPE_EXFAT: u32 = 7;
const FS_TYPE_COREFS: u32 = 6;
const WORKER_CONFIG_ENV: &str = "VFSSTRESS_WORKER_CONFIG";
const WORKER_BATCH_ENV: &str = "VFSSTRESS_WORKER_BATCH";

fn anyos_version() -> &'static str {
    option_env!("ANYOS_VERSION").unwrap_or("dev")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Quick,
    Normal,
    Heavy,
    Soak,
}

struct Config {
    dir: String,
    repeat: u32,
    total_bytes: u32,
    profile: Profile,
    keep: bool,
    sync_each: bool,
    workers: u32,
    ops: u32,
    seed: u32,
    enospc_kb: u32,
    scratch_device: String,
    scratch_fs: String,
    scratch_mount: String,
    seconds: u32,
    json: bool,
}

struct WorkerConfig {
    dir: String,
    worker: u32,
    workers: u32,
    ops: u32,
    seed: u32,
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
            workers: 4,
            ops: 128,
            seed: 0xA11C_E5ED,
            enospc_kb: 4096,
            scratch_device: String::new(),
            scratch_fs: String::from("corefs"),
            scratch_mount: String::from("/tmp/vfsstress-scratch"),
            seconds: 0,
            json: false,
        }
    }

    fn profile_name(&self) -> &'static str {
        match self.profile {
            Profile::Quick => "quick",
            Profile::Normal => "normal",
            Profile::Heavy => "heavy",
            Profile::Soak => "soak",
        }
    }
}

struct Summary {
    tests: u32,
    failures: u32,
    warnings: u32,
    skips: u32,
}

impl Summary {
    fn new() -> Self {
        Self {
            tests: 0,
            failures: 0,
            warnings: 0,
            skips: 0,
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

    fn warn(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.warnings += 1;
        println!("  [WARN] {:<22} {}", name, detail);
    }

    fn skip(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.skips += 1;
        println!("  [SKIP] {:<22} {}", name, detail);
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
    if let Some(worker) = parse_worker_config() {
        let code = match run_fsstress_worker(&worker) {
            Ok(()) => 0,
            Err(err) => {
                println!(
                    "vfsstress worker {} FAIL {} op={} dir={}",
                    worker.worker, err, worker.ops, worker.dir
                );
                1
            }
        };
        process::exit(code);
    }

    let Some(mut cfg) = parse_config() else {
        return;
    };
    apply_profile(&mut cfg);

    println!();
    println!("vfsstress {} - VFS Diagnose", VERSION);
    println!("============================");
    println!("  anyOS:     {}", anyos_version());
    println!("  profile:   {}", cfg.profile_name());
    println!("  repeat:    {}", cfg.repeat);
    println!("  total:     {} KB", cfg.total_bytes / 1024);
    println!("  dir:       {}", cfg.dir);
    println!("  sync-each: {}", if cfg.sync_each { "yes" } else { "no" });
    println!("  workers:   {}", cfg.workers);
    println!("  ops:       {}", cfg.ops);
    println!("  seed:      {}", cfg.seed);
    println!("  enospc:    {} KB cap", cfg.enospc_kb);
    if cfg.seconds > 0 {
        println!("  seconds:   {}", cfg.seconds);
    }
    if !cfg.scratch_device.is_empty() {
        println!(
            "  scratch:   fs={} device={} mount={}",
            cfg.scratch_fs, cfg.scratch_device, cfg.scratch_mount
        );
    }
    println!();

    let started = sys::uptime_ms();
    let mut summary = Summary::new();
    if prepare_dir(&cfg.dir) {
        summary.ok("work-dir", &format!("{} writable", cfg.dir));
    } else {
        summary.fail("work-dir", &format!("{} nicht beschreibbar", cfg.dir));
        print_summary(&cfg, &summary, started);
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
    if cfg.seconds > 0 {
        run_soak_suite(&cfg, &mut summary);
    }

    print_protocol(&results);
    if !cfg.keep {
        for r in &results {
            let _ = fs::unlink(&r.path);
        }
    } else {
        println!();
        println!("Artefakte bleiben erhalten in {}", cfg.dir);
    }
    print_summary(&cfg, &summary, started);
    if cfg.json {
        print_json_summary(&cfg, &summary, elapsed_ms(started));
    }
}

fn parse_worker_config() -> Option<WorkerConfig> {
    let mut buf = [0u8; 512];
    let raw = process::args(&mut buf);

    if has_arg(raw, "--worker") {
        if let Some(path) = arg_value(raw, "--config") {
            if let Some(cfg) = read_worker_config_file(path) {
                return Some(cfg);
            }
        } else if let Some(cfg) = parse_worker_args(raw) {
            return Some(cfg);
        }
    }
    if has_arg(raw, "--worker-batch") {
        if let Some(cfg) = parse_worker_batch(raw) {
            return Some(cfg);
        }
    }

    let mut env_buf = [0u8; 257];
    let len = env::get(WORKER_CONFIG_ENV, &mut env_buf);
    if len != u32::MAX && len > 0 {
        let n = (len as usize).min(env_buf.len());
        if let Ok(path) = core::str::from_utf8(&env_buf[..n]) {
            if let Some(cfg) = read_worker_config_file(path.trim_matches(char::from(0))) {
                return Some(cfg);
            }
        }
    }

    let len = env::get(WORKER_BATCH_ENV, &mut env_buf);
    if len != u32::MAX && len > 0 {
        let n = (len as usize).min(env_buf.len());
        if let Ok(raw) = core::str::from_utf8(&env_buf[..n]) {
            if let Some(cfg) = parse_worker_batch(raw.trim_matches(char::from(0))) {
                return Some(cfg);
            }
        }
    }

    None
}

fn parse_worker_args(raw: &str) -> Option<WorkerConfig> {
    if !has_arg(raw, "--worker") {
        return None;
    }
    let args: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut cfg = WorkerConfig {
        dir: String::from(DEFAULT_DIR),
        worker: 0,
        workers: 1,
        ops: 1,
        seed: 1,
    };
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--worker" => {}
            "--config" => {
                i += 1;
            }
            "--dir" | "-d" => {
                i += 1;
                if i < args.len() {
                    cfg.dir = String::from(args[i]);
                }
            }
            "--worker-id" => {
                i += 1;
                if i < args.len() {
                    cfg.worker = parse_u32(args[i]).unwrap_or(cfg.worker);
                }
            }
            "--workers" => {
                i += 1;
                if i < args.len() {
                    cfg.workers = clamp(parse_u32(args[i]).unwrap_or(cfg.workers), 1, 32);
                }
            }
            "--ops" => {
                i += 1;
                if i < args.len() {
                    cfg.ops = clamp(parse_u32(args[i]).unwrap_or(cfg.ops), 1, 100_000);
                }
            }
            "--seed" => {
                i += 1;
                if i < args.len() {
                    cfg.seed = parse_u32(args[i]).unwrap_or(cfg.seed);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Some(cfg)
}

fn has_arg(raw: &str, needle: &str) -> bool {
    raw.split_ascii_whitespace().any(|arg| arg == needle)
}

fn arg_value<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    let args: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0usize;
    while i < args.len() {
        if args[i] == key {
            return args.get(i + 1).copied();
        }
        i += 1;
    }
    None
}

fn read_worker_config_file(path: &str) -> Option<WorkerConfig> {
    let raw = read_small_text_file(path, 512).ok()?;
    parse_worker_args(&raw)
}

fn parse_worker_batch(raw: &str) -> Option<WorkerConfig> {
    if !has_arg(raw, "--worker-batch") {
        return None;
    }
    let mut cfg = parse_worker_args(&raw.replace("--worker-batch", "--worker"))?;
    cfg.worker = claim_worker_id(&cfg.dir, cfg.workers)?;
    cfg.seed ^= cfg.worker.wrapping_mul(0x9E37);
    Some(cfg)
}

fn claim_worker_id(root: &str, workers: u32) -> Option<u32> {
    let claims = format!("{}/claims", root);
    let _ = fs::mkdir(&claims);
    for worker in 0..workers {
        let claim = format!("{}/w{:02}", claims, worker);
        if fs::mkdir(&claim) == 0 {
            return Some(worker);
        }
    }
    None
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
                    println!("vfsstress: --profile braucht quick|normal|heavy|soak");
                    return None;
                }
                cfg.profile = match args[i] {
                    "quick" => Profile::Quick,
                    "normal" => Profile::Normal,
                    "heavy" => Profile::Heavy,
                    "soak" => Profile::Soak,
                    other => {
                        println!("vfsstress: unbekanntes Profil '{}'", other);
                        return None;
                    }
                };
            }
            "--sync-each" => cfg.sync_each = true,
            "--workers" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --workers braucht eine Zahl");
                    return None;
                }
                cfg.workers = clamp(parse_u32(args[i]).unwrap_or(cfg.workers), 1, 32);
            }
            "--ops" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --ops braucht eine Zahl");
                    return None;
                }
                cfg.ops = clamp(parse_u32(args[i]).unwrap_or(cfg.ops), 1, 100_000);
            }
            "--seed" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --seed braucht eine Zahl");
                    return None;
                }
                cfg.seed = parse_u32(args[i]).unwrap_or(cfg.seed);
            }
            "--enospc-kb" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --enospc-kb braucht eine Zahl");
                    return None;
                }
                cfg.enospc_kb = clamp(parse_u32(args[i]).unwrap_or(cfg.enospc_kb), 64, 1024 * 1024);
            }
            "--scratch-device" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --scratch-device braucht eine Device-ID");
                    return None;
                }
                cfg.scratch_device = String::from(args[i]);
            }
            "--scratch-fs" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --scratch-fs braucht corefs|exfat");
                    return None;
                }
                match args[i] {
                    "corefs" | "exfat" => cfg.scratch_fs = String::from(args[i]),
                    other => {
                        println!("vfsstress: unbekanntes Scratch-FS '{}'", other);
                        return None;
                    }
                }
            }
            "--scratch-mount" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --scratch-mount braucht einen Pfad");
                    return None;
                }
                cfg.scratch_mount = String::from(args[i]);
            }
            "--seconds" => {
                i += 1;
                if i >= args.len() {
                    println!("vfsstress: --seconds braucht eine Zahl");
                    return None;
                }
                cfg.seconds = clamp(parse_u32(args[i]).unwrap_or(cfg.seconds), 1, 24 * 60 * 60);
            }
            "--soak" => {
                cfg.profile = Profile::Soak;
                if cfg.seconds == 0 {
                    cfg.seconds = 3600;
                }
            }
            "--json" => cfg.json = true,
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
            if cfg.workers == Config::default().workers {
                cfg.workers = 2;
            }
            if cfg.ops == Config::default().ops {
                cfg.ops = 32;
            }
            if cfg.enospc_kb == Config::default().enospc_kb {
                cfg.enospc_kb = 1024;
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
            if cfg.workers == Config::default().workers {
                cfg.workers = 8;
            }
            if cfg.ops == Config::default().ops {
                cfg.ops = 1024;
            }
            if cfg.enospc_kb == Config::default().enospc_kb {
                cfg.enospc_kb = 64 * 1024;
            }
        }
        Profile::Soak => {
            if cfg.repeat == Config::default().repeat {
                cfg.repeat = 4;
            }
            if cfg.total_bytes == Config::default().total_bytes {
                cfg.total_bytes = 16 * 1024 * 1024;
            }
            if cfg.workers == Config::default().workers {
                cfg.workers = 8;
            }
            if cfg.ops == Config::default().ops {
                cfg.ops = 512;
            }
            if cfg.enospc_kb == Config::default().enospc_kb {
                cfg.enospc_kb = 32 * 1024;
            }
            if cfg.seconds == 0 {
                cfg.seconds = 3600;
            }
        }
    }
}

fn print_usage() {
    println!("vfsstress - VFS Schreib-/Readback-Stresstest");
    println!("anyOS {}", anyos_version());
    println!();
    println!("Usage: vfsstress [options]");
    println!("  --profile P       quick | normal | heavy (default: normal)");
    println!("  --soak            Profil soak und default 3600 Sekunden Laufzeit");
    println!("  --seconds N       zusaetzliche Soak-Dauer in Sekunden");
    println!("  --json            Summary zusaetzlich als JSON-Zeile ausgeben");
    println!("  --repeat N        Wiederholungen pro Blockgroesse");
    println!("  --total-kb N      Dateigroesse pro Testfall in KB");
    println!("  --dir PATH        Testverzeichnis (default: /tmp/vfsstress)");
    println!("  --sync-each       fsync nach jedem Write-Chunk");
    println!("  --workers N       parallele fsstress-Worker (default: profilabhaengig)");
    println!("  --ops N           Operationen pro fsstress-Worker");
    println!("  --seed N          deterministischer Seed fuer Random-Workloads");
    println!("  --enospc-kb N     Schreiblimit fuer ENOSPC/Accounting-Probe");
    println!("  --scratch-device ID  GEFAEHRLICH: Device fuer mkfs/mount/remount-Test");
    println!("  --scratch-fs FS      corefs | exfat (default: corefs)");
    println!("  --scratch-mount PATH Mountpoint fuer Scratch-Test");
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
    run_named(summary, "parallel fsstress", parallel_fsstress_case(cfg));
    run_named_with_warning(summary, "enospc accounting", enospc_accounting_case(cfg));
    if !cfg.scratch_device.is_empty() {
        run_named_with_warning(summary, "scratch lifecycle", scratch_lifecycle_case(cfg));
    }
    run_named_with_warning(summary, "symlink eloop", symlink_eloop_case(cfg));
    run_named(summary, "fsx random model", fsx_random_model_case(cfg));
    run_named(summary, "sparse eof gaps", sparse_eof_gap_case(cfg));
    run_named(summary, "sparse hole matrix", sparse_hole_matrix_case(cfg));
    run_named(summary, "fsstress metadata", fsstress_metadata_case(cfg));
    run_named(summary, "close reopen sync", close_reopen_sync_case(cfg));
    run_named(summary, "open unlink rename", open_unlink_rename_case(cfg));
    run_named(summary, "metadata rename", metadata_rename_case(cfg));
    run_named(summary, "directory churn", directory_churn_case(cfg));
    run_named(summary, "metadata perf", metadata_perf_case(cfg));
    run_named(summary, "sequential perf", sequential_io_perf_case(cfg));
    run_named(summary, "overwrite perf", random_overwrite_perf_case(cfg));
    run_named(summary, "sync latency", sync_latency_perf_case(cfg));
    run_named(
        summary,
        "readdir mutation",
        readdir_while_mutating_case(cfg),
    );
    run_named(summary, "large directory", large_directory_case(cfg));
    run_named(summary, "long names", long_name_case(cfg));
    run_named_with_warning(
        summary,
        "permission metadata",
        permission_metadata_case(cfg),
    );
    run_named_with_warning(summary, "fsync ordering", fsync_ordering_case(cfg));
    run_named_with_warning(summary, "statfs accounting", statfs_accounting_case(cfg));
    run_named_with_warning(summary, "path resolution", path_resolution_case(cfg));
    run_named_with_warning(summary, "feature gates", feature_gate_case(cfg));
    run_named(summary, "fd offsets", fd_offsets_case(cfg));
    run_named(summary, "user copy boundary", user_copy_boundary_case(cfg));
    run_named(summary, "stack write 64k", stack_write_64k_case(cfg));
    run_named(
        summary,
        "stream write 16k",
        writeback_stream_case(cfg, WRITEBACK_STREAM_CHUNK_16K),
    );
    run_named(
        summary,
        "stream write 32k",
        writeback_stream_case(cfg, WRITEBACK_STREAM_CHUNK_32K),
    );

    fs::sync();
}

fn run_soak_suite(cfg: &Config, summary: &mut Summary) {
    println!();
    println!("--- Soak-/Dauerlauf ---");
    let started = sys::uptime_ms();
    let limit_ms = cfg.seconds.saturating_mul(1000);
    let mut round = 0u32;
    while elapsed_ms(started) < limit_ms {
        round += 1;
        println!(
            "  soak round={} elapsed={} ms seed={}",
            round,
            elapsed_ms(started),
            cfg.seed ^ round
        );
        run_named(summary, "soak fsx", fsx_random_model_case(cfg));
        run_named(summary, "soak metadata", fsstress_metadata_case(cfg));
        run_named(summary, "soak parallel", parallel_fsstress_case(cfg));
        run_named_with_warning(summary, "soak enospc", enospc_accounting_case(cfg));
        run_named(summary, "soak sequential", sequential_io_perf_case(cfg));
        run_named(summary, "soak sync", sync_latency_perf_case(cfg));
        fs::sync();
        if summary.failures != 0 {
            println!("  soak: Abbruch nach Fehler in Runde {}", round);
            break;
        }
    }
}

fn run_named(summary: &mut Summary, name: &str, result: Result<String, &'static str>) {
    match result {
        Ok(detail) => summary.ok(name, &detail),
        Err(err) => summary.fail(name, err),
    }
}

enum TestOutcome {
    Ok(String),
    Warn(String),
    Skip(String),
    Fail(&'static str),
}

fn run_named_with_warning(summary: &mut Summary, name: &str, result: TestOutcome) {
    match result {
        TestOutcome::Ok(detail) => summary.ok(name, &detail),
        TestOutcome::Warn(detail) => summary.warn(name, &detail),
        TestOutcome::Skip(detail) => summary.skip(name, &detail),
        TestOutcome::Fail(err) => summary.fail(name, err),
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

fn parallel_fsstress_case(cfg: &Config) -> Result<String, &'static str> {
    let root = format!("{}/full-parallel-fsstress-{}", cfg.dir, sys::uptime_ms());
    mkdir_parents(&root);
    let shared = format!("{}/shared", root);
    let _ = fs::mkdir(&shared);
    let claims = format!("{}/claims", root);
    let _ = fs::mkdir(&claims);
    let batch_args = format!(
        "--worker-batch --dir {} --workers {} --ops {} --seed {}",
        root, cfg.workers, cfg.ops, cfg.seed
    );
    env::set(WORKER_BATCH_ENV, &batch_args);

    let mut tids = Vec::new();
    for worker in 0..cfg.workers {
        let worker_dir = format!("{}/w{:02}", root, worker);
        let _ = fs::mkdir(&worker_dir);
        let worker_args = format!(
            "--worker --dir {} --worker-id {} --workers {} --ops {} --seed {}",
            root,
            worker,
            cfg.workers,
            cfg.ops,
            cfg.seed ^ worker.wrapping_mul(0x9E37)
        );
        let config_path = format!("{}/worker.cfg", worker_dir);
        write_bytes_file(&config_path, worker_args.as_bytes())?;
        let tid = spawn_vfsstress(&batch_args);
        if tid == u32::MAX {
            env::unset(WORKER_BATCH_ENV);
            return Err("spawn-worker");
        }
        tids.push(tid);
    }

    let mut failures = 0u32;
    for tid in tids {
        let code = process::waitpid(tid);
        if code != 0 {
            failures += 1;
        }
    }
    env::unset(WORKER_BATCH_ENV);
    if failures != 0 {
        return Err(read_parallel_worker_failure(&root, cfg.workers));
    }

    fs::sync();
    for worker in 0..cfg.workers {
        let done = format!("{}/w{:02}/done.txt", root, worker);
        let expected = format!("worker={} ops={} ok\n", worker, cfg.ops);
        let data = read_exact_file_retry(&done, expected.len(), 32).map_err(|_| "done-open")?;
        if data.as_slice() != expected.as_bytes() {
            return Err("done-verify");
        }
    }
    verify_shared_hot_dir(&shared)?;
    fs::sync();
    Ok(format!(
        "{} Worker x {} Operationen + Shared-Hot-Dir + Manifest ok",
        cfg.workers, cfg.ops
    ))
}

fn spawn_vfsstress(args: &str) -> u32 {
    let tid = process::spawn("/System/bin/vfsstress", args);
    if tid != u32::MAX {
        return tid;
    }
    process::spawn("vfsstress", args)
}

fn read_parallel_worker_failure(root: &str, workers: u32) -> &'static str {
    for worker in 0..workers {
        let fail = format!("{}/w{:02}/fail.txt", root, worker);
        if let Ok(data) = read_small_text_file(&fail, 192) {
            println!("    worker-fail: {}", data.trim());
            return "worker-failed-detail";
        }
    }
    "worker-failed"
}

fn write_worker_failure(cfg: &WorkerConfig, op: u32, err: &str) {
    let own = format!("{}/w{:02}", cfg.dir, cfg.worker);
    let fail = format!("{}/fail.txt", own);
    let detail = format!(
        "worker={} op={} ops={} seed={} error={}\n",
        cfg.worker, op, cfg.ops, cfg.seed, err
    );
    let _ = write_bytes_file(&fail, detail.as_bytes());
}

fn run_fsstress_worker(cfg: &WorkerConfig) -> Result<(), &'static str> {
    let own = format!("{}/w{:02}", cfg.dir, cfg.worker);
    let shared = format!("{}/shared", cfg.dir);
    let _ = fs::mkdir(&own);
    let _ = fs::mkdir(&shared);
    let mut rng = Lcg::new(cfg.seed ^ cfg.worker.rotate_left(7));
    let mut buf = [0u8; 2048];

    for op in 0..cfg.ops {
        macro_rules! worker_try {
            ($expr:expr) => {
                if let Err(err) = $expr {
                    write_worker_failure(cfg, op, err);
                    return Err(err);
                }
            };
        }
        let slot = rng.range(0, 31);
        match rng.next() % 11 {
            0 => {
                let path = format!("{}/f{:02}.dat", own, slot);
                let len = rng.range(1, buf.len() as u32) as usize;
                fill_pattern(&mut buf[..len], 0, cfg.seed ^ slot);
                worker_try!(write_bytes_file(&path, &buf[..len]));
                worker_try!(verify_file_pattern_large(
                    &path,
                    len as u32,
                    cfg.seed ^ slot,
                    1024
                ));
            }
            1 => {
                let path = format!("{}/f{:02}.dat", own, slot);
                let len = rng.range(1, 512) as usize;
                fill_pattern(&mut buf[..len], stat_size_or_zero(&path), cfg.seed ^ op);
                worker_try!(append_to_file(&path, &buf[..len]));
                worker_try!(stat_size(&path).map(|_| ()));
            }
            2 => {
                let a = format!("{}/f{:02}.dat", own, slot);
                let b = format!("{}/g{:02}.dat", own, slot);
                if stat_size(&a).is_ok() {
                    let _ = fs::rename(&a, &b);
                    let _ = fs::rename(&b, &a);
                }
            }
            3 => {
                let path = format!("{}/f{:02}.dat", own, slot);
                let _ = fs::unlink(&path);
            }
            4 => {
                let path = format!("{}/f{:02}.dat", own, slot);
                let _ = truncate_to_zero(&path);
            }
            5 => {
                let path = format!("{}/hot-w{:02}-{:04}.dat", shared, cfg.worker, op);
                let renamed = format!("{}/hot-w{:02}-{:04}.ren", shared, cfg.worker, op);
                fill_pattern(&mut buf[..128], op, cfg.seed);
                worker_try!(write_bytes_file(&path, &buf[..128]));
                if fs::rename(&path, &renamed) != 0 {
                    write_worker_failure(cfg, op, "shared-rename");
                    return Err("shared-rename");
                }
                let size = match stat_size(&renamed) {
                    Ok(size) => size,
                    Err(err) => {
                        write_worker_failure(cfg, op, err);
                        return Err(err);
                    }
                };
                if size != 128 {
                    write_worker_failure(cfg, op, "shared-stat");
                    return Err("shared-stat");
                }
                let _ = fs::unlink(&renamed);
            }
            6 => {
                let name = format!("f{:02}.dat", slot);
                let _ = dir_contains(&own, &name);
            }
            7 => {
                let hot = rng.range(0, cfg.workers.saturating_mul(2).max(1) - 1);
                let path = format!("{}/hot{:02}.dat", shared, hot);
                let len = 64 + (rng.range(0, 7) as usize * 31);
                fill_pattern(
                    &mut buf[..len],
                    cfg.worker.wrapping_mul(4096).wrapping_add(op),
                    cfg.seed,
                );
                let _ = write_bytes_file(&path, &buf[..len]);
                let _ = stat_size(&path);
            }
            8 => {
                let hot = rng.range(0, cfg.workers.saturating_mul(2).max(1) - 1);
                let a = format!("{}/hot{:02}.dat", shared, hot);
                let b = format!("{}/hot{:02}.swap", shared, hot);
                if fs::rename(&a, &b) == 0 {
                    let _ = fs::rename(&b, &a);
                }
            }
            9 => {
                let hot = rng.range(0, cfg.workers.saturating_mul(2).max(1) - 1);
                let path = format!("{}/hot{:02}.dat", shared, hot);
                let _ = fs::unlink(&path);
            }
            _ => {
                fs::sync();
            }
        }
        if op % 17 == 0 {
            process::yield_cpu();
        }
    }

    let done = format!("{}/done.txt", own);
    let body = format!("worker={} ops={} ok\n", cfg.worker, cfg.ops);
    write_bytes_file(&done, body.as_bytes())?;
    fs::sync();
    Ok(())
}

fn verify_shared_hot_dir(path: &str) -> Result<(), &'static str> {
    let mut buf = Vec::new();
    buf.resize(fs::READDIR_LONG_ENTRY_SIZE * 256, 0);
    let count = fs::readdir_long(path, &mut buf);
    if count == u32::MAX {
        return Err("shared-readdir");
    }
    for i in 0..count as usize {
        let off = i * fs::READDIR_LONG_ENTRY_SIZE;
        if off + fs::READDIR_LONG_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        if name_len == 0 || name_len > 256 {
            continue;
        }
        let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len]).unwrap_or("");
        if name == "." || name == ".." {
            continue;
        }
        let file = format!("{}/{}", path, name);
        let size = stat_size(&file)?;
        if size > 2048 {
            return Err("shared-size");
        }
        if size > 0 {
            let fd = fs::open(&file, 0);
            if fd == u32::MAX {
                return Err("shared-open");
            }
            let mut one = [0u8; 1];
            let n = fs::read(fd, &mut one);
            fs::close(fd);
            if n != 1 {
                return Err("shared-read");
            }
        }
    }
    Ok(())
}

fn enospc_accounting_case(cfg: &Config) -> TestOutcome {
    let dir = format!("{}/full-enospc", cfg.dir);
    let _ = fs::mkdir(&dir);
    let before = fs::statfs(&cfg.dir);
    let cap_bytes = cfg.enospc_kb.saturating_mul(1024);
    let mut buf = Vec::new();
    buf.resize(32 * 1024, 0);
    let mut written = 0u32;
    let mut hit_enospc = false;
    let mut files = 0u32;

    while written < cap_bytes {
        let path = format!("{}/fill{:04}.bin", dir, files);
        let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            hit_enospc = true;
            break;
        }
        let mut file_written = 0u32;
        while file_written < 256 * 1024 && written < cap_bytes {
            let want = (cap_bytes - written).min(buf.len() as u32) as usize;
            fill_pattern(&mut buf[..want], written, cfg.seed ^ files);
            let n = fs::write(fd, &buf[..want]);
            if n != want as u32 {
                hit_enospc = true;
                break;
            }
            written += want as u32;
            file_written += want as u32;
        }
        let _ = fs::fsync(fd as i32);
        fs::close(fd);
        files += 1;
        if hit_enospc {
            break;
        }
    }
    fs::sync();
    let filled = fs::statfs(&cfg.dir);

    let mut deleted_bytes = 0u32;
    for i in 0..files {
        if i % 2 == 0 {
            let path = format!("{}/fill{:04}.bin", dir, i);
            deleted_bytes = deleted_bytes.saturating_add(stat_size_or_zero(&path));
            let _ = fs::unlink(&path);
        }
    }
    fs::sync();
    let after_delete = fs::statfs(&cfg.dir);

    let refill_bytes = deleted_bytes.min(512 * 1024).min(cap_bytes / 2).max(4096);
    let refill_path = format!("{}/refill.bin", dir);
    let mut refilled = 0u32;
    if deleted_bytes > 0 {
        let fd = fs::open(&refill_path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            return TestOutcome::Fail("refill-open");
        }
        while refilled < refill_bytes {
            let want = (refill_bytes - refilled).min(buf.len() as u32) as usize;
            fill_pattern(&mut buf[..want], refilled, cfg.seed ^ 0xE05C_0001);
            let n = fs::write(fd, &buf[..want]);
            if n != want as u32 {
                fs::close(fd);
                return TestOutcome::Fail("refill-write");
            }
            refilled += want as u32;
        }
        if !fs::fsync(fd as i32) {
            fs::close(fd);
            return TestOutcome::Fail("refill-fsync");
        }
        fs::close(fd);
        if verify_file_pattern_large(&refill_path, refilled, cfg.seed ^ 0xE05C_0001, 8192).is_err()
        {
            return TestOutcome::Fail("refill-verify");
        }
    }

    for i in 0..files {
        if i % 2 != 0 {
            let path = format!("{}/fill{:04}.bin", dir, i);
            let _ = fs::unlink(&path);
        }
    }
    let _ = fs::unlink(&refill_path);
    fs::sync();

    if written == 0 && !hit_enospc {
        return TestOutcome::Fail("no-write");
    }
    if let (Some(b), Some(f), Some(a)) = (before, filled, after_delete) {
        if f.free_bytes > b.free_bytes {
            return TestOutcome::Fail("statfs-free-increased-while-filling");
        }
        if a.free_bytes < f.free_bytes {
            return TestOutcome::Fail("statfs-free-did-not-recover");
        }
        let detail = format!(
            "{} KB geschrieben, refill {} KB, free {} -> {} -> {}, enospc={}",
            written / 1024,
            refilled / 1024,
            b.free_bytes / 1024,
            f.free_bytes / 1024,
            a.free_bytes / 1024,
            if hit_enospc { "yes" } else { "no-within-cap" }
        );
        if hit_enospc {
            TestOutcome::Ok(detail)
        } else {
            TestOutcome::Warn(detail)
        }
    } else if hit_enospc {
        TestOutcome::Ok(format!(
            "{} KB geschrieben, ENOSPC ohne statfs ok",
            written / 1024
        ))
    } else {
        TestOutcome::Warn(format!(
            "{} KB geschrieben, kein ENOSPC innerhalb Limit und statfs nicht verfuegbar",
            written / 1024
        ))
    }
}

fn scratch_lifecycle_case(cfg: &Config) -> TestOutcome {
    if cfg.scratch_device.is_empty() {
        return TestOutcome::Warn("skip: --scratch-device nicht gesetzt".into());
    }
    if cfg.scratch_fs.as_str() != "corefs" && cfg.scratch_fs.as_str() != "exfat" {
        return TestOutcome::Fail("scratch-fs");
    }

    let _ = fs::umount(&cfg.scratch_mount);
    mkdir_parents(&cfg.scratch_mount);

    let mkfs_bin = if cfg.scratch_fs.as_str() == "exfat" {
        "/System/sbin/mkfs.exfat"
    } else {
        "/System/sbin/mkfs.corefs"
    };
    let mkfs_args = format!("--device {} --label vfsstress", cfg.scratch_device);
    let mkfs_tid = process::spawn(mkfs_bin, &mkfs_args);
    if mkfs_tid == u32::MAX {
        return TestOutcome::Fail("spawn-mkfs");
    }
    if process::waitpid(mkfs_tid) != 0 {
        return TestOutcome::Fail("mkfs");
    }

    let fs_type_id = if cfg.scratch_fs.as_str() == "exfat" {
        FS_TYPE_EXFAT
    } else {
        FS_TYPE_COREFS
    };
    if fs::mount(&cfg.scratch_mount, &cfg.scratch_device, fs_type_id) == u32::MAX {
        return TestOutcome::Fail("mount");
    }

    let sentinel = format!("{}/sentinel.bin", cfg.scratch_mount);
    if write_pattern_file(&sentinel, 128 * 1024, 4096, cfg.seed ^ 0xC0FE).is_err() {
        let _ = fs::umount(&cfg.scratch_mount);
        return TestOutcome::Fail("write-sentinel");
    }
    if verify_file_pattern_large(&sentinel, 128 * 1024, cfg.seed ^ 0xC0FE, 8192).is_err() {
        let _ = fs::umount(&cfg.scratch_mount);
        return TestOutcome::Fail("verify-sentinel-mounted");
    }

    let scratch_cfg = Config {
        dir: cfg.scratch_mount.clone(),
        repeat: 1,
        total_bytes: 512 * 1024,
        profile: Profile::Quick,
        keep: true,
        sync_each: false,
        workers: 2,
        ops: 16,
        seed: cfg.seed ^ 0x51A7,
        enospc_kb: 512,
        scratch_device: String::new(),
        scratch_fs: cfg.scratch_fs.clone(),
        scratch_mount: cfg.scratch_mount.clone(),
        seconds: 0,
        json: false,
    };
    if parallel_fsstress_case(&scratch_cfg).is_err() {
        let _ = fs::umount(&cfg.scratch_mount);
        return TestOutcome::Fail("scratch-parallel");
    }

    fs::sync();
    if fs::umount(&cfg.scratch_mount) == u32::MAX {
        return TestOutcome::Fail("umount");
    }
    if run_scratch_fsck(cfg).is_err() {
        return TestOutcome::Fail("fsck");
    }
    if fs::mount(&cfg.scratch_mount, &cfg.scratch_device, fs_type_id) == u32::MAX {
        return TestOutcome::Fail("remount");
    }
    if verify_file_pattern_large(&sentinel, 128 * 1024, cfg.seed ^ 0xC0FE, 16 * 1024).is_err() {
        let _ = fs::umount(&cfg.scratch_mount);
        return TestOutcome::Fail("verify-after-remount");
    }
    if fs::umount(&cfg.scratch_mount) == u32::MAX {
        return TestOutcome::Fail("umount-after-remount");
    }
    if cfg.scratch_fs.as_str() == "corefs" && run_corefs_scrub(cfg).is_err() {
        return TestOutcome::Fail("corefs-scrub");
    }

    TestOutcome::Ok(format!(
        "device {} als {} formatiert, fsck, mount/remount/readback{} ok",
        cfg.scratch_device,
        cfg.scratch_fs,
        if cfg.scratch_fs.as_str() == "corefs" {
            "/scrub"
        } else {
            ""
        }
    ))
}

fn symlink_eloop_case(cfg: &Config) -> TestOutcome {
    let dir = format!("{}/full-symlink-eloop", cfg.dir);
    let target = format!("{}/target.txt", dir);
    let link = format!("{}/link.txt", dir);
    let chain1 = format!("{}/chain1", dir);
    let chain2 = format!("{}/chain2", dir);
    let dangling = format!("{}/dangling", dir);
    let self_loop = format!("{}/self-loop", dir);
    let cycle_a = format!("{}/cycle-a", dir);
    let cycle_b = format!("{}/cycle-b", dir);
    let long_link = format!("{}/long-target", dir);

    let _ = fs::mkdir(&dir);
    for path in [
        &target, &link, &chain1, &chain2, &dangling, &self_loop, &cycle_a, &cycle_b, &long_link,
    ] {
        let _ = fs::unlink(path);
    }

    if write_bytes_file(&target, b"symlink-target").is_err() {
        return TestOutcome::Fail("write-target");
    }

    if fs::symlink(&target, &link) != 0 {
        let _ = fs::unlink(&target);
        return TestOutcome::Warn("skip: symlink unsupported".into());
    }
    if !readlink_matches(&link, &target) {
        return TestOutcome::Fail("readlink-normal");
    }
    if !lstat_is_symlink(&link) {
        return TestOutcome::Fail("lstat-symlink");
    }
    if verify_file_bytes(&link, b"symlink-target").is_err() {
        return TestOutcome::Fail("read-through-link");
    }

    if fs::symlink(&link, &chain1) != 0 {
        return TestOutcome::Fail("symlink-chain1");
    }
    if fs::symlink(&chain1, &chain2) != 0 {
        return TestOutcome::Fail("symlink-chain2");
    }
    if !readlink_matches(&chain2, &chain1) {
        return TestOutcome::Fail("readlink-chain");
    }
    if verify_file_bytes(&chain2, b"symlink-target").is_err() {
        return TestOutcome::Fail("read-through-chain");
    }

    let missing = format!("{}/missing-target", dir);
    if fs::symlink(&missing, &dangling) != 0 {
        return TestOutcome::Fail("symlink-dangling");
    }
    if !readlink_matches(&dangling, &missing) {
        return TestOutcome::Fail("readlink-dangling");
    }
    if fs::open(&dangling, 0) != u32::MAX {
        return TestOutcome::Fail("dangling-open-succeeded");
    }

    if fs::symlink(&self_loop, &self_loop) != 0 {
        return TestOutcome::Fail("symlink-self-loop");
    }
    if fs::open(&self_loop, 0) != u32::MAX {
        return TestOutcome::Fail("self-loop-open-succeeded");
    }

    if fs::symlink(&cycle_b, &cycle_a) != 0 || fs::symlink(&cycle_a, &cycle_b) != 0 {
        return TestOutcome::Fail("symlink-cycle");
    }
    if fs::open(&cycle_a, 0) != u32::MAX || fs::open(&cycle_b, 0) != u32::MAX {
        return TestOutcome::Fail("cycle-open-succeeded");
    }

    let mut long_target = String::from("/tmp/");
    for _ in 0..180 {
        long_target.push('x');
    }
    if fs::symlink(&long_target, &long_link) != 0 {
        return TestOutcome::Fail("symlink-long-target");
    }
    if !readlink_matches(&long_link, &long_target) {
        return TestOutcome::Fail("readlink-long-target");
    }

    if !cfg.keep {
        for path in [
            &long_link, &cycle_a, &cycle_b, &self_loop, &dangling, &chain2, &chain1, &link, &target,
        ] {
            let _ = fs::unlink(path);
        }
    }

    TestOutcome::Ok("normal/link-chain/dangling/ELOOP/long-target ok".into())
}

fn readlink_matches(path: &str, expected: &str) -> bool {
    let mut buf = [0u8; 257];
    let n = fs::readlink(path, &mut buf);
    if n == u32::MAX || n as usize != expected.len() {
        return false;
    }
    &buf[..n as usize] == expected.as_bytes()
}

fn lstat_is_symlink(path: &str) -> bool {
    let mut stat = [0u32; 7];
    fs::lstat(path, &mut stat) == 0 && (stat[2] & 1) != 0
}

fn run_scratch_fsck(cfg: &Config) -> Result<(), &'static str> {
    let device_id = parse_u32(&cfg.scratch_device).ok_or("scratch-device-id")?;
    let (bin, args) = if cfg.scratch_fs.as_str() == "exfat" {
        (
            "/System/sbin/fsck.exfat",
            format!("--device {} --json", device_id),
        )
    } else {
        let sectors = device_size_sectors(device_id).ok_or("scratch-device-size")?;
        (
            "/System/sbin/fsck.corefs",
            format!("--device {} --capacity {} --json", device_id, sectors * 512),
        )
    };
    let tid = process::spawn(bin, &args);
    if tid == u32::MAX {
        return Err("spawn-fsck");
    }
    if process::waitpid(tid) == 0 {
        Ok(())
    } else {
        Err("fsck-failed")
    }
}

fn run_corefs_scrub(cfg: &Config) -> Result<(), &'static str> {
    let device_id = parse_u32(&cfg.scratch_device).ok_or("scratch-device-id")?;
    let sectors = device_size_sectors(device_id).ok_or("scratch-device-size")?;
    let args = format!(
        "--device {} --capacity {} --mode read-only --json",
        device_id,
        sectors * 512
    );
    let tid = process::spawn("/System/sbin/corefs-scrub", &args);
    if tid == u32::MAX {
        return Err("spawn-scrub");
    }
    if process::waitpid(tid) == 0 {
        Ok(())
    } else {
        Err("scrub-failed")
    }
}

fn fsx_random_model_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-fsx-random.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let ops = match cfg.profile {
        Profile::Quick => FSX_QUICK_OPS,
        Profile::Normal => FSX_NORMAL_OPS,
        Profile::Heavy => FSX_HEAVY_OPS,
        Profile::Soak => FSX_HEAVY_OPS,
    };
    let max_len = match cfg.profile {
        Profile::Quick => 128 * 1024usize,
        Profile::Normal => 512 * 1024usize,
        Profile::Heavy => 2 * 1024 * 1024usize,
        Profile::Soak => 2 * 1024 * 1024usize,
    };
    let mut rng = Lcg::new(0x4653_5831);
    let mut model = Vec::new();
    let mut write_buf = Vec::new();
    write_buf.resize(16 * 1024, 0);

    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-create");
    }
    fs::close(fd);

    for op in 0..ops {
        match rng.next() % 6 {
            0 | 1 | 2 => {
                let len = rng.range(1, write_buf.len() as u32) as usize;
                let limit = max_len.saturating_sub(len).max(1);
                let off = rng.range(0, limit as u32) as usize;
                fill_pattern(&mut write_buf[..len], off as u32, 0xF5F5_0000 ^ op);
                write_at(&path, off as u32, &write_buf[..len])?;
                if off > model.len() {
                    model.resize(off, 0);
                }
                if off + len > model.len() {
                    model.resize(off + len, 0);
                }
                model[off..off + len].copy_from_slice(&write_buf[..len]);
            }
            3 => {
                let len = rng.range(1, 8192) as usize;
                fill_pattern(&mut write_buf[..len], model.len() as u32, 0xA9A9_0000 ^ op);
                append_to_file(&path, &write_buf[..len])?;
                model.extend_from_slice(&write_buf[..len]);
                if model.len() > max_len {
                    model.clear();
                    truncate_to_zero(&path)?;
                }
            }
            4 => {
                if !model.is_empty() {
                    let off = rng.range(0, model.len() as u32) as usize;
                    let max_read = (model.len() - off).min(8192);
                    let len = rng.range(1, max_read as u32) as usize;
                    if let Err(err) = verify_read_at(&path, off as u32, &model[off..off + len]) {
                        println!(
                            "    fsx-fail: op={} read off={} len={} model_len={} err={}",
                            op,
                            off,
                            len,
                            model.len(),
                            err
                        );
                        return Err("verify-read");
                    }
                }
            }
            _ => {
                truncate_to_zero(&path)?;
                model.clear();
            }
        }

        if op % 31 == 0 {
            fs::sync();
            if let Err(err) = verify_file_bytes(&path, &model) {
                println!(
                    "    fsx-fail: op={} checkpoint model_len={} err={}",
                    op,
                    model.len(),
                    err
                );
                return Err("verify-checkpoint");
            }
        }
    }

    fs::sync();
    if let Err(err) = verify_file_bytes(&path, &model) {
        println!(
            "    fsx-fail: final ops={} model_len={} err={}",
            ops,
            model.len(),
            err
        );
        return Err("verify-final");
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!("{} deterministische fsx-Operationen ok", ops))
}

fn sparse_eof_gap_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-sparse-gap.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let gap = 24 * 1024u32;
    let tail_len = 1024usize;
    let mut tail = [0u8; 1024];
    fill_pattern(&mut tail, gap, 0x5A5A_0001);
    write_at(&path, gap, &tail)?;
    fs::sync();

    if stat_size(&path)? != gap + tail_len as u32 {
        return Err("stat-size");
    }

    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = [0xCCu8; 4096];
    let mut off = 0u32;
    while off < gap {
        let want = (gap - off).min(buf.len() as u32) as usize;
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("read-gap");
        }
        if buf[..want].iter().any(|&b| b != 0) {
            fs::close(fd);
            return Err("gap-not-zero");
        }
        off += want as u32;
    }
    let n = fs::read(fd, &mut buf[..tail_len]);
    fs::close(fd);
    if n != tail_len as u32 || buf[..tail_len] != tail {
        return Err("tail-verify");
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok("EOF-Erweiterung mit Null-Gap ok".into())
}

fn sparse_hole_matrix_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-sparse-hole-matrix.bin", cfg.dir);
    let _ = fs::unlink(&path);

    let head = b"sparse-head";
    let mid = b"sparse-mid";
    let tail = b"sparse-tail";
    let mid_off = 32 * 1024u32;
    let tail_off = 96 * 1024u32;

    write_at(&path, 0, head)?;
    write_at(&path, tail_off, tail)?;
    fs::sync();

    if stat_size(&path)? != tail_off + tail.len() as u32 {
        return Err("stat-tail-size");
    }
    verify_read_at(&path, 0, head)?;
    verify_zero_range(&path, head.len() as u32, mid_off - head.len() as u32)?;
    verify_zero_range(&path, mid_off, tail_off - mid_off)?;
    verify_read_at(&path, tail_off, tail)?;

    write_at(&path, mid_off, mid)?;
    if stat_size(&path)? != tail_off + tail.len() as u32 {
        return Err("stat-mid-size");
    }
    verify_read_at(&path, 0, head)?;
    verify_zero_range(&path, head.len() as u32, mid_off - head.len() as u32)?;
    verify_read_at(&path, mid_off, mid)?;
    verify_zero_range(
        &path,
        mid_off + mid.len() as u32,
        tail_off - mid_off - mid.len() as u32,
    )?;
    verify_read_at(&path, tail_off, tail)?;

    if fs::truncate(&path) != 0 {
        return Err("truncate-zero");
    }
    if stat_size(&path)? != 0 {
        return Err("truncate-zero-size");
    }

    write_at(&path, 4096, tail)?;
    if stat_size(&path)? != 4096 + tail.len() as u32 {
        return Err("stat-regrow-size");
    }
    verify_zero_range(&path, 0, 4096)?;
    verify_read_at(&path, 4096, tail)?;

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok("Head/Mid/Tail-Holes, Teil-Overwrite und Truncate-Regrow ok".into())
}

fn fsstress_metadata_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-fsstress-meta", cfg.dir);
    let _ = fs::mkdir(&dir);
    let count = match cfg.profile {
        Profile::Quick => 24u32,
        Profile::Normal => 96u32,
        Profile::Heavy => 256u32,
        Profile::Soak => 256u32,
    };

    for i in 0..count {
        let a = format!("{}/a{:03}.dat", dir, i);
        let b = format!("{}/b{:03}.dat", dir, i);
        let c = format!("{}/c{:03}.dat", dir, i);
        let _ = fs::unlink(&a);
        let _ = fs::unlink(&b);
        let _ = fs::unlink(&c);
    }

    let mut body = [0u8; 257];
    for i in 0..count {
        let path = format!("{}/a{:03}.dat", dir, i);
        let len = 1 + (i as usize % body.len());
        fill_pattern(&mut body[..len], i, 0xD17E_0001);
        write_bytes_file(&path, &body[..len])?;
    }
    for i in 0..count {
        if i % 3 == 0 {
            let from = format!("{}/a{:03}.dat", dir, i);
            let to = format!("{}/b{:03}.dat", dir, i);
            if fs::rename(&from, &to) != 0 {
                return Err("rename-a-b");
            }
        } else if i % 3 == 1 {
            let path = format!("{}/a{:03}.dat", dir, i);
            if fs::unlink(&path) != 0 {
                return Err("unlink");
            }
            let path = format!("{}/c{:03}.dat", dir, i);
            write_bytes_file(&path, b"recreated")?;
        }
    }
    fs::sync();

    let mut found = 0u32;
    for i in 0..count {
        let name = if i % 3 == 0 {
            format!("b{:03}.dat", i)
        } else if i % 3 == 1 {
            format!("c{:03}.dat", i)
        } else {
            format!("a{:03}.dat", i)
        };
        if dir_contains(&dir, &name) {
            found += 1;
        }
    }
    if found != count {
        return Err("readdir-count");
    }

    for i in 0..count {
        let path = if i % 3 == 0 {
            format!("{}/b{:03}.dat", dir, i)
        } else if i % 3 == 1 {
            format!("{}/c{:03}.dat", dir, i)
        } else {
            format!("{}/a{:03}.dat", dir, i)
        };
        let expected = if i % 3 == 1 {
            9
        } else {
            1 + (i % body.len() as u32)
        };
        if stat_size(&path)? != expected {
            return Err("stat-size");
        }
    }

    if !cfg.keep {
        for i in 0..count {
            let path = if i % 3 == 0 {
                format!("{}/b{:03}.dat", dir, i)
            } else if i % 3 == 1 {
                format!("{}/c{:03}.dat", dir, i)
            } else {
                format!("{}/a{:03}.dat", dir, i)
            };
            let _ = fs::unlink(&path);
        }
    }
    Ok(format!(
        "{} create/rename/unlink/readdir Operationen ok",
        count
    ))
}

fn close_reopen_sync_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-close-reopen-sync.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let rounds = match cfg.profile {
        Profile::Quick => 12u32,
        Profile::Normal => 48u32,
        Profile::Heavy => 192u32,
        Profile::Soak => 192u32,
    };
    let mut expected = Vec::new();
    let mut buf = [0u8; 1536];
    for round in 0..rounds {
        let len = 128 + ((round as usize * 37) % (buf.len() - 128));
        fill_pattern(&mut buf[..len], expected.len() as u32, 0xC105_E000 ^ round);
        append_to_file(&path, &buf[..len])?;
        expected.extend_from_slice(&buf[..len]);
        if round % 4 == 0 {
            fs::sync();
        }
        verify_file_bytes(&path, &expected)?;
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!("{} Close/Reopen-Zyklen ok", rounds))
}

fn open_unlink_rename_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-open-unlink-rename", cfg.dir);
    let dir2 = format!("{}/full-open-unlink-rename-2", cfg.dir);
    let _ = fs::mkdir(&dir);
    let _ = fs::mkdir(&dir2);

    let open_unlink = format!("{}/open-unlink.txt", dir);
    let rename_fd_old = format!("{}/rename-fd-old.txt", dir);
    let rename_fd_new = format!("{}/rename-fd-new.txt", dir);
    let overwrite_a = format!("{}/overwrite-a.txt", dir);
    let overwrite_b = format!("{}/overwrite-b.txt", dir);
    let self_path = format!("{}/self-rename.txt", dir);
    let cross_a = format!("{}/cross-a.txt", dir);
    let cross_b = format!("{}/cross-a.txt", dir2);
    let missing = format!("{}/missing.txt", dir);
    let missing_dir_target = format!("{}/missing-dir/target.txt", dir);

    for path in [
        &open_unlink,
        &rename_fd_old,
        &rename_fd_new,
        &overwrite_a,
        &overwrite_b,
        &self_path,
        &cross_a,
        &cross_b,
    ] {
        let _ = fs::unlink(path);
    }

    write_bytes_file(&open_unlink, b"fd survives unlink")?;
    let fd = fs::open(&open_unlink, 0);
    if fd == u32::MAX {
        return Err("open-unlink-open");
    }
    if fs::unlink(&open_unlink) != 0 {
        fs::close(fd);
        return Err("open-unlink-unlink");
    }
    let mut stat = [0u32; 7];
    if fs::stat(&open_unlink, &mut stat) == 0 {
        fs::close(fd);
        return Err("open-unlink-path-visible");
    }
    read_fd_exact(fd, b"fd survives unlink")?;
    fs::close(fd);
    write_bytes_file(&open_unlink, b"new name content")?;
    verify_file_bytes(&open_unlink, b"new name content")?;

    write_bytes_file(&rename_fd_old, b"fd survives rename")?;
    let fd = fs::open(&rename_fd_old, 0);
    if fd == u32::MAX {
        return Err("open-rename-open");
    }
    if fs::rename(&rename_fd_old, &rename_fd_new) != 0 {
        fs::close(fd);
        return Err("open-rename-rename");
    }
    if fs::stat(&rename_fd_old, &mut stat) == 0 {
        fs::close(fd);
        return Err("open-rename-old-visible");
    }
    verify_file_bytes(&rename_fd_new, b"fd survives rename")?;
    read_fd_exact(fd, b"fd survives rename")?;
    fs::close(fd);

    write_bytes_file(&overwrite_a, b"replacement")?;
    write_bytes_file(&overwrite_b, b"old-target")?;
    if fs::rename(&overwrite_a, &overwrite_b) != 0 {
        return Err("rename-overwrite");
    }
    if fs::stat(&overwrite_a, &mut stat) == 0 {
        return Err("rename-overwrite-source-visible");
    }
    verify_file_bytes(&overwrite_b, b"replacement")?;

    write_bytes_file(&self_path, b"self")?;
    if fs::rename(&self_path, &self_path) != 0 {
        return Err("rename-self");
    }
    verify_file_bytes(&self_path, b"self")?;

    write_bytes_file(&cross_a, b"cross-directory")?;
    if fs::rename(&cross_a, &cross_b) != 0 {
        return Err("rename-cross-dir");
    }
    if fs::stat(&cross_a, &mut stat) == 0 {
        return Err("rename-cross-old-visible");
    }
    verify_file_bytes(&cross_b, b"cross-directory")?;

    if fs::rename(&missing, &format!("{}/still-missing.txt", dir)) == 0 {
        return Err("rename-missing-source-succeeded");
    }
    if fs::rename(&self_path, &missing_dir_target) == 0 {
        return Err("rename-missing-dir-succeeded");
    }
    verify_file_bytes(&self_path, b"self")?;

    if !cfg.keep {
        for path in [
            &open_unlink,
            &rename_fd_new,
            &overwrite_b,
            &self_path,
            &cross_b,
        ] {
            let _ = fs::unlink(path);
        }
    }

    Ok("FD nach unlink/rename, overwrite, self/cross-dir und Fehlerpfade ok".into())
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

fn metadata_perf_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-metadata-perf", cfg.dir);
    let _ = fs::mkdir(&dir);
    let count = match cfg.profile {
        Profile::Quick => 64u32,
        Profile::Normal => 256u32,
        Profile::Heavy => 1024u32,
        Profile::Soak => 1024u32,
    };
    for i in 0..count {
        let _ = fs::unlink(&format!("{}/p{:04}.dat", dir, i));
        let _ = fs::unlink(&format!("{}/r{:04}.dat", dir, i));
    }

    let create_start = sys::uptime_ms();
    for i in 0..count {
        let path = format!("{}/p{:04}.dat", dir, i);
        write_bytes_file(&path, &[(i & 0xff) as u8])?;
    }
    let create_ms = elapsed_ms(create_start);

    let stat_start = sys::uptime_ms();
    for i in 0..count {
        let path = format!("{}/p{:04}.dat", dir, i);
        if stat_size(&path)? != 1 {
            return Err("stat-size");
        }
    }
    let stat_ms = elapsed_ms(stat_start);

    let rename_start = sys::uptime_ms();
    for i in 0..count {
        let from = format!("{}/p{:04}.dat", dir, i);
        let to = format!("{}/r{:04}.dat", dir, i);
        if fs::rename(&from, &to) != 0 {
            return Err("rename");
        }
    }
    let rename_ms = elapsed_ms(rename_start);

    let readdir_start = sys::uptime_ms();
    let seen = count_dir_entries(&dir, "r")?;
    let readdir_ms = elapsed_ms(readdir_start);
    if seen != count {
        return Err("readdir-count");
    }

    let unlink_start = sys::uptime_ms();
    for i in 0..count {
        let path = format!("{}/r{:04}.dat", dir, i);
        if fs::unlink(&path) != 0 {
            return Err("unlink");
        }
    }
    let unlink_ms = elapsed_ms(unlink_start);

    Ok(format!(
        "{} Eintraege: create={} ops/s, stat={} ops/s, rename={} ops/s, readdir={} entries/s, unlink={} ops/s",
        count,
        ops_per_s(count, create_ms),
        ops_per_s(count, stat_ms),
        ops_per_s(count, rename_ms),
        ops_per_s(count, readdir_ms),
        ops_per_s(count, unlink_ms)
    ))
}

fn sequential_io_perf_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-sequential-perf.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let total = match cfg.profile {
        Profile::Quick => 2 * 1024 * 1024u32,
        Profile::Normal => cfg.total_bytes.max(8 * 1024 * 1024),
        Profile::Heavy => cfg.total_bytes.max(32 * 1024 * 1024),
        Profile::Soak => cfg.total_bytes.max(32 * 1024 * 1024),
    };
    let chunk = 64 * 1024usize;
    let seed = cfg.seed ^ 0x5E90_1001;
    let mut buf = Vec::new();
    buf.resize(chunk, 0);

    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }
    let write_start = sys::uptime_ms();
    let mut offset = 0u32;
    while offset < total {
        let len = (total - offset).min(chunk as u32) as usize;
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
    let write_ms = elapsed_ms(write_start);

    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let read_start = sys::uptime_ms();
    offset = 0;
    while offset < total {
        let len = (total - offset).min(chunk as u32) as usize;
        let n = fs::read(fd, &mut buf[..len]);
        if n != len as u32 {
            fs::close(fd);
            return Err("read");
        }
        if verify_pattern(&buf[..len], offset, seed).is_some() {
            fs::close(fd);
            return Err("verify");
        }
        offset += len as u32;
    }
    fs::close(fd);
    let read_ms = elapsed_ms(read_start);

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!(
        "{} KB sequenziell: write={} KB/s read={} KB/s chunk={}K",
        total / 1024,
        kb_per_s(total, write_ms),
        kb_per_s(total, read_ms),
        chunk / 1024
    ))
}

fn random_overwrite_perf_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-random-overwrite-perf.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let total = match cfg.profile {
        Profile::Quick => 512 * 1024u32,
        Profile::Normal => 2 * 1024 * 1024u32,
        Profile::Heavy => 8 * 1024 * 1024u32,
        Profile::Soak => 8 * 1024 * 1024u32,
    };
    let ops = match cfg.profile {
        Profile::Quick => 64u32,
        Profile::Normal => 256u32,
        Profile::Heavy => 1024u32,
        Profile::Soak => 1024u32,
    };
    let patch_len = 4096usize;
    write_pattern_file(&path, total, 64 * 1024, cfg.seed ^ 0xB45E_0001)?;

    let fd = fs::open(&path, fs::O_WRITE);
    if fd == u32::MAX {
        return Err("open-write");
    }
    let mut rng = Lcg::new(cfg.seed ^ 0x0E41_2026);
    let mut buf = [0u8; 4096];
    let started = sys::uptime_ms();
    for op in 0..ops {
        let max_slot = total.saturating_sub(patch_len as u32) / patch_len as u32;
        let slot = rng.range(0, max_slot.max(1) - 1);
        let off = slot.saturating_mul(patch_len as u32);
        fill_pattern(&mut buf, off, cfg.seed ^ 0x0F0F_0000 ^ op);
        if fs::lseek(fd, off as i32, fs::SEEK_SET) != off {
            fs::close(fd);
            return Err("seek");
        }
        if fs::write(fd, &buf) != patch_len as u32 {
            fs::close(fd);
            return Err("write");
        }
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);
    let ms = elapsed_ms(started);

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!(
        "{} x {}K Random-Overwrites: {} ops/s",
        ops,
        patch_len / 1024,
        ops_per_s(ops, ms)
    ))
}

fn sync_latency_perf_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-sync-latency", cfg.dir);
    let _ = fs::mkdir(&dir);
    let rounds = match cfg.profile {
        Profile::Quick => 8u32,
        Profile::Normal => 24u32,
        Profile::Heavy => 64u32,
        Profile::Soak => 64u32,
    };
    let mut fsync_min = u32::MAX;
    let mut fsync_max = 0u32;
    let mut fsync_sum = 0u32;
    let mut sync_min = u32::MAX;
    let mut sync_max = 0u32;
    let mut sync_sum = 0u32;
    let mut buf = [0u8; 4096];

    for round in 0..rounds {
        let path = format!("{}/sync{:03}.bin", dir, round);
        let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            return Err("open");
        }
        fill_pattern(&mut buf, round * 4096, cfg.seed ^ 0x51C0_0001);
        if fs::write(fd, &buf) != buf.len() as u32 {
            fs::close(fd);
            return Err("write");
        }
        let started = sys::uptime_ms();
        if !fs::fsync(fd as i32) {
            fs::close(fd);
            return Err("fsync");
        }
        let fsync_ms = elapsed_ms(started);
        fs::close(fd);
        fsync_min = fsync_min.min(fsync_ms);
        fsync_max = fsync_max.max(fsync_ms);
        fsync_sum = fsync_sum.saturating_add(fsync_ms);

        let started = sys::uptime_ms();
        fs::sync();
        let sync_ms = elapsed_ms(started);
        sync_min = sync_min.min(sync_ms);
        sync_max = sync_max.max(sync_ms);
        sync_sum = sync_sum.saturating_add(sync_ms);
    }

    if !cfg.keep {
        for round in 0..rounds {
            let _ = fs::unlink(&format!("{}/sync{:03}.bin", dir, round));
        }
    }
    Ok(format!(
        "{} Runden: fsync min/avg/max={}/{}/{} ms, sync min/avg/max={}/{}/{} ms",
        rounds,
        fsync_min,
        fsync_sum / rounds,
        fsync_max,
        sync_min,
        sync_sum / rounds,
        sync_max
    ))
}

fn readdir_while_mutating_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-readdir-mutate", cfg.dir);
    let _ = fs::mkdir(&dir);
    let rounds = match cfg.profile {
        Profile::Quick => 32u32,
        Profile::Normal => 128u32,
        Profile::Heavy => 512u32,
        Profile::Soak => 512u32,
    };

    for i in 0..4u32 {
        let path = format!("{}/sentinel{:02}.dat", dir, i);
        write_bytes_file(&path, b"sentinel")?;
    }
    for i in 0..32u32 {
        let _ = fs::unlink(&format!("{}/tmp{:03}.dat", dir, i));
        let _ = fs::unlink(&format!("{}/ren{:03}.dat", dir, i));
    }

    let mut sentinel_hits = [0u32; 4];
    for round in 0..rounds {
        let slot = round % 32;
        let tmp = format!("{}/tmp{:03}.dat", dir, slot);
        let ren = format!("{}/ren{:03}.dat", dir, slot);
        let _ = fs::unlink(&tmp);
        let _ = fs::unlink(&ren);

        write_bytes_file(&tmp, b"mutating")?;
        validate_readdir_snapshot(&dir, &mut sentinel_hits)?;
        if fs::rename(&tmp, &ren) != 0 {
            return Err("rename-mutating");
        }
        validate_readdir_snapshot(&dir, &mut sentinel_hits)?;
        if round % 3 == 0 {
            let _ = fs::unlink(&ren);
        }
        validate_readdir_snapshot(&dir, &mut sentinel_hits)?;
    }

    for hits in &sentinel_hits {
        if *hits < rounds {
            return Err("sentinel-visibility");
        }
    }

    if !cfg.keep {
        for i in 0..4u32 {
            let _ = fs::unlink(&format!("{}/sentinel{:02}.dat", dir, i));
        }
        for i in 0..32u32 {
            let _ = fs::unlink(&format!("{}/tmp{:03}.dat", dir, i));
            let _ = fs::unlink(&format!("{}/ren{:03}.dat", dir, i));
        }
    }

    Ok(format!(
        "{} Mutationsrunden mit Readdir-Snapshots ok",
        rounds
    ))
}

fn large_directory_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-large-dir", cfg.dir);
    let _ = fs::mkdir(&dir);
    let count = match cfg.profile {
        Profile::Quick => 128u32,
        Profile::Normal => 512u32,
        Profile::Heavy => 1536u32,
        Profile::Soak => 1536u32,
    };
    let started = sys::uptime_ms();

    for i in 0..count {
        let _ = fs::unlink(&format!("{}/entry{:04}.dat", dir, i));
    }
    for i in 0..count {
        let path = format!("{}/entry{:04}.dat", dir, i);
        let byte = (i ^ 0x5A) as u8;
        write_bytes_file(&path, &[byte])?;
    }
    fs::sync();

    let seen = count_dir_entries(&dir, "entry")?;
    if seen != count {
        return Err("large-dir-count");
    }
    for i in 0..count {
        if i % 17 == 0 || i + 1 == count {
            let path = format!("{}/entry{:04}.dat", dir, i);
            verify_file_bytes(&path, &[(i ^ 0x5A) as u8])?;
        }
    }

    let create_ms = sys::uptime_ms().saturating_sub(started);
    let cleanup_started = sys::uptime_ms();
    if !cfg.keep {
        for i in 0..count {
            let _ = fs::unlink(&format!("{}/entry{:04}.dat", dir, i));
        }
    }
    let cleanup_ms = sys::uptime_ms().saturating_sub(cleanup_started);

    Ok(format!(
        "{} Eintraege, create/stat/readdir={} ms, cleanup={} ms",
        count, create_ms, cleanup_ms
    ))
}

fn long_name_case(cfg: &Config) -> Result<String, &'static str> {
    let dir = format!("{}/full-long-names", cfg.dir);
    let _ = fs::mkdir(&dir);
    let base_len = dir.len() + 1;
    let path_budget = 255usize.saturating_sub(base_len);
    let mut tested = 0u32;

    for &requested in &[63usize, 127, 180, 220] {
        let name_len = requested.min(path_budget);
        if name_len < 16 {
            continue;
        }
        let name = repeated_name("long", name_len);
        let path = format!("{}/{}", dir, name);
        let _ = fs::unlink(&path);
        let payload = [name_len as u8, requested as u8, 0xA5, 0x5A];
        write_bytes_file(&path, &payload)?;
        verify_file_bytes(&path, &payload)?;
        if !dir_contains_long(&dir, &name) {
            return Err("long-name-readdir");
        }
        let mut stat = [0u32; 7];
        if fs::stat(&path, &mut stat) != 0 || stat[1] != payload.len() as u32 {
            return Err("long-name-stat");
        }
        tested += 1;
        if !cfg.keep {
            let _ = fs::unlink(&path);
        }
    }

    let case_dir = format!("{}/case-matrix", dir);
    let _ = fs::mkdir(&case_dir);
    let upper = format!("{}/CaseName.TXT", case_dir);
    let lower = format!("{}/casename.txt", case_dir);
    let _ = fs::unlink(&upper);
    let _ = fs::unlink(&lower);
    write_bytes_file(&upper, b"upper")?;
    let lower_create = write_bytes_file(&lower, b"lower");
    let case_detail = if lower_create.is_ok() {
        let upper_data = read_exact_file(&upper, 5)?;
        let lower_data = read_exact_file(&lower, 5)?;
        if upper_data.as_slice() == b"upper" && lower_data.as_slice() == b"lower" {
            "case-sensitive"
        } else {
            "case-folded"
        }
    } else {
        "case-collision"
    };
    if !cfg.keep {
        let _ = fs::unlink(&upper);
        let _ = fs::unlink(&lower);
    }

    if tested == 0 {
        return Err("long-name-budget");
    }
    Ok(format!(
        "{} lange Pfadnamen bis {} Byte, {}",
        tested, path_budget, case_detail
    ))
}

fn permission_metadata_case(cfg: &Config) -> TestOutcome {
    let dir = format!("{}/full-permission-metadata", cfg.dir);
    let _ = fs::mkdir(&dir);
    let path = format!("{}/perm.txt", dir);
    let _ = fs::unlink(&path);

    if write_bytes_file(&path, b"metadata").is_err() {
        return TestOutcome::Fail("write");
    }

    let mut before = [0u32; 7];
    if fs::stat(&path, &mut before) != 0 {
        return TestOutcome::Fail("stat-before");
    }

    let chmod_supported = fs::chmod(&path, 0o640) == 0;
    if chmod_supported {
        if !stat_mode_is(&path, 0o640) {
            return TestOutcome::Fail("chmod-stat");
        }
        fs::sync();
        if !stat_mode_is(&path, 0o640) {
            return TestOutcome::Fail("chmod-sync-stat");
        }
        if verify_file_bytes(&path, b"metadata").is_err() {
            return TestOutcome::Fail("chmod-content");
        }
        if fs::chmod(&path, 0o600) != 0 || !stat_mode_is(&path, 0o600) {
            return TestOutcome::Fail("chmod-second");
        }
    }

    let chown_supported = fs::chown(&path, 12, 34) == 0;
    if chown_supported {
        let mut after = [0u32; 7];
        if fs::stat(&path, &mut after) != 0 {
            return TestOutcome::Fail("chown-stat");
        }
        if after[3] != 12 || after[4] != 34 {
            return TestOutcome::Fail("chown-values");
        }
        fs::sync();
        if fs::stat(&path, &mut after) != 0 || after[3] != 12 || after[4] != 34 {
            return TestOutcome::Fail("chown-sync-stat");
        }
    }

    let mtime_before = before[6];
    if append_to_file(&path, b"-touch").is_err() {
        return TestOutcome::Fail("append");
    }
    let mut after_write = [0u32; 7];
    if fs::stat(&path, &mut after_write) != 0 {
        return TestOutcome::Fail("stat-after-write");
    }
    if after_write[1] != 14 {
        return TestOutcome::Fail("stat-size-after-write");
    }
    let mtime_detail = if after_write[6] >= mtime_before {
        "mtime-ok"
    } else {
        return TestOutcome::Fail("mtime-regressed");
    };

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }

    let detail = format!(
        "chmod={}, chown={}, {}",
        if chmod_supported { "ok" } else { "unsupported" },
        if chown_supported { "ok" } else { "unsupported" },
        mtime_detail
    );
    if chmod_supported || chown_supported {
        TestOutcome::Ok(detail)
    } else {
        TestOutcome::Warn(detail)
    }
}

fn fsync_ordering_case(cfg: &Config) -> TestOutcome {
    let dir = format!("{}/full-fsync-ordering", cfg.dir);
    let _ = fs::mkdir(&dir);

    let close_only = format!("{}/close-only.txt", dir);
    let file_fsync = format!("{}/file-fsync.txt", dir);
    let global_sync = format!("{}/global-sync.txt", dir);
    let rename_file_tmp = format!("{}/rename-file.tmp", dir);
    let rename_file_final = format!("{}/rename-file.txt", dir);
    let rename_sync_tmp = format!("{}/rename-sync.tmp", dir);
    let rename_sync_final = format!("{}/rename-sync.txt", dir);

    for path in [
        &close_only,
        &file_fsync,
        &global_sync,
        &rename_file_tmp,
        &rename_file_final,
        &rename_sync_tmp,
        &rename_sync_final,
    ] {
        let _ = fs::unlink(path);
    }

    if write_file_ordered(&close_only, b"close-only", SyncMode::CloseOnly).is_err() {
        return TestOutcome::Fail("close-only-write");
    }
    if verify_file_bytes(&close_only, b"close-only").is_err() {
        return TestOutcome::Fail("close-only-verify");
    }
    fs::sync();
    if verify_file_bytes(&close_only, b"close-only").is_err() {
        return TestOutcome::Fail("close-only-sync-verify");
    }

    if write_file_ordered(&file_fsync, b"file-fsync", SyncMode::FileFsync).is_err() {
        return TestOutcome::Fail("file-fsync-write");
    }
    if verify_file_bytes(&file_fsync, b"file-fsync").is_err() {
        return TestOutcome::Fail("file-fsync-verify");
    }

    if write_file_ordered(&global_sync, b"global-sync", SyncMode::GlobalSync).is_err() {
        return TestOutcome::Fail("global-sync-write");
    }
    if verify_file_bytes(&global_sync, b"global-sync").is_err() {
        return TestOutcome::Fail("global-sync-verify");
    }

    if write_file_ordered(&rename_file_tmp, b"rename-file-fsync", SyncMode::FileFsync).is_err() {
        return TestOutcome::Fail("rename-file-write");
    }
    if fs::rename(&rename_file_tmp, &rename_file_final) != 0 {
        return TestOutcome::Fail("rename-file-rename");
    }
    if stat_exists(&rename_file_tmp) {
        return TestOutcome::Fail("rename-file-old-visible");
    }
    if verify_file_bytes(&rename_file_final, b"rename-file-fsync").is_err() {
        return TestOutcome::Fail("rename-file-verify");
    }

    if write_file_ordered(&rename_sync_tmp, b"rename-global-sync", SyncMode::CloseOnly).is_err() {
        return TestOutcome::Fail("rename-sync-write");
    }
    if fs::rename(&rename_sync_tmp, &rename_sync_final) != 0 {
        return TestOutcome::Fail("rename-sync-rename");
    }
    fs::sync();
    if stat_exists(&rename_sync_tmp) {
        return TestOutcome::Fail("rename-sync-old-visible");
    }
    if verify_file_bytes(&rename_sync_final, b"rename-global-sync").is_err() {
        return TestOutcome::Fail("rename-sync-verify");
    }

    let dir_fsync = match fsync_path(&dir) {
        FsyncPathResult::Ok => "dir-fsync=ok",
        FsyncPathResult::Unsupported => "dir-fsync=unsupported",
    };

    if !cfg.keep {
        for path in [
            &close_only,
            &file_fsync,
            &global_sync,
            &rename_file_final,
            &rename_sync_final,
        ] {
            let _ = fs::unlink(path);
        }
    }

    TestOutcome::Ok(format!(
        "close/file-fsync/global-sync/rename Reihenfolgen ok, {}",
        dir_fsync
    ))
}

fn statfs_accounting_case(cfg: &Config) -> TestOutcome {
    let probe = statfs_probe_path(cfg);
    let Some(before) = fs::statfs(&probe) else {
        return TestOutcome::Warn("skip: statfs nicht verfuegbar".into());
    };

    let dir = format!("{}/full-statfs-accounting", cfg.dir);
    let _ = fs::mkdir(&dir);
    let path = format!("{}/statfs.bin", dir);
    let _ = fs::unlink(&path);

    let total = match cfg.profile {
        Profile::Quick => 128 * 1024u32,
        Profile::Normal => 512 * 1024u32,
        Profile::Heavy => 2 * 1024 * 1024u32,
        Profile::Soak => 2 * 1024 * 1024u32,
    };
    if write_pattern_file(&path, total, 32 * 1024, cfg.seed ^ 0x57A7_F500).is_err() {
        return TestOutcome::Fail("write");
    }
    fs::sync();
    let Some(after_write) = fs::statfs(&probe) else {
        let _ = fs::unlink(&path);
        return TestOutcome::Warn("statfs-after-write-unavailable".into());
    };

    if after_write.total_bytes != before.total_bytes {
        let _ = fs::unlink(&path);
        return TestOutcome::Fail("total-changed");
    }
    if after_write.free_bytes > before.free_bytes {
        let _ = fs::unlink(&path);
        return TestOutcome::Fail("free-increased-after-write");
    }
    if after_write.used_bytes < before.used_bytes {
        let _ = fs::unlink(&path);
        return TestOutcome::Fail("used-decreased-after-write");
    }

    if fs::truncate(&path) != 0 {
        let _ = fs::unlink(&path);
        return TestOutcome::Fail("truncate");
    }
    fs::sync();
    let after_truncate = fs::statfs(&probe);

    let _ = fs::unlink(&path);
    fs::sync();
    let after_unlink = fs::statfs(&probe);

    if let Some(ref t) = after_truncate {
        if t.free_bytes < after_write.free_bytes {
            return TestOutcome::Fail("free-decreased-after-truncate");
        }
    }
    if let (Some(ref t), Some(ref u)) = (&after_truncate, &after_unlink) {
        if u.free_bytes < t.free_bytes {
            return TestOutcome::Fail("free-decreased-after-unlink");
        }
    }

    let write_delta = before.free_bytes.saturating_sub(after_write.free_bytes);
    let trunc_free = after_truncate
        .map(|s| s.free_bytes / 1024)
        .unwrap_or(u64::MAX);
    let unlink_free = after_unlink
        .map(|s| s.free_bytes / 1024)
        .unwrap_or(u64::MAX);
    let detail = format!(
        "probe={} write={} KB, free {} -> {} -> {} -> {} KB",
        probe,
        total / 1024,
        before.free_bytes / 1024,
        after_write.free_bytes / 1024,
        trunc_free,
        unlink_free
    );
    if write_delta == 0 && after_write.used_bytes == before.used_bytes {
        TestOutcome::Warn(format!("{}; kein sichtbarer statfs-Delta", detail))
    } else {
        TestOutcome::Ok(detail)
    }
}

fn path_resolution_case(cfg: &Config) -> TestOutcome {
    let saved_cwd = match current_cwd() {
        Some(cwd) => cwd,
        None => return TestOutcome::Warn("skip: getcwd nicht verfuegbar".into()),
    };

    let dir = format!("{}/full-path-resolution", cfg.dir);
    let base = format!("{}/base", dir);
    let sub = format!("{}/sub", base);
    mkdir_parents(&sub);

    let target = format!("{}/target.txt", sub);
    if write_bytes_file(&target, b"path-ok").is_err() {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("write-target");
    }

    let abs_dot = format!("{}/./target.txt", sub);
    let abs_parent = format!("{}/../sub/target.txt", sub);
    let abs_double = format!("{}//target.txt", sub);
    if verify_file_bytes(&abs_dot, b"path-ok").is_err() {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("abs-dot");
    }
    if verify_file_bytes(&abs_parent, b"path-ok").is_err() {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("abs-parent");
    }
    if verify_file_bytes(&abs_double, b"path-ok").is_err() {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("abs-double-slash");
    }

    if fs::chdir(&base) != 0 {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Warn("skip: chdir nicht verfuegbar".into());
    }
    if verify_file_bytes("sub/target.txt", b"path-ok").is_err()
        || verify_file_bytes("./sub/../sub/target.txt", b"path-ok").is_err()
    {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("relative-base");
    }
    if fs::chdir("sub") != 0 {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("relative-chdir-sub");
    }
    if verify_file_bytes("../sub/target.txt", b"path-ok").is_err() {
        let _ = fs::chdir(&saved_cwd);
        return TestOutcome::Fail("relative-parent");
    }
    let _ = fs::chdir(&saved_cwd);

    let deep_leaf = create_deep_path(&dir, 16);
    let deep_file = format!("{}/leaf.txt", deep_leaf);
    if write_bytes_file(&deep_file, b"deep-ok").is_err()
        || verify_file_bytes(&deep_file, b"deep-ok").is_err()
    {
        return TestOutcome::Fail("deep-path");
    }

    let near_name = make_name_to_path_budget(&dir, 240);
    let near_path = format!("{}/{}", dir, near_name);
    if write_bytes_file(&near_path, b"near-limit").is_err()
        || verify_file_bytes(&near_path, b"near-limit").is_err()
    {
        return TestOutcome::Fail("near-limit");
    }
    let overlong = format!(
        "{}-this-suffix-must-not-alias-the-near-limit-file",
        near_path
    );
    let mut stat = [0u32; 7];
    if fs::stat(&overlong, &mut stat) == 0 {
        return TestOutcome::Fail("overlong-aliased");
    }

    if !cfg.keep {
        let _ = fs::unlink(&target);
        let _ = fs::unlink(&deep_file);
        let _ = fs::unlink(&near_path);
    }

    TestOutcome::Ok("relative/./../double-slash/deep/near-limit Pfade ok".into())
}

fn feature_gate_case(cfg: &Config) -> TestOutcome {
    let symlink = if symlink_supported(cfg) {
        "symlink=yes"
    } else {
        "symlink=no"
    };
    let chmod = if chmod_supported_probe(cfg) {
        "chmod=yes"
    } else {
        "chmod=no"
    };
    let chown = if chown_supported_probe(cfg) {
        "chown=yes"
    } else {
        "chown=no"
    };
    let scratch = if cfg.scratch_device.is_empty() {
        "scratch=no-device"
    } else {
        "scratch=configured"
    };
    TestOutcome::Skip(format!(
        "{}, {}, {}, {}, hardlink=unsupported, special-files=unsupported, file-mmap=unsupported, direct-io=unsupported, xattr=unsupported, overlay=unsupported, whiteout=unsupported, namespace=unsupported",
        symlink, chmod, chown, scratch
    ))
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

fn user_copy_boundary_case(cfg: &Config) -> Result<String, &'static str> {
    let path = format!("{}/full-user-copy-boundary.bin", cfg.dir);
    let _ = fs::unlink(&path);
    let total = match cfg.profile {
        Profile::Quick => 96 * 1024,
        _ => USER_COPY_BOUNDARY_TOTAL,
    };
    let seed = 0x5C0F_FEE1;
    let shifts = [1usize, 17, 4095, 4111, 8191];
    let lens = [32 * 1024usize, 64 * 1024, 33 * 1024 + 17, 7 * 1024 + 31];
    let guard = 16usize;

    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }

    let mut offset = 0u32;
    let mut round = 0usize;
    while offset < total {
        let shift = shifts[round % shifts.len()];
        let want = lens[round % lens.len()].min((total - offset) as usize);
        let mut backing = Vec::new();
        backing.resize(shift + want + guard, 0xA5);
        fill_pattern(&mut backing[shift..shift + want], offset, seed);
        let n = fs::write(fd, &backing[shift..shift + want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("write");
        }
        if !guard_intact(&backing, shift, want, 0xA5) {
            fs::close(fd);
            return Err("write-guard");
        }
        offset += want as u32;
        round += 1;
    }

    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync");
    }
    fs::close(fd);

    if stat_size(&path)? != total {
        return Err("stat-size");
    }

    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    offset = 0;
    round = 0;
    while offset < total {
        let shift = shifts[(round + 2) % shifts.len()];
        let want = lens[(round + 1) % lens.len()].min((total - offset) as usize);
        let mut backing = Vec::new();
        backing.resize(shift + want + guard, 0x5A);
        let n = fs::read(fd, &mut backing[shift..shift + want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("read-short");
        }
        if verify_pattern(&backing[shift..shift + want], offset, seed).is_some() {
            fs::close(fd);
            return Err("verify");
        }
        if !guard_intact(&backing, shift, want, 0x5A) {
            fs::close(fd);
            return Err("read-guard");
        }
        offset += want as u32;
        round += 1;
    }
    fs::close(fd);

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!(
        "{} bytes via misaligned 32K/64K user buffers ok",
        total
    ))
}

fn writeback_stream_case(cfg: &Config, chunk_size: usize) -> Result<String, &'static str> {
    let path = format!("{}/full-writeback-stream-{}.bin", cfg.dir, chunk_size);
    let _ = fs::unlink(&path);
    let total = match cfg.profile {
        Profile::Quick => 2 * 1024 * 1024,
        _ => WRITEBACK_STREAM_SIZE,
    };
    let seed = 0x1208_5491;

    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }

    let mut buf = Vec::new();
    buf.resize(chunk_size, 0);
    let mut offset = 0u32;
    while offset < total {
        let len = (total - offset).min(chunk_size as u32) as usize;
        fill_pattern(&mut buf[..len], offset, seed);
        let n = fs::write(fd, &buf[..len]);
        if n != len as u32 {
            fs::close(fd);
            return Err("write");
        }
        offset += len as u32;
    }

    // Intentionally no fsync here: this matches wget/lxe, where close()
    // must commit dirty block-cache data before the next process reads it.
    fs::close(fd);

    if stat_size(&path)? != total {
        return Err("stat-size");
    }
    verify_file_pattern_large(&path, total, seed, 32 * 1024)?;

    fs::sync();
    verify_file_pattern_large(&path, total, seed, 64 * 1024)?;

    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
    Ok(format!(
        "{} bytes via {}K writes ohne fsync, readback vor/nach sync ok",
        total,
        chunk_size / 1024
    ))
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

struct Lcg {
    state: u32,
}

impl Lcg {
    fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.state
    }

    fn range(&mut self, min: u32, max: u32) -> u32 {
        if max <= min {
            return min;
        }
        min + (self.next() % (max - min + 1))
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

fn guard_intact(buf: &[u8], start: usize, len: usize, value: u8) -> bool {
    buf[..start].iter().all(|&b| b == value) && buf[start + len..].iter().all(|&b| b == value)
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

enum SyncMode {
    CloseOnly,
    FileFsync,
    GlobalSync,
}

fn write_file_ordered(path: &str, bytes: &[u8], mode: SyncMode) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }
    if fs::write(fd, bytes) != bytes.len() as u32 {
        fs::close(fd);
        return Err("write");
    }
    match mode {
        SyncMode::CloseOnly => {}
        SyncMode::FileFsync => {
            if !fs::fsync(fd as i32) {
                fs::close(fd);
                return Err("fsync");
            }
        }
        SyncMode::GlobalSync => fs::sync(),
    }
    fs::close(fd);
    Ok(())
}

enum FsyncPathResult {
    Ok,
    Unsupported,
}

fn fsync_path(path: &str) -> FsyncPathResult {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return FsyncPathResult::Unsupported;
    }
    let ok = fs::fsync(fd as i32);
    fs::close(fd);
    if ok {
        FsyncPathResult::Ok
    } else {
        FsyncPathResult::Unsupported
    }
}

fn write_at(path: &str, offset: u32, bytes: &[u8]) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE);
    if fd == u32::MAX {
        return Err("open-write-at");
    }
    if fs::lseek(fd, offset as i32, fs::SEEK_SET) != offset {
        fs::close(fd);
        return Err("seek-write-at");
    }
    if fs::write(fd, bytes) != bytes.len() as u32 {
        fs::close(fd);
        return Err("write-at");
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync-write-at");
    }
    fs::close(fd);
    Ok(())
}

fn append_to_file(path: &str, bytes: &[u8]) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_APPEND);
    if fd == u32::MAX {
        return Err("open-append");
    }
    if fs::write(fd, bytes) != bytes.len() as u32 {
        fs::close(fd);
        return Err("append");
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync-append");
    }
    fs::close(fd);
    Ok(())
}

fn truncate_to_zero(path: &str) -> Result<(), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-truncate");
    }
    if !fs::fsync(fd as i32) {
        fs::close(fd);
        return Err("fsync-truncate");
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

fn verify_file_pattern_large(
    path: &str,
    total_bytes: u32,
    seed: u32,
    read_size: usize,
) -> Result<(), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = Vec::new();
    buf.resize(read_size.min(MAX_BLOCK), 0);
    let mut offset = 0u32;
    while offset < total_bytes {
        let want = (total_bytes - offset).min(buf.len() as u32) as usize;
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
    }
    fs::close(fd);
    Ok(())
}

fn verify_read_at(path: &str, offset: u32, expected: &[u8]) -> Result<(), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read-at");
    }
    if fs::lseek(fd, offset as i32, fs::SEEK_SET) != offset {
        fs::close(fd);
        return Err("seek-read-at");
    }
    let mut buf = Vec::new();
    buf.resize(expected.len(), 0);
    let mut done = 0usize;
    while done < expected.len() {
        let n = fs::read(fd, &mut buf[done..]);
        if n == 0 || n == u32::MAX {
            fs::close(fd);
            return Err("read-at-short");
        }
        done += (n as usize).min(expected.len() - done);
    }
    fs::close(fd);
    if buf.as_slice() != expected {
        return Err("read-at-verify");
    }
    Ok(())
}

fn verify_zero_range(path: &str, offset: u32, len: u32) -> Result<(), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read-zero");
    }
    if fs::lseek(fd, offset as i32, fs::SEEK_SET) != offset {
        fs::close(fd);
        return Err("seek-zero");
    }
    let mut buf = [0xCCu8; 2048];
    let mut done = 0u32;
    while done < len {
        let want = (len - done).min(buf.len() as u32) as usize;
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            fs::close(fd);
            return Err("read-zero");
        }
        if buf[..want].iter().any(|&b| b != 0) {
            fs::close(fd);
            return Err("zero-verify");
        }
        done += want as u32;
    }
    fs::close(fd);
    Ok(())
}

fn verify_file_bytes(path: &str, expected: &[u8]) -> Result<(), &'static str> {
    let size = stat_size(path)?;
    if size != expected.len() as u32 {
        println!(
            "    verify-file: path={} size={} expected_size={}",
            path,
            size,
            expected.len()
        );
        return Err("stat-size");
    }
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = [0u8; 4096];
    let mut done = 0usize;
    while done < expected.len() {
        let want = (expected.len() - done).min(buf.len());
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            fs::close(fd);
            println!(
                "    verify-file: path={} read_short off={} got={} want={}",
                path, done, n, want
            );
            return Err("read-short");
        }
        if buf[..want] != expected[done..done + want] {
            for i in 0..want {
                if buf[i] != expected[done + i] {
                    println!(
                        "    verify-file: path={} bad_off={} got={} expected={}",
                        path,
                        done + i,
                        buf[i],
                        expected[done + i]
                    );
                    break;
                }
            }
            fs::close(fd);
            return Err("verify");
        }
        done += want;
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

fn read_small_text_file(path: &str, max_len: usize) -> Result<String, &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut out = Vec::new();
    let mut chunk = [0u8; 128];
    while out.len() < max_len {
        let want = (max_len - out.len()).min(chunk.len());
        let n = fs::read(fd, &mut chunk[..want]);
        if n == u32::MAX {
            fs::close(fd);
            return Err("read");
        }
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n as usize]);
    }
    fs::close(fd);
    core::str::from_utf8(&out)
        .map(String::from)
        .map_err(|_| "utf8")
}

fn read_exact_file_retry(path: &str, len: usize, retries: u32) -> Result<Vec<u8>, &'static str> {
    let mut last = "open-read";
    for attempt in 0..=retries {
        match read_exact_file(path, len) {
            Ok(data) => return Ok(data),
            Err(err) => {
                last = err;
                if attempt < retries {
                    process::yield_cpu();
                }
            }
        }
    }
    Err(last)
}

fn read_fd_exact(fd: u32, expected: &[u8]) -> Result<(), &'static str> {
    if fs::lseek(fd, 0, fs::SEEK_SET) != 0 {
        return Err("fd-seek");
    }
    let mut buf = [0u8; 64];
    let mut done = 0usize;
    while done < expected.len() {
        let want = (expected.len() - done).min(buf.len());
        let n = fs::read(fd, &mut buf[..want]);
        if n != want as u32 {
            return Err("fd-read");
        }
        if buf[..want] != expected[done..done + want] {
            return Err("fd-verify");
        }
        done += want;
    }
    Ok(())
}

fn stat_size(path: &str) -> Result<u32, &'static str> {
    let mut stat = [0u32; 7];
    if fs::stat(path, &mut stat) != 0 {
        return Err("stat");
    }
    Ok(stat[1])
}

fn stat_exists(path: &str) -> bool {
    let mut stat = [0u32; 7];
    fs::stat(path, &mut stat) == 0
}

fn statfs_probe_path(cfg: &Config) -> String {
    for path in [&cfg.dir, "/tmp", "/"] {
        if fs::statfs(path).is_some() {
            return String::from(path);
        }
    }
    cfg.dir.clone()
}

fn current_cwd() -> Option<String> {
    let mut buf = [0u8; 257];
    let len = fs::getcwd(&mut buf);
    if len == u32::MAX || len == 0 {
        return None;
    }
    core::str::from_utf8(&buf[..len as usize])
        .ok()
        .map(String::from)
}

fn create_deep_path(root: &str, depth: u32) -> String {
    let mut path = String::from(root);
    for i in 0..depth {
        path.push_str(&format!("/d{:02}", i));
        let _ = fs::mkdir(&path);
    }
    path
}

fn make_name_to_path_budget(dir: &str, budget: usize) -> String {
    let mut name = String::from("near");
    while dir.len() + 1 + name.len() < budget {
        name.push('n');
    }
    name
}

fn symlink_supported(cfg: &Config) -> bool {
    let dir = format!("{}/feature-gates", cfg.dir);
    let _ = fs::mkdir(&dir);
    let target = format!("{}/symlink-target.txt", dir);
    let link = format!("{}/symlink-link.txt", dir);
    let _ = fs::unlink(&link);
    let _ = fs::unlink(&target);
    if write_bytes_file(&target, b"feature").is_err() {
        return false;
    }
    let ok = fs::symlink(&target, &link) == 0 && readlink_matches(&link, &target);
    let _ = fs::unlink(&link);
    let _ = fs::unlink(&target);
    ok
}

fn chmod_supported_probe(cfg: &Config) -> bool {
    let dir = format!("{}/feature-gates", cfg.dir);
    let _ = fs::mkdir(&dir);
    let path = format!("{}/chmod.txt", dir);
    let _ = fs::unlink(&path);
    if write_bytes_file(&path, b"feature").is_err() {
        return false;
    }
    let ok = fs::chmod(&path, 0o640) == 0 && stat_mode_is(&path, 0o640);
    let _ = fs::unlink(&path);
    ok
}

fn chown_supported_probe(cfg: &Config) -> bool {
    let dir = format!("{}/feature-gates", cfg.dir);
    let _ = fs::mkdir(&dir);
    let path = format!("{}/chown.txt", dir);
    let _ = fs::unlink(&path);
    if write_bytes_file(&path, b"feature").is_err() {
        return false;
    }
    let mut stat = [0u32; 7];
    let ok = fs::chown(&path, 12, 34) == 0
        && fs::stat(&path, &mut stat) == 0
        && stat[3] == 12
        && stat[4] == 34;
    let _ = fs::unlink(&path);
    ok
}

fn stat_size_or_zero(path: &str) -> u32 {
    stat_size(path).unwrap_or(0)
}

fn stat_mode_is(path: &str, expected: u32) -> bool {
    let mut stat = [0u32; 7];
    fs::stat(path, &mut stat) == 0 && (stat[5] & 0o777) == expected
}

fn validate_readdir_snapshot(path: &str, sentinel_hits: &mut [u32; 4]) -> Result<(), &'static str> {
    let mut buf = Vec::new();
    buf.resize(fs::READDIR_LONG_ENTRY_SIZE * 256, 0);
    let count = fs::readdir_long(path, &mut buf);
    if count == u32::MAX {
        return Err("readdir");
    }
    for i in 0..count as usize {
        let off = i * fs::READDIR_LONG_ENTRY_SIZE;
        if off + fs::READDIR_LONG_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        if name_len > 256 {
            return Err("readdir-name-len");
        }
        let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len])
            .map_err(|_| "readdir-name-utf8")?;
        if name.len() == 14 && name.starts_with("sentinel") && name.ends_with(".dat") {
            let idx = decimal2(&name.as_bytes()[8..10]);
            if idx < sentinel_hits.len() as u32 {
                sentinel_hits[idx as usize] = sentinel_hits[idx as usize].saturating_add(1);
            }
        }
    }
    Ok(())
}

fn count_dir_entries(path: &str, prefix: &str) -> Result<u32, &'static str> {
    let mut buf = Vec::new();
    buf.resize(fs::READDIR_LONG_ENTRY_SIZE * 2048, 0);
    let count = fs::readdir_long(path, &mut buf);
    if count == u32::MAX {
        return Err("readdir");
    }
    let mut found = 0u32;
    for i in 0..count as usize {
        let off = i * fs::READDIR_LONG_ENTRY_SIZE;
        if off + fs::READDIR_LONG_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        if name_len > 256 {
            return Err("readdir-name-len");
        }
        let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len])
            .map_err(|_| "readdir-name-utf8")?;
        if name.starts_with(prefix) {
            found += 1;
        }
    }
    Ok(found)
}

fn decimal2(bytes: &[u8]) -> u32 {
    if bytes.len() < 2 || !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() {
        return u32::MAX;
    }
    ((bytes[0] - b'0') as u32) * 10 + (bytes[1] - b'0') as u32
}

fn repeated_name(prefix: &str, total_len: usize) -> String {
    let mut name = String::from(prefix);
    while name.len() < total_len {
        let idx = name.len() % 26;
        name.push((b'a' + idx as u8) as char);
    }
    name
}

fn dir_contains(path: &str, needle: &str) -> bool {
    let mut buf = Vec::new();
    buf.resize(fs::READDIR_LONG_ENTRY_SIZE * 96, 0);
    let count = fs::readdir_long(path, &mut buf);
    if count == u32::MAX {
        return false;
    }
    for i in 0..count as usize {
        let off = i * fs::READDIR_LONG_ENTRY_SIZE;
        if off + fs::READDIR_LONG_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        if name_len > 256 {
            continue;
        }
        let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len]).unwrap_or("");
        if name == needle {
            return true;
        }
    }
    false
}

fn dir_contains_long(path: &str, needle: &str) -> bool {
    let mut buf = Vec::new();
    buf.resize(fs::READDIR_LONG_ENTRY_SIZE * 256, 0);
    let count = fs::readdir_long(path, &mut buf);
    if count == u32::MAX {
        return false;
    }
    for i in 0..count as usize {
        let off = i * fs::READDIR_LONG_ENTRY_SIZE;
        if off + fs::READDIR_LONG_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        if name_len > 256 {
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

fn print_summary(cfg: &Config, summary: &Summary, started: u32) {
    println!();
    println!("=== Zusammenfassung ===");
    println!("  Tests:    {}", summary.tests);
    println!("  Warns:    {}", summary.warnings);
    println!("  Skips:    {}", summary.skips);
    println!("  Fehler:   {}", summary.failures);
    println!("  Laufzeit: {} ms", elapsed_ms(started));
    if summary.failures == 0 {
        println!("  Ergebnis: PASS");
    } else {
        println!("  Ergebnis: FAIL");
        println!(
            "  Repro:    vfsstress --profile {} --seed {} --dir {} --keep",
            cfg.profile_name(),
            cfg.seed,
            cfg.dir
        );
    }
}

fn print_json_summary(cfg: &Config, summary: &Summary, elapsed: u32) {
    println!(
        "{{\"tool\":\"vfsstress\",\"version\":\"{}\",\"anyos\":\"{}\",\"profile\":\"{}\",\"dir\":\"{}\",\"seed\":{},\"seconds\":{},\"elapsed_ms\":{},\"tests\":{},\"failures\":{},\"warnings\":{},\"skips\":{},\"result\":\"{}\"}}",
        VERSION,
        anyos_version(),
        cfg.profile_name(),
        cfg.dir,
        cfg.seed,
        cfg.seconds,
        elapsed,
        summary.tests,
        summary.failures,
        summary.warnings,
        summary.skips,
        if summary.failures == 0 { "PASS" } else { "FAIL" }
    );
}

fn kb_per_s(bytes: u32, ms: u32) -> u32 {
    if ms == 0 {
        return 0;
    }
    ((bytes as u64 * 1000) / (ms as u64 * 1024)) as u32
}

fn ops_per_s(ops: u32, ms: u32) -> u32 {
    if ms == 0 {
        return ops.saturating_mul(1000);
    }
    ((ops as u64 * 1000) / ms as u64) as u32
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

fn device_size_sectors(device_id: u32) -> Option<u64> {
    let mut buf = [0u8; 4096];
    let count = sys::disk_list(&mut buf);
    if count == u32::MAX {
        return None;
    }
    let n = (count as usize).min(buf.len() / 32);
    for idx in 0..n {
        let base = idx * 32;
        if buf[base] as u32 == device_id {
            return Some(read_le64(&buf, base + 16));
        }
    }
    None
}

fn read_le64(buf: &[u8], off: usize) -> u64 {
    let mut value = 0u64;
    for i in 0..8 {
        value |= (buf[off + i] as u64) << (i * 8);
    }
    value
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
