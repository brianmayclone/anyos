//! Parallel job execution for build tasks.
//!
//! Manages a pool of compilation jobs that can run concurrently,
//! respecting dependency ordering from the topological sort.

use crate::prelude::*;
use anyos_std::println;

/// State of a job in the build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Waiting for dependencies to complete.
    Pending,
    /// Currently compiling.
    Running,
    /// Compilation succeeded.
    Done,
    /// Compilation failed.
    Failed,
}

/// A compilation job.
pub struct Job {
    pub index: usize,
    pub name: String,
    pub state: JobState,
    /// Indices of jobs this depends on.
    pub deps: Vec<usize>,
}

/// Simple job scheduler that respects dependency order.
/// For anyOS, where threads use Thread::spawn_with_stack(), we keep it
/// sequential for now but structure it for easy parallelization.
pub struct JobQueue {
    jobs: Vec<Job>,
    max_parallel: u32,
}

impl JobQueue {
    pub fn new(max_parallel: u32) -> Self {
        Self {
            jobs: Vec::new(),
            max_parallel: max_parallel.max(1),
        }
    }

    pub fn add_job(&mut self, index: usize, name: String, deps: Vec<usize>) {
        self.jobs.push(Job {
            index,
            name,
            state: JobState::Pending,
            deps,
        });
    }

    /// Get the next job that is ready to run (all deps are Done).
    pub fn next_ready(&self) -> Option<usize> {
        for (i, job) in self.jobs.iter().enumerate() {
            if job.state != JobState::Pending {
                continue;
            }
            let all_deps_done = job.deps.iter().all(|&dep_idx| {
                self.jobs.iter().any(|j| j.index == dep_idx && j.state == JobState::Done)
            });
            if all_deps_done {
                return Some(i);
            }
        }
        None
    }

    /// Mark a job as running.
    pub fn mark_running(&mut self, queue_idx: usize) {
        self.jobs[queue_idx].state = JobState::Running;
    }

    /// Mark a job as done.
    pub fn mark_done(&mut self, queue_idx: usize) {
        self.jobs[queue_idx].state = JobState::Done;
    }

    /// Mark a job as failed.
    pub fn mark_failed(&mut self, queue_idx: usize) {
        self.jobs[queue_idx].state = JobState::Failed;
    }

    /// Check if all jobs are done or failed.
    pub fn is_finished(&self) -> bool {
        self.jobs.iter().all(|j| j.state == JobState::Done || j.state == JobState::Failed)
    }

    /// Check if any job failed.
    pub fn has_failures(&self) -> bool {
        self.jobs.iter().any(|j| j.state == JobState::Failed)
    }

    /// Count running jobs.
    pub fn running_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.state == JobState::Running).count()
    }

    /// Get the build order as a sequence of job indices.
    /// For sequential execution, this returns jobs in dependency order.
    pub fn sequential_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        let mut done = Vec::new();

        loop {
            let mut found = false;
            for (i, job) in self.jobs.iter().enumerate() {
                if done.contains(&i) { continue; }
                let all_deps_done = job.deps.iter().all(|&dep_idx| {
                    self.jobs.iter().enumerate().any(|(j, jj)| jj.index == dep_idx && done.contains(&j))
                });
                if all_deps_done {
                    order.push(i);
                    done.push(i);
                    found = true;
                    break;
                }
            }
            if !found { break; }
        }

        order
    }
}
