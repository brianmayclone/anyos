use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::fs;
use liblxecore::config::LxeConfig;

use crate::threads::is_thread_alive;

pub(crate) struct RuntimeState {
    pub(crate) config: LxeConfig,
    active_runs: Vec<u32>,
    active_processes: Vec<TrackedProcess>,
    writer_tid: u32,
    requests: u32,
    syncs: u32,
}

struct TrackedProcess {
    owner_tid: u32,
    process_tid: u32,
}

impl RuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            config: LxeConfig::load(),
            active_runs: Vec::new(),
            active_processes: Vec::new(),
            writer_tid: 0,
            requests: 0,
            syncs: 0,
        }
    }

    pub(crate) fn prepare(&self) {
        liblxecore::runtime::prepare_rootfs(&self.config);
    }

    pub(crate) fn reload(&mut self) {
        self.config = LxeConfig::load();
        self.prepare();
    }

    pub(crate) fn record_request(&mut self) {
        self.requests = self.requests.wrapping_add(1);
    }

    pub(crate) fn cleanup_dead_leases(&mut self) {
        self.active_runs.retain(|tid| is_thread_alive(*tid));
        self.active_processes
            .retain(|process| is_thread_alive(process.process_tid));
        if self.writer_tid != 0 && !is_thread_alive(self.writer_tid) {
            self.writer_tid = 0;
        }
    }

    pub(crate) fn begin_run(&mut self, tid: u32) -> String {
        self.cleanup_dead_leases();
        if self.writer_tid != 0 && self.writer_tid != tid {
            return format!("BUSY\twriter={}\n", self.writer_tid);
        }
        if !contains_tid(&self.active_runs, tid) {
            self.active_runs.push(tid);
        }
        self.ok()
    }

    pub(crate) fn end_run(&mut self, tid: u32) -> String {
        remove_tid(&mut self.active_runs, tid);
        self.sync_if_idle();
        self.ok()
    }

    pub(crate) fn begin_write(&mut self, tid: u32) -> String {
        self.cleanup_dead_leases();
        if self.writer_tid != 0 && self.writer_tid != tid {
            return format!("BUSY\twriter={}\n", self.writer_tid);
        }
        if self
            .active_runs
            .iter()
            .any(|active_tid| *active_tid != tid && is_thread_alive(*active_tid))
        {
            return format!("BUSY\tactive_runs={}\n", self.active_runs.len());
        }
        if self
            .active_processes
            .iter()
            .any(|process| process.owner_tid != tid && is_thread_alive(process.process_tid))
        {
            return format!("BUSY\tactive_processes={}\n", self.active_processes.len());
        }
        self.writer_tid = tid;
        self.ok()
    }

    pub(crate) fn end_write(&mut self, tid: u32) -> String {
        if self.writer_tid == tid {
            self.writer_tid = 0;
            self.sync_rootfs();
        }
        self.ok()
    }

    pub(crate) fn track_process(&mut self, owner_tid: u32, process_tid: u32) -> String {
        self.cleanup_dead_leases();
        if process_tid == 0 || process_tid == u32::MAX || !is_thread_alive(process_tid) {
            return format!("ERR\tinvalid process {}\n", process_tid);
        }
        if !self
            .active_processes
            .iter()
            .any(|process| process.process_tid == process_tid)
        {
            self.active_processes.push(TrackedProcess {
                owner_tid,
                process_tid,
            });
        }
        self.ok()
    }

    pub(crate) fn untrack_process(&mut self, process_tid: u32) -> String {
        self.active_processes
            .retain(|process| process.process_tid != process_tid);
        self.sync_if_idle();
        self.ok()
    }

    pub(crate) fn sync_rootfs(&mut self) {
        fs::sync();
        self.syncs = self.syncs.wrapping_add(1);
    }

    pub(crate) fn ok(&self) -> String {
        format!(
            "OK\troot={}\trootfs={}\tactive_runs={}\tactive_processes={}\twriter={}\trequests={}\tsyncs={}\n",
            self.config.root,
            self.config.rootfs,
            self.active_runs.len(),
            self.active_processes.len(),
            self.writer_tid,
            self.requests,
            self.syncs
        )
    }

    pub(crate) fn active_runs_len(&self) -> usize {
        self.active_runs.len()
    }

    pub(crate) fn active_processes_len(&self) -> usize {
        self.active_processes.len()
    }

    pub(crate) fn writer_tid(&self) -> u32 {
        self.writer_tid
    }

    pub(crate) fn syncs(&self) -> u32 {
        self.syncs
    }

    fn sync_if_idle(&mut self) {
        self.cleanup_dead_leases();
        if self.writer_tid == 0 && self.active_runs.is_empty() && self.active_processes.is_empty() {
            self.sync_rootfs();
        }
    }
}

fn contains_tid(values: &[u32], needle: u32) -> bool {
    values.iter().any(|value| *value == needle)
}

fn remove_tid(values: &mut Vec<u32>, needle: u32) {
    values.retain(|value| *value != needle);
}
