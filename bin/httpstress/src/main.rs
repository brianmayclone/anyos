#![no_std]
#![no_main]

use alloc::format;
use anyos_std::{crypto, fs, print, println, process, sys, String, Vec};

anyos_std::entry!(main);

const VERSION: &str = "0.1";
const DEFAULT_TEST_URL: &str =
    "http://deb.debian.org/debian/dists/bookworm/main/binary-amd64/Packages.gz";
const DEFAULT_SMALL_URL: &str = "http://archive.debian.org/debian/README";
const DEFAULT_OUT_DIR: &str = "/tmp/httpstress";
const DEFAULT_WGET: &str = "/System/bin/wget";
const LARGE_GET_BUF_SIZE: usize = 12 * 1024 * 1024;
const GZIP_EXPECTED_UNPACKED_MIN: u32 = 20 * 1024 * 1024;

fn anyos_version() -> &'static str {
    option_env!("ANYOS_VERSION").unwrap_or("dev")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Quick,
    Normal,
    Heavy,
}

struct Config {
    repeat: u32,
    url: String,
    small_url: String,
    out_dir: String,
    wget_path: String,
    profile: Profile,
    keep: bool,
    gzip_check: bool,
    wget_check: bool,
    memory_rounds: u32,
    wget_rounds: u32,
}

impl Config {
    fn default() -> Self {
        Self {
            repeat: 3,
            url: String::from(DEFAULT_TEST_URL),
            small_url: String::from(DEFAULT_SMALL_URL),
            out_dir: String::from(DEFAULT_OUT_DIR),
            wget_path: String::from(DEFAULT_WGET),
            profile: Profile::Normal,
            keep: false,
            gzip_check: true,
            wget_check: true,
            memory_rounds: 3,
            wget_rounds: 3,
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

#[derive(Clone)]
struct Artifact {
    path: String,
    size: u32,
    crc32: u32,
    md5_hex: [u8; 32],
    download_ms: u32,
    status: u32,
    error: u32,
    gzip: GzipVerdict,
}

#[derive(Clone)]
struct Reference {
    size: u32,
    crc32: u32,
    md5_hex: [u8; 32],
    gzip: GzipVerdict,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GzipVerdict {
    NotChecked,
    NotGzip,
    Ok(u32),
    Failed(u32),
}

struct Summary {
    tests: u32,
    failures: u32,
    warnings: u32,
}

static mut PROGRESS_CALLS: u32 = 0;
static mut PROGRESS_LAST: u32 = 0;
static mut PROGRESS_TOTAL: u32 = 0;

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
        println!("  [OK]   {:<24} {}", name, detail);
    }

    fn warn(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.warnings += 1;
        println!("  [WARN] {:<24} {}", name, detail);
    }

    fn fail(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.failures += 1;
        println!("  [FAIL] {:<24} {}", name, detail);
    }
}

fn main() {
    let Some(mut cfg) = parse_config() else {
        return;
    };
    apply_profile(&mut cfg);

    println!();
    println!("httpstress {} - libhttp Diagnose", VERSION);
    println!("================================");
    println!("  anyOS:     {}", anyos_version());
    println!("  profile:   {}", cfg.profile_name());
    println!("  repeat:    {}", cfg.repeat);
    println!("  url:       {}", cfg.url);
    println!("  small-url: {}", cfg.small_url);
    println!("  out-dir:   {}", cfg.out_dir);
    println!(
        "  wget:      {}",
        if cfg.wget_check {
            cfg.wget_path.as_str()
        } else {
            "skip"
        }
    );
    println!();

    let started = sys::uptime_ms();
    let mut summary = Summary::new();

    if !libhttp_client::init() {
        summary.fail("libhttp init", "libhttp.so konnte nicht geladen werden");
        print_summary(&summary, started);
        return;
    }
    summary.ok("libhttp init", "loaded");

    if cfg.gzip_check && !libzip_client::init() {
        summary.warn("libzip init", "libzip.so nicht verfuegbar; gzip-check aus");
        cfg.gzip_check = false;
    }

    let out_ready = prepare_out_dir(&cfg.out_dir);
    if out_ready {
        summary.ok("out-dir", &format!("{} writable", cfg.out_dir));
    } else {
        summary.fail(
            "out-dir",
            &format!(
                "{} nicht beschreibbar; Download-Runden werden uebersprungen",
                cfg.out_dir
            ),
        );
    }

    run_small_get(&cfg, &mut summary);
    let reference = run_large_get_probe(&cfg, &mut summary);
    let memory_artifacts = run_memory_matrix(&cfg, reference.as_ref(), &mut summary);
    let artifacts = if out_ready {
        run_download_matrix(&cfg, &mut summary)
    } else {
        Vec::new()
    };
    let wget_artifacts = if out_ready && cfg.wget_check {
        run_wget_matrix(&cfg, reference.as_ref(), &mut summary)
    } else {
        Vec::new()
    };
    compare_memory_artifacts(&memory_artifacts, reference.as_ref(), &mut summary);
    compare_artifacts(&artifacts, reference.as_ref(), &mut summary);
    compare_wget_artifacts(
        &wget_artifacts,
        reference.as_ref(),
        artifacts.first(),
        &mut summary,
    );
    run_drain_probe(&cfg, &artifacts, &mut summary);
    run_full_suite(
        &cfg,
        reference.as_ref(),
        &artifacts,
        out_ready,
        &mut summary,
    );
    print_protocol(&artifacts);
    print_wget_protocol(&wget_artifacts);

    if !cfg.keep {
        cleanup_artifacts(&artifacts);
        cleanup_artifacts(&wget_artifacts);
    } else {
        println!();
        println!("Artefakte bleiben erhalten in {}", cfg.out_dir);
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
                    println!("httpstress: --repeat braucht eine Zahl");
                    return None;
                }
                cfg.repeat = clamp(parse_u32(args[i]).unwrap_or(3), 1, 1000);
            }
            "--url" | "-u" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --url braucht eine URL");
                    return None;
                }
                cfg.url = String::from(args[i]);
            }
            "--small-url" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --small-url braucht eine URL");
                    return None;
                }
                cfg.small_url = String::from(args[i]);
            }
            "--out-dir" | "-o" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --out-dir braucht einen Pfad");
                    return None;
                }
                cfg.out_dir = String::from(args[i]);
            }
            "--profile" | "-p" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --profile braucht quick|normal|heavy");
                    return None;
                }
                cfg.profile = match args[i] {
                    "quick" => Profile::Quick,
                    "normal" => Profile::Normal,
                    "heavy" => Profile::Heavy,
                    other => {
                        println!("httpstress: unbekanntes Profil '{}'", other);
                        return None;
                    }
                };
            }
            "--keep" => cfg.keep = true,
            "--no-gzip" => cfg.gzip_check = false,
            "--no-wget" => cfg.wget_check = false,
            "--wget" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --wget braucht einen Pfad");
                    return None;
                }
                cfg.wget_path = String::from(args[i]);
            }
            "--memory-rounds" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --memory-rounds braucht eine Zahl");
                    return None;
                }
                cfg.memory_rounds = clamp(parse_u32(args[i]).unwrap_or(cfg.memory_rounds), 0, 100);
            }
            "--wget-rounds" => {
                i += 1;
                if i >= args.len() {
                    println!("httpstress: --wget-rounds braucht eine Zahl");
                    return None;
                }
                cfg.wget_rounds = clamp(parse_u32(args[i]).unwrap_or(cfg.wget_rounds), 0, 100);
            }
            other => {
                println!("httpstress: unbekannte Option '{}'", other);
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
        }
        Profile::Normal => {}
        Profile::Heavy => {
            if cfg.repeat == Config::default().repeat {
                cfg.repeat = 10;
            }
            if cfg.memory_rounds == Config::default().memory_rounds {
                cfg.memory_rounds = 10;
            }
            if cfg.wget_rounds == Config::default().wget_rounds {
                cfg.wget_rounds = 10;
            }
        }
    }
}

fn print_usage() {
    println!("httpstress - libhttp Stresstest und Download-Protokoll");
    println!("anyOS {}", anyos_version());
    println!();
    println!("Usage: httpstress [options]");
    println!();
    println!("Options:");
    println!("  --profile P       quick | normal | heavy (default: normal)");
    println!("  --repeat N        Download-Wiederholungen (default: 3, heavy: 10)");
    println!("  --url URL         grosse Test-URL (default: Debian Packages.gz)");
    println!("  --small-url URL   kleine GET-Test-URL");
    println!("  --out-dir PATH    Artefakt-Verzeichnis (default: /tmp/httpstress)");
    println!("  --memory-rounds N zusaetzliche In-Memory-Downloads (default: 3, heavy: 10)");
    println!("  --wget-rounds N   zusaetzliche wget-Downloads (default: 3, heavy: 10)");
    println!("  --wget PATH       wget-Binary (default: /System/bin/wget)");
    println!("  --no-gzip         gzip-Dekompressionscheck auslassen");
    println!("  --no-wget         wget-Vergleich auslassen");
    println!("  --keep            heruntergeladene Dateien behalten");
    println!("  --help, -h        diese Hilfe anzeigen");
    println!();
    println!("Beispiele:");
    println!("  httpstress --repeat 20 --profile heavy --keep");
    println!("  httpstress --repeat 5");
    println!("  httpstress --url http://speedtest.tele2.net/10MB.zip --no-gzip");
}

fn run_small_get(cfg: &Config, summary: &mut Summary) {
    let start = sys::uptime_ms();
    match libhttp_client::get(&cfg.small_url) {
        Some(body) if !body.is_empty() => {
            let ms = elapsed_ms(start);
            let md5 = crypto::md5_hex(&body);
            summary.ok(
                "small GET",
                &format!("{} bytes in {} ms md5={}", body.len(), ms, hex_str(&md5)),
            );
        }
        Some(_) => summary.fail("small GET", "empty body"),
        None => summary.fail(
            "small GET",
            &format!(
                "status={} error={} ({})",
                libhttp_client::last_status(),
                libhttp_client::last_error(),
                http_error_text(libhttp_client::last_error())
            ),
        ),
    }
}

fn run_large_get_probe(cfg: &Config, summary: &mut Summary) -> Option<Reference> {
    let mut buf = Vec::new();
    if buf.try_reserve_exact(LARGE_GET_BUF_SIZE).is_err() {
        summary.warn("large GET", "kein Speicher fuer Referenzpuffer");
        return None;
    }
    buf.resize(LARGE_GET_BUF_SIZE, 0);

    let start = sys::uptime_ms();
    match libhttp_client::get_into(&cfg.url, &mut buf) {
        Some(n) if n > 0 => {
            buf.truncate(n);
            let ms = elapsed_ms(start);
            let crc32 = crc32(&buf);
            let md5 = crypto::md5_hex(&buf);
            let gzip = if cfg.gzip_check {
                gzip_verdict(&buf)
            } else {
                GzipVerdict::NotChecked
            };
            let size = n.min(u32::MAX as usize) as u32;
            summary.ok(
                "large GET reference",
                &format!(
                    "{} bytes in {} ms crc32=0x{:08x} md5={} gzip={}",
                    size,
                    ms,
                    crc32,
                    hex_str(&md5),
                    gzip_text(gzip)
                ),
            );
            Some(Reference {
                size,
                crc32,
                md5_hex: md5,
                gzip,
            })
        }
        Some(_) => {
            summary.fail("large GET reference", "empty body");
            None
        }
        None => {
            let err = libhttp_client::last_error();
            summary.fail(
                "large GET reference",
                &format!(
                    "status={} error={} ({})",
                    libhttp_client::last_status(),
                    err,
                    http_error_text(err)
                ),
            );
            None
        }
    }
}

fn run_memory_matrix(
    cfg: &Config,
    reference: Option<&Reference>,
    summary: &mut Summary,
) -> Vec<Reference> {
    let mut results = Vec::new();
    if cfg.memory_rounds == 0 {
        return results;
    }

    println!();
    println!("--- In-Memory-Runden ---");
    for round in 1..=cfg.memory_rounds {
        let mut buf = Vec::new();
        if buf.try_reserve_exact(LARGE_GET_BUF_SIZE).is_err() {
            summary.fail("memory GET", &format!("round={} kein Speicher", round));
            continue;
        }
        buf.resize(LARGE_GET_BUF_SIZE, 0);

        let start = sys::uptime_ms();
        match libhttp_client::get_into(&cfg.url, &mut buf) {
            Some(n) if n > 0 => {
                buf.truncate(n);
                let ms = elapsed_ms(start);
                let crc32 = crc32(&buf);
                let md5 = crypto::md5_hex(&buf);
                let gzip = if cfg.gzip_check {
                    gzip_verdict(&buf)
                } else {
                    GzipVerdict::NotChecked
                };
                let size = n.min(u32::MAX as usize) as u32;
                println!(
                    "  {:>3}: {} bytes  {:>5} ms  crc32=0x{:08x}  md5={}  gzip={}",
                    round,
                    size,
                    ms,
                    crc32,
                    hex_str(&md5),
                    gzip_text(gzip)
                );
                let mut ok = true;
                if let Some(reference) = reference {
                    ok = size == reference.size
                        && crc32 == reference.crc32
                        && md5 == reference.md5_hex
                        && gzip_text(gzip) == gzip_text(reference.gzip);
                }
                if ok {
                    summary.ok("memory GET", &format!("round={} ok", round));
                } else {
                    summary.fail("memory GET", &format!("round={} mismatch", round));
                }
                results.push(Reference {
                    size,
                    crc32,
                    md5_hex: md5,
                    gzip,
                });
            }
            Some(_) => summary.fail("memory GET", &format!("round={} empty body", round)),
            None => {
                let err = libhttp_client::last_error();
                summary.fail(
                    "memory GET",
                    &format!(
                        "round={} status={} error={} ({})",
                        round,
                        libhttp_client::last_status(),
                        err,
                        http_error_text(err)
                    ),
                );
            }
        }
    }
    results
}

fn run_download_matrix(cfg: &Config, summary: &mut Summary) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    println!();
    println!("--- Download-Runden ---");
    for round in 1..=cfg.repeat {
        let path = format!("{}/httpstress-{}.bin", cfg.out_dir, round);
        let _ = fs::unlink(&path);
        let start = sys::uptime_ms();
        let ok = libhttp_client::download(&cfg.url, &path);
        let ms = elapsed_ms(start);
        let status = libhttp_client::last_status();
        let error = libhttp_client::last_error();

        if !ok {
            summary.fail(
                "download",
                &format!(
                    "round={} status={} error={} ({})",
                    round,
                    status,
                    error,
                    http_error_text(error)
                ),
            );
            continue;
        }

        let Ok(data) = fs::read_to_vec(&path) else {
            summary.fail(
                "download readback",
                &format!("round={} cannot read file", round),
            );
            continue;
        };
        if data.is_empty() {
            summary.fail("download readback", &format!("round={} empty file", round));
            continue;
        }

        let crc32 = crc32(&data);
        let md5 = crypto::md5_hex(&data);
        let gzip = if cfg.gzip_check {
            gzip_verdict(&data)
        } else {
            GzipVerdict::NotChecked
        };
        let size = data.len().min(u32::MAX as usize) as u32;
        let artifact = Artifact {
            path,
            size,
            crc32,
            md5_hex: md5,
            download_ms: ms,
            status,
            error,
            gzip,
        };
        print_artifact_line(round, &artifact);
        artifacts.push(artifact);
    }
    artifacts
}

fn run_wget_matrix(
    cfg: &Config,
    reference: Option<&Reference>,
    summary: &mut Summary,
) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    println!();
    println!("--- wget-Runden ---");

    if cfg.wget_rounds == 0 {
        summary.warn("wget count", "wget-Runden deaktiviert");
        return artifacts;
    }
    if !path_exists(&cfg.wget_path) {
        summary.warn("wget binary", &format!("{} fehlt", cfg.wget_path));
        return artifacts;
    }

    for round in 1..=cfg.wget_rounds {
        let path = format!("{}/httpstress-wget-{}.bin", cfg.out_dir, round);
        let _ = fs::unlink(&path);
        let args = format!("wget -q -O {} {}", path, cfg.url);
        let start = sys::uptime_ms();
        let tid = process::spawn(&cfg.wget_path, &args);
        if tid == u32::MAX {
            summary.fail("wget download", &format!("round={} spawn failed", round));
            continue;
        }
        let code = process::waitpid(tid);
        let ms = elapsed_ms(start);
        if code == process::STILL_RUNNING {
            summary.fail("wget download", &format!("round={} still running", round));
            continue;
        }
        if code == u32::MAX {
            summary.fail("wget download", &format!("round={} wait failed", round));
            continue;
        }
        if code != 0 {
            summary.fail(
                "wget download",
                &format!("round={} exit={} after {} ms", round, code, ms),
            );
            continue;
        }

        let Ok(data) = fs::read_to_vec(&path) else {
            summary.fail(
                "wget readback",
                &format!("round={} cannot read file", round),
            );
            continue;
        };
        if data.is_empty() {
            summary.fail("wget readback", &format!("round={} empty file", round));
            continue;
        }

        let crc32 = crc32(&data);
        let md5 = crypto::md5_hex(&data);
        let gzip = if cfg.gzip_check {
            gzip_verdict(&data)
        } else {
            GzipVerdict::NotChecked
        };
        let size = data.len().min(u32::MAX as usize) as u32;
        let artifact = Artifact {
            path,
            size,
            crc32,
            md5_hex: md5,
            download_ms: ms,
            status: code,
            error: 0,
            gzip,
        };
        print_artifact_line(round, &artifact);

        if let Some(reference) = reference {
            if artifact.size == reference.size
                && artifact.crc32 == reference.crc32
                && artifact.md5_hex == reference.md5_hex
                && gzip_text(artifact.gzip) == gzip_text(reference.gzip)
            {
                summary.ok("wget download", &format!("round={} ok", round));
            } else {
                summary.fail(
                    "wget download",
                    &format!(
                        "round={} mismatch ref_crc32=0x{:08x}",
                        round, reference.crc32
                    ),
                );
            }
        } else {
            summary.ok("wget download", &format!("round={} captured", round));
        }

        artifacts.push(artifact);
    }
    artifacts
}

fn compare_artifacts(artifacts: &[Artifact], reference: Option<&Reference>, summary: &mut Summary) {
    println!();
    println!("--- Konsistenz ---");
    if artifacts.is_empty() {
        summary.fail("artifact count", "keine erfolgreichen Downloads");
        return;
    }
    if artifacts.len() == 1 {
        summary.warn(
            "artifact count",
            "nur ein erfolgreicher Download; kein Vergleich",
        );
    } else {
        summary.ok(
            "artifact count",
            &format!("{} erfolgreiche Downloads", artifacts.len()),
        );
    }

    let first = &artifacts[0];
    let mut size_mismatch = 0u32;
    let mut crc_mismatch = 0u32;
    let mut hash_mismatch = 0u32;
    let mut gzip_fail = 0u32;
    for artifact in artifacts {
        if artifact.size != first.size {
            size_mismatch += 1;
        }
        if artifact.crc32 != first.crc32 {
            crc_mismatch += 1;
        }
        if artifact.md5_hex != first.md5_hex {
            hash_mismatch += 1;
        }
        if matches!(artifact.gzip, GzipVerdict::Failed(_)) {
            gzip_fail += 1;
        }
    }

    if size_mismatch == 0 {
        summary.ok("size stable", &format!("{} bytes", first.size));
    } else {
        summary.fail("size stable", &format!("{} mismatches", size_mismatch));
    }

    if crc_mismatch == 0 {
        summary.ok("crc32 stable", &format!("0x{:08x}", first.crc32));
    } else {
        summary.fail("crc32 stable", &format!("{} mismatches", crc_mismatch));
    }

    if hash_mismatch == 0 {
        summary.ok("md5 stable", &format!("{}", hex_str(&first.md5_hex)));
    } else {
        summary.fail("md5 stable", &format!("{} mismatches", hash_mismatch));
    }

    if gzip_fail == 0 {
        summary.ok(
            "gzip checks",
            "keine gzip-Fehler in erfolgreichen Downloads",
        );
    } else {
        summary.fail("gzip checks", &format!("{} gzip-Fehler", gzip_fail));
    }

    if let Some(reference) = reference {
        let mut ref_size_mismatch = 0u32;
        let mut ref_crc_mismatch = 0u32;
        let mut ref_hash_mismatch = 0u32;
        let mut ref_gzip_mismatch = 0u32;
        for artifact in artifacts {
            if artifact.size != reference.size {
                ref_size_mismatch += 1;
            }
            if artifact.crc32 != reference.crc32 {
                ref_crc_mismatch += 1;
            }
            if artifact.md5_hex != reference.md5_hex {
                ref_hash_mismatch += 1;
            }
            if gzip_text(artifact.gzip) != gzip_text(reference.gzip) {
                ref_gzip_mismatch += 1;
            }
        }

        if ref_size_mismatch == 0 {
            summary.ok("reference size", &format!("{} bytes", reference.size));
        } else {
            summary.fail(
                "reference size",
                &format!("{} mismatches", ref_size_mismatch),
            );
        }

        if ref_crc_mismatch == 0 {
            summary.ok("reference crc32", &format!("0x{:08x}", reference.crc32));
        } else {
            summary.fail(
                "reference crc32",
                &format!("{} mismatches gegen large GET", ref_crc_mismatch),
            );
        }

        if ref_hash_mismatch == 0 {
            summary.ok("reference md5", &format!("{}", hex_str(&reference.md5_hex)));
        } else {
            summary.fail(
                "reference md5",
                &format!("{} mismatches gegen large GET", ref_hash_mismatch),
            );
        }

        if ref_gzip_mismatch == 0 {
            summary.ok("reference gzip", gzip_text(reference.gzip));
        } else {
            summary.fail(
                "reference gzip",
                &format!("{} mismatches gegen large GET", ref_gzip_mismatch),
            );
        }
    }
}

fn compare_wget_artifacts(
    artifacts: &[Artifact],
    reference: Option<&Reference>,
    file_reference: Option<&Artifact>,
    summary: &mut Summary,
) {
    println!();
    println!("--- wget-Konsistenz ---");
    if artifacts.is_empty() {
        summary.warn("wget count", "keine erfolgreichen wget-Downloads");
        return;
    }

    let first = &artifacts[0];
    let mut size_mismatch = 0u32;
    let mut crc_mismatch = 0u32;
    let mut hash_mismatch = 0u32;
    let mut gzip_fail = 0u32;
    let mut ref_mismatch = 0u32;

    for artifact in artifacts {
        if artifact.size != first.size {
            size_mismatch += 1;
        }
        if artifact.crc32 != first.crc32 {
            crc_mismatch += 1;
        }
        if artifact.md5_hex != first.md5_hex {
            hash_mismatch += 1;
        }
        if matches!(artifact.gzip, GzipVerdict::Failed(_)) {
            gzip_fail += 1;
        }
        if let Some(reference) = reference {
            if artifact.size != reference.size
                || artifact.crc32 != reference.crc32
                || artifact.md5_hex != reference.md5_hex
                || gzip_text(artifact.gzip) != gzip_text(reference.gzip)
            {
                ref_mismatch += 1;
            }
        }
    }

    if size_mismatch == 0 {
        summary.ok("wget size", &format!("{} bytes", first.size));
    } else {
        summary.fail("wget size", &format!("{} mismatches", size_mismatch));
    }
    if crc_mismatch == 0 {
        summary.ok("wget crc32 stable", &format!("0x{:08x}", first.crc32));
    } else {
        summary.fail("wget crc32 stable", &format!("{} mismatches", crc_mismatch));
    }
    if hash_mismatch == 0 {
        summary.ok("wget md5 stable", hex_str(&first.md5_hex));
    } else {
        summary.fail("wget md5 stable", &format!("{} mismatches", hash_mismatch));
    }
    if gzip_fail == 0 {
        summary.ok("wget gzip", "keine gzip-Fehler");
    } else {
        summary.fail("wget gzip", &format!("{} gzip-Fehler", gzip_fail));
    }
    if reference.is_some() {
        if ref_mismatch == 0 {
            summary.ok("wget reference", "alle gleich large GET");
        } else {
            summary.fail("wget reference", &format!("{} mismatches", ref_mismatch));
        }
    }

    if let Some(file_reference) = file_reference {
        report_wget_first_diffs(artifacts, file_reference, summary);
    }
}

fn report_wget_first_diffs(
    artifacts: &[Artifact],
    file_reference: &Artifact,
    summary: &mut Summary,
) {
    let Ok(reference_bytes) = fs::read_to_vec(&file_reference.path) else {
        summary.warn("wget first-diff", "libhttp-Referenzdatei nicht lesbar");
        return;
    };
    let mut mismatches = 0u32;
    let mut reported = 0u32;
    for artifact in artifacts {
        if artifact.crc32 == file_reference.crc32 && artifact.md5_hex == file_reference.md5_hex {
            continue;
        }
        mismatches += 1;
        if reported >= 4 {
            continue;
        }
        let Ok(bytes) = fs::read_to_vec(&artifact.path) else {
            summary.warn(
                "wget first-diff",
                &format!("{} nicht lesbar", artifact.path),
            );
            reported += 1;
            continue;
        };
        match first_diff(&reference_bytes, &bytes) {
            Some(off) => summary.fail(
                "wget first-diff",
                &format!(
                    "{} offset={} mod4k={} mod32k={} ref=0x{:02x} got=0x{:02x} zero_run={}",
                    artifact.path,
                    off,
                    off % 4096,
                    off % (32 * 1024),
                    byte_at(&reference_bytes, off),
                    byte_at(&bytes, off),
                    byte_run(&bytes, off, 0)
                ),
            ),
            None => summary.fail(
                "wget first-diff",
                &format!("{} gleicher Inhalt trotz Hash-Mismatch", artifact.path),
            ),
        }
        reported += 1;
    }
    if mismatches == 0 {
        summary.ok("wget first-diff", "keine Abweichung zur libhttp-Datei");
    }
}

fn compare_memory_artifacts(
    artifacts: &[Reference],
    reference: Option<&Reference>,
    summary: &mut Summary,
) {
    println!();
    println!("--- Memory-Konsistenz ---");
    if artifacts.is_empty() {
        summary.warn("memory count", "keine In-Memory-Runden");
        return;
    }

    let first = &artifacts[0];
    let mut size_mismatch = 0u32;
    let mut crc_mismatch = 0u32;
    let mut hash_mismatch = 0u32;
    let mut gzip_fail = 0u32;
    for artifact in artifacts {
        if artifact.size != first.size {
            size_mismatch += 1;
        }
        if artifact.crc32 != first.crc32 {
            crc_mismatch += 1;
        }
        if artifact.md5_hex != first.md5_hex {
            hash_mismatch += 1;
        }
        if matches!(artifact.gzip, GzipVerdict::Failed(_)) {
            gzip_fail += 1;
        }
    }

    if size_mismatch == 0 {
        summary.ok("memory size", &format!("{} bytes", first.size));
    } else {
        summary.fail("memory size", &format!("{} mismatches", size_mismatch));
    }
    if crc_mismatch == 0 {
        summary.ok("memory crc32", &format!("0x{:08x}", first.crc32));
    } else {
        summary.fail("memory crc32", &format!("{} mismatches", crc_mismatch));
    }
    if hash_mismatch == 0 {
        summary.ok("memory md5", hex_str(&first.md5_hex));
    } else {
        summary.fail("memory md5", &format!("{} mismatches", hash_mismatch));
    }
    if gzip_fail == 0 {
        summary.ok("memory gzip", "keine gzip-Fehler");
    } else {
        summary.fail("memory gzip", &format!("{} gzip-Fehler", gzip_fail));
    }

    if let Some(reference) = reference {
        let mut ref_mismatch = 0u32;
        for artifact in artifacts {
            if artifact.size != reference.size
                || artifact.crc32 != reference.crc32
                || artifact.md5_hex != reference.md5_hex
                || gzip_text(artifact.gzip) != gzip_text(reference.gzip)
            {
                ref_mismatch += 1;
            }
        }
        if ref_mismatch == 0 {
            summary.ok("memory reference", "alle gleich large GET");
        } else {
            summary.fail("memory reference", &format!("{} mismatches", ref_mismatch));
        }
    }
}

fn run_drain_probe(cfg: &Config, artifacts: &[Artifact], summary: &mut Summary) {
    println!();
    println!("--- Drain-Probe ---");
    let start = sys::uptime_ms();
    match libhttp_client::drain_progress(&cfg.url, drain_progress, 0) {
        Some(bytes) => {
            let ms = elapsed_ms(start);
            if let Some(first) = artifacts.first() {
                if bytes == first.size {
                    summary.ok("drain bytes", &format!("{} bytes in {} ms", bytes, ms));
                } else {
                    summary.warn(
                        "drain bytes",
                        &format!("{} bytes, file-download had {}", bytes, first.size),
                    );
                }
            } else {
                summary.ok("drain bytes", &format!("{} bytes in {} ms", bytes, ms));
            }
        }
        None => summary.fail(
            "drain bytes",
            &format!(
                "status={} error={} ({})",
                libhttp_client::last_status(),
                libhttp_client::last_error(),
                http_error_text(libhttp_client::last_error())
            ),
        ),
    }
}

extern "C" fn drain_progress(received: u32, total: u32, _userdata: u64) {
    if total > 0 && received >= total {
        print!("\r  drain: {} / {} bytes", received, total);
    }
}

fn run_full_suite(
    cfg: &Config,
    reference: Option<&Reference>,
    artifacts: &[Artifact],
    out_ready: bool,
    summary: &mut Summary,
) {
    println!();
    println!("--- Full-Suite ---");

    run_invalid_url_probe(summary);
    run_tiny_buffer_probe(cfg, summary);
    run_request_headers_probe(cfg, summary);
    run_reinit_probe(summary);
    if out_ready {
        run_progress_download_probe(cfg, reference, summary);
        run_resume_probe(cfg, reference, artifacts, summary);
    } else {
        summary.warn("progress download", "out-dir fehlt; uebersprungen");
        summary.warn("resume download", "out-dir fehlt; uebersprungen");
    }
}

fn run_invalid_url_probe(summary: &mut Summary) {
    match libhttp_client::get("http://") {
        Some(_) => summary.fail("invalid url", "GET war unerwartet erfolgreich"),
        None => {
            let err = libhttp_client::last_error();
            if err == 1 {
                summary.ok("invalid url", "ERR_INVALID_URL");
            } else {
                summary.fail(
                    "invalid url",
                    &format!("error={} ({})", err, http_error_text(err)),
                );
            }
        }
    }
}

fn run_tiny_buffer_probe(cfg: &Config, summary: &mut Summary) {
    let mut tiny = [0u8; 32];
    match libhttp_client::get_into(&cfg.small_url, &mut tiny) {
        Some(n) if n == tiny.len() => {
            let err = libhttp_client::last_error();
            if err == 8 {
                summary.ok("tiny buffer", "GET begrenzt und ERR_BUFFER_TOO_SMALL");
            } else {
                summary.warn(
                    "tiny buffer",
                    &format!("{} bytes, error={} ({})", n, err, http_error_text(err)),
                );
            }
        }
        Some(n) => summary.warn("tiny buffer", &format!("nur {} bytes; Body evtl. klein", n)),
        None => {
            let err = libhttp_client::last_error();
            summary.fail(
                "tiny buffer",
                &format!(
                    "status={} error={} ({})",
                    libhttp_client::last_status(),
                    err,
                    http_error_text(err)
                ),
            );
        }
    }
}

fn run_request_headers_probe(cfg: &Config, summary: &mut Summary) {
    let headers = "User-Agent: httpstress-full\r\nX-Httpstress: full\r\n";
    match libhttp_client::request_with_headers(&cfg.small_url, "GET", b"", "", headers) {
        Some(body) if !body.is_empty() => {
            let status = libhttp_client::last_status();
            let last_headers = libhttp_client::last_headers();
            if status == 200 && !last_headers.is_empty() {
                summary.ok(
                    "request headers",
                    &format!(
                        "{} bytes, response headers {} bytes",
                        body.len(),
                        last_headers.len()
                    ),
                );
            } else {
                summary.warn(
                    "request headers",
                    &format!(
                        "{} bytes, status={}, headers={}",
                        body.len(),
                        status,
                        last_headers.len()
                    ),
                );
            }
        }
        Some(_) => summary.fail("request headers", "empty body"),
        None => {
            let err = libhttp_client::last_error();
            summary.fail(
                "request headers",
                &format!(
                    "status={} error={} ({})",
                    libhttp_client::last_status(),
                    err,
                    http_error_text(err)
                ),
            );
        }
    }
}

fn run_reinit_probe(summary: &mut Summary) {
    if libhttp_client::init() {
        summary.ok("libhttp reinit", "zweiter init-Aufruf ok");
    } else {
        summary.fail("libhttp reinit", "zweiter init-Aufruf fehlgeschlagen");
    }
}

fn run_progress_download_probe(cfg: &Config, reference: Option<&Reference>, summary: &mut Summary) {
    let path = format!("{}/httpstress-progress.bin", cfg.out_dir);
    let _ = fs::unlink(&path);
    reset_progress();
    let start = sys::uptime_ms();
    let ok = libhttp_client::download_progress(&cfg.url, &path, progress_probe, 0);
    let ms = elapsed_ms(start);
    if !ok {
        let err = libhttp_client::last_error();
        summary.fail(
            "progress download",
            &format!(
                "status={} error={} ({})",
                libhttp_client::last_status(),
                err,
                http_error_text(err)
            ),
        );
        let _ = fs::unlink(&path);
        return;
    }

    let Ok(data) = fs::read_to_vec(&path) else {
        summary.fail("progress download", "readback fehlgeschlagen");
        let _ = fs::unlink(&path);
        return;
    };
    let md5 = crypto::md5_hex(&data);
    let size = data.len().min(u32::MAX as usize) as u32;
    let (calls, last, total) = progress_snapshot();
    let mut ok = calls > 0 && last == size;
    if let Some(reference) = reference {
        ok = ok && size == reference.size && md5 == reference.md5_hex;
    }
    if ok {
        summary.ok(
            "progress download",
            &format!(
                "{} bytes in {} ms, progress {}/{} calls={}",
                size, ms, last, total, calls
            ),
        );
    } else {
        summary.fail(
            "progress download",
            &format!(
                "size={} progress={}/{} calls={} md5={}",
                size,
                last,
                total,
                calls,
                hex_str(&md5)
            ),
        );
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
}

fn run_resume_probe(
    cfg: &Config,
    reference: Option<&Reference>,
    artifacts: &[Artifact],
    summary: &mut Summary,
) {
    let Some(reference) = reference else {
        summary.warn("resume download", "keine Referenz");
        return;
    };
    let Some(source) = artifacts.first() else {
        summary.warn("resume download", "kein Download-Artefakt fuer Prefix");
        return;
    };
    let Ok(source_data) = fs::read_to_vec(&source.path) else {
        summary.warn("resume download", "Prefix-Quelle nicht lesbar");
        return;
    };
    if source_data.len() < 64 * 1024 {
        summary.warn("resume download", "Artefakt zu klein fuer Resume-Prefix");
        return;
    }

    let path = format!("{}/httpstress-resume.bin", cfg.out_dir);
    let _ = fs::unlink(&path);
    if fs::write_bytes(&path, &source_data[..64 * 1024]).is_err() {
        summary.fail("resume download", "Prefix schreiben fehlgeschlagen");
        return;
    }

    reset_progress();
    let start = sys::uptime_ms();
    let ok =
        libhttp_client::download_progress_resume(&cfg.url, &path, progress_probe, 0, 64 * 1024);
    let ms = elapsed_ms(start);
    if !ok {
        let err = libhttp_client::last_error();
        summary.fail(
            "resume download",
            &format!(
                "status={} error={} ({})",
                libhttp_client::last_status(),
                err,
                http_error_text(err)
            ),
        );
        let _ = fs::unlink(&path);
        return;
    }

    let Ok(data) = fs::read_to_vec(&path) else {
        summary.fail("resume download", "readback fehlgeschlagen");
        let _ = fs::unlink(&path);
        return;
    };
    let md5 = crypto::md5_hex(&data);
    let size = data.len().min(u32::MAX as usize) as u32;
    let (calls, last, total) = progress_snapshot();
    if size == reference.size && md5 == reference.md5_hex {
        summary.ok(
            "resume download",
            &format!(
                "{} bytes in {} ms, progress {}/{} calls={}",
                size, ms, last, total, calls
            ),
        );
    } else {
        summary.fail(
            "resume download",
            &format!(
                "size={} ref={} md5={} ref={}",
                size,
                reference.size,
                hex_str(&md5),
                hex_str(&reference.md5_hex)
            ),
        );
    }
    if !cfg.keep {
        let _ = fs::unlink(&path);
    }
}

extern "C" fn progress_probe(received: u32, total: u32, _userdata: u64) {
    unsafe {
        PROGRESS_CALLS = PROGRESS_CALLS.saturating_add(1);
        PROGRESS_LAST = received;
        PROGRESS_TOTAL = total;
    }
}

fn reset_progress() {
    unsafe {
        PROGRESS_CALLS = 0;
        PROGRESS_LAST = 0;
        PROGRESS_TOTAL = 0;
    }
}

fn progress_snapshot() -> (u32, u32, u32) {
    unsafe { (PROGRESS_CALLS, PROGRESS_LAST, PROGRESS_TOTAL) }
}

fn gzip_verdict(data: &[u8]) -> GzipVerdict {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b {
        return GzipVerdict::NotGzip;
    }
    match libzip_client::gunzip(data) {
        Some(unpacked) => {
            let len = unpacked.len().min(u32::MAX as usize) as u32;
            if len < GZIP_EXPECTED_UNPACKED_MIN {
                GzipVerdict::Ok(len)
            } else {
                GzipVerdict::Ok(len)
            }
        }
        None => GzipVerdict::Failed(gzip_isize(data).unwrap_or(0)),
    }
}

fn gzip_isize(data: &[u8]) -> Option<u32> {
    if data.len() < 8 {
        return None;
    }
    let t = &data[data.len() - 8..];
    Some(u32::from_le_bytes([t[4], t[5], t[6], t[7]]))
}

fn print_artifact_line(round: u32, artifact: &Artifact) {
    let gzip = format!("gzip={}", gzip_text(artifact.gzip));
    println!(
        "  {:>3}: {} bytes  {:>5} ms  crc32=0x{:08x}  md5={}  {}",
        round,
        artifact.size,
        artifact.download_ms,
        artifact.crc32,
        hex_str(&artifact.md5_hex),
        gzip
    );
}

fn print_protocol(artifacts: &[Artifact]) {
    println!();
    println!("--- Protokoll ---");
    if artifacts.is_empty() {
        println!("  keine Artefakte");
        return;
    }
    println!("  round | bytes | ms | status | error | crc32 | md5 | gzip | path");
    for (i, a) in artifacts.iter().enumerate() {
        println!(
            "  {} | {} | {} | {} | {} | 0x{:08x} | {} | {} | {}",
            i + 1,
            a.size,
            a.download_ms,
            a.status,
            a.error,
            a.crc32,
            hex_str(&a.md5_hex),
            gzip_text(a.gzip),
            a.path
        );
    }
}

fn print_wget_protocol(artifacts: &[Artifact]) {
    println!();
    println!("--- wget-Protokoll ---");
    if artifacts.is_empty() {
        println!("  keine Artefakte");
        return;
    }
    println!("  round | bytes | ms | exit | crc32 | md5 | gzip | path");
    for (i, a) in artifacts.iter().enumerate() {
        println!(
            "  {} | {} | {} | {} | 0x{:08x} | {} | {} | {}",
            i + 1,
            a.size,
            a.download_ms,
            a.status,
            a.crc32,
            hex_str(&a.md5_hex),
            gzip_text(a.gzip),
            a.path
        );
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

fn cleanup_artifacts(artifacts: &[Artifact]) {
    for artifact in artifacts {
        let _ = fs::unlink(&artifact.path);
    }
}

fn prepare_out_dir(path: &str) -> bool {
    mkdir_parents(path);
    let probe = format!("{}/.httpstress-write-test", path);
    if fs::write_bytes(&probe, b"ok").is_err() {
        return false;
    }
    let _ = fs::unlink(&probe);
    true
}

fn path_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
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

fn http_error_text(error: u32) -> &'static str {
    match error {
        0 => "none",
        1 => "invalid url",
        2 => "dns failure",
        3 => "connect failure",
        4 => "send failure",
        5 => "no response",
        6 => "too many redirects",
        7 => "tls handshake failed",
        8 => "buffer too small",
        9 => "file write",
        _ => "unknown",
    }
}

fn gzip_text(gzip: GzipVerdict) -> &'static str {
    match gzip {
        GzipVerdict::NotChecked => "skip",
        GzipVerdict::NotGzip => "not-gzip",
        GzipVerdict::Ok(_) => "ok",
        GzipVerdict::Failed(_) => "fail",
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.len().min(b.len());
    for i in 0..common {
        if a[i] != b[i] {
            return Some(i);
        }
    }
    if a.len() != b.len() {
        Some(common)
    } else {
        None
    }
}

fn byte_at(data: &[u8], off: usize) -> u8 {
    data.get(off).copied().unwrap_or(0)
}

fn byte_run(data: &[u8], off: usize, value: u8) -> usize {
    let mut len = 0usize;
    let mut i = off;
    while i < data.len() && data[i] == value {
        len += 1;
        i += 1;
    }
    len
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

fn hex_str(hex: &[u8; 32]) -> &str {
    core::str::from_utf8(hex).unwrap_or("?")
}
