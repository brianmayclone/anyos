use alloc::vec::Vec;

pub const MAX_CPUS: usize = 16;
pub const MAX_TASKS: usize = 128;
pub const THREAD_ENTRY_SIZE: usize = 80;
pub const ICON_SIZE: u32 = 16;
pub const GRAPH_SAMPLES: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResourceView {
    Overview,
    Processes,
    Cpu,
    Memory,
    Disk,
    Network,
    System,
}

impl ResourceView {
    pub const COUNT: usize = 7;

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Processes,
            2 => Self::Cpu,
            3 => Self::Memory,
            4 => Self::Disk,
            5 => Self::Network,
            6 => Self::System,
            _ => Self::Overview,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Processes => "Processes",
            Self::Cpu => "Processor",
            Self::Memory => "Memory",
            Self::Disk => "Storage",
            Self::Network => "Network",
            Self::System => "System",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "Live resources",
            Self::Processes => "Threads and applications",
            Self::Cpu => "Utilization and clock",
            Self::Memory => "RAM, swap, and kernel heap",
            Self::Disk => "I/O activity",
            Self::Network => "Throughput and TCP",
            Self::System => "Hardware and display",
        }
    }

    pub fn accent(self) -> u32 {
        match self {
            Self::Overview => 0xFF8AB4F8,
            Self::Processes => 0xFFE8EAED,
            Self::Cpu => 0xFF4EA1FF,
            Self::Memory => 0xFFE044A7,
            Self::Disk => 0xFFFF8A00,
            Self::Network => 0xFF26A6B8,
            Self::System => 0xFF8BC34A,
        }
    }
}

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
    pub io_read_bps: u32,
    pub io_write_bps: u32,
    pub parent_tid: u32,
    pub is_child_thread: bool,
    /// Network rate in kbit/s (tx+rx combined).
    pub net_kbit: u32,
    pub net_rx_bps: u32,
    pub net_tx_bps: u32,
}

/// Describes one visible row in the process grid.
#[derive(Clone, Copy)]
pub struct DisplayRow {
    /// 0 = standalone process (single thread), 1 = group header, 2 = child thread row
    pub kind: u8,
    /// Index into the tasks Vec.
    pub task_idx: u16,
    /// Number of threads in the group (only meaningful for kind==1).
    pub thread_count: u16,
    /// Aggregated CPU% x10 for group headers (sum of all threads).
    pub agg_cpu: u32,
    /// Aggregated network rate in kbit/s for group headers (sum of all threads).
    pub agg_net: u32,
    pub agg_pages: u32,
    pub agg_read_bps: u32,
    pub agg_write_bps: u32,
    /// Best state among group threads for headers.
    pub agg_state: u8,
}

pub struct PrevTicks {
    pub entries: [(u32, u32); MAX_TASKS],
    pub net_entries: [(u32, u64, u64); MAX_TASKS],
    pub io_entries: [(u32, u64, u64); MAX_TASKS],
    pub count: usize,
    pub prev_total: u32,
}

pub struct MemInfo {
    pub total_frames: u32,
    pub free_frames: u32,
    pub heap_used: u32,
    pub heap_total: u32,
    pub swap_total_pages: u32,
    pub swap_free_pages: u32,
    pub swap_areas: u32,
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
    pub total_cpu_freq_mhz: u32,
    pub max_freq_mhz: u32,
    pub power_features: u32,
    pub power_driver: u32,
    pub power_profile: u32,
    pub core_freq_mhz: [u32; MAX_CPUS],
}

#[derive(Clone, Copy)]
pub struct NetTotals {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub tcp_curr_established: u32,
    pub tcp_conn_errors: u32,
}

pub struct CpuState {
    pub num_cpus: u32,
    pub total_sched_ticks: u32,
    pub overall_pct: u32,
    pub avg_freq_mhz: u32,
    pub total_freq_mhz: u32,
    pub max_freq_mhz: u32,
    pub core_pct: [u32; MAX_CPUS],
    pub core_freq_mhz: [u32; MAX_CPUS],
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
            avg_freq_mhz: 0,
            total_freq_mhz: 0,
            max_freq_mhz: 0,
            core_pct: [0; MAX_CPUS],
            core_freq_mhz: [0; MAX_CPUS],
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
        CpuHistory {
            samples: [[0; GRAPH_SAMPLES]; MAX_CPUS],
            pos: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, cpu: &CpuState) {
        for i in 0..(cpu.num_cpus as usize).min(MAX_CPUS) {
            self.samples[i][self.pos] = cpu.core_pct[i].min(100) as u8;
        }
        self.pos = (self.pos + 1) % GRAPH_SAMPLES;
        if self.count < GRAPH_SAMPLES {
            self.count += 1;
        }
    }

    pub fn get(&self, core: usize, age: usize) -> u8 {
        if age >= self.count {
            return 0;
        }
        let idx = (self.pos + GRAPH_SAMPLES - 1 - age) % GRAPH_SAMPLES;
        self.samples[core][idx]
    }
}

pub struct MetricHistory {
    pub samples: [u32; GRAPH_SAMPLES],
    pub pos: usize,
    pub count: usize,
    pub last_was_decay: bool,
}

impl MetricHistory {
    pub fn new() -> Self {
        Self {
            samples: [0; GRAPH_SAMPLES],
            pos: 0,
            count: 0,
            last_was_decay: false,
        }
    }

    pub fn push(&mut self, value: u32) {
        self.samples[self.pos] = value;
        self.pos = (self.pos + 1) % GRAPH_SAMPLES;
        if self.count < GRAPH_SAMPLES {
            self.count += 1;
        }
        self.last_was_decay = false;
    }

    pub fn push_burst_smoothed(&mut self, value: u32) {
        if value > 0 {
            self.push(value);
            return;
        }

        let decayed = if self.count > 0 && !self.last_was_decay {
            self.get(0) / 2
        } else {
            0
        };

        self.samples[self.pos] = decayed;
        self.pos = (self.pos + 1) % GRAPH_SAMPLES;
        if self.count < GRAPH_SAMPLES {
            self.count += 1;
        }
        self.last_was_decay = decayed > 0;
    }

    pub fn get(&self, age: usize) -> u32 {
        if age >= self.count {
            return 0;
        }
        let idx = (self.pos + GRAPH_SAMPLES - 1 - age) % GRAPH_SAMPLES;
        self.samples[idx]
    }

    pub fn max(&self) -> u32 {
        let mut m = 0;
        for i in 0..self.count {
            let v = self.get(i);
            if v > m {
                m = v;
            }
        }
        m
    }
}

pub struct ActivityHistory {
    pub cpu: MetricHistory,
    pub cpu_freq: MetricHistory,
    pub memory: MetricHistory,
    pub heap: MetricHistory,
    pub swap: MetricHistory,
    pub disk_read: MetricHistory,
    pub disk_write: MetricHistory,
    pub net_rx: MetricHistory,
    pub net_tx: MetricHistory,
    pub process_count: MetricHistory,
}

impl ActivityHistory {
    pub fn new() -> Self {
        Self {
            cpu: MetricHistory::new(),
            cpu_freq: MetricHistory::new(),
            memory: MetricHistory::new(),
            heap: MetricHistory::new(),
            swap: MetricHistory::new(),
            disk_read: MetricHistory::new(),
            disk_write: MetricHistory::new(),
            net_rx: MetricHistory::new(),
            net_tx: MetricHistory::new(),
            process_count: MetricHistory::new(),
        }
    }
}

pub struct IconEntry {
    pub name: alloc::string::String,
    pub pixels: Vec<u32>,
}
