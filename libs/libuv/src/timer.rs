use alloc::vec::Vec;

use crate::loop_::UvLoop;
use crate::time::now;

pub type UvTimerCallback = extern "C" fn(*mut UvTimer);

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

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[no_mangle]
pub extern "C" fn uv_timer_init(loop_: *mut UvLoop, timer: *mut UvTimer) -> i32 {
    if loop_.is_null() || timer.is_null() {
        return crate::UV_EINVAL;
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
        return crate::UV_EINVAL;
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
        return crate::UV_EINVAL;
    }
    unsafe {
        (*timer).active = false;
    }
    0
}
