//! anyBench — Comprehensive system benchmark for anyOS.
//!
//! CPU tests (SingleCore & MultiCore):
//!   1. Integer Math     — Prime sieve (Eratosthenes)
//!   2. Floating-Point   — Mandelbrot set computation
//!   3. Memory Bandwidth — Sequential buffer copy
//!   4. Matrix Math      — Dense matrix multiplication
//!   5. Crypto           — SHA-256–like hash chain
//!   6. Sorting          — Quicksort on random data
//!
//! GPU tests (OnScreen & OffScreen):
//!   1. Fill Rate        — Rectangle fill throughput
//!   2. Pixel Throughput — Individual pixel write speed
//!   3. Line Drawing     — Line rendering throughput
//!   4. Circle Rendering — Filled circle throughput
//!   5. Blending         — Alpha-blended rectangle compositing

#![no_std]
#![no_main]

mod workloads;

use alloc::format;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use libanyui_client as anyui;
use anyui::Widget;

use workloads::{
    NUM_CPU_TESTS, NUM_GPU_TESTS,
    CPU_BASELINES, GPU_BASELINES,
    CPU_TEST_NAMES, GPU_TEST_NAMES,
    run_cpu_bench, run_gpu_test,
};

anyos_std::entry!(main);

const TEXT_ALIGN_CENTER: u32 = 1;
const TEXT_ALIGN_RIGHT: u32 = 2;

const MAX_CORES: usize = 64;

// Colors
const BG_DARK: u32       = 0xFF1C1C1E;
const ACCENT: u32        = 0xFF0A84FF;
const ACCENT_GREEN: u32  = 0xFF30D158;
const TEXT_PRIMARY: u32  = 0xFFFFFFFF;
const TEXT_SECONDARY: u32 = 0xFF8E8E93;
const SCORE_GOLD: u32    = 0xFFFFD60A;

// ════════════════════════════════════════════════════════════════════════
//  Shared state for benchmark workers (fork-based)
// ════════════════════════════════════════════════════════════════════════

// Which benchmark to run (1-6 for CPU tests)
static BENCH_ID: AtomicU32 = AtomicU32::new(0);
// Child process TIDs from fork()
static mut CHILD_TIDS: [u32; MAX_CORES] = [0; MAX_CORES];
// Results collected from child exit codes
static mut CHILD_RESULTS: [u64; MAX_CORES] = [0; MAX_CORES];
// How many children were forked so far (may still be forking one per tick)
static mut CHILDREN_FORKED: u32 = 0;
// Total children needed for this test
static mut NUM_CHILDREN: u32 = 0;
// How many have been reaped via waitpid
static mut CHILDREN_REAPED: u32 = 0;

// ════════════════════════════════════════════════════════════════════════
//  Global application state
// ════════════════════════════════════════════════════════════════════════

struct AppState {
    win: anyui::Window,
    num_cpus: u32,

    // Tab navigation
    tabs: anyui::SegmentedControl,

    // Overview panel
    panel_overview: anyui::View,
    lbl_subtitle: anyui::Label,
    progress: anyui::ProgressBar,
    lbl_status: anyui::Label,
    btn_run_all: anyui::Button,
    btn_run_cpu: anyui::Button,
    btn_run_gpu: anyui::Button,

    // CPU panel
    panel_cpu: anyui::View,
    lbl_cpu_single_score: anyui::Label,
    lbl_cpu_multi_score: anyui::Label,
    cpu_single_labels: [anyui::Label; NUM_CPU_TESTS],
    cpu_single_scores: [anyui::Label; NUM_CPU_TESTS],
    cpu_multi_labels: [anyui::Label; NUM_CPU_TESTS],
    cpu_multi_scores: [anyui::Label; NUM_CPU_TESTS],

    // GPU panel
    panel_gpu: anyui::View,
    lbl_gpu_onscreen_score: anyui::Label,
    lbl_gpu_offscreen_score: anyui::Label,
    gpu_on_labels: [anyui::Label; NUM_GPU_TESTS],
    gpu_on_scores: [anyui::Label; NUM_GPU_TESTS],
    gpu_off_labels: [anyui::Label; NUM_GPU_TESTS],
    gpu_off_scores: [anyui::Label; NUM_GPU_TESTS],
    canvas: anyui::Canvas,

    // Benchmark state
    running: bool,
    phase: BenchPhase,
    current_test: usize,
    timer_id: u32,

    // Results storage
    cpu_single_raw: [u64; NUM_CPU_TESTS],
    cpu_multi_raw: [u64; NUM_CPU_TESTS],
    gpu_on_raw: [u64; NUM_GPU_TESTS],
    gpu_off_raw: [u64; NUM_GPU_TESTS],
}

#[derive(Clone, Copy, PartialEq)]
enum BenchPhase {
    Idle,
    CpuSingle,
    CpuMulti,
    GpuOnScreen,
    GpuOffScreen,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("app not init") }
}

// ════════════════════════════════════════════════════════════════════════
//  Fork-based benchmark worker
// ════════════════════════════════════════════════════════════════════════

/// Fork a child process that runs the given benchmark and exits with the result.
/// Returns the child TID in the parent, or 0 on fork failure.
fn fork_bench_worker(bench_id: u32) -> u32 {
    let child = anyos_std::process::fork();
    if child == 0 {
        // Child process: run benchmark, exit with result as exit code
        let result = run_cpu_bench(bench_id);
        // Cap at u32::MAX - 2 to avoid conflicting with STILL_RUNNING / error sentinels
        let code = if result > 0xFFFF_FFFD { 0xFFFF_FFFD } else { result as u32 };
        anyos_std::process::exit(code);
    }
    // Parent: child == child TID (>0), or u32::MAX on error
    if child == u32::MAX { 0 } else { child }
}

// ════════════════════════════════════════════════════════════════════════
//  Scoring
// ════════════════════════════════════════════════════════════════════════

fn compute_score(raw: u64, baseline: u64) -> u32 {
    if baseline == 0 { return 0; }
    ((raw * 1000) / baseline) as u32
}

/// Geometric mean of scores (integer approximation via log-sum in fixed-point).
fn geometric_mean(scores: &[u32]) -> u32 {
    if scores.is_empty() { return 0; }
    let mut log_sum: u64 = 0;
    let mut count = 0u32;
    for &s in scores {
        if s > 0 {
            log_sum += int_log2_fp(s as u64);
            count += 1;
        }
    }
    if count == 0 { return 0; }
    let avg_log = log_sum / count as u64;
    int_exp2_fp(avg_log)
}

/// Fixed-point log2 (16.16 format): returns log2(x) * 65536.
///
/// Uses the integer part (position of highest set bit) plus 16 iterations
/// of repeated squaring to refine the fractional part.
fn int_log2_fp(x: u64) -> u64 {
    if x <= 1 { return 0; }
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

/// Fixed-point exp2 (16.16 format): returns 2^(x / 65536).
///
/// Uses a 3rd-order polynomial (Horner form) for the fractional part:
///   2^f ~ 1 + f * ln2 + f^2 * ln2^2/2 + f^3 * ln2^3/6
fn int_exp2_fp(x: u64) -> u32 {
    let int_part = (x >> 16) as u32;
    if int_part >= 31 { return u32::MAX; }
    let base = 1u64 << int_part;
    let f = (x & 0xFFFF) as u64;
    if f == 0 { return base as u32; }
    const C1: u64 = 45426; // ln(2)       * 65536
    const C2: u64 = 15743; // ln(2)^2 / 2 * 65536
    const C3: u64 = 3634;  // ln(2)^3 / 6 * 65536
    let mut r = C3;
    r = (r * f) >> 16;
    r += C2;
    r = (r * f) >> 16;
    r += C1;
    r = (r * f) >> 16;
    r += 65536;
    let result = (base * r) >> 16;
    if result > u32::MAX as u64 { u32::MAX } else { result as u32 }
}

// ════════════════════════════════════════════════════════════════════════
//  UI Construction
// ════════════════════════════════════════════════════════════════════════

fn make_label_pair(parent: &anyui::View, name: &str, y: i32) -> (anyui::Label, anyui::Label) {
    let lbl_name = anyui::Label::new(name);
    lbl_name.set_position(16, y);
    lbl_name.set_size(200, 20);
    lbl_name.set_text_color(TEXT_PRIMARY);
    lbl_name.set_font_size(13);
    parent.add(&lbl_name);

    let lbl_score = anyui::Label::new("-");
    lbl_score.set_position(230, y);
    lbl_score.set_size(100, 20);
    lbl_score.set_text_color(TEXT_SECONDARY);
    lbl_score.set_font_size(13);
    lbl_score.set_state(TEXT_ALIGN_CENTER);
    parent.add(&lbl_score);

    (lbl_name, lbl_score)
}

fn build_ui() {
    let win = anyui::Window::new("anyBench", -1, -1, 640, 520);
    win.set_color(BG_DARK);

    let num_cpus = anyos_std::sys::sysinfo(2, &mut [0u8; 4]);
    let num_cpus = if num_cpus == 0 { 1 } else { num_cpus };

    // ── Tab bar ──
    let tabs = anyui::SegmentedControl::new("Overview|CPU|GPU");
    tabs.set_dock(anyui::DOCK_TOP);
    tabs.set_size(640, 36);
    tabs.set_margin(8, 8, 8, 0);
    win.add(&tabs);

    // ════════════════════════════════════════════════════════════════
    //  Overview Panel
    // ════════════════════════════════════════════════════════════════

    let panel_overview = anyui::View::new();
    panel_overview.set_dock(anyui::DOCK_FILL);
    panel_overview.set_color(BG_DARK);
    win.add(&panel_overview);

    let lbl_title = anyui::Label::new("anyBench");
    lbl_title.set_position(0, 20);
    lbl_title.set_size(640, 36);
    lbl_title.set_font_size(28);
    lbl_title.set_font(1);
    lbl_title.set_text_color(TEXT_PRIMARY);
    lbl_title.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_title);

    let lbl_subtitle = anyui::Label::new(&format!("Comprehensive System Benchmark  |  {} CPU Core{} detected",
        num_cpus, if num_cpus != 1 { "s" } else { "" }));
    lbl_subtitle.set_position(0, 60);
    lbl_subtitle.set_size(640, 20);
    lbl_subtitle.set_font_size(13);
    lbl_subtitle.set_text_color(TEXT_SECONDARY);
    lbl_subtitle.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_subtitle);

    let btn_run_all = anyui::Button::new("Run All Tests");
    btn_run_all.set_position(200, 110);
    btn_run_all.set_size(240, 40);
    btn_run_all.set_color(ACCENT);
    btn_run_all.set_text_color(TEXT_PRIMARY);
    btn_run_all.set_font_size(15);
    panel_overview.add(&btn_run_all);

    let btn_run_cpu = anyui::Button::new("CPU Only");
    btn_run_cpu.set_position(170, 165);
    btn_run_cpu.set_size(140, 34);
    btn_run_cpu.set_font_size(13);
    panel_overview.add(&btn_run_cpu);

    let btn_run_gpu = anyui::Button::new("GPU Only");
    btn_run_gpu.set_position(330, 165);
    btn_run_gpu.set_size(140, 34);
    btn_run_gpu.set_font_size(13);
    panel_overview.add(&btn_run_gpu);

    let progress = anyui::ProgressBar::new(0);
    progress.set_position(40, 225);
    progress.set_size(560, 10);
    panel_overview.add(&progress);

    let lbl_status = anyui::Label::new("Ready. Press 'Run All Tests' to begin.");
    lbl_status.set_position(0, 245);
    lbl_status.set_size(640, 20);
    lbl_status.set_font_size(12);
    lbl_status.set_text_color(TEXT_SECONDARY);
    lbl_status.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_status);

    // Score summary cards
    let card_labels = ["CPU Single-Core", "CPU Multi-Core", "GPU OnScreen", "GPU OffScreen"];
    let card_x = [32, 172, 340, 480];
    let _lbl_summary_titles: Vec<anyui::Label> = (0..4).map(|i| {
        let l = anyui::Label::new(card_labels[i]);
        l.set_position(card_x[i], 285);
        l.set_size(130, 16);
        l.set_font_size(11);
        l.set_text_color(TEXT_SECONDARY);
        l.set_state(TEXT_ALIGN_CENTER);
        panel_overview.add(&l);
        l
    }).collect();

    let lbl_summary_scores: Vec<anyui::Label> = (0..4).map(|i| {
        let l = anyui::Label::new("-");
        l.set_position(card_x[i], 305);
        l.set_size(130, 28);
        l.set_font_size(22);
        l.set_font(1);
        l.set_text_color(SCORE_GOLD);
        l.set_state(TEXT_ALIGN_CENTER);
        panel_overview.add(&l);
        l
    }).collect();

    let div = anyui::Divider::new();
    div.set_position(32, 350);
    div.set_size(576, 1);
    panel_overview.add(&div);

    let lbl_info = anyui::Label::new("anyBench measures CPU and GPU performance with standardized workloads.");
    lbl_info.set_position(0, 370);
    lbl_info.set_size(640, 16);
    lbl_info.set_font_size(11);
    lbl_info.set_text_color(TEXT_SECONDARY);
    lbl_info.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_info);

    let lbl_info2 = anyui::Label::new("Higher scores indicate better performance. Baseline: 1000 pts.");
    lbl_info2.set_position(0, 390);
    lbl_info2.set_size(640, 16);
    lbl_info2.set_font_size(11);
    lbl_info2.set_text_color(TEXT_SECONDARY);
    lbl_info2.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_info2);

    // ════════════════════════════════════════════════════════════════
    //  CPU Panel
    // ════════════════════════════════════════════════════════════════

    let panel_cpu = anyui::View::new();
    panel_cpu.set_dock(anyui::DOCK_FILL);
    panel_cpu.set_color(BG_DARK);
    panel_cpu.set_visible(false);
    win.add(&panel_cpu);

    let lbl_sc_header = anyui::Label::new("Single-Core Performance");
    lbl_sc_header.set_position(16, 12);
    lbl_sc_header.set_size(300, 24);
    lbl_sc_header.set_font_size(16);
    lbl_sc_header.set_font(1);
    lbl_sc_header.set_text_color(TEXT_PRIMARY);
    panel_cpu.add(&lbl_sc_header);

    let lbl_cpu_single_score = anyui::Label::new("Score: -");
    lbl_cpu_single_score.set_position(400, 12);
    lbl_cpu_single_score.set_size(220, 24);
    lbl_cpu_single_score.set_font_size(16);
    lbl_cpu_single_score.set_font(1);
    lbl_cpu_single_score.set_text_color(SCORE_GOLD);
    lbl_cpu_single_score.set_state(TEXT_ALIGN_RIGHT);
    panel_cpu.add(&lbl_cpu_single_score);

    let mut cpu_single_labels = [anyui::Label::new(""); NUM_CPU_TESTS];
    let mut cpu_single_scores = [anyui::Label::new(""); NUM_CPU_TESTS];
    for i in 0..NUM_CPU_TESTS {
        let y = 44 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_cpu, CPU_TEST_NAMES[i], y);
        cpu_single_labels[i] = ln;
        cpu_single_scores[i] = ls;
    }

    let lbl_mc_header = anyui::Label::new("Multi-Core Performance");
    lbl_mc_header.set_position(16, 226);
    lbl_mc_header.set_size(300, 24);
    lbl_mc_header.set_font_size(16);
    lbl_mc_header.set_font(1);
    lbl_mc_header.set_text_color(TEXT_PRIMARY);
    panel_cpu.add(&lbl_mc_header);

    let lbl_mc_cores = anyui::Label::new(&format!("({} Cores)", num_cpus));
    lbl_mc_cores.set_position(220, 226);
    lbl_mc_cores.set_size(100, 24);
    lbl_mc_cores.set_font_size(13);
    lbl_mc_cores.set_text_color(TEXT_SECONDARY);
    panel_cpu.add(&lbl_mc_cores);

    let lbl_cpu_multi_score = anyui::Label::new("Score: -");
    lbl_cpu_multi_score.set_position(400, 226);
    lbl_cpu_multi_score.set_size(220, 24);
    lbl_cpu_multi_score.set_font_size(16);
    lbl_cpu_multi_score.set_font(1);
    lbl_cpu_multi_score.set_text_color(SCORE_GOLD);
    lbl_cpu_multi_score.set_state(TEXT_ALIGN_RIGHT);
    panel_cpu.add(&lbl_cpu_multi_score);

    let mut cpu_multi_labels = [anyui::Label::new(""); NUM_CPU_TESTS];
    let mut cpu_multi_scores = [anyui::Label::new(""); NUM_CPU_TESTS];
    for i in 0..NUM_CPU_TESTS {
        let y = 258 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_cpu, CPU_TEST_NAMES[i], y);
        cpu_multi_labels[i] = ln;
        cpu_multi_scores[i] = ls;
    }

    // ════════════════════════════════════════════════════════════════
    //  GPU Panel
    // ════════════════════════════════════════════════════════════════

    let panel_gpu = anyui::View::new();
    panel_gpu.set_dock(anyui::DOCK_FILL);
    panel_gpu.set_color(BG_DARK);
    panel_gpu.set_visible(false);
    win.add(&panel_gpu);

    let canvas = anyui::Canvas::new(300, 180);
    canvas.set_position(320, 16);
    canvas.set_size(300, 180);
    canvas.clear(0xFF000000);
    panel_gpu.add(&canvas);

    let lbl_canvas_title = anyui::Label::new("Render Preview");
    lbl_canvas_title.set_position(320, 200);
    lbl_canvas_title.set_size(300, 16);
    lbl_canvas_title.set_font_size(10);
    lbl_canvas_title.set_text_color(TEXT_SECONDARY);
    lbl_canvas_title.set_state(TEXT_ALIGN_CENTER);
    panel_gpu.add(&lbl_canvas_title);

    let lbl_on_header = anyui::Label::new("OnScreen");
    lbl_on_header.set_position(16, 12);
    lbl_on_header.set_size(200, 24);
    lbl_on_header.set_font_size(16);
    lbl_on_header.set_font(1);
    lbl_on_header.set_text_color(TEXT_PRIMARY);
    panel_gpu.add(&lbl_on_header);

    let lbl_gpu_onscreen_score = anyui::Label::new("Score: -");
    lbl_gpu_onscreen_score.set_position(140, 12);
    lbl_gpu_onscreen_score.set_size(160, 24);
    lbl_gpu_onscreen_score.set_font_size(16);
    lbl_gpu_onscreen_score.set_font(1);
    lbl_gpu_onscreen_score.set_text_color(SCORE_GOLD);
    lbl_gpu_onscreen_score.set_state(TEXT_ALIGN_RIGHT);
    panel_gpu.add(&lbl_gpu_onscreen_score);

    let mut gpu_on_labels = [anyui::Label::new(""); NUM_GPU_TESTS];
    let mut gpu_on_scores = [anyui::Label::new(""); NUM_GPU_TESTS];
    for i in 0..NUM_GPU_TESTS {
        let y = 44 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_gpu, GPU_TEST_NAMES[i], y);
        gpu_on_labels[i] = ln;
        gpu_on_scores[i] = ls;
    }

    let lbl_off_header = anyui::Label::new("OffScreen");
    lbl_off_header.set_position(16, 230);
    lbl_off_header.set_size(200, 24);
    lbl_off_header.set_font_size(16);
    lbl_off_header.set_font(1);
    lbl_off_header.set_text_color(TEXT_PRIMARY);
    panel_gpu.add(&lbl_off_header);

    let lbl_gpu_offscreen_score = anyui::Label::new("Score: -");
    lbl_gpu_offscreen_score.set_position(140, 230);
    lbl_gpu_offscreen_score.set_size(160, 24);
    lbl_gpu_offscreen_score.set_font_size(16);
    lbl_gpu_offscreen_score.set_font(1);
    lbl_gpu_offscreen_score.set_text_color(SCORE_GOLD);
    lbl_gpu_offscreen_score.set_state(TEXT_ALIGN_RIGHT);
    panel_gpu.add(&lbl_gpu_offscreen_score);

    let mut gpu_off_labels = [anyui::Label::new(""); NUM_GPU_TESTS];
    let mut gpu_off_scores = [anyui::Label::new(""); NUM_GPU_TESTS];
    for i in 0..NUM_GPU_TESTS {
        let y = 262 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_gpu, GPU_TEST_NAMES[i], y);
        gpu_off_labels[i] = ln;
        gpu_off_scores[i] = ls;
    }

    // ── Tab switching ──
    tabs.connect_panels(&[&panel_overview, &panel_cpu, &panel_gpu]);

    unsafe {
        SUMMARY_SCORE_IDS = [
            lbl_summary_scores[0].id(),
            lbl_summary_scores[1].id(),
            lbl_summary_scores[2].id(),
            lbl_summary_scores[3].id(),
        ];
    }

    // ── Create state ──
    unsafe {
        APP = Some(AppState {
            win,
            num_cpus,
            tabs,
            panel_overview,
            lbl_subtitle,
            progress,
            lbl_status,
            btn_run_all,
            btn_run_cpu,
            btn_run_gpu,
            panel_cpu,
            lbl_cpu_single_score,
            lbl_cpu_multi_score,
            cpu_single_labels,
            cpu_single_scores,
            cpu_multi_labels,
            cpu_multi_scores,
            panel_gpu,
            lbl_gpu_onscreen_score,
            lbl_gpu_offscreen_score,
            gpu_on_labels,
            gpu_on_scores,
            gpu_off_labels,
            gpu_off_scores,
            canvas,
            running: false,
            phase: BenchPhase::Idle,
            current_test: 0,
            timer_id: 0,
            cpu_single_raw: [0; NUM_CPU_TESTS],
            cpu_multi_raw: [0; NUM_CPU_TESTS],
            gpu_on_raw: [0; NUM_GPU_TESTS],
            gpu_off_raw: [0; NUM_GPU_TESTS],
        });
    }

    // ── Button callbacks ──
    btn_run_all.on_click(|_| start_benchmark(BenchMode::All));
    btn_run_cpu.on_click(|_| start_benchmark(BenchMode::CpuOnly));
    btn_run_gpu.on_click(|_| start_benchmark(BenchMode::GpuOnly));

    win.on_close(|_| anyui::quit());
}

static mut SUMMARY_SCORE_IDS: [u32; 4] = [0; 4];
static mut BENCH_MODE: BenchMode = BenchMode::All;

#[derive(Clone, Copy, PartialEq)]
enum BenchMode {
    All,
    CpuOnly,
    GpuOnly,
}

// ════════════════════════════════════════════════════════════════════════
//  Benchmark orchestration
// ════════════════════════════════════════════════════════════════════════

fn start_benchmark(mode: BenchMode) {
    let a = app();
    if a.running { return; }

    a.running = true;
    unsafe { BENCH_MODE = mode; }

    // Reset scores
    a.cpu_single_raw = [0; NUM_CPU_TESTS];
    a.cpu_multi_raw = [0; NUM_CPU_TESTS];
    a.gpu_on_raw = [0; NUM_GPU_TESTS];
    a.gpu_off_raw = [0; NUM_GPU_TESTS];

    for i in 0..NUM_CPU_TESTS {
        a.cpu_single_scores[i].set_text("-");
        a.cpu_multi_scores[i].set_text("-");
    }
    for i in 0..NUM_GPU_TESTS {
        a.gpu_on_scores[i].set_text("-");
        a.gpu_off_scores[i].set_text("-");
    }
    a.lbl_cpu_single_score.set_text("Score: -");
    a.lbl_cpu_multi_score.set_text("Score: -");
    a.lbl_gpu_onscreen_score.set_text("Score: -");
    a.lbl_gpu_offscreen_score.set_text("Score: -");

    a.btn_run_all.set_enabled(false);
    a.btn_run_cpu.set_enabled(false);
    a.btn_run_gpu.set_enabled(false);

    match mode {
        BenchMode::All | BenchMode::CpuOnly => {
            a.phase = BenchPhase::CpuSingle;
            a.current_test = 0;
            a.progress.set_state(0);
            a.lbl_status.set_text("Starting CPU Single-Core tests...");
        }
        BenchMode::GpuOnly => {
            a.phase = BenchPhase::GpuOnScreen;
            a.current_test = 0;
            a.progress.set_state(0);
            a.lbl_status.set_text("Starting GPU OnScreen tests...");
        }
    }

    a.timer_id = anyui::set_timer(50, tick_benchmark);
}

fn total_steps() -> u32 {
    let mode = unsafe { BENCH_MODE };
    match mode {
        BenchMode::All => (NUM_CPU_TESTS * 2 + NUM_GPU_TESTS * 2) as u32,
        BenchMode::CpuOnly => (NUM_CPU_TESTS * 2) as u32,
        BenchMode::GpuOnly => (NUM_GPU_TESTS * 2) as u32,
    }
}

fn current_step() -> u32 {
    let a = app();
    let mode = unsafe { BENCH_MODE };
    let base = match a.phase {
        BenchPhase::CpuSingle => 0,
        BenchPhase::CpuMulti => NUM_CPU_TESTS as u32,
        BenchPhase::GpuOnScreen => {
            if mode == BenchMode::GpuOnly { 0 }
            else { (NUM_CPU_TESTS * 2) as u32 }
        }
        BenchPhase::GpuOffScreen => {
            if mode == BenchMode::GpuOnly { NUM_GPU_TESTS as u32 }
            else { (NUM_CPU_TESTS * 2 + NUM_GPU_TESTS) as u32 }
        }
        BenchPhase::Idle => return total_steps(),
    };
    base + a.current_test as u32
}

static BENCH_STATE: AtomicU32 = AtomicU32::new(0); // 0=ready, 1=running

fn tick_benchmark() {
    let a = app();
    if !a.running { return; }

    let state = BENCH_STATE.load(Ordering::SeqCst);

    if state == 0 {
        let progress_pct = if total_steps() > 0 {
            (current_step() * 100 / total_steps()).min(100)
        } else { 0 };
        a.progress.set_state(progress_pct);

        match a.phase {
            BenchPhase::CpuSingle => {
                if a.current_test >= NUM_CPU_TESTS {
                    a.phase = BenchPhase::CpuMulti;
                    a.current_test = 0;
                    update_cpu_single_summary();
                    a.lbl_status.set_text("Starting CPU Multi-Core tests...");
                    return;
                }
                let name = CPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("CPU Single-Core: {}...", name));
                a.cpu_single_scores[a.current_test].set_text("...");
                a.cpu_single_scores[a.current_test].set_text_color(ACCENT);

                let bench_id = (a.current_test + 1) as u32;
                BENCH_ID.store(bench_id, Ordering::SeqCst);
                unsafe {
                    NUM_CHILDREN = 1;
                    CHILDREN_FORKED = 1;
                    CHILDREN_REAPED = 0;
                    CHILD_RESULTS[0] = 0;
                    let tid = fork_bench_worker(bench_id);
                    CHILD_TIDS[0] = tid;
                    if tid == 0 {
                        CHILDREN_REAPED = 1;
                    }
                }
                BENCH_STATE.store(1, Ordering::SeqCst);
            }
            BenchPhase::CpuMulti => {
                if a.current_test >= NUM_CPU_TESTS {
                    update_cpu_multi_summary();
                    let mode = unsafe { BENCH_MODE };
                    if mode == BenchMode::CpuOnly {
                        finish_benchmark();
                        return;
                    }
                    a.phase = BenchPhase::GpuOnScreen;
                    a.current_test = 0;
                    a.lbl_status.set_text("Starting GPU OnScreen tests...");
                    return;
                }
                let name = CPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("CPU Multi-Core ({}T): {}...", a.num_cpus, name));
                a.cpu_multi_scores[a.current_test].set_text("...");
                a.cpu_multi_scores[a.current_test].set_text_color(ACCENT);

                let bench_id = (a.current_test + 1) as u32;
                BENCH_ID.store(bench_id, Ordering::SeqCst);
                let n = a.num_cpus.min(MAX_CORES as u32);
                unsafe {
                    NUM_CHILDREN = n;
                    CHILDREN_FORKED = 1;
                    CHILDREN_REAPED = 0;
                    CHILD_RESULTS[0] = 0;
                    let tid = fork_bench_worker(bench_id);
                    CHILD_TIDS[0] = tid;
                    if tid == 0 {
                        CHILDREN_REAPED += 1;
                    }
                }
                BENCH_STATE.store(1, Ordering::SeqCst);
            }
            BenchPhase::GpuOnScreen => {
                if a.current_test >= NUM_GPU_TESTS {
                    update_gpu_on_summary();
                    a.phase = BenchPhase::GpuOffScreen;
                    a.current_test = 0;
                    a.lbl_status.set_text("Starting GPU OffScreen tests...");
                    return;
                }
                let name = GPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("GPU OnScreen: {}...", name));
                a.gpu_on_scores[a.current_test].set_text("...");
                a.gpu_on_scores[a.current_test].set_text_color(ACCENT);
                BENCH_STATE.store(1, Ordering::SeqCst);

                a.canvas.clear(0xFF000000);
                let result = run_gpu_test(a.current_test, &a.canvas, false);
                a.gpu_on_raw[a.current_test] = result;
                let score = compute_score(result, GPU_BASELINES[a.current_test]);
                a.gpu_on_scores[a.current_test].set_text(&format!("{}", score));
                a.gpu_on_scores[a.current_test].set_text_color(score_color(score));
                a.current_test += 1;
                BENCH_STATE.store(0, Ordering::SeqCst);
            }
            BenchPhase::GpuOffScreen => {
                if a.current_test >= NUM_GPU_TESTS {
                    update_gpu_off_summary();
                    finish_benchmark();
                    return;
                }
                let name = GPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("GPU OffScreen: {}...", name));
                a.gpu_off_scores[a.current_test].set_text("...");
                a.gpu_off_scores[a.current_test].set_text_color(ACCENT);
                BENCH_STATE.store(1, Ordering::SeqCst);

                let result = run_gpu_test(a.current_test, &a.canvas, true);
                a.gpu_off_raw[a.current_test] = result;
                let score = compute_score(result, GPU_BASELINES[a.current_test]);
                a.gpu_off_scores[a.current_test].set_text(&format!("{}", score));
                a.gpu_off_scores[a.current_test].set_text_color(score_color(score));
                a.current_test += 1;
                BENCH_STATE.store(0, Ordering::SeqCst);
            }
            BenchPhase::Idle => {}
        }
    } else if state == 1 {
        // Fork remaining children one per tick (avoids kernel lock contention)
        let forked = unsafe { CHILDREN_FORKED };
        let total = unsafe { NUM_CHILDREN };
        if forked < total {
            let i = forked as usize;
            let bench_id = BENCH_ID.load(Ordering::SeqCst);
            unsafe {
                CHILD_RESULTS[i] = 0;
                let tid = fork_bench_worker(bench_id);
                CHILD_TIDS[i] = tid;
                CHILDREN_FORKED += 1;
                if tid == 0 {
                    CHILDREN_REAPED += 1;
                }
            }
            return;
        }

        // All children forked — check if they are done via try_waitpid
        let n = total as usize;
        for i in 0..n {
            let tid = unsafe { CHILD_TIDS[i] };
            if tid == 0 { continue; }
            let status = anyos_std::process::try_waitpid(tid);
            if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
                unsafe {
                    CHILD_RESULTS[i] = status as u64;
                    CHILD_TIDS[i] = 0;
                    CHILDREN_REAPED += 1;
                }
            }
        }
        if unsafe { CHILDREN_REAPED } >= unsafe { NUM_CHILDREN } {
            let mut total: u64 = 0;
            for i in 0..n {
                total += unsafe { CHILD_RESULTS[i] };
            }
            match a.phase {
                BenchPhase::CpuSingle => {
                    a.cpu_single_raw[a.current_test] = total;
                    let score = compute_score(total, CPU_BASELINES[a.current_test]);
                    a.cpu_single_scores[a.current_test].set_text(&format!("{}", score));
                    a.cpu_single_scores[a.current_test].set_text_color(score_color(score));
                    a.current_test += 1;
                }
                BenchPhase::CpuMulti => {
                    a.cpu_multi_raw[a.current_test] = total;
                    let score = compute_score(total, CPU_BASELINES[a.current_test]);
                    a.cpu_multi_scores[a.current_test].set_text(&format!("{}", score));
                    a.cpu_multi_scores[a.current_test].set_text_color(score_color(score));
                    a.current_test += 1;
                }
                _ => {}
            }
            BENCH_STATE.store(0, Ordering::SeqCst);
        }
    }
}

fn score_color(score: u32) -> u32 {
    if score >= 1200 { ACCENT_GREEN }
    else if score >= 800 { SCORE_GOLD }
    else if score >= 400 { 0xFFFF9F0A }
    else { 0xFFFF453A }
}

fn update_cpu_single_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_CPU_TESTS)
        .map(|i| compute_score(a.cpu_single_raw[i], CPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_cpu_single_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[0];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_cpu_multi_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_CPU_TESTS)
        .map(|i| compute_score(a.cpu_multi_raw[i], CPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_cpu_multi_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[1];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_gpu_on_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_GPU_TESTS)
        .map(|i| compute_score(a.gpu_on_raw[i], GPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_gpu_onscreen_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[2];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_gpu_off_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_GPU_TESTS)
        .map(|i| compute_score(a.gpu_off_raw[i], GPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_gpu_offscreen_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[3];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn finish_benchmark() {
    let a = app();
    a.running = false;
    a.phase = BenchPhase::Idle;
    a.progress.set_state(100);
    a.lbl_status.set_text("Benchmark complete!");
    a.lbl_status.set_text_color(ACCENT_GREEN);

    a.btn_run_all.set_enabled(true);
    a.btn_run_cpu.set_enabled(true);
    a.btn_run_gpu.set_enabled(true);

    if a.timer_id != 0 {
        anyui::kill_timer(a.timer_id);
        a.timer_id = 0;
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════════════

fn main() {
    if !anyui::init() {
        anyos_std::println!("anybench: failed to load libanyui.so");
        return;
    }

    build_ui();
    anyui::run();
}
