//! libuv-compatible event loop foundation for anyOS.
//!
//! This crate intentionally mirrors the shape of libuv without pulling native
//! Linux/Windows libuv into anyOS. It gives higher layers such as `libnode`
//! stable loop, timer and run-mode concepts that can grow toward fuller Node.js
//! compatibility.

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

pub type UvTimerCallback = extern "C" fn(*mut UvTimer);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvRunMode {
    Default = 0,
    Once = 1,
    NoWait = 2,
}

#[repr(C)]
#[derive(Debug)]
pub struct UvLoop {
    pub id: u32,
    pub now_ms: u64,
    pub active_handles: u32,
    stop_requested: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct UvTimer {
    pub active: bool,
    pub timeout_ms: u64,
    pub repeat_ms: u64,
    pub due_ms: u64,
    pub callback: Option<UvTimerCallback>,
    pub data: *mut u8,
}

static NEXT_LOOP_ID: AtomicU32 = AtomicU32::new(1);
static mut DEFAULT_LOOP: UvLoop = UvLoop {
    id: 0,
    now_ms: 0,
    active_handles: 0,
    stop_requested: false,
};

impl UvLoop {
    pub fn new() -> Self {
        Self {
            id: NEXT_LOOP_ID.fetch_add(1, Ordering::Relaxed),
            now_ms: now(),
            active_handles: 0,
            stop_requested: false,
        }
    }

    pub fn update_time(&mut self) {
        self.now_ms = now();
    }

    pub fn stop(&mut self) {
        self.stop_requested = true;
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_requested
    }
}

impl Default for UvLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl UvTimer {
    pub const fn new() -> Self {
        Self {
            active: false,
            timeout_ms: 0,
            repeat_ms: 0,
            due_ms: 0,
            callback: None,
            data: core::ptr::null_mut(),
        }
    }
}

impl Default for UvTimer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TimerQueue {
    timers: Vec<*mut UvTimer>,
}

impl TimerQueue {
    pub fn new() -> Self {
        Self { timers: Vec::new() }
    }

    pub fn push(&mut self, timer: *mut UvTimer) {
        if timer.is_null() || self.timers.iter().any(|candidate| *candidate == timer) {
            return;
        }
        self.timers.push(timer);
    }

    pub fn tick(&mut self, loop_: &mut UvLoop) -> usize {
        loop_.update_time();
        let mut fired = 0usize;
        for timer in &self.timers {
            let timer = unsafe { &mut **timer };
            if !timer.active || timer.due_ms > loop_.now_ms {
                continue;
            }
            fired += 1;
            if let Some(callback) = timer.callback {
                callback(timer as *mut UvTimer);
            }
            if timer.repeat_ms == 0 {
                timer.active = false;
            } else {
                timer.due_ms = loop_.now_ms.saturating_add(timer.repeat_ms);
            }
        }
        self.timers.retain(|timer| unsafe { (**timer).active });
        fired
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Timer,
    Immediate,
}

#[derive(Clone, Debug)]
pub struct ScheduledTask {
    pub id: TaskId,
    pub kind: TaskKind,
    pub due_ms: u64,
    pub repeat_ms: u64,
    pub active: bool,
}

#[derive(Debug)]
pub struct EventLoop {
    loop_: UvLoop,
    tasks: Vec<ScheduledTask>,
    next_task_id: u64,
}

impl EventLoop {
    pub fn new() -> Self {
        Self {
            loop_: UvLoop::new(),
            tasks: Vec::new(),
            next_task_id: 1,
        }
    }

    pub fn uv_loop(&self) -> &UvLoop {
        &self.loop_
    }

    pub fn uv_loop_mut(&mut self) -> &mut UvLoop {
        &mut self.loop_
    }

    pub fn now(&mut self) -> u64 {
        self.loop_.update_time();
        self.loop_.now_ms
    }

    pub fn schedule_timer(&mut self, timeout_ms: u64, repeat_ms: u64) -> TaskId {
        let id = self.next_id();
        let due_ms = self.now().saturating_add(timeout_ms);
        self.tasks.push(ScheduledTask {
            id,
            kind: TaskKind::Timer,
            due_ms,
            repeat_ms,
            active: true,
        });
        self.loop_.active_handles = self.tasks.iter().filter(|task| task.active).count() as u32;
        id
    }

    pub fn schedule_immediate(&mut self) -> TaskId {
        let id = self.next_id();
        let due_ms = self.now();
        self.tasks.push(ScheduledTask {
            id,
            kind: TaskKind::Immediate,
            due_ms,
            repeat_ms: 0,
            active: true,
        });
        self.loop_.active_handles = self.tasks.iter().filter(|task| task.active).count() as u32;
        id
    }

    pub fn cancel(&mut self, id: TaskId) -> bool {
        let mut cancelled = false;
        for task in &mut self.tasks {
            if task.id == id && task.active {
                task.active = false;
                cancelled = true;
            }
        }
        self.compact();
        cancelled
    }

    pub fn drain_due(&mut self) -> Vec<TaskId> {
        let now = self.now();
        let mut due = Vec::new();
        for task in &mut self.tasks {
            if !task.active || task.due_ms > now {
                continue;
            }
            due.push(task.id);
            if task.repeat_ms == 0 {
                task.active = false;
            } else {
                task.due_ms = now.saturating_add(task.repeat_ms);
            }
        }
        self.compact();
        due
    }

    pub fn run_once(&mut self) -> Vec<TaskId> {
        let due = self.drain_due();
        if !due.is_empty() {
            return due;
        }
        if let Some(wait_ms) = self.next_timeout_ms() {
            if wait_ms > 0 {
                anyos_std::process::sleep(wait_ms.min(u32::MAX as u64) as u32);
            }
        }
        self.drain_due()
    }

    pub fn run(&mut self, mode: UvRunMode) -> Vec<TaskId> {
        match mode {
            UvRunMode::NoWait => self.drain_due(),
            UvRunMode::Once => self.run_once(),
            UvRunMode::Default => {
                let mut fired = Vec::new();
                while self.has_active_tasks() && !self.loop_.stop_requested {
                    fired.extend(self.run_once());
                }
                fired
            }
        }
    }

    pub fn has_active_tasks(&self) -> bool {
        self.tasks.iter().any(|task| task.active)
    }

    pub fn next_timeout_ms(&mut self) -> Option<u64> {
        let now = self.now();
        self.tasks
            .iter()
            .filter(|task| task.active)
            .map(|task| task.due_ms.saturating_sub(now))
            .min()
    }

    fn next_id(&mut self) -> TaskId {
        let id = TaskId(self.next_task_id);
        self.next_task_id = self.next_task_id.saturating_add(1).max(1);
        id
    }

    fn compact(&mut self) {
        self.tasks.retain(|task| task.active);
        self.loop_.active_handles = self.tasks.len() as u32;
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub fn now() -> u64 {
    anyos_std::sys::uptime_ms() as u64
}

#[no_mangle]
pub extern "C" fn uv_default_loop() -> *mut UvLoop {
    unsafe {
        if DEFAULT_LOOP.id == 0 {
            DEFAULT_LOOP = UvLoop::new();
        }
        &raw mut DEFAULT_LOOP
    }
}

#[no_mangle]
pub extern "C" fn uv_loop_init(out_loop: *mut UvLoop) -> i32 {
    if out_loop.is_null() {
        return -1;
    }
    unsafe {
        *out_loop = UvLoop::new();
    }
    0
}

#[no_mangle]
pub extern "C" fn uv_loop_close(loop_: *mut UvLoop) -> i32 {
    if loop_.is_null() {
        return -1;
    }
    unsafe {
        (*loop_).active_handles = 0;
        (*loop_).stop_requested = true;
    }
    0
}

#[no_mangle]
pub extern "C" fn uv_update_time(loop_: *mut UvLoop) {
    if !loop_.is_null() {
        unsafe { (*loop_).update_time() };
    }
}

#[no_mangle]
pub extern "C" fn uv_now(loop_: *const UvLoop) -> u64 {
    if loop_.is_null() {
        return now();
    }
    unsafe { (*loop_).now_ms }
}

#[no_mangle]
pub extern "C" fn uv_stop(loop_: *mut UvLoop) {
    if !loop_.is_null() {
        unsafe { (*loop_).stop() };
    }
}

#[no_mangle]
pub extern "C" fn uv_run(loop_: *mut UvLoop, _mode: UvRunMode) -> i32 {
    if loop_.is_null() {
        return -1;
    }
    unsafe {
        (*loop_).update_time();
        if (*loop_).stop_requested {
            0
        } else {
            (*loop_).active_handles as i32
        }
    }
}

#[no_mangle]
pub extern "C" fn uv_timer_init(loop_: *mut UvLoop, timer: *mut UvTimer) -> i32 {
    if loop_.is_null() || timer.is_null() {
        return -1;
    }
    unsafe {
        *timer = UvTimer::new();
    }
    0
}

#[no_mangle]
pub extern "C" fn uv_timer_start(
    timer: *mut UvTimer,
    callback: Option<UvTimerCallback>,
    timeout_ms: u64,
    repeat_ms: u64,
) -> i32 {
    if timer.is_null() {
        return -1;
    }
    unsafe {
        (*timer).active = true;
        (*timer).timeout_ms = timeout_ms;
        (*timer).repeat_ms = repeat_ms;
        (*timer).due_ms = now().saturating_add(timeout_ms);
        (*timer).callback = callback;
    }
    0
}

#[no_mangle]
pub extern "C" fn uv_timer_stop(timer: *mut UvTimer) -> i32 {
    if timer.is_null() {
        return -1;
    }
    unsafe {
        (*timer).active = false;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_drains_immediate_task() {
        let mut loop_ = EventLoop::new();
        let id = loop_.schedule_immediate();
        let fired = loop_.run(UvRunMode::NoWait);
        assert_eq!(fired, alloc::vec![id]);
        assert!(!loop_.has_active_tasks());
    }

    #[test]
    fn can_cancel_scheduled_task() {
        let mut loop_ = EventLoop::new();
        let id = loop_.schedule_timer(100, 0);
        assert!(loop_.cancel(id));
        assert!(!loop_.has_active_tasks());
        assert!(loop_.run(UvRunMode::NoWait).is_empty());
    }
}
