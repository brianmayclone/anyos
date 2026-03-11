use alloc::vec::Vec;

pub const MAX_CPUS: usize = 16;
pub const MAX_TASKS: usize = 64;
pub const THREAD_ENTRY_SIZE: usize = 60;
pub const ICON_SIZE: u32 = 16;
pub const GRAPH_SAMPLES: usize = 60;

pub struct TaskEntry {
    pub tid: u32,
    pub name: [u8; 24],
    pub name_len: usize,
    pub state: u8,
    pub priority: u8,
    pub arch: u8,
    pub uid: u16,
    pub user_pages: u32,
    pub cpu_pct_x10: u32,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

pub struct PrevTicks {
    pub entries: [(u32, u32); MAX_TASKS],
    pub count: usize,
    pub prev_total: u32,
}

pub struct MemInfo {
    pub total_frames: u32,
    pub free_frames: u32,
    pub heap_used: u32,
    pub heap_total: u32,
}

pub struct HwInfo {
    pub brand: [u8; 48],
    pub vendor: [u8; 16],
    pub tsc_mhz: u32,
    pub cpu_count: u32,
    pub boot_mode: u32,
    pub total_mem_mib: u32,
    pub free_mem_mib: u32,
    pub fb_width: u32,
    pub fb_height: u32,
    pub fb_bpp: u32,
    pub cpu_freq_mhz: u32,
    pub max_freq_mhz: u32,
    pub power_features: u32,
}

pub struct CpuState {
    pub num_cpus: u32,
    pub total_sched_ticks: u32,
    pub overall_pct: u32,
    pub core_pct: [u32; MAX_CPUS],
    pub prev_total: u32,
    pub prev_idle: u32,
    pub prev_core_total: [u32; MAX_CPUS],
    pub prev_core_idle: [u32; MAX_CPUS],
}

impl CpuState {
    pub fn new() -> Self {
        CpuState {
            num_cpus: 1,
            total_sched_ticks: 0,
            overall_pct: 0,
            core_pct: [0; MAX_CPUS],
            prev_total: 0,
            prev_idle: 0,
            prev_core_total: [0; MAX_CPUS],
            prev_core_idle: [0; MAX_CPUS],
        }
    }
}

pub struct CpuHistory {
    pub samples: [[u8; GRAPH_SAMPLES]; MAX_CPUS],
    pub pos: usize,
    pub count: usize,
}

impl CpuHistory {
    pub fn new() -> Self {
        CpuHistory { samples: [[0; GRAPH_SAMPLES]; MAX_CPUS], pos: 0, count: 0 }
    }

    pub fn push(&mut self, cpu: &CpuState) {
        for i in 0..(cpu.num_cpus as usize).min(MAX_CPUS) {
            self.samples[i][self.pos] = cpu.core_pct[i].min(100) as u8;
        }
        self.pos = (self.pos + 1) % GRAPH_SAMPLES;
        if self.count < GRAPH_SAMPLES { self.count += 1; }
    }

    pub fn get(&self, core: usize, age: usize) -> u8 {
        if age >= self.count { return 0; }
        let idx = (self.pos + GRAPH_SAMPLES - 1 - age) % GRAPH_SAMPLES;
        self.samples[core][idx]
    }
}

pub struct IconEntry {
    pub name: alloc::string::String,
    pub pixels: Vec<u32>,
}
