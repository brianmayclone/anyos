# Buddy allocator — lifting the 16 GiB hard cap

Status: **planned**, no code yet.

The current physmem ceiling is **16 GiB** (`buddy::MAX_MEMORY` and
`physical::STAGE1_MAX_MEMORY`). Hit by the static-storage cost of
the per-zone metadata: at 16 GiB each ZONE carries 16 MiB of
`order_of_alloc` + 2 MiB of `used_bitmap`. Two zones = ~36 MiB
in `.data`, accommodated by the 64 MiB higher-half kernel mapping
in `virtual_mem::init`.

Lifting to 64 GiB or beyond means leaving statically-sized arrays
behind. Linux solved this in 1996 by sizing every per-frame array
at runtime from the actual E820 (now memblock) extents and putting
them on the kernel heap. We can do the same; it's not difficult,
just touches more code.

## Why "no cap" matters

* Server installations: 32 GiB+ has been workstation-norm for a
  while; 256 GiB single-socket machines exist.
* Fewer foot-guns: today, plugging 17 GiB RAM into anyOS silently
  truncates to 16 GiB at boot. Better to either use it all or
  warn loudly.
* Future: NUMA, memory hotplug, memory zones for ARM big.LITTLE —
  all assume per-region runtime sizing.

## Migration plan

### Phase 1: heap-allocated order map per zone

`BuddyZone::order_of_alloc` is the big-ticket array
(`[u8; MAX_FRAMES]`, 16 MiB at 16 GiB). Move it to a heap-backed
`Vec<u8>` sized at zone construction.

Issues:
- BuddyZone is currently `const fn new()` so it can sit in a
  `Spinlock<BuddyZone>` static. With a `Vec`, construction must
  defer until `init()` runs (after heap is up).
- Boot order: bootmem allocator must come up FIRST (it doesn't
  need the buddy). Then heap. Then buddy::init can `Vec::with_capacity`
  the right size and migrate from bootmem.
- This actually works cleanly with the current bootmem→buddy
  staged init: bootmem stays static-array-backed (it's small,
  ~2 MiB at 16 GiB), buddy goes heap-backed.

Files to touch:
- `kernel/src/memory/buddy.rs`:
  - Replace `order_of_alloc: [u8; MAX_FRAMES]` with `order_of_alloc: Vec<u8>`.
  - Replace `used_bitmap: [u8; BITMAP_BYTES]` with `used_bitmap: Vec<u8>`.
  - Replace `const fn new()` with two-phase: `const fn empty()` for the
    static slot + `init(frame_count)` to allocate the vectors.
  - Drop `MAX_MEMORY`, `MAX_FRAMES`, `BITMAP_BYTES` constants — sizing
    becomes per-zone, decided at init.
- `kernel/src/memory/physical.rs`:
  - `STAGE1_MAX_MEMORY` likewise — bootmem can stay static-array-backed
    but the array size needs to be decided at build time. Decision
    point: keep a generous static cap (say 64 GiB → 2 MiB BSS) for the
    bootmem bitmap, OR also heap-allocate it once kernel heap is up.
    Static cap is simpler and bootmem is short-lived; keep static.

### Phase 2: dynamic zone sizing

Today ZONE_DMA = `[0, 128 MiB)` and ZONE_NORMAL = `[128 MiB, MAX_MEMORY)`.
After phase 1, ZONE_NORMAL's allocator state is sized to actual RAM at
runtime, so this is automatic.

### Phase 3: E820/DTB-driven highest-frame discovery

`physical::init` already walks E820 to find `max_usable_addr`. Pass
that to `buddy::ZONE_NORMAL::init(max_usable_addr / FRAME_SIZE)`. No
hard cap needed.

ARM64 `physical::init_arm64` already takes `(ram_base, ram_size)`
explicitly — same pattern.

### Phase 4: kernel image mapping bump

Already a one-line change in `virtual_mem::init` (the `for mb in
0..32u64` loop counter). With heap-allocated buddy state, the
kernel image mapping no longer scales with RAM size — it stays at
~10 MiB for text + boot-time data and we can drop it back to the
old 16 MiB. The heap (which holds the buddy's runtime arrays)
lives at `0xffffffff82000000` and grows separately.

### Phase 5: per-zone allocators for NUMA (out of scope here)

Linux supports one buddy zone per NUMA node. Not needed for the
"no cap" goal but would be the natural follow-up.

## Risk assessment

* **Low**: phases 1–4 don't change the buddy algorithm itself,
  only the storage of its tables. Existing kunit tests catch any
  regression in the algorithm.
* **Test additions**: a kunit case that hands the buddy a > 4 GiB
  zone and exercises alloc/free across the boundary would prove
  out the new sizing.

## Effort estimate

~1 day of focused work:

| Step | LoC | Risk |
| --- | --- | --- |
| Vec-backed order_of_alloc, used_bitmap | ~80 | low |
| Two-phase init in buddy.rs                  | ~40 | low |
| physical.rs adapter                         | ~30 | low |
| Drop MAX_MEMORY / MAX_FRAMES constants      | ~20 | none |
| kunit test for > 4 GiB zone                 | ~50 | low |
| Boot test with `qemu -m 32G`                | —   | manual |

## Out of scope

* Memory hotplug — needs zone resize at runtime.
* NUMA — multiple buddy zones per memory region.
* Compaction — Linux's defrag thread; useful but not blocker.
* Per-CPU PCP magazines — orthogonal, see Phase 7 in the original
  buddy plan.
