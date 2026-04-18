//! CPU sampling profiler.
//!
//! Periodically suspends/resumes the target thread and records RIP to
//! build an instruction-pointer histogram for hotspot detection.

use alloc::vec::Vec;

/// A single CPU sample.
#[derive(Clone, Copy)]
pub struct Sample {
    /// Timestamp (uptime_ms at sample time).
    pub timestamp: u64,
    /// Target TID.
    pub tid: u32,
    /// Instruction pointer at sample time.
    pub rip: u64,
}

/// Sampling profiler state.
pub struct Sampler {
    /// Collected samples.
    pub samples: Vec<Sample>,
    /// Whether sampling is active.
    pub active: bool,
    /// Sample interval in milliseconds.
    pub interval_ms: u32,
    /// Maximum number of samples to collect.
    pub max_samples: usize,
}

impl Sampler {
    /// Create a new sampler.
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            active: false,
            interval_ms: 10,
            max_samples: 10000,
        }
    }

    /// Start sampling.
    pub fn start(&mut self) {
        self.samples.clear();
        self.active = true;
    }

    /// Stop sampling.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Record a sample if active and below limit.
    pub fn record(&mut self, tid: u32, rip: u64) {
        if !self.active || self.samples.len() >= self.max_samples {
            return;
        }
        let timestamp = anyos_std::sys::uptime_ms() as u64;
        self.samples.push(Sample { timestamp, tid, rip });
    }

    /// Clear all samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}
