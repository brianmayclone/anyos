//! CPU utilization monitor kernel thread.
//!
//! Samples scheduler tick counters every ~100 ms, computes the CPU busy percentage,
//! and writes the result to the `sys:cpu_load` named pipe for consumption by userspace
//! (e.g. the Settings or system monitor application).

use crate::ipc::pipe;
use crate::task::scheduler;

static mut CPU_PIPE_ID: u32 = 0;

/// Entry point for the cpu_monitor kernel thread.
pub extern "C" fn start() {
    crate::serial_println!("  cpu_monitor started");

    // Create the named pipe for CPU load data
    let pipe_id = pipe::create("sys:cpu_load");
    unsafe { CPU_PIPE_ID = pipe_id; }

    let mut prev_total = scheduler::total_sched_ticks();
    let mut prev_idle = scheduler::idle_sched_ticks();

    loop {
        // Sleep ~100ms using blocking sleep (no CPU waste)
        let sleep_ticks = crate::arch::x86::pit::TICK_HZ / 10;
        let wake_at = crate::arch::x86::pit::get_ticks().wrapping_add(sleep_ticks);
        scheduler::sleep_until(wake_at);

        let total = scheduler::total_sched_ticks();
        let idle = scheduler::idle_sched_ticks();

        let delta_total = total.wrapping_sub(prev_total);
        let delta_idle = idle.wrapping_sub(prev_idle);

        let cpu_pct = if delta_total > 0 {
            100u32.saturating_sub(delta_idle.saturating_mul(100) / delta_total)
        } else {
            0
        };

        prev_total = total;
        prev_idle = idle;

        // Write cpu_pct as 4 bytes LE to the pipe (overwrite: clear then write)
        pipe::clear(pipe_id);
        let bytes = cpu_pct.to_le_bytes();
        pipe::write(pipe_id, &bytes);
    }
}
