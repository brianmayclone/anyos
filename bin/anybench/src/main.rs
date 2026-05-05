#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

#[path = "../../../apps/anybench/src/workloads/crypto_hash.rs"]
mod crypto_hash;
#[path = "../../../apps/anybench/src/workloads/disk_io.rs"]
mod disk_io;
#[path = "../../../apps/anybench/src/workloads/mandelbrot.rs"]
mod mandelbrot;
#[path = "../../../apps/anybench/src/workloads/matrix_multiply.rs"]
mod matrix_multiply;
#[path = "../../../apps/anybench/src/workloads/memory_copy.rs"]
mod memory_copy;
#[path = "../../../apps/anybench/src/workloads/prime_sieve.rs"]
mod prime_sieve;
#[path = "../../../apps/anybench/src/workloads/sort.rs"]
mod sort;

anyos_std::entry!(main);

const APP_PATH: &str = "/Applications/anyBench.app/anyBench";
const MAX_CORES: usize = 64;

pub const CPU_TEST_MS: u32 = 3000;
pub const DISK_TEST_MS: u32 = 3000;

const NUM_CPU_TESTS: usize = 6;
const NUM_DISK_TESTS: usize = 4;

const CPU_BASELINES: [u64; NUM_CPU_TESTS] = [
    30_000_000, 10_000_000, 500_000_000, 500_000_000, 500_000, 10_000_000,
];

const DISK_BASELINES: [u64; NUM_DISK_TESTS] = [8_000_000, 4_000_000, 2_000, 500];

const CPU_TEST_NAMES: [&str; NUM_CPU_TESTS] = [
    "Integer Math",
    "Floating-Point",
    "Memory Bandwidth",
    "Matrix Math",
    "Crypto Hash",
    "Sorting",
];

const DISK_TEST_NAMES: [&str; NUM_DISK_TESTS] = [
    "Sequential Read",
    "Sequential Write",
    "Random Read",
    "File Create/Delete",
];

#[derive(Clone, Copy, PartialEq)]
enum BenchMode {
    All,
    CpuOnly,
    DiskOnly,
}

struct BenchResults {
    mode: BenchMode,
    num_cpus: u32,
    cpu_single_raw: [u64; NUM_CPU_TESTS],
    cpu_multi_raw: [u64; NUM_CPU_TESTS],
    disk_raw: [u64; NUM_DISK_TESTS],
}

impl BenchResults {
    fn empty(mode: BenchMode, num_cpus: u32) -> Self {
        Self {
            mode,
            num_cpus,
            cpu_single_raw: [0; NUM_CPU_TESTS],
            cpu_multi_raw: [0; NUM_CPU_TESTS],
            disk_raw: [0; NUM_DISK_TESTS],
        }
    }

    fn cpu_single_score(&self) -> u32 {
        let scores: Vec<u32> = (0..NUM_CPU_TESTS)
            .filter(|&i| self.cpu_single_raw[i] > 0)
            .map(|i| compute_score(self.cpu_single_raw[i], CPU_BASELINES[i]))
            .collect();
        geometric_mean(&scores)
    }

    fn cpu_multi_score(&self) -> u32 {
        let scores: Vec<u32> = (0..NUM_CPU_TESTS)
            .filter(|&i| self.cpu_multi_raw[i] > 0)
            .map(|i| compute_score(self.cpu_multi_raw[i], CPU_BASELINES[i]))
            .collect();
        geometric_mean(&scores)
    }

    fn disk_score(&self) -> u32 {
        let scores: Vec<u32> = (0..NUM_DISK_TESTS)
            .filter(|&i| self.disk_raw[i] > 0)
            .map(|i| compute_score(self.disk_raw[i], DISK_BASELINES[i]))
            .collect();
        geometric_mean(&scores)
    }
}

fn print_usage() {
    anyos_std::println!("anyBench");
    anyos_std::println!("Usage:");
    anyos_std::println!("  anybench                       Start GUI");
    anyos_std::println!("  anybench --cli [--cpu|--disk|--all] [--format text|md|json] [--out PATH]");
    anyos_std::println!("");
    anyos_std::println!("Terminal mode runs CPU and Disk I/O tests. GPU and 3D tests need the GUI canvas.");
}

fn mode_name(mode: BenchMode) -> &'static str {
    match mode {
        BenchMode::All => "all",
        BenchMode::CpuOnly => "cpu",
        BenchMode::DiskOnly => "disk",
    }
}

fn run_cpu_bench(bench_id: u32) -> u64 {
    match bench_id {
        1 => prime_sieve::bench_prime_sieve(),
        2 => mandelbrot::bench_mandelbrot(),
        3 => memory_copy::bench_memory_copy(),
        4 => matrix_multiply::bench_matrix_multiply(),
        5 => crypto_hash::bench_crypto_hash(),
        6 => sort::bench_sort(),
        _ => 0,
    }
}

fn run_disk_bench(bench_id: u32) -> u64 {
    match bench_id {
        1 => disk_io::bench_seq_read(),
        2 => disk_io::bench_seq_write(),
        3 => disk_io::bench_random_read(),
        4 => disk_io::bench_file_create(),
        _ => 0,
    }
}

fn fork_bench_worker(bench_id: u32) -> u32 {
    let child = anyos_std::process::fork();
    if child == 0 {
        let result = run_cpu_bench(bench_id);
        let code = if result > 0xFFFF_FFFD {
            0xFFFF_FFFD
        } else {
            result as u32
        };
        anyos_std::process::exit(code);
    }
    if child == u32::MAX {
        0
    } else {
        child
    }
}

fn run_cpu_group_cli(bench_id: u32, workers: u32) -> u64 {
    let n = workers.max(1).min(MAX_CORES as u32);
    let mut tids = [0u32; MAX_CORES];
    let mut results = [0u64; MAX_CORES];
    let mut reaped = 0u32;
    for i in 0..n as usize {
        let tid = fork_bench_worker(bench_id);
        tids[i] = tid;
        if tid == 0 {
            reaped += 1;
        }
    }
    while reaped < n {
        for i in 0..n as usize {
            let tid = tids[i];
            if tid == 0 {
                continue;
            }
            let status = anyos_std::process::try_waitpid(tid);
            if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
                results[i] = status as u64;
                tids[i] = 0;
                reaped += 1;
            }
        }
        if reaped < n {
            anyos_std::process::sleep(20);
        }
    }
    let mut total = 0u64;
    for i in 0..n as usize {
        total += results[i];
    }
    total
}

fn compute_score(raw: u64, baseline: u64) -> u32 {
    if baseline == 0 {
        return 0;
    }
    ((raw * 1000) / baseline) as u32
}

fn geometric_mean(scores: &[u32]) -> u32 {
    if scores.is_empty() {
        return 0;
    }
    let mut log_sum: u64 = 0;
    let mut count = 0u32;
    for &s in scores {
        if s > 0 {
            log_sum += int_log2_fp(s as u64);
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    int_exp2_fp(log_sum / count as u64)
}

fn int_log2_fp(x: u64) -> u64 {
    if x <= 1 {
        return 0;
    }
    let mut val = x;
    let mut result: u64 = 0;
    while val >= 2 {
        val >>= 1;
        result += 65536;
    }
    let int_part = (result >> 16) as u32;
    let mut frac_val: u64 = if int_part < 48 {
        (x << 16) >> int_part
    } else {
        65536
    };
    let mut bit: u64 = 32768;
    for _ in 0..16 {
        frac_val = (frac_val * frac_val) >> 16;
        if frac_val >= 2 * 65536 {
            frac_val >>= 1;
            result += bit;
        }
        bit >>= 1;
    }
    result
}

fn int_exp2_fp(x: u64) -> u32 {
    let int_part = (x >> 16) as u32;
    if int_part >= 31 {
        return u32::MAX;
    }
    let base = 1u64 << int_part;
    let f = x & 0xFFFF;
    if f == 0 {
        return base as u32;
    }
    const C1: u64 = 45426;
    const C2: u64 = 15743;
    const C3: u64 = 3634;
    let mut r = C3;
    r = (r * f) >> 16;
    r += C2;
    r = (r * f) >> 16;
    r += C1;
    r = (r * f) >> 16;
    r += 65536;
    let result = (base * r) >> 16;
    if result > u32::MAX as u64 {
        u32::MAX
    } else {
        result as u32
    }
}

fn disk_rate_text(index: usize, raw: u64) -> String {
    if raw == 0 {
        return String::from("-");
    }
    let test_ms = DISK_TEST_MS as u64;
    if index < 2 {
        let mb10 = raw * 1000 * 10 / test_ms / (1024 * 1024);
        anyos_std::format!("{}.{} MB/s", mb10 / 10, mb10 % 10)
    } else {
        anyos_std::format!("{} ops/s", raw * 1000 / test_ms)
    }
}

fn format_results_text(r: &BenchResults) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "anyBench results");
    let _ = writeln!(out, "mode: {}", mode_name(r.mode));
    let _ = writeln!(out, "cpu_cores: {}", r.num_cpus);
    let _ = writeln!(out);
    if r.cpu_single_score() > 0 {
        let _ = writeln!(out, "CPU Single-Core: {}", r.cpu_single_score());
        for i in 0..NUM_CPU_TESTS {
            let score = compute_score(r.cpu_single_raw[i], CPU_BASELINES[i]);
            let _ = writeln!(
                out,
                "  {:<18} score={} raw={}",
                CPU_TEST_NAMES[i], score, r.cpu_single_raw[i]
            );
        }
    }
    if r.cpu_multi_score() > 0 {
        let _ = writeln!(out, "CPU Multi-Core: {}", r.cpu_multi_score());
        for i in 0..NUM_CPU_TESTS {
            let score = compute_score(r.cpu_multi_raw[i], CPU_BASELINES[i]);
            let _ = writeln!(
                out,
                "  {:<18} score={} raw={}",
                CPU_TEST_NAMES[i], score, r.cpu_multi_raw[i]
            );
        }
    }
    if r.disk_score() > 0 {
        let _ = writeln!(out, "Disk I/O: {}", r.disk_score());
        for i in 0..NUM_DISK_TESTS {
            let score = compute_score(r.disk_raw[i], DISK_BASELINES[i]);
            let _ = writeln!(
                out,
                "  {:<18} score={} raw={} rate={}",
                DISK_TEST_NAMES[i],
                score,
                r.disk_raw[i],
                disk_rate_text(i, r.disk_raw[i])
            );
        }
    }
    out
}

fn write_markdown_section(
    out: &mut String,
    title: &str,
    overall: u32,
    names: &[&str],
    raw: &[u64],
    baselines: &[u64],
    disk: bool,
) {
    if overall == 0 {
        return;
    }
    let _ = writeln!(out, "## {}: {}", title, overall);
    if disk {
        let _ = writeln!(out, "| Test | Score | Raw | Rate |");
        let _ = writeln!(out, "| --- | ---: | ---: | --- |");
    } else {
        let _ = writeln!(out, "| Test | Score | Raw |");
        let _ = writeln!(out, "| --- | ---: | ---: |");
    }
    for i in 0..names.len() {
        let score = compute_score(raw[i], baselines[i]);
        if disk {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} |",
                names[i],
                score,
                raw[i],
                disk_rate_text(i, raw[i])
            );
        } else {
            let _ = writeln!(out, "| {} | {} | {} |", names[i], score, raw[i]);
        }
    }
    let _ = writeln!(out);
}

fn format_results_markdown(r: &BenchResults) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# anyBench Results");
    let _ = writeln!(out);
    let _ = writeln!(out, "- Mode: `{}`", mode_name(r.mode));
    let _ = writeln!(out, "- CPU cores: {}", r.num_cpus);
    let _ = writeln!(out);
    write_markdown_section(
        &mut out,
        "CPU Single-Core",
        r.cpu_single_score(),
        &CPU_TEST_NAMES,
        &r.cpu_single_raw,
        &CPU_BASELINES,
        false,
    );
    write_markdown_section(
        &mut out,
        "CPU Multi-Core",
        r.cpu_multi_score(),
        &CPU_TEST_NAMES,
        &r.cpu_multi_raw,
        &CPU_BASELINES,
        false,
    );
    write_markdown_section(
        &mut out,
        "Disk I/O",
        r.disk_score(),
        &DISK_TEST_NAMES,
        &r.disk_raw,
        &DISK_BASELINES,
        true,
    );
    out
}

fn write_json_array(out: &mut String, names: &[&str], raw: &[u64], baselines: &[u64], disk: bool) {
    let _ = writeln!(out, "    [");
    for i in 0..names.len() {
        let score = compute_score(raw[i], baselines[i]);
        let comma = if i + 1 == names.len() { "" } else { "," };
        if disk {
            let _ = writeln!(
                out,
                "      {{\"name\":\"{}\",\"score\":{},\"raw\":{},\"rate\":\"{}\"}}{}",
                names[i],
                score,
                raw[i],
                disk_rate_text(i, raw[i]),
                comma
            );
        } else {
            let _ = writeln!(
                out,
                "      {{\"name\":\"{}\",\"score\":{},\"raw\":{}}}{}",
                names[i], score, raw[i], comma
            );
        }
    }
    let _ = write!(out, "    ]");
}

fn format_results_json(r: &BenchResults) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"app\": \"anyBench\",");
    let _ = writeln!(out, "  \"mode\": \"{}\",", mode_name(r.mode));
    let _ = writeln!(out, "  \"cpu_cores\": {},", r.num_cpus);
    let _ = writeln!(out, "  \"scores\": {{");
    let _ = writeln!(out, "    \"cpu_single\": {},", r.cpu_single_score());
    let _ = writeln!(out, "    \"cpu_multi\": {},", r.cpu_multi_score());
    let _ = writeln!(out, "    \"disk\": {}", r.disk_score());
    let _ = writeln!(out, "  }},");
    let _ = writeln!(out, "  \"tests\": {{");
    let _ = writeln!(out, "    \"cpu_single\": ");
    write_json_array(&mut out, &CPU_TEST_NAMES, &r.cpu_single_raw, &CPU_BASELINES, false);
    let _ = writeln!(out, ",");
    let _ = writeln!(out, "    \"cpu_multi\": ");
    write_json_array(&mut out, &CPU_TEST_NAMES, &r.cpu_multi_raw, &CPU_BASELINES, false);
    let _ = writeln!(out, ",");
    let _ = writeln!(out, "    \"disk\": ");
    write_json_array(&mut out, &DISK_TEST_NAMES, &r.disk_raw, &DISK_BASELINES, true);
    let _ = writeln!(out);
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    out
}

fn run_cli(args: &[String]) {
    let mut mode = BenchMode::All;
    let mut format = "text";
    let mut out_path: Option<&str> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "--cpu" => mode = BenchMode::CpuOnly,
            "--disk" | "--io" => mode = BenchMode::DiskOnly,
            "--all" | "--cli" => mode = BenchMode::All,
            "--format" | "-f" => {
                if i + 1 < args.len() {
                    format = args[i + 1].as_str();
                    i += 1;
                }
            }
            "--json" => format = "json",
            "--md" | "--markdown" => format = "md",
            "--out" | "-o" => {
                if i + 1 < args.len() {
                    out_path = Some(args[i + 1].as_str());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let num_cpus = anyos_std::sys::sysinfo(2, &mut [0u8; 4]);
    let num_cpus = if num_cpus == 0 { 1 } else { num_cpus };
    let mut results = BenchResults::empty(mode, num_cpus);

    if mode == BenchMode::All || mode == BenchMode::CpuOnly {
        anyos_std::println!("Running CPU single-core tests...");
        for i in 0..NUM_CPU_TESTS {
            anyos_std::println!("  {}", CPU_TEST_NAMES[i]);
            results.cpu_single_raw[i] = run_cpu_group_cli((i + 1) as u32, 1);
        }
        anyos_std::println!("Running CPU multi-core tests ({} workers)...", num_cpus);
        for i in 0..NUM_CPU_TESTS {
            anyos_std::println!("  {}", CPU_TEST_NAMES[i]);
            results.cpu_multi_raw[i] = run_cpu_group_cli((i + 1) as u32, num_cpus);
        }
    }

    if mode == BenchMode::All || mode == BenchMode::DiskOnly {
        anyos_std::println!("Running Disk I/O tests...");
        for i in 0..NUM_DISK_TESTS {
            anyos_std::println!("  {}", DISK_TEST_NAMES[i]);
            results.disk_raw[i] = run_disk_bench((i + 1) as u32);
        }
    }

    let output = match format {
        "json" => format_results_json(&results),
        "md" | "markdown" => format_results_markdown(&results),
        _ => format_results_text(&results),
    };
    anyos_std::print!("{}", output);
    if let Some(path) = out_path {
        match anyos_std::fs::write_bytes(path, output.as_bytes()) {
            Ok(()) => anyos_std::println!("Wrote {}", path),
            Err(_) => anyos_std::println!("Could not write {}", path),
        }
    }
}

fn main() {
    let mut arg_buf = [0u8; 512];
    let raw_args = anyos_std::process::args(&mut arg_buf);
    let args = anyos_std::args::tokenize(raw_args);

    if args.is_empty() {
        let tid = anyos_std::process::launch_app(APP_PATH, "");
        if tid == u32::MAX {
            anyos_std::println!("anybench: could not start anyBench GUI");
        }
        return;
    }

    run_cli(&args);
}
