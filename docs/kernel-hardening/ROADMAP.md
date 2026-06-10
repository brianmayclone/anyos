# anyOS Kernel + LXE + Compositor — Hardening Roadmap

_Generated from a 12-agent deep audit (branch `feat/kernel-lxe-hardening`). Raw findings: [audit-raw.json](audit-raw.json), per-subsystem detail: [AUDIT-FINDINGS.md](AUDIT-FINDINGS.md)._

## Scope Decisions (2026-05-31)

1. **Test-ownership = mixed.** Claude builds + runs the kunit harness and host `cargo test` headlessly; the user drives visual/GUI QEMU runs.
2. **aarch64 = best-effort.** x86_64 is the shipping target; the full HAL trait boundary and cfg-fork cleanup are not mandatory (deferred for ARM).
3. **Windows/WXE ABI = frozen.** Neither hardened nor removed; focus is anyOS + LXE.
4. **CoreFS = kept** as the template for the `Filesystem`-trait migration (it is the only FS already routed fully through the trait).
5. **BIOS dropped — UEFI-only.** The kunit harness boots the UEFI image (OVMF). The stage1/stage2 bootloader, `bios-image`/`run-bios` cmake targets, and `test.sh`'s BIOS assumption are removed in Phase 0. (Default `ninja` already builds `anyos-uefi.img`, which is why `test.sh --kunit` could not find the BIOS `anyos.img`.)

## Baseline (2026-05-31)

Headless UEFI boot (OVMF + AHCI, `-display none -serial file:`) reaches both kunit summaries in **~8 s wall**. Integration tests: **ALL PASS** (3 suites, 79 assertions). Unit tests: **FAIL — 22/1319 assertions, 5/16 suites** — pre-existing, not introduced by this branch:

| Suite | Failing case(s) | Likely cause |
|---|---|---|
| `net::types` | `ipv4_parse_valid_packet` → got `None` | parser-vs-test mismatch — investigate which side is wrong |
| `memory` | `heap_stats_nonzero` → used bytes == 0 | heap "used bytes" stat appears unwired (real observability gap) |
| `heap::stress` | `heap_stats_monotone` → used bytes == 0 | same heap-stat root cause |
| `task::capabilities` | `parse_multiple_caps`, `cap_all_has_all_bits` (CAP_ALL=0xFFFF vs test's 0x7FFF), `required_cap_basic_syscalls_zero`, `required_cap_fs_always_allowed` | stale tests — capability model changed (more bits, stricter); reconcile to the intended model |

**Phase 0 gate:** drive these to green (fixing the correct side — code vs test — per case), then "both ALL PASS" + isa-debug-exit `rc=0` becomes the enforced regression gate.

### Baseline update — Phase 0 Batch 1 (2026-05-31): GREEN

All five failing suites were **stale/fragile tests, not code bugs** (verified per case):
- `task::capabilities` — capability model grew to 16 bits (`CAP_DISPLAY`, `CAP_HYPERVISOR`; `CAP_ALL = 0xFFFF`) and filesystem syscalls are now correctly gated by `CAP_FILESYSTEM`; tests updated to the current model.
- `net::types::ipv4_parse_valid_packet` — `ipv4::parse` correctly validates the IPv4 header checksum ([ipv4.rs:53](../../kernel/src/net/ipv4.rs#L53)); test packet had an invalid (zero) checksum → fixed to `0xB77B`.
- `memory::heap_stats_nonzero` + `heap::stress::heap_stats_monotone` — `heap_stats().used` correctly **excludes** the per-CPU bucket cache, so it reads ~0 right after heap init when nothing is live; tests now hold a live allocation before asserting.
- `vfs_readahead` — read-ahead window/cap constants were retuned 4× larger (clean named constants); the suite now asserts the **policy** against `pub(crate)` `EXFAT_READAHEAD_*` constants so it won't rot on the next retune.

Plus a **deterministic completion signal**: `kunit::report_and_exit` prints `KUNIT-DONE rc=N` and writes QEMU `isa-debug-exit` (port `0xf4`) → QEMU exits **33 = all pass / 35 = failures**. Result: `KUnit unit: ALL PASS — 16 suites, 1328 assertions`, `integration: ALL PASS — 3 suites, 79 assertions`, headless UEFI boot in ~8 s. This is now the regression gate.

## Executive Summary

anyOS is a large (~156k-line), genuinely ambitious no_std Rust OS with several well-designed cores (O(1) bitmap run-queue, table-driven PCI probe, two-stage buddy+physmap, a clean staged boot orchestrator, a real kunit framework). But it is carrying heavy accreted-stability-patch debt and three pervasive architectural anti-patterns that explain ALL nine reported problems: (1) one global IRQ-disabling SCHEDULER spinlock that every syscall takes (current_thread_abi() is confirmed lock-based at thread_config.rs:89-98 despite a comment claiming lock-free); (2) clean traits that exist but are bypassed (Filesystem trait coexists with 18+ fs_id match sites and 11 typed VfsState fields in a 6952-line vfs/mod.rs; arch::hal bypassed by 420 direct arch::x86:: refs; drivers::hal::Driver too thin so storage/net/gpu each invent their own singleton model); and (3) raw user-pointer dereferences and data-derived panics on the kernel trust boundary (sys_time/sys_set_time confirmed deref user ptrs with only a NULL check; cluster_to_lba panics the whole kernel on a bad cluster). Swap is confirmed a non-functional registry-only stub (its own docstring admits 'policy bits out of the first slice', and write_page/read_page have zero callers). The single most important sequencing decision: there is NO fast headless test loop today (kunit needs a full userspace image build + QEMU boot, and exits only via log-scraping + SIGKILL), so EVERY later fix is currently regression-unsafe. The four things to fix FIRST, in order: (A) build a fast, deterministic, kernel-only kunit harness with isa-debug-exit so Claude can run regression-safe headless tests in seconds; (B) close the kernel-memory-corruption trust-boundary holes (raw user-ptr derefs, cluster_to_lba panic, wait4 signal encoding, the heap grow_heap OOM watermark inconsistency, the futex lost-wakeup and deferred_wake lost-wakeup); (C) only with tests in place, kill the per-syscall global SCHEDULER lock and route VFS through the Filesystem trait — the two refactors that unblock both performance and the file-length problem; (D) implement the swap reclaim/eviction policy half and CoW fork, which are the foundation for graceful behavior under memory pressure. The honest state: stability and correctness are the priority over features; the engine bones are good but the trust boundaries and the global lock must be hardened before any further feature or refactor work is safe.

## Problem Assessments (the 9 stated problems)

### Problem 1: Files are far too long / unmaintainable  —  **severity: high**

**Current state:** Confirmed across every subsystem. The worst offenders: kernel/src/fs/vfs/mod.rs (6952 lines, the largest file, with read()/write() being ~250-line if-else ladders over magic fs_id integers plus ~30 parallel Detached* lock-free plan functions); kernel/src/fs/exfat.rs (4072); libs/libanyui/src/lib.rs (4808, 241 #[no_mangle] FFI exports for every control mixed together); libs/libanyui/src/event_loop.rs (4270); kernel/src/drivers/gpu/virtio_gpu.rs (3191); system/compositor/compositor/src/desktop/window.rs (3123); kernel/src/task/scheduler/mod.rs (2786, a god-module with schedule_inner alone ~610 lines plus 7 inline kunit tests); kernel/src/drivers/storage/ahci.rs (2648); kernel/src/arch/x86/idt.rs (2356, mixing fault recovery with an embedded UART printf and vsyscall emulation); kernel/src/syscall/handlers/display.rs (2156, with every sys_* duplicated per-arch); libs/liblxecore/src/package.rs (2834, an 8-concern god-file); plus five LXE files 1071-1788 lines.

**Root causes:** Two structural drivers: (a) clean dispatch traits exist but are bypassed, so all per-FS/per-device/per-control logic accretes inline in one dispatch file instead of in driver modules; (b) per-function #[cfg(target_arch)] duplication (590 cfg attrs in portable code) writes every arch's body twice in the same file. Each auditor supplied concrete split_suggestions.

**Key findings:**
- vfs/mod.rs:3976 read() and :4221 write() are if-else ladders on file.fs_id; grep finds 36 fs_id branch sites; ~30 Detached* prepare/execute/finish functions duplicate the locked path (vfs/mod.rs:985-3947).
- libanyui lib.rs:423 single static mut STATE + 241 FFI exports; event_loop.rs inlines ComboBox/DropDown/AutoComplete key handling (event_loop.rs:1860-2095) that belongs in the controls.
- scheduler/debug_trace.rs (1297) is the anyTrace debugger living inside the scheduler god-module; should move to kernel/src/debug/.
- virtual_mem.rs:1262-1342 (clone walker) and 1695-1786 (destroy walker) are near-identical 4-deep page-table recursions with subtly different skip rules — divergence is a real double-free/leak hazard, not just length.
- Each auditor gave explicit module boundaries (e.g. vfs -> mount/open/io/detached/exfat_glue/meta/sync; ahci -> regs/port/command/irq/atapi/init; lib.rs -> per-control ffi modules).

### Problem 2: No clean enforced architecture / layering  —  **severity: high**

**Current state:** The intended layering exists only as artifacts that are pervasively bypassed. arch-build measured the leakage: 420 direct crate::arch::x86:: references outside arch/boot, 590 #[cfg(target_arch)] attributes in portable modules, syscall->task coupling = 469 refs, drivers->task = 44 (AHCI/NVMe call scheduler::schedule()/deferred_wake directly), net->task = 30. There are TWO unrelated things both named 'HAL' (arch/hal.rs CPU abstraction vs drivers/hal.rs device registry). Global init is 46 Spinlock<Option<T>>/Mutex<Option<T>> singletons plus 69 raw static mut. The Filesystem trait (vfs/types.rs:69) is bypassed by enum dispatch; the Driver trait (drivers/hal.rs:104) is a byte-stream that fits no real device class so storage/net/gpu each use a different ownership convention.

**Root causes:** Traits/HALs were added as 'Phase 6'-style migrations but the old enum/free-function dispatch was never removed, doubling the surface. arch::hal is free-functions with paired #[cfg] bodies, not a trait, so there is no compile-time guarantee both arches implement the same surface — every portable module that reaches into arch::x86 must be hand-forked.

**Key findings:**
- VfsState (vfs/mod.rs:569) holds BOTH a generic trait path (root_other) AND 11 typed per-FS fields (exfat_fs/fat_fs/ntfs_fs/corefs_driver/mounted_exfat...); the :589 comment admits this is to preserve borrow-splitting the legacy dispatch relies on.
- Only CoreFS dispatches through the Filesystem trait end-to-end (vfs/mod.rs:4096); exFAT/NTFS/ISO/Overlay/SMB use hand-written fs_id branches.
- drivers/hal.rs register_device is arch-forked (x86 takes Option<PciDevice>, aarch64 takes Option<u64>), pushing cfg-noise onto every caller.
- IRQ-unmask logic (apic-vs-pic) is copy-pasted into 12 driver files reaching directly into arch::x86::{apic,ioapic,pic} because arch::hal exposes no irq_unmask/irq_register.
- arch-build proposed a strict L0 arch-trait -> L1 sync/memory -> L2 ipc/driver-model -> L3 drivers -> L4 fs/net -> L5 task -> L6 syscall layering enforceable via module visibility + a CI grep-gate (no crate::task:: under drivers/, no crate::arch::x86:: outside arch/).

### Problem 3: Instability: crashes, freezes, kernel-memory corruption  —  **severity: critical**

**Current state:** Multiple confirmed crash/corruption vectors of distinct classes. Trust-boundary: sys_time/sys_set_time/sys_net_config/sys_fstat/sys_audio_write deref user pointers with only NULL checks (confirmed at system.rs:20-53) = arbitrary kernel read/write or #PF-panic from any process; is_valid_user_ptr is a range-only check (does not verify mapping) used in all 15 handler files while the mapping-aware is_user_range_accessible is used in only 3. Data-derived panics: exfat.rs:916 cluster_to_lba panic!s the whole kernel on a bad cluster (exFAT is the root FS). Lost wakeups: the LXE futex (process.rs:1672) has a value-recheck/block race, and the scheduler deferred_wake overwrite path (mod.rs:352-370) silently drops a wakeup when 256 slots fill. Allocator: heap grow_heap_exact (heap.rs:751) advances HEAP_COMMITTED before mapping pages so OOM leaves a partially-mapped 'valid' heap window; bucket_dealloc has zero double-free detection. The huge volume of defensive scaffolding (GPU vtable validation on every call, per-tick TSS/canary checks in idt.rs:2181-2273, the arch/hal.rs:254 RBP-clobber panic guard, context_switch canary+XOR checksum) is direct evidence of chronic, unresolved memory corruption.

**Root causes:** Three converging root causes: (a) raw user-pointer access on the syscall/foreign-ABI boundary instead of mandatory copy_user_bytes + a copy-fault handler; (b) raw static mut per-CPU arrays (69 of them) and a single global lock whose 100-400ms hold windows create huge priority-inversion and wakeup-latency surfaces; (c) data-derived index/cluster values that panic instead of returning errors. The defensive scaffolding treats symptoms (skip the call, poison the GPU, blank the screen) rather than the cause.

**Key findings:**
- system.rs:20-53 sys_time writes 8 bytes / sys_set_time reads via raw *(buf_ptr) with only a NULL guard — arbitrary kernel write from an unprivileged process (CONFIRMED by direct read).
- exfat.rs:916-931 cluster_to_lba does panic!() on out-of-range cluster (validate_cluster already exists as a non-panicking Result alternative at :897).
- process.rs:1672 futex FUTEX_WAIT releases the waiter lock before scheduler::schedule(), so a FUTEX_WAKE on another CPU can wake-before-block -> permanent hang of glibc/pthread mutexes (matches LXE freezes).
- scheduler/mod.rs:352-370 deferred_wake full-slots fallback overwrites a slot without incrementing PENDING, dropping a wakeup and desyncing the counter (Relaxed fetch_sub can underflow to ~u32::MAX).
- heap.rs:751-793 grow_heap_exact publishes the committed watermark + a full-growth FreeBlock before mapping frames; OOM mid-loop returns false with HEAP_COMMITTED already advanced -> later #PF on a 'valid' heap address (likely triple fault).
- e1000.rs:740 IRQ handler does Vec::with_capacity in interrupt context (classic allocator-lock deadlock); net stack runs the FULL TCP state machine + ARP-resolve-which-re-enters-poll() inside the NIC IRQ (e1000.rs:740-777, arp.rs:101) = re-entrancy/deadlock/freeze.
- compositor present_ipc_window (window.rs:2305) builds from_raw_parts of client-declared width*height with NO SHM-size validation (no shm_size syscall exists) and panic=abort -> one bad client frame aborts the whole desktop.
- physmap.rs:99 omits NX on the direct map -> a W+X kernel alias of all RAM (hardening gap).

### Problem 4: No stable LXE (Linux execution environment)  —  **severity: critical**

**Current state:** Confirmed multiple structural correctness holes that directly explain instability. wait4 (process.rs:679) NEVER reports WIFSIGNALED — a SIGSEGV-killed child returns as 'exited 139', breaking bash/make/apt/dpkg crash detection. The futex has a lost-wakeup race (process.rs:1672) and a global 64x8 waiter table with no per-process namespacing (cross-process address collisions cross-wake). mprotect is a complete no-op (memory.rs:431) so PROT_NONE guards and W^X are silently ignored. MAP_SHARED writable file mmap is ENOSYS; MAP_PRIVATE is an eager full-file copy with no writeback. epoll/eventfd/signalfd/timerfd are missing (ENOSYS), and poll/select are busy-spin loops burning a CPU. clone CLONE_FILES/CLONE_SIGHAND clone-by-value instead of sharing, and vfork is a full fork. execve leaks page directories under SMP. On the userspace side, package.rs (2834 lines) has FOUR divergent dependency resolvers, lxed leases are keyed by reusable raw TIDs, and the host test harness re-implements parsing rather than testing the production install/extract/dpkg code (zero coverage of the real pipeline).

**Root causes:** The scheduler stores only a single u32 exit_code with signals stored AS the code (no exited-vs-signalled discriminator), so wait4 cannot encode WIFSIGNALED without a struct change. The futex/poll/epoll problems all stem from the missing 'block atomically under a token, woken by a real wait-queue' primitive in the scheduler. The kernel-side LXE files (1071-1788 lines) mix many syscall families, and the userspace install pipeline has no layering between pure parsing and side-effecting fs/network code, so it cannot be tested.

**Key findings:**
- process.rs:679 wait4 always encodes (code & 0xff) << 8 (WIFEXITED); lifecycle.rs:212/339/794 store signal deaths as Some(9)/Some(139) so a crashed child looks like a clean weird-status exit.
- memory.rs:431-437 linux_mprotect ignores addr and prot, returns 0 — never touches page tables.
- LXE pread/mmap-fill (memory.rs:625-688) is the exact apt/exFAT read-after-write coherence surface; the comment at memory.rs:667 documents a previously-fixed ar-header corruption at a FAT-run boundary.
- process.rs:365 linux_vfork just calls linux_fork; spawn.rs:289 clones fd_table+signals by value, violating CLONE_FILES/CLONE_SIGHAND POSIX semantics.
- package.rs has install_package_inner (L425) AND resolve_package_plan_inner (L625) AND a manager.rs fallback AND hosttest.rs resolve_package — four resolvers that can install different sets.
- lxed/state.rs keys leases by raw recyclable TIDs (threads.rs caps at 256 threads, hardcoded 80-byte ABI) -> stuck/stolen write leases corrupt the rootfs; daemon.rs reply-pipe name is pid-only so nested requests race.
- tools/lxe-hosttests exercises hosttest.rs private re-implementations; production package.rs is cfg-gated out under 'host' (lib.rs L19-24) and never tested.

### Problem 5: No clean HAL / driver architecture  —  **severity: high**

**Current state:** Two disjoint driver worlds and two things named HAL. The good piece is the table-driven PCI probe (pci_drivers.rs PCI_DRIVER_TABLE + probe_and_bind_all). But the drivers::hal::Driver trait is a byte-stream (read/write at usize offset) that fits no real class, so storage uses free-fn + static mut BACKEND enum, GPU uses Box<dyn GpuDriver> behind a Mutex with per-call vtable validation, network uses Box<dyn NetworkDriver> + a side singleton for RX — three different ownership conventions. There is no BlockDevice/NetDevice/InputDevice/CharDevice trait. The entire aarch64 device stack (drivers/arm/*) is a full parallel reimplementation of the VirtIO transport/virtqueue/gpu/block that does NOT touch hal.rs, the PCI table, the block cache, or async_io — so x86-only gains (block cache, readahead, retry) never reach ARM. NVMe is a static mut CTRL taken as &mut with no internal lock (aliasing UB), fully polled with a 10M-iteration spin and queue depth 8. xHCI is fully polled. MMIO VAs are hardcoded per-driver constants partitioned by comments. arch::hal exposes no irq_register/irq_unmask so 12 drivers copy-paste apic-vs-pic.

**Root causes:** The Driver trait was designed as a lowest-common-denominator byte stream instead of category sub-traits, so it could never mediate real device I/O and every subsystem routed around it. The arch HAL stops at the CPU/MMU layer and is free-functions not a trait, so drivers reach past it into arch::x86 directly and the ARM stack had to be built from scratch.

**Key findings:**
- storage/mod.rs:67-74 static mut BACKEND enum + match arms; adding a backend means editing a closed enum + 4 match sites; secondary AHCI is a separate static mut SECONDARY_AHCI whose MSI handler only services the primary (ahci.rs:558).
- nvme.rs:145 static mut CTRL taken as &mut from 2 async worker threads; soundness depends entirely on an EXTERNAL IO_LOCK not enforced inside the driver = latent aliasing UB.
- drivers/arm/mod.rs reimplements VirtIO MMIO transport; boot/arm64/mod.rs:69 hand-dispatches match device_id; fs/exfat.rs:29 calls drivers::arm::storage::read_sectors directly, bypassing the cache.
- 12 files (ahci/e1000/igc/rtl8125/rtl8168/iwl/hda/ac97/vmware_svga/virtio input+net/serial) copy-paste the apic::is_initialized?ioapic:pic IRQ-unmask block.
- e1000 IRQ handler allocates in IRQ context; NVMe/xHCI have no interrupt-driven completion; both are concrete IO perf + deadlock issues.
- arch-build's smallest-highest-leverage recommendation: add arch::hal::irq_register/irq_unmask/irq_mask and delete the 12 inline blocks; then define BlockDevice/NetDevice sub-traits and a hal::map_mmio VA allocator.

### Problem 6: No clean swap implementation  —  **severity: high**

**Current state:** CONFIRMED by direct read: swap.rs (290 lines) is a backing-store-and-slot-registry STUB ONLY. Its own docstring (lines 3-6) states it 'deliberately keeps the policy bits (page reclaim, victim selection, COW integration) out of the first slice'. swapon/swapoff register a file and a slot bitmap, but write_page/read_page/allocate_slot/free_slot have ZERO callers anywhere in the tree. There is no page-out/eviction path, no victim selection (no clock/LRU/second-chance), no kswapd-equivalent, no swap-in on page fault, and no PTE swap-entry encoding. swapon() succeeds and reports stats but no page is ever evicted. Compounding this: fork is EAGER full-copy with no copy-on-write (virtual_mem.rs:1207), which multiplies peak memory and makes OOM worse; under pressure the kernel returns None up the stack (heap.rs:732 grow_heap returns false; virtual_mem.rs alloc_frame returns None) instead of reclaiming -> hard OOM and allocation-failure cascades.

**Root causes:** Only the mechanism half (backing store) was built; the policy half (reclaim daemon, victim selection, PTE swap encoding, page-fault swap-in) was deferred indefinitely. The eager-copy fork compounds the pressure that swap would relieve.

**Key findings:**
- swap.rs:202 write_page / :217 read_page / :167 allocate_slot have zero callers (grep across kernel/src confirms only swap.rs references them) — CONFIRMED by direct read of the module docstring.
- The #PF handler (idt.rs:1837-1861) only handles not-present demand paging for heap/mmap/DLL and never consults swap; there is no swap-entry PTE encoding.
- fork has no CoW: virtual_mem.rs:1207-1466 clone_user_page_directory_inner physically duplicates every writable page synchronously with the 'quarantine live parent frames' hack (1394-1428) that exists only because eager copy races the allocator.
- Recommended path: add a swap-entry PTE encoding distinct from PTE_GUARD bit 10, a reclaim_pages(n) that does an accessed-bit clock sweep and rewrites PTEs to swap entries, a #PF swap-in arm, and triggers from grow_heap/alloc_frame OOM plus a low-watermark kthread.
- WRITE_BACK_MAX_SECTORS=8 in storage/mod.rs creates fragmentation-dependent mixed write-back/direct durability within a single file write — a related crash-consistency hazard flagged under the swap category by vfs-io.

### Problem 7: Fragile compositor  —  **severity: critical**

**Current state:** Single point of failure for the entire GUI with no fault isolation. panic=abort + one global static mut DESKTOP_PTR + no per-client validation means one malformed client message aborts the whole desktop session. The headline bug: present_ipc_window (window.rs:2305) and copy_shm_to_pixels (window.rs:3109) build from_raw_parts of client-declared width*height with NO SHM-byte-size validation (no shm_size syscall exists), and the SCALED present path (window.rs:2322) indexes src_slice with NO bound check — a client lying about its surface size triggers an OOB read -> page fault -> whole desktop dies. CMD_PRESENT has no owner-TID check (any app can present/dirty another app's window). The crash-reap path (destroy_window, window.rs:848) never calls shm_unmap, leaking a compositor SHM mapping + pinned physical frames for every closed/crashed window. Events are silently dropped at a 256-deep queue and emit_to_target can block the shared management/render thread on one stuck client, starving input to all other apps. Secondary monitors fully re-composite (full nearest-neighbor wallpaper rescale) every frame the primary has damage.

**Root causes:** No SHM-size validation primitive in the kernel; the god-struct Desktop (35+ fields, methods scattered across files) gives no encapsulation so the shm_ptr/width/height invariant is unenforced; panic=abort with no supervisor makes any unguarded index fatal; the compositor trusts caller-supplied owner_tid instead of a kernel-verified sender.

**Key findings:**
- window.rs:2305 from_raw_parts((shm_w*shm_h)) where w/h are client-declared and only bounded against 8192/16M pixels; ipc::shm_map returns only an address (libs/stdlib/src/ipc.rs:161) — no size query exists.
- window.rs:2322 scaled-present path indexes src_slice[src_row_off+src_x] with NO bound check (the row-copy path at :2337 does clamp — the asymmetry is the bug).
- destroy_window (window.rs:848-889) never shm_unmaps; only the explicit CMD_DESTROY_WINDOW IPC path (ipc.rs:143) unmaps, so crash-reap leaks the mapping permanently.
- CMD_PRESENT (ipc.rs:167) / CMD_SET_TITLE / CMD_GET_WINDOW_POS accept any window_id with no owns_window() guard (unlike CMD_MOVE/MINIMIZE which do).
- render.rs:176 evt_chan_emit_to can block on a full client channel; management.rs:512 emits serially so one hung app stalls input to all.
- compositor/mod.rs:666-767 render_secondary_outputs ignores per-output damage and rescales the static wallpaper per-frame on any primary damage.
- Recommended fixes: add SYS_SHM_SIZE syscall + WindowSurface encapsulation that only returns length-correct slices; tag commands with kernel-verified sender TID; move shm_unmap into destroy_window; non-blocking try_emit with per-client unresponsive detection; make the compositor restartable.

### Problem 8: Finicky libanyui  —  **severity: high**

**Current state:** A 34k-line retained-mode toolkit with zero automated tests, two latent UB sources, an O(n^2) core, and structural layout fragility. The client closure registry (events/mod.rs:25-45) leaks every registered callback forever and holds a &mut into the HANDLERS Vec across the user callback, so a callback that registers another callback (the common case) causes mutable aliasing + use-after-realloc UB. anyui_message_box (lib.rs:3789) runs a nested run_once() which takes a second &mut to the same global static mut STATE — two live &mut = UB. find_idx (control.rs:1047) is an O(n) linear scan called 127 times incl. recursively in hit_test on every MOUSE_MOVE, making traversal and UI construction O(n^2). The single-pass dock layout (layout.rs:25) is order-dependent (the documented 'DOCK_FILL must be added last' and 'toolbar (0,0) invisible' traps are engine fragilities, not just docs). Control removal (anyui_remove, lib.rs:3888) does not clear all dangling ControlId references (tooltip/last_click/drag/popup/modal_stack), so removing a control with an open context menu can dispatch against a freed id.

**Root causes:** Global static mut singletons (STATE/HANDLERS/QUEUE) make the engine re-entrancy-unsafe and untestable; the closure thunk holds a live borrow during the call; no id->index map; layout is single-pass insertion-order-dependent; teardown is not centralized. The engine is structured for headless testing (pure Surface/hit_test/layout) but no tests exist.

**Key findings:**
- events/mod.rs:32 closure_thunk takes h=handlers() (&'static mut) then calls h[idx](...); a nested register()->handlers().push() reallocs and invalidates h[idx] = UB; handlers are never removed = unbounded leak.
- lib.rs:3789 anyui_message_box nested run_once() -> second &mut STATE while the outer run_once Phase-3 callback already holds one.
- control.rs:1047 find_idx O(n) called 127x; every MOUSE_MOVE runs hit_test_any+cursor_at_point+abs_position = multiple O(n^2) walks; every FFI mutator does its own iter().find.
- layout.rs:85 DockStyle::Fill consumes remaining area at its insertion moment -> later docked siblings overlap; Top/Bottom take child's current .h so a default-(0,0) control stays invisible.
- anyui_remove/clear_tracking_for only reset focused/pressed/hovered, not active_tooltip/last_click_id/drag/incoming_drag/popup/modal_stack.
- marshal.rs silently drops UI updates on 256-entry overflow and runs Dispatch callbacks mid-drain (same nested-&mut hazard).
- Zero #[test] anywhere; the host feature already proves the crates build for std, so layout/hit_test/render-into-Vec<u32> are host-unit-testable with a stub compositor.

### Problem 9: Poor performance: scheduler, net, IO, memory-alloc  —  **severity: high**

**Current state:** Confirmed bottlenecks in all four named areas, dominated by one global lock and a busy-poll/per-packet-alloc network model. SCHEDULER: current_thread_abi() takes the global IRQ-disabling SCHEDULER spinlock on EVERY x86_64 syscall (CONFIRMED at thread_config.rs:89, despite a comment elsewhere calling it lock-free), plus 2-3 more global-lock acquisitions per IO/path syscall for IO accounting and rootfs lookup; waitpid polls on every timer tick; no priority inheritance so a low-priority lock holder starves the compositor. NET: no congestion control (no cwnd/slow-start), fixed 3s RTO with no RTT estimation (one lost segment on a 1ms LAN stalls 3s), the retransmit timer services only ONE connection per call (return instead of continue, timer.rs), the entire TCP stack is one IF-off spinlock, and the RX path heap-allocates + triple-copies every packet. IO: NVMe is polled with a 10M spin + depth 8; AHCI NCQ is disabled so all I/O serializes; large-write cache invalidation does a full 16384-slot linear scan under the spinlock per chunk. ALLOC: per-thread 2MiB kernel stacks (100 threads = 200MiB), eager-copy fork, per-CPU heap-cache stranding under migration (the historical 73KiB-alloc panic after 10h uptime), exFAT double-caches every cluster in both the block cache and a per-cluster Vec.

**Root causes:** One global SCHEDULER lock on the syscall hot path; a busy-poll network model where every blocked socket re-drives poll() + a 256-slot timer scan; per-packet Vec allocation; disabled NCQ + polled NVMe; eager fork + oversized fixed kernel stacks. The lock-free per-CPU caches needed to fix the scheduler lock already exist but were reverted because they go stale (the keystone refactor: validate them against PER_CPU_CURRENT_TID).

**Key findings:**
- thread_config.rs:89 current_thread_abi() = SCHEDULER.lock() on every syscall (CONFIRMED); diagnostics.rs:190 record_io_* + thread_config.rs:129 rootfs lookup add 1-2 more global locks per IO/path syscall.
- tcp/send.rs:415 send window = peer-advertised only (no cwnd); tcb.rs:34 fixed RETRANSMIT_TICKS=300 (3s) with no SRTT/RTTVAR; timer.rs retransmit branches `return;` so one pass services one connection.
- e1000.rs:703 + net/mod.rs:157 + tcb.rs:354 = three Vec allocs/copies per packet under the IF-off global TCP lock; arp::resolve re-enters poll() (arp.rs:101).
- nvme.rs:297 io_wait_completion spins 0..10_000_000, depth 8 (nvme.rs:157), no MSI completion; AHCI_ENABLE_NCQ=false (ahci.rs:72) so the multi-slot path is dead and all AHCI I/O serializes through IO_LOCK.
- blockcache.rs:587 invalidate_range scans all 16384 slots under the spinlock for any write >32 sectors; storage/mod.rs:1009 calls it per large-write chunk.
- thread.rs:277 KERNEL_STACK_SIZE=2MiB per thread from the heap; reaper holds zombies 1000-5000ms; exFAT read_cluster (exfat.rs:1044) duplicates data already in the block cache.
- Keystone fix (scheduler auditor): a typed PerCpuCache that packs (tid,value) atomically and validates against PER_CPU_CURRENT_TID, then build abi/rootfs/uid on it — removes the global lock from the syscall hot path.

## Phased Roadmap

### Phase 0: Fast, deterministic, Claude-runnable test harness

**Goal:** Make every later change regression-safe and verifiable in seconds, not minutes, with a machine-checkable exit code. This MUST come first so all subsequent stability/correctness/refactor work is guarded.

**Depends on:** none

**Workstreams:**
- Add a cmake kunit-kernel target that builds ONLY the kernel ELF + a minimal bootable image (no apps/libc64/libcxx) into a separate build-kunit/ dir so dev builds are never disturbed (test-infra finding: today --kunit rebuilds the entire Rust+clang userspace).
- Add a kernel-side completion signal under feature=kunit: print a 'KUNIT-DONE rc=N' sentinel then write the QEMU isa-debug-exit port (iobase=0xf4) with rc = (total_fail==0?0:1); add -device isa-debug-exit to the QEMU line and key success off the exit code, not log-scraping + SIGKILL.
- Extract the no-hardware unit suites (buddy, heap, slab/alloc, vma, net types, crypto, blockcache hash) into a host-buildable crate behind a std shim so they also run via `cargo +stable test` for a sub-second inner loop (the host feature already exists for surf-host/libanyui).
- Factor the duplicated unit/integration suite-runner loops into one shared function and add a register_kunit_suite! macro (link-section/inventory) so adding a suite is one step and cannot silently drift across the two static arrays.
- Fix the per-suite N/N miscount (runner.rs:93 uses cases_pass as both numerator and denominator) and add a 'skipped' counter so precondition early-returns are visible, not silently green.

**Deliverables:**
- `ninja run-kunit` (or scripts/test.sh --kunit) that boots headless, exits with rc 0/1, completes in seconds, and never touches the dev build dir.
- `cargo +stable test` running the pure-algorithm suites on the host.
- A documented one-liner QEMU invocation (isa-debug-exit + serial log) usable by Claude/CI.
- Renamed memory/virtual_mem_stub.rs -> arch/arm64/paging.rs (it is the production ARM64 impl, not a stub) to remove the misnomer.

**Test plan:**
- Meta-test: deliberately break one assertion and confirm the harness exits rc=1 and the failing suite is named.
- Confirm the kunit-kernel image boots both 'KUnit unit: ALL PASS' and 'KUnit integration: ALL PASS' lines without building any app.
- Confirm `cargo +stable test` runs buddy/heap/vma/net-types suites green on the host.

**Risk:** Low. Pure infrastructure; the kunit framework and suites already exist (kernel/src/kunit/). Main risk is the minimal-image bootstrap omitting something the boot path needs before run_unit_tests() — mitigated because unit tests run after heap init but before userspace.

### Phase 1: Kernel trust-boundary hardening (memory-corruption + lost-wakeup + panic-on-data class)

**Goal:** Eliminate the confirmed arbitrary-kernel-read/write, data-derived panic, lost-wakeup, and allocator-inconsistency bugs that turn user input or on-disk data into a system-wide crash. These are the direct, highest-severity causes of problem #3 (and a major part of #4 and #7).

**Depends on:** Phase 0

**Workstreams:**
- User-pointer discipline: route EVERY user-memory access through mapping-validated copy_user_bytes/copy_to_user_bytes; fix the confirmed raw-deref handlers (sys_time/sys_set_time system.rs:20-53, sys_net_config net.rs:30, sys_fstat/sys_pipe2 io.rs:553, sys_audio_write display.rs:259, sys_screen_size/capture/grant_framebuffer); add a CI grep-gate forbidding `as *mut`/from_raw_parts in handlers/ not preceded by validation.
- Add a copy-fault exception table (Linux __ex_table style) so a faulting user access returns -EFAULT instead of panicking — closes the entire TOCTOU/unmapped-page panic class including SMP munmap races.
- Make data-derived conversions non-panicking: convert exfat.rs:916 cluster_to_lba to Result<u32,FsError> (validate_cluster already exists) and propagate through read_cluster/write_cluster/plan builders; fix the getdents reclen padding underflow (abi.rs / fs.rs:1562 saturating_sub).
- Fix the heap grow_heap_exact OOM watermark inconsistency (heap.rs:751): map all frames into a scratch list first, only then advance HEAP_COMMITTED and publish a correctly-sized FreeBlock; roll back on partial failure. Add a bucket-path double-free guard (heap.rs:224).
- Fix the two lost-wakeup bugs: replace the scheduler deferred_wake linear-probe array (mod.rs:352) with a real MPSC ring (or at minimum keep PENDING consistent and never overwrite an un-drained slot); fix the vma.rs:315 live-mmap-path expect() to fail the syscall instead of panicking.
- Replace scheduler/loader raw static mut [T;MAX_CPUS] per-CPU arrays with atomics or a checked per_cpu(cpu) accessor centralized in scheduler/percpu.rs; once root-caused, gate/remove the RBP-clobber guard (arch/hal.rs:254) and per-tick TSS/canary checks (idt.rs:2181) behind a debug feature.

**Deliverables:**
- All 15 handler files use mapping-validated transfers; CI grep-gate enforces it.
- Copy-fault handler returning -EFAULT.
- cluster_to_lba and getdents return errors, never panic.
- grow_heap OOM-consistent; bucket double-free detected.
- deferred_wake structurally cannot drop a wakeup; corrupt VMA list fails the mmap not the kernel.

**Test plan:**
- kunit syscall-safety suite: call each pointer-taking syscall with NULL, a kernel-half address, and an in-range-but-unmapped address; assert it returns an error, not a panic (would immediately surface sys_time/net_config/fstat).
- kunit heap grow-under-low-frames test: drive the buddy near-empty, force a heap growth OOM, assert HEAP_COMMITTED stays consistent with mappings and a subsequent in-range access does not fault.
- kunit double-free test: free a bucket-sized alloc twice, alloc twice, assert the two pointers differ (currently fails — documents the bug).
- kunit no-lost-wakeup test: flood deferred_wake() past 256 slots and assert every distinct TID is eventually moved Blocked->Ready; assert DEFERRED_WAKE_PENDING never underflows.
- kunit exfat cluster_to_lba test: feed an out-of-range cluster and assert Err, not abort.
- kunit getdents test: feed directory names of lengths that hit reclen==19+name and assert no usize underflow in the pad write.

**Risk:** Medium. Touching the user-access path and the heap is delicate, but Phase 0's harness makes each fix verifiable. The copy-fault table needs careful asm/IDT integration. The static-mut per-CPU refactor risks subtle SMP races if the accessor is wrong — mitigate with the per-CPU consistency assertion already in the stability suite.

### Phase 2: LXE correctness (stable Linux process execution)

**Goal:** Make LXE stable enough to run apt/dpkg/bash without silent mis-sequencing or hangs, by fixing the confirmed wait4/futex/mprotect/clone holes and unifying the read-after-write VFS path. This is problem #4 and removes a major class of freeze (#3).

**Depends on:** Phase 1 (needs the copy-fault handler and the lost-wakeup-safe wake primitive)

**Workstreams:**
- Add an exited-vs-signalled discriminator to the scheduler exit state (alongside exit_code) and fix linux_wait4 (process.rs:679) to encode WIFSIGNALED (signal & 0x7f, optional WCOREDUMP) for signal deaths and <<8 only for true exits.
- Build the atomic block-under-token wait primitive in the scheduler (block while holding token X, commit the block in schedule(), recheck condition before releasing the run queue); reuse it to fix the futex lost-wakeup (process.rs:1672), namespace futexes by (page-directory, uaddr), and convert poll/select/epoll_wait from busy-spin to real blocking.
- Implement real mprotect (memory.rs:431): walk page tables for the range and set present/writable/NX per PROT flags with TLB shootdown; make mmap honor prot for the initial mapping (PROT_NONE guard pages must fault).
- Add a minimal epoll + eventfd backed by the readiness wait queue (modern apt http method and event loops need it).
- Make CLONE_FILES/CLONE_SIGHAND share refcounted fd-table/sighand instead of cloning by value (spawn.rs:289); implement vfork correctly or reject unsupported sharing with EINVAL.
- Unify LXE positional reads (pread, mmap-fill, sendfile) onto one coherent VFS primitive that always flushes writers AND reads through the block cache with correct multi-sector invalidation (the apt/exFAT read-after-write surface, memory.rs:625).
- Userspace LXE: collapse the four dependency resolvers into one pure resolve_plan(); extract pure parse/resolve/deb/dpkg modules from package.rs so the host harness tests the REAL code; replace TID-keyed lxed leases with opaque cookie+TTL tokens and add a kernel is_tid_alive syscall.

**Deliverables:**
- wait4 reports WIFSIGNALED/WTERMSIG correctly.
- Futex/poll/epoll block atomically with no lost wakeups and no CPU burn.
- mprotect enforces W^X and PROT_NONE.
- Single dependency resolver used by CLI/apt/manager/hosttest; production install path under host test.
- lxed leases keyed by cookie+TTL, not reusable TIDs.

**Test plan:**
- kunit wait4 test: fork a child, kill it with each signal, assert the returned status decodes to WIFSIGNALED/WTERMSIG (would have caught the critical wait4 bug).
- kunit futex contention test: N clone-threads ping-pong FUTEX_WAIT/FUTEX_WAKE on shared words; assert no lost wakeups, headless over serial.
- kunit mprotect test: map RW, mprotect to PROT_NONE, assert a subsequent access faults (catches the no-op mprotect).
- kunit/vfsstress read-after-write: write a multi-MiB file on fd A then pread it at non-sector-aligned offsets on fd B, byte-compare (locks down the exFAT/block-cache corruption regression).
- Host cargo test for package parse/resolve/ar-tar/dpkg-record using the real (now-extracted) functions; assert the single resolver produces the bootstrap closure.
- Headless QEMU smoke: `lxe init` non-interactively asserts bootstrap-state == complete, then `lxe run /bin/true` returns 0; a lxed two-writer concurrency test asserts exactly one wins.

**Risk:** High. The atomic block-under-token primitive is a scheduler change with deep reach (futex/poll/epoll/sigsuspend all depend on it). mprotect + TLB shootdown is SMP-delicate. The VFS read-path unification risks reintroducing the corruption it fixes — mitigated by the read-after-write kunit/vfsstress test landing first.

### Phase 3: VFS/blockcache coherence as an API invariant + storage durability

**Goal:** Make read-after-write coherence a structural invariant of the cache API (not an emergent 'always overlay after read' convention), fix the data-loss durability gaps, and remove the latent write-plan landmine. Directly addresses the corruption class behind #3/#4/#6.

**Depends on:** Phase 0 (tests), Phase 1 (cluster_to_lba no longer panics)

**Workstreams:**
- Fold overlay_cached INTO the cache read API: a single cache_read_or_fill(disk,lba,count,buf,backend_read_fn) that reads the whole range, overlays all dirty cached sectors, and populates clean ones — so callers physically cannot read the backend without the overlay (closes the fragility where lookup_range stops at first miss, CONFIRMED at blockcache.rs:282, and coherence depends on a separate overlay_range call).
- Fix the execute_write_range base_offset=0 landmine (exfat.rs:708) to use self.base_offset like execute_range, or assert base_offset==0.
- Establish exFAT write ordering + durability: order data->FAT->bitmap->dirent with a hardware barrier at the FAT->dirent boundary on fsync/close; make fsync and the close() path actually durable; document the no-fsync durability contract.
- Make the write-back/direct decision logical not fragmentation-dependent: route file data writes direct-to-disk with hash-targeted invalidate, reserve write-back for small metadata (FAT/bitmap/dirent) (removes the WRITE_BACK_MAX_SECTORS=8 mixed-residency hazard).
- Replace the O(16384) full-slot scan in invalidate_range/overlay (blockcache.rs:587) with hash-targeted invalidation when count is bounded and a per-disk dirty count to skip the scan when zero dirty sectors exist.
- Correct the stale UnsafeCell SAFETY comments in exfat.rs:1046 (the real invariant is the inner driver Mutex, not the VFS Mutex).

**Deliverables:**
- cache_read_or_fill API; no caller can read the backend without overlay.
- execute_write_range uses base_offset.
- fsync/close are durable; ordered metadata writes with a barrier.
- Bounded-count invalidation instead of full-slot scan.
- Accurate cluster-cache safety documentation.

**Test plan:**
- kunit blockcache coherence test: write_back(disk,lba,N) for N spanning past a forced cache miss, then a FULL-MISS read of [lba..lba+N] must return the written bytes (pins down the overlay invariant).
- kunit dirty-eviction test: fill the cache with dirty sectors to force dirty eviction; assert insert_with_dirty returns the evicted (disk,lba,data) and writeback never silently drops a dirty slot; scribble a slot key and assert key_check quarantine drops, not flushes.
- kunit exFAT plan symmetry test: build get_file_read_plan_range with base_offset!=0 and assert execute_write_range targets the same absolute LBAs as execute_range (catches the :708 bug).
- vfsstress additions: fragmented-file writes straddling the WRITE_BACK_MAX_SECTORS=8 boundary; partial-cache-hit reads ([0..k] clean, k missing, k+1.. dirty); cross-fd coherence (write via append-buffered fd A, read same path via fd B).
- Headless: confirm the historical apt/exFAT corruption against a REAL apt install (the memory note's open 'confirm with real apt install').

**Risk:** Medium-High. The cache-read API change touches the hottest IO path; the durability ordering may surface latent driver-flush bugs. The Phase 0/2 read-after-write tests must be green before and after each change.

### Phase 4: Kill the per-syscall global SCHEDULER lock (the keystone performance refactor)

**Goal:** Remove the confirmed global IRQ-disabling SCHEDULER lock from the syscall hot path and the per-IO accounting/rootfs lookups. This is the single dominant performance ceiling (#9) and a freeze amplifier (#3), and it unblocks LXE startup throughput (#4).

**Depends on:** Phase 0 (the cache-coherence test that validates the fix), Phase 1 (per-CPU static-mut now uses checked accessors)

**Workstreams:**
- Build a typed PerCpuCache that packs (tid, value) atomically and validates the cached TID against PER_CPU_CURRENT_TID before trusting it; fall back to the lock only on mismatch. Audit and fix EVERY thread-on-CPU transition (normal switch, idle-on-exit in lifecycle.rs, try_exit_current, bad_rsp_recovery, fork_return_to_user, AP bring-up, signal-trampoline return) to update the cache — this is exactly the staleness that forced the lock-based revert (CONFIRMED comment at thread_config.rs:82-88).
- Build current_thread_abi / linux_rootfs / uid / gid / capabilities on the validated PerCpuCache so the syscall entry reads them lock-free.
- Move IO/net accounting (record_io_read/write/net_tx/rx, diagnostics.rs:190) to lock-free per-CPU atomics flushed lazily in schedule_inner or the reaper (removes 1-2 more global locks per IO syscall).
- Route all by-TID lookups through find_idx (replace the dozens of iter().find scans in signals/spawn/priority/fork) and replace full TID-cache invalidation on swap_remove with a targeted fixup.
- Trust the exit_waiter wakeup in waitpid (wait.rs:206) instead of per-tick poll-halt; convert the timer-path try_lock-skip-drain to a lock-free deferred-wake drain.
- Stop the spinlock re-enabling interrupts mid-spin (spinlock.rs:217) once hold times shrink; ensure_idle_thread must not allocate a 2MiB stack under the lock (mod.rs:885) — pre-allocate idle stacks at AP bring-up.

**Deliverables:**
- Lock-free, TID-validated per-CPU cache; syscall entry takes zero global locks for ABI/rootfs/accounting.
- All by-TID lookups O(1) via find_idx with incremental cache fixup.
- waitpid does not scale with timer-tick rate.
- No allocation under SCHEDULER; spinlock no longer re-enables IRQs mid-spin.

**Test plan:**
- kunit ABI/rootfs cache coherence test: after EVERY thread-on-CPU transition path (normal switch, idle-on-exit, try_exit_current, bad_rsp_recovery) assert the lock-free PER_CPU_CURRENT_ABI/LINUX_ROOTFS equals the locked read for the current TID — the highest-value test, guards the exact staleness that forced the revert.
- kunit waitpid wakeup-not-poll test: count SCHEDULER lock acquisitions while N parents wait on long sleeper children; assert it does not scale with timer-tick count.
- kunit cascade-kill test: build a 3-level parent->child->grandchild tree; assert exit/kill marks every descendant Terminated exactly once and frees the shared PD only when the last live thread dies.
- Headless SMP throughput probe: a syscall-heavy loop on 2+ cores logging lock-contention counters before/after, asserting the per-syscall global-lock acquisitions drop to ~0.

**Risk:** High. This is the bug the team already reverted once; the failure mode (a native thread seeing ABI=Linux and mis-dispatching every syscall -> freeze) is severe. The TID-validation + exhaustive transition-site audit is the mitigation, and the cache-coherence kunit test from this phase must gate it. Do NOT attempt before Phase 0's harness exists.

### Phase 5: Swap reclaim policy + copy-on-write fork

**Goal:** Turn swap from a non-functional registry into a real reclaim path and stop fork from eagerly copying the whole address space, so the system degrades gracefully under memory pressure instead of hard-OOM. This is problem #6 and a large part of #9.

**Depends on:** Phase 1 (per-CPU/heap hardening), Phase 3 (durable, coherent block IO for the swap backing store)

**Workstreams:**
- Add a swap-entry PTE encoding (non-present PTE + software bit + SwapSlot in bits 12-62, distinct from PTE_GUARD bit 10 / PTE_VRAM bit 9).
- Add reclaim_pages(n): accessed-bit (PTE bit 5) clock sweep over per-process VMAs to select victims, call swap::allocate_slot()+write_page(), rewrite the PTE to the swap entry, free the frame.
- Add the #PF swap-in arm: detect a swap-entry PTE, read_page() into a fresh frame, restore the PTE.
- Trigger reclaim from grow_heap and alloc_frame OOM plus a low-watermark reclaim kthread.
- Implement CoW fork (virtual_mem.rs:1207): map parent+child PTEs read-only with a COW software bit + per-frame refcount; on write #PF copy once and restore writability — also removes the 'quarantine live parent frames' hack.
- Reduce per-thread kernel stack from a fixed 2MiB heap allocation to role-sized (64-256KiB with a guard page, 2MiB only for compositor/VFS-deep threads) or demand-mapped stack pages.

**Deliverables:**
- swapon actually relieves memory pressure (page-out + page-in round-trip).
- fork is O(1) in resident size; writes diverge on demand.
- Kernel stack memory footprint scales with usage, not 2MiB/thread.

**Test plan:**
- kunit swap_tests: ramfs swap file, swapon, allocate_slot/write_page/read_page round-trip (verify bytes survive), free_slot, swapoff-busy rejection, double-swapon rejection, NoSpace exhaustion — would immediately flag the I/O functions are never exercised today.
- kunit eviction/swap-in test: mmap+touch enough anonymous pages to force reclaim, then re-read and assert contents (proves page-out+page-in round-trips through swap).
- kunit CoW test: in a synthetic two-PD scenario (map_page_in_pd/virt_to_phys), assert parent and child share frames until a write, then diverge — testable without spawning userspace.
- kunit reclaim_empty_user_tables test: an mmap/munmap cycle asserting page-table frame count returns to baseline (guards the '96 leaked pages after 192MiB' regression).

**Risk:** Medium-High. Reclaim + swap-in touches the #PF handler and PTE encoding; CoW interacts with the destroy/clone walkers (which are themselves duplicated — see Phase 6). Sequencing CoW after Phase 6's unified PTE walker would be cleaner, but reclaim is independently valuable; land swap reclaim first, CoW after the walker is unified.

### Phase 6: Architecture: unify dispatch traits and split god-files (now that tests exist)

**Goal:** Collapse the parallel enum/free-function dispatch onto the existing traits and physically split the oversized files — addressing problems #1 and #2 — with regression coverage already in place from Phases 0-5 so refactors are safe.

**Depends on:** Phase 0 (harness), and ideally Phases 3/4 (so VFS and scheduler internals are stabilized before splitting)

**Workstreams:**
- VFS: make every mount a Box<dyn Filesystem> in one mount table; delete the 11 typed VfsState fields and the 18+ match fs_id sites; move exFAT commit/sync/retarget into fs/exfat behind a trait durability hook; then split vfs/mod.rs into mount/open/io/detached/exfat_glue/meta/sync. Unify the locked and Detached* paths.
- arch HAL: convert arch::hal into traits (CpuOps/MmuOps/IrqController/Timer) with one impl per arch (missing methods become compile errors); add irq_register/irq_unmask/irq_mask and delete the 12 copy-pasted apic-vs-pic blocks; sweep the 420 direct arch::x86 refs through the HAL.
- Driver model: define BlockDevice/NetDevice/InputDevice/CharDevice sub-traits and a hal::map_mmio VA allocator; register Arc<dyn BlockDevice> per disk, removing static mut BACKEND, the disk_id special-cases, and IO_OVERRIDES; wrap NvmeController in its own lock (fixes the aliasing UB).
- Extract a transport-agnostic VirtIO core over a VirtioTransport trait (PCI + MMIO impls) so drivers/arm stops duplicating drivers/virtio (~880 lines of duplicated virtqueue).
- Extract one for_each_user_pte() recursive-map iterator and route clone/destroy/flag-update through it (removes ~600 duplicated lines in virtual_mem.rs and the clone/destroy skip-logic divergence); one coalesce_into_sorted_list() helper in heap.rs.
- Split the remaining god-files per the auditors' boundaries (scheduler/mod.rs, ahci.rs, virtio_gpu.rs, idt.rs, display.rs, package.rs, libanyui lib.rs/event_loop.rs, the LXE 1000+ line files); drive syscall dispatch from a table of (nr,name,fn,arg-width,required_cap) shared by native/linux/windows and enforce capabilities at the foreign-ABI boundary.
- Add a CI grep-gate forbidding upward edges (no crate::task:: under drivers/, no crate::arch::x86:: outside arch/, no raw user-ptr in handlers/).

**Deliverables:**
- VFS read()/write() = one mount-lookup + trait call; vfs/mod.rs < ~1500 lines split into 7 modules.
- arch::hal is a trait; irq_unmask exists; 12 inline blocks gone.
- BlockDevice/NetDevice traits with per-disk Arc<dyn>; NVMe internally locked.
- Unified VirtIO core shared by x86 and ARM.
- One PTE-walk iterator; no clone/destroy skip-logic divergence.
- Table-driven syscall dispatch with capability enforcement on all 3 ABIs.
- CI grep-gate enforcing layering.

**Test plan:**
- Re-run ALL Phase 1-5 kunit suites after each split/migration — the refactors must be byte-for-byte behavior-preserving (this is why splitting comes AFTER coverage).
- kunit HAL-registry probe test: feed a synthetic PCI device list through probe_and_bind_all and assert specificity-based binding (pure logic).
- kunit BlockDevice test: register a RAM-backed Arc<dyn BlockDevice>, run cached read/write/flush, assert read-after-write coherence (also re-validates Phase 3).
- kunit for_each_user_pte test: a mmap/munmap cycle through the unified iterator asserting clone and destroy agree on which PTEs are private/shared (no double-free/leak).
- CI grep-gate as a build step that fails on a forbidden cross-layer reference.

**Risk:** Medium given coverage exists, High without it. These are large mechanical refactors; the borrow-splitting rationale documented in VfsState makes the trait migration non-trivial. The unified PTE walker is a correctness-critical merge of two divergent walkers. Land each split behind green kunit + cargo test before the next.

### Phase 7: Compositor + libanyui robustness and testability

**Goal:** Stop one bad client frame from aborting the whole desktop (#7) and remove the libanyui UB/leak/O(n^2)/layout fragility (#8), with headless offscreen-SHM tests so these become regression-guarded.

**Depends on:** Phase 0 (harness pattern), and a kernel SYS_SHM_SIZE syscall (small kernel change, can land in Phase 1 or here)

**Workstreams:**
- Add SYS_SHM_SIZE and a WindowSurface type whose only pixel accessor returns a length-correct slice; validate width*height*4 <= region size on CREATE_WINDOW/RESIZE_SHM; immediately bound-check the scaled-present path (window.rs:2322). Move shm_unmap into destroy_window so crash-reap stops leaking mappings.
- Tag every event-channel command with the kernel-verified sender TID; gate CMD_PRESENT/SET_TITLE/GET_WINDOW_POS on owns_window().
- Make event emit non-blocking (try_emit) with per-client unresponsive detection so one hung app can't stall global input; never drop MOUSE_UP/WINDOW_CLOSE/FOCUS_LOST; prefer a focus-history stack over windows.last() on destroy.
- Per-output damage tracking + cached scaled wallpaper for secondary monitors.
- libanyui: replace the leaking re-entrancy-unsafe HANDLERS Vec with an id+event-keyed registry that copies the call target out before invoking; add an id->index map (O(1) find_idx); make message boxes non-blocking via the existing modal_stack + a re-entrancy guard on state(); make dock layout order-independent (process Fill last) with sane default extents; centralize control teardown to clear all dangling ControlId fields.
- Build AnyStream Phase 1 (offscreen WindowType::Stream + DirtyNotification pipe) per CLAUDE.md so the compositor and libanyui can be driven headlessly by anyos_testkit, plus host cargo-test for libanyui layout/hit-test/render-into-Vec<u32> with a stub compositor.
- Make the compositor restartable (supervisor re-exec + client reconnect) given panic=abort.

**Deliverables:**
- Oversized/undersized-SHM client frames are rejected, not fatal; no SHM leak on crash-reap; ownership-gated commands.
- One hung app cannot stall input to others; correct focus fallback.
- libanyui: no callback leak/UB, O(1) widget lookup, non-blocking modals, order-independent dock layout, complete teardown.
- Headless compositor tests via offscreen SHM; host unit tests for libanyui layout/hit-test/render.
- Restartable compositor.

**Test plan:**
- Headless (offscreen SHM) malformed-IPC fuzz: CREATE_WINDOW/RESIZE_SHM/PRESENT with oversized w/h vs a tiny SHM; assert rejection not abort.
- Headless crash-isolation test: spawn a client, kill it mid-frame; assert the compositor survives, the window is destroyed, and the live-SHM-mapping count returns to baseline.
- Headless focus/ownership tests: app1 sends CMD_PRESENT/MOVE for app2's window_id and is rejected; close focused window and assert focus falls back correctly (not to a popup); a client that never drains its channel must not stall a second client's events.
- Host cargo test for libanyui: perform_layout asserts child geometry (guards DOCK_FILL order + zero-size docked control); hit_test returns expected ControlId incl. ScrollView offset; render_tree into Vec<u32> asserts dirty-rect union after set_position/set_size; client closure registry replaces on re-register and survives a callback that registers another callback.

**Risk:** Medium. The SHM-size validation needs a small kernel syscall; the libanyui state() re-entrancy guard and non-blocking modal change touch the core event loop. Headless testability (offscreen SHM + host layout tests) is the prerequisite and biggest enabler here.

### Phase 8: Network stack performance and IRQ-context safety

**Goal:** Make TCP behave on real (lossy/bandwidth-limited) networks and remove the IRQ-context re-entrancy/allocation hazards — addressing the network half of #9 and a freeze vector in #3.

**Depends on:** Phase 0 (clock + mock-NIC test seams), Phase 4 (lock discipline) so the global TCP lock work doesn't collide with the SCHEDULER lock work

**Workstreams:**
- Introduce a net softirq/worker thread that exclusively owns net::poll() and the TCP timers; IRQ handlers only enqueue RX frames + wake (fixes e1000.rs:740 full-stack-in-IRQ, the arp::resolve re-entrancy at arp.rs:101, and the IRQ-context Vec alloc).
- Add cwnd/ssthresh + RFC 6298 RTT-based RTO to the TCB (the single highest-impact real-network change); fix timer.rs to service ALL connections per pass (deferred-collect instead of `return;`).
- Replace per-packet Vec allocation with a recycled fixed-size frame buffer pool; make TcpSegment.payload a borrowed &[u8]; wire up the already-written e1000::transmit_batch (0 call sites today).
- Make IP_ID atomic and fold CONN_HASH inside the TCP_CONNECTIONS-protected struct (remove the static mut data races); gate trace::record_frame on an AtomicBool before locking; extend the hash index to IPv6.
- Convert busy-poll blocking syscalls (recv/send/connect/accept/dns) to sleep on per-socket wait queues woken by the worker (reuse the Phase 2 atomic-block primitive).

**Deliverables:**
- All protocol processing in a worker thread; IRQ handlers only enqueue + wake.
- TCP has congestion control and RTT-based RTO; retransmit services all connections per pass.
- No per-packet heap alloc; zero-copy payload; batched TX.
- No static-mut net data races; IPv6 O(1) lookup.

**Test plan:**
- kunit pure-function: is_seq_gt/gte/lte u32-wraparound; parse_tcp MSS/Wscale/NOP/EOL incl. malformed; advertised_window scaling; tcp_checksum_valid against a known-good segment.
- kunit state-machine via mock NIC: register a fake NetworkDriver that captures TX and injects scripted RX; connect->SYN, inject SYN-ACK, assert ESTABLISHED + ACK; inject data, assert recv() + ACK; 3 dup-ACKs schedule fast-retransmit; RST->Closed+wake.
- kunit retransmit/RTO with an injectable mock clock: send data with no ACK, advance past RTO, run check_retransmissions, assert ONE retransmit per connection AND that MULTIPLE connections are all serviced in one pass (catches the return-instead-of-continue bug).
- kunit OOO reassembly fuzz: insert randomly-ordered overlapping segments, assert recv_buf reconstructs the original byte stream.
- Headless serial regression: loopback TCP echo (listen 127.0.0.1, connect, send N MiB, verify byte-exact echo) printing PASS/FAIL — surfaces deadlocks/freezes from the global lock + IRQ re-entrancy.

**Risk:** Medium-High. Moving protocol processing off the IRQ and adding a worker thread is a structural change; the biggest test enabler is injecting the clock (arch::hal::timer_current_ticks) and the NIC (drivers::network::transmit/poll_rx_into), both currently hard global dependencies — that seam work should land first within this phase.

## Test Strategy

Expand kunit/ktest into a TWO-TIER, per-subsystem, headless, regression-safe harness, and establish it in Phase 0 before any other change. TIER A (in-kernel kunit) exercises REAL code paths that need the kernel runtime, against RAM-backed devices and synthetic threads; the existing pattern where a subsystem exposes pub fn kunit_*() helpers (scheduler already does this) keeps test files thin and the invariant next to the code. TIER B (host `cargo +stable test`) covers platform-neutral pure logic (buddy/heap/vma/net-types/crypto already exist in-kernel and can be extracted behind a std shim; libanyui layout/hit-test/render and compositor damage-rect math run host-side with a stub compositor, leveraging the existing `host` feature). Make it Claude-runnable and CI-ready by: (1) a kunit-kernel cmake target that builds ONLY the kernel + a minimal image (today --kunit rebuilds all userspace), in a separate build dir so dev builds are untouched; (2) a kernel-side completion sentinel + isa-debug-exit write under feature=kunit so a run yields a deterministic non-zero exit code instead of log-scraping + SIGKILL; (3) a register_kunit_suite! inventory macro so suites cannot silently drift across the two static arrays; (4) fix the per-suite N/N miscount and add a skipped counter so failed preconditions are visible. Per-subsystem coverage to add, mapped to the painful bugs: MEMORY/SWAP — swap round-trip (allocate_slot/write_page/read_page bytes-survive, currently zero callers), eviction+swap-in (mmap+touch to force reclaim, re-read assert), CoW share-then-diverge, reclaim_empty_user_tables baseline, heap grow-OOM watermark consistency, bucket double-free. SCHEDULER — no-lost-wakeup (flood deferred_wake past 256 slots), PENDING-never-underflows, fairness (priority-127 ready thread picked within one tick even when priority-1 holds the lock), the keystone ABI/rootfs cache coherence (after EVERY thread-on-CPU transition the lock-free per-CPU value equals the locked read — guards the exact staleness that forced the revert), cascade-kill exactly-once, waitpid-not-poll lock-count. LXE — wait4 WIFSIGNALED encoding (fork+kill-by-each-signal), futex contention no-lost-wakeup, mprotect-then-fault, getdents reclen no-underflow, struct-layout golden bytes, plus a headless `lxe init`/`lxe run /bin/true` smoke and a lxed two-writer concurrency test. VFS CACHE COHERENCY (the exFAT large-block read-after-write bug) — write_back then FULL-MISS read returns written bytes (pins the lookup_range-stop-at-miss + overlay invariant), dirty-eviction never-drops-a-slot, key_check quarantine drops-not-flushes, execute_write_range base_offset symmetry, plus vfsstress additions for fragmentation-boundary, partial-cache-hit, and cross-fd coherence, and finally confirmation against a REAL apt install. DRIVER/HAL — probe_and_bind_all specificity binding (pure logic), RAM-backed Arc<dyn BlockDevice> cached read/write/flush coherence, async_io scheduler ordering with a mock backend, mock NIC frame round-trip. NET — mock-clock retransmit (ONE per connection AND all connections serviced per pass), mock-NIC state-machine, OOO reassembly fuzz, loopback echo PASS/FAIL. COMPOSITOR — via AnyStream offscreen-SHM (CLAUDE.md Phase 1): malformed-IPC rejection, crash-isolation + SHM-mapping-count-to-baseline, focus/ownership, event backpressure. The ordering principle is non-negotiable: the harness and the coherence/no-lost-wakeup/cache tests land FIRST so the scheduler-lock removal, VFS-trait migration, swap, and all file-splitting refactors are guarded by green TIER A + TIER B before they begin.

## Quick Wins

- Add SYS_SHM_SIZE and immediately bound-check the compositor scaled-present path (window.rs:2322) — turns the headline whole-desktop-abort bug into a rejected frame (small, compositor finding #1).
- Move shm_unmap into destroy_window() so crash-reap stops permanently leaking a compositor SHM mapping + pinned frames per closed window (small, compositor finding #3).
- Gate trace::record_frame (net) and validate_gpu_vtable hot-path checks behind an AtomicBool/debug feature — removes a global IRQ-off lock per packet and a transmute+branch per GPU op (small).
- Convert exfat.rs:916 cluster_to_lba from panic!() to Result (validate_cluster already exists) — stops the root FS from taking down the whole kernel on one bad cluster (small).
- Fix sys_time/sys_set_time/sys_fstat/sys_pipe2 to use copy_user_bytes instead of raw NULL-only-guarded derefs (small, closes a confirmed arbitrary-kernel-write).
- Make IP_ID atomic and fold CONN_HASH into the TCP_CONNECTIONS struct — removes confirmed static-mut data races (small, net).
- Add arch::hal::irq_register/irq_unmask/irq_mask and delete the 12 copy-pasted apic-vs-pic blocks — smallest change, biggest HAL-portability win (small, drivers-hal).
- Add the kunit isa-debug-exit completion signal + a kunit-kernel-only build target — makes headless runs deterministic and fast immediately (small/medium, test-infra).
- Replace the libanyui dock layout to process DockStyle::Fill last regardless of insertion order and give docked controls a default extent — turns two documented developer traps into engine guarantees (medium-but-localized).
- Wire the already-written e1000::transmit_batch (currently 0 call sites) into the send.rs batching path (small, net).

## Biggest Risks

- Re-enabling the lock-free per-CPU ABI cache (Phase 4) is the bug the team already reverted: a stale cache makes a native thread see ABI=Linux and mis-dispatch EVERY syscall, freezing the system (thread_config.rs:82-88 documents this). It must not be attempted before the cache-coherence kunit test exists and every thread-on-CPU transition site is audited — otherwise it reintroduces a total-freeze regression.
- The VFS read-after-write coherence is an emergent 'always overlay after read' convention, not an API invariant (lookup_range stops at first miss, CONFIRMED at blockcache.rs:282). Any refactor that moves a read without the matching overlay_range silently reintroduces the apt/exFAT data corruption — the read-after-write test must gate every change to the storage/VFS path.
- Swap (#6) and CoW are both XL and touch the #PF handler + PTE encoding; getting victim selection, swap-entry encoding, or the COW refcount wrong corrupts user memory silently. Sequencing CoW after the unified for_each_user_pte walker reduces (but does not remove) the risk that clone/destroy disagree about which frames are shared.
- Without per-client fault isolation, panic=abort makes the compositor a single point of failure for the entire GUI — until SHM-size validation + a restartable compositor land, one buggy app can still abort every window. This caps how 'stable' the system can feel regardless of kernel fixes.
- The aarch64 device stack is a full parallel reimplementation that bypasses the HAL, PCI table, block cache and async_io (drivers/arm/*); every x86 fix in Phases 1-5 silently does NOT reach ARM. If ARM parity is in scope, the VirtIO-core unification (Phase 6) must precede further ARM driver work or the divergence compounds.
- The huge volume of defensive scaffolding (GPU vtable validation per call, per-tick TSS/canary checks idt.rs:2181, RBP-clobber panic guard arch/hal.rs:254, context_switch checksum) is evidence of an UNRESOLVED chronic corruption source. If the static-mut per-CPU refactor (Phase 1) does not actually root-cause it, removing the scaffolding to recover hot-path cycles will re-expose the corruption — keep the guards until a test demonstrably reproduces and then proves the fix.
- Several refactors have deep blast radius under the SAME global lock (scheduler lock removal in Phase 4 and the TCP single-lock work in Phase 8). Doing them concurrently risks merge-conflicting lock-discipline changes; they must be sequenced (scheduler first, net second) and each gated independently.
