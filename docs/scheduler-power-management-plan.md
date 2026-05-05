# Scheduler, Memory, and CPU Power Management Plan

This document tracks the stability contract and rollout plan for scheduler,
context-switch, memory-management, and CPU power-management work.

## Current State

Implemented in this branch:

- x86-64 context switching validates context canary, checksum, stack pointer,
  and return address before loading a saved context.
- AArch64 context switching now follows the same save/restore contract: context
  fields are written first, then canary and checksum, then `save_complete` is
  published after a store barrier. Restore validates `save_complete`, canary,
  checksum, stack pointer, and kernel return address before branching.
- The scheduler skips ready threads while `save_complete == 0`; this prevents a
  second CPU from restoring a context that is still being saved by another CPU.
- CPU power management now has a common HAL-facing policy layer plus separate
  x86 platform backends:
  - `kernel/src/arch/x86/power/intel.rs`
  - `kernel/src/arch/x86/power/amd.rs`
  - `kernel/src/arch/x86/power/kvm.rs`
- The HAL exposes CPU power profile query/set/sync functions.
- `sys_sysinfo(cmd=5)` exposes CPU power status and profile control to
  privileged userspace.
- `anyos_std::sys` exposes `cpu_power_info()` and `set_cpu_power_profile()`.
- The Settings app has a Power page with persisted profile, placement policy,
  and efficiency-bias settings.
- `init` registers the power-profile confd schema and applies the stored profile
  during boot after confd is ready.

## Absolute Context-Switch Stability Contract

Context switches must never jump to a stale, partially saved, user-controlled,
or corrupted return address. The invariant is:

1. A runnable thread may only be restored when `save_complete == 1`.
2. The assembly save path must write all CPU context fields before publishing
   `save_complete`.
3. The final save path must write:
   - all register fields,
   - stack pointer,
   - return/program counter,
   - address-space pointer,
   - canary,
   - checksum,
   - then `save_complete`.
4. The restore path must validate before loading state:
   - `save_complete == 1`,
   - canary matches `CANARY_MAGIC`,
   - checksum matches the saved fields,
   - stack pointer is in a valid kernel stack range,
   - return/program counter is in valid kernel text/trampoline range.
5. Context restore must halt the current CPU or enter a non-returning diagnostic
   path on validation failure. It must never "try anyway".
6. Scheduler code must clear `save_complete` before releasing a context to the
   low-level switch path and may only requeue a thread as eligible after the save
   side marks it complete.

Near-term hardening:

- Add KUnit tests for `CpuContext::compute_checksum()` and corrupted context
  rejection on both x86-64 and AArch64.
- Add a context-switch torture test that forces migration, timer preemption,
  thread exit, signal delivery, and address-space switches under load.
- Add per-thread counters for `save_in_progress`, context restore failures,
  and stale TID discard events.
- Add explicit linker-provided kernel text bounds so assembly range checks do not
  rely on fixed constants.
- Add stack metadata validation against the scheduler's known kernel-stack
  allocations once a lock-free or interrupt-safe lookup is available.

## Memory-Management Hot Paths

Stability expectations:

- Kernel stacks must have guard pages wherever the architecture mapping layer can
  enforce them.
- Page-table switches must be synchronized with scheduler state so a thread never
  resumes with another process' address space.
- TLB/PCID handling must preserve correctness before performance. Avoiding TLB
  flushes is only allowed when CR3/PCID identity is known.
- Heap and frame allocators must never reuse memory that still belongs to a
  runnable or save-in-progress thread.
- Deferred thread reaping must keep stacks and contexts alive until no CPU can be
  executing, saving, or restoring them.

Planned checks:

- Audit all paths that free `Thread`, kernel stack, user page table, signal
  frame, and FPU state.
- Add a "poison after safe point" mode for freed kernel stacks and contexts.
- Add per-CPU epoch accounting for scheduler-critical reclamation.
- Add stress tests for fork/exec/exit with forced preemption and cross-CPU wakeup.
- Add MMU fault telemetry that records current TID, CPU, CR3/TTBR, PC, and SP.

## CPU Power HAL

The common power policy lives in `kernel/src/arch/x86/power.rs`; vendor and
virtualization behavior is split into platform files.

Driver IDs:

- `0`: none
- `1`: Intel HWP
- `2`: Intel legacy PERF_CTL
- `3`: AMD P-state
- `4`: KVM/host fallback

Profiles:

- `0`: Power saver
- `1`: Balanced
- `2`: Performance

Behavior:

- Intel HWP writes `MSR_HWP_REQUEST` with min/max ratio and EPP-style profile
  hints.
- Intel legacy writes `MSR_PERF_CTL` ratio limits when HWP is unavailable.
- AMD writes `MSR_AMD_PSTATE_CTRL` when P-state MSRs are available and the system
  is not a hypervisor guest.
- KVM/hypervisor mode records the host-facing CPU surface and reports frequency
  using TSC/APERF data, but does not perform unsafe host P-state writes unless a
  safe synthetic backend is added later.

Next steps:

- Add APERF/MPERF per-CPU sampling instead of global deltas.
- Add CPPC/ACPI support for modern AMD/Intel systems where firmware owns
  frequency selection.
- Add explicit virtualization backends for KVM paravirtual hints if the host
  exposes them.
- Add package/core idle state tracking and residency counters.
- Add thermal throttling and battery/AC state input once platform firmware
  surfaces are available.

## confd Schema

Namespace: `profile/power`

System-scope keys:

- `config/profile`: active energy profile, default `1`.
- `scheduler/placement`: task placement policy, default `1`.
- `scheduler/efficiency_bias`: scheduler efficiency bias percent, default `50`.

Boot behavior:

1. `init` waits for confd readiness.
2. `init` registers the `profile/power` schema.
3. `init` loads the stored `config/profile`.
4. `init` applies the profile through `anyos_std::sys::set_cpu_power_profile()`.

Settings behavior:

- The Settings Power page reads and writes these keys through confd.
- Profile changes are applied immediately through the kernel sysinfo control path.
- Scheduler placement and efficiency bias are currently persisted and ready for
  the next scheduler integration stage.

## Power-Aware Scheduler Roadmap

Current scheduler strengths:

- O(1) priority selection using bitmap-indexed per-priority FIFO queues.
- Per-CPU run queues.
- Work stealing from the lowest-priority queued task.
- Cross-CPU wakeup support.
- AArch64 continuation pinning for contexts that must resume on the last CPU.
- Context-save gating with `save_complete`.

Current scheduler gaps versus Linux and Windows:

- No virtual runtime or EEVDF-style fairness model.
- No utilization tracking comparable to Linux PELT.
- No CPU capacity model for heterogeneous cores.
- No energy model for idle consolidation or package-level savings.
- No dynamic priority boosting similar to Windows interactive scheduling.
- No deadline scheduling class.
- No NUMA/cache topology awareness.
- No policy feedback from actual frequency, APERF/MPERF, thermal state, or idle
  residency.

Target design:

1. CPU topology model
   - Track package, core, SMT sibling, NUMA node, cache domain, and capacity.
   - Classify cores as performance, efficiency, or symmetric.
   - Store per-CPU capacity and energy cost.

2. Task utilization model
   - Track runnable time, sleep time, wakeup frequency, latency sensitivity, and
     migration cost.
   - Maintain an EWMA utilization signal per task and per CPU.
   - Record interactive wakeups separately from CPU-bound behavior.

3. Scheduler classes
   - Realtime class: strict priority and affinity.
   - Deadline class: later phase, EDF/constant-bandwidth style.
   - Fair class: virtual runtime or EEVDF-style eligible deadline.
   - Idle/background class: explicitly migratable and energy biased.

4. Energy profiles
   - Power saver: consolidate background/fair work onto efficiency cores or a
     minimal CPU set, keep performance cores idle longer, avoid turbo requests.
   - Balanced: prefer efficiency cores for low utilization, use performance
     cores for latency-sensitive or bursty tasks.
   - Performance: spread load earlier, prefer performance cores, allow higher
     frequency requests.

5. Placement policy
   - Wakeup path chooses the cheapest CPU that satisfies latency and capacity.
   - Periodic balancer moves long-running work away from saturated CPUs.
   - Work stealing becomes capacity- and energy-aware rather than simply taking
     the lowest-priority queued task.
   - Migration must respect affinity, pinned continuations, realtime/deadline
     classes, and cache-hot thresholds.

6. Frequency feedback
   - Scheduler profile changes update the CPU power HAL.
   - Per-CPU frequency sampling feeds capacity estimates.
   - Sustained saturation can request higher profile behavior within policy.
   - Low utilization can bias toward idle consolidation and lower caps.

7. Validation
   - Compare fairness with `hackbench`-style and mixed interactive workloads.
   - Compare latency with timer wakeup and input-event microbenchmarks.
   - Compare throughput with CPU-bound multi-thread tests.
   - Compare efficiency with idle residency, average MHz, and work-per-joule
     proxies where hardware data is available.

## Linux and Windows Parity Assessment

AnyOS is not yet equivalent to Linux or Windows scheduling. The current design is
deterministic and fast for priority selection, but Linux and Windows both have
many years of work in fairness, utilization estimation, heterogeneous CPU
placement, thermal response, and interactive latency.

Linux comparison:

- Linux CFS/EEVDF provides fine-grained fairness and lag/deadline based
  selection.
- Linux energy-aware scheduling uses CPU capacity, utilization, and energy
  models.
- Linux has mature cpufreq/cpuidle integration and topology-aware load balance.

Windows comparison:

- Windows uses priority-driven scheduling with dynamic boosts, quantum behavior,
  processor groups, core parking, and heterogeneous scheduling policies.
- Modern Windows integrates preferred-core and performance/efficiency-core data
  from firmware and hardware feedback.

AnyOS target:

- Keep the current O(1) priority queues for realtime and fast dispatch.
- Add a fair scheduling class for general tasks.
- Add CPU capacity and energy models before claiming heterogeneous-core parity.
- Add benchmark gates so changes are measured against fairness, tail latency,
  throughput, migration rate, and energy proxy metrics.

## Rollout Order

1. Stabilize context switch invariants on all architectures.
2. Add context/MM torture tests and scheduler telemetry.
3. Land CPU power HAL backends and userspace settings persistence.
4. Add per-CPU power/frequency telemetry.
5. Add topology discovery and P/E-core classification.
6. Integrate persisted scheduler placement and efficiency bias.
7. Add fair scheduling class and utilization tracking.
8. Make work stealing and wakeup placement energy/capacity aware.
9. Add benchmark suite and publish Linux/Windows comparison data.

