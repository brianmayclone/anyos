# Multi-Monitor Architecture

Status: **In Arbeit (Branch `worktree-multimonitor`)**

This document fixes the architectural decisions for anyOS multi-monitor
support so that the implementation, which spans the kernel, the
compositor, system services and user apps, stays coherent across
multiple sessions.

The design is informed by how mature systems solve the same problem
(Linux DRM/KMS, the virtio-gpu OASIS spec, and the wlroots / Wayland
output protocols) so that anyOS follows established conventions instead
of inventing one-off mechanisms.

## Industry baseline (what we are aligning with)

### Linux DRM/KMS

The DRM display pipeline separates concerns into:

```
Framebuffer  →  Plane  →  CRTC  →  Encoder  →  Connector
```

- **Framebuffer**: pixel storage. *Shared* across the system — multiple
  planes on different CRTCs may reference the same framebuffer object
  (this is how DRM models mirroring).
- **Plane**: a rectangular source/destination assignment of a framebuffer
  region to one CRTC.
- **CRTC**: an independent timing generator that drives one output. Two
  CRTCs scan out simultaneously without interfering.
- **Encoder + Connector**: the physical link and the endpoint
  (HDMI/DP/etc., EDID lives here).

Layout changes happen via *atomic commit* IOCTLs — the whole proposed
state is validated, then either applied entirely or rejected. There is no
half-applied state visible to user space.

Hotplug is signalled to user space as uevents; user space queries the
new connector state and re-runs an atomic commit.

### virtio-gpu (OASIS spec, 2D path)

- `virtio_gpu_config.num_scanouts` (1..16) advertises how many display
  outputs the device can drive.
- Per scanout, the standard sequence is:
  1. `RESOURCE_CREATE_2D` — create a host pixel resource.
  2. `RESOURCE_ATTACH_BACKING` — attach guest pages as the backing store.
  3. `SET_SCANOUT(scanout_id, resource_id, rect)` — link resource to
     output. `resource_id == 0` disables the scanout.
- A single resource may be linked to several scanouts. The spec calls
  this out explicitly: "Create a single framebuffer, link it to all
  displays (mirroring)." Overlapping scanouts are allowed.
- Hotplug: device sets the `VIRTIO_GPU_EVENT_DISPLAY` bit in
  `events_read`; the driver acks via `events_clear` and re-issues
  `GET_DISPLAY_INFO`.

### wlroots / Wayland output model

- The compositor exposes each output as `wl_output` with its rectangle in
  a virtual desktop, scale factor, refresh rate, physical mm and
  manufacturer info.
- A separate **output-management protocol** (`wlr-output-management-unstable-v1`)
  is used by external tools (wdisplays, shikane, …) to *propose* a layout;
  the compositor either applies the whole layout atomically or rejects
  it. The compositor is not the policy owner — it executes layouts.
- Per-output workspaces are conventional (each monitor has its own
  independent window strip / workspace).

## anyOS layered design

```
┌─────────────────────────────────────────────────────────┐
│  apps/display-settings   (GUI)                          │
│   • drag-arrange outputs in virtual desktop             │
│   • pick resolution / refresh / scale per output        │
│   • mirror toggle, primary toggle                       │
│   • persists /System/etc/display.conf                   │
└──────────────────────────┬──────────────────────────────┘
                           │  libdisplay_client (NEW)
┌──────────────────────────▼──────────────────────────────┐
│  services/displayd       (user-space daemon, NEW)       │
│   • owns the current display layout                     │
│   • applies persisted /System/etc/display.conf at boot  │
│   • reacts to hotplug events from compositor            │
│   • re-applies sane fallback when an output disappears  │
│   • IPC: GetOutputs, ApplyLayout, SubscribeHotplug      │
└──────────────────────────┬──────────────────────────────┘
                           │  Compositor IPC (extended)
                           │   • CMD_GET_OUTPUTS
                           │   • CMD_APPLY_LAYOUT (atomic)
                           │   • EVT_OUTPUT_CHANGED (push)
┌──────────────────────────▼──────────────────────────────┐
│  system/compositor                                      │
│   • Vec<Output { id, virtual_rect, scale, primary,      │
│                  mode, edid_hash, framebuffer_ref }>    │
│   • Windows live in virtual-desktop coordinates         │
│   • Per-output damage rings + render passes             │
│   • Cursor crosses output boundaries seamlessly         │
│   • Window can straddle outputs (clip per output)       │
└──────────────────────────┬──────────────────────────────┘
                           │  Display syscalls (NEW + existing)
                           │   • SYS_DISPLAY_LIST
                           │   • SYS_DISPLAY_SET_LAYOUT  (atomic)
                           │   • SYS_DISPLAY_MAP_FB(output)
                           │   • SYS_DISPLAY_FLUSH(output, rect)
                           │   • SYS_DISPLAY_POLL_EVENT
┌──────────────────────────▼──────────────────────────────┐
│  Kernel display subsystem                               │
│   • DisplayManager: cached Vec<DisplayOutput>           │
│   • Hotplug event queue (drained via SYS_DISPLAY_POLL_EVENT) │
│   • GpuDriver trait — per-output methods                │
│   • Layout validation (resolution supported? scanout    │
│     index in range?) before delegating to driver        │
└──────────────────────────┬──────────────────────────────┘
                           │  GpuDriver trait
┌──────────────────────────▼──────────────────────────────┐
│  drivers/gpu/virtio_gpu.rs                              │
│   • num_scanouts read from virtio_gpu_config            │
│   • Vec<ScanoutState { resource_id, fb_phys,            │
│                        fb_pages, mode }>                │
│   • Mirroring = SET_SCANOUT(s, shared_resource_id, …)   │
│   • VIRTIO_GPU_EVENT_DISPLAY → ack + GET_DISPLAY_INFO + │
│     enqueue hotplug event                               │
└─────────────────────────────────────────────────────────┘
```

### Why split `displayd` from the compositor?

The compositor's job is "given a layout and per-window pixels, paint the
outputs". The job of "remember the user's layout, validate proposals,
react to plug events with sensible defaults" is policy and persistence.
Wlroots learnt this the hard way and pulled the policy out into separate
tools (wdisplays / shikane); we do that from the start. Concrete benefits:

- The compositor can be restarted/re-initialized after a GPU poison
  event without losing the layout (displayd re-applies it).
- The display-settings app talks to displayd, not to the compositor;
  this keeps the compositor IPC surface small.
- Headless or alternate compositors (future) can reuse displayd.

## Data model (canonical types)

Defined once in `kernel/src/drivers/gpu/output.rs`, then re-exported to
user space via `libdisplay_client`.

```rust
pub struct OutputMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,   // millihertz, 0 = unknown (matches DRM convention)
    pub bpp: u8,            // 32 for now
}

pub struct OutputInfo {
    pub id: u32,            // stable per-boot scanout index
    pub connected: bool,
    pub current_mode: Option<OutputMode>,
    pub preferred_mode: Option<OutputMode>,
    pub modes: Vec<OutputMode>,
    pub physical_mm: (u16, u16),  // from EDID, 0,0 if unknown
    pub edid_hash: u64,           // crc64 of EDID for stable identification
    pub manufacturer: [u8; 4],    // 3-letter PNPID + null, EDID-derived
}

pub struct OutputLayoutEntry {
    pub id: u32,
    pub virtual_rect: Rect,       // position in the virtual desktop
    pub mode: OutputMode,
    pub scale: u16,               // 100 = 1.0x, 150 = 1.5x, 200 = 2.0x
    pub mirror_of: Option<u32>,   // None = own framebuffer; Some(id) = mirrors output `id`
    pub primary: bool,
}

pub struct OutputLayout {
    pub entries: Vec<OutputLayoutEntry>,  // exactly one entry per active output
}
```

Layout application is **atomic**: the kernel either accepts the entire
`OutputLayout` (after validating modes, mirror chains and rect overlap
rules) or rejects it without touching the current configuration.

## Mirroring model

Mirroring is implemented as a *backing-store-shared* scanout, exactly
how virtio-gpu and DRM model it:

- The mirrored-from output owns a `resource_id` and a guest-RAM framebuffer.
- Each mirroring output issues `SET_SCANOUT(scanout_id, shared_resource_id, rect)`.
- Damage delivered to the source output also flushes the mirror outputs
  (compositor walks `mirror_of` back-references).
- Mirroring requires the same resolution on the mirror outputs (the
  compositor's layout validator enforces this).

This costs zero extra guest RAM and zero extra `TRANSFER_TO_HOST_2D`
calls — the only added work is the extra `RESOURCE_FLUSH` per mirror.

## Hotplug protocol

1. `virtio_gpu` ISR observes `events_read & VIRTIO_GPU_EVENT_DISPLAY`.
2. Driver writes the same bit to `events_clear` to ack.
3. Driver issues `GET_DISPLAY_INFO`, refreshes its cached
   `Vec<ScanoutState>`.
4. Driver pushes a `DisplayEvent::Hotplug { added: …, removed: … }` into
   the kernel display event queue.
5. Compositor wakes from `SYS_DISPLAY_POLL_EVENT`, forwards the event
   over IPC as `EVT_OUTPUT_CHANGED`.
6. `displayd` receives it, looks up the new EDID hashes against the
   persisted `display.conf`, derives a new layout (re-using saved
   positions for previously-seen outputs, default-stacking new ones to
   the right of the primary) and pushes `CMD_APPLY_LAYOUT` to the
   compositor.
7. Compositor calls `SYS_DISPLAY_SET_LAYOUT` — kernel validates and
   applies atomically.

Failure path: if any step fails, the *previous* layout stays effective.
There is no intermediate state where some outputs are reconfigured and
others are not.

## display.conf format

`/System/etc/display.conf` is libini-format, one section per output
identified by **EDID hash** (so reconnecting the same monitor to a
different physical port restores the same configuration):

```ini
[output:0x9a4f3c2b00112233]
mode = 1920x1080@60000
position = 0,0
scale = 100
primary = true

[output:0xc1b2a3d499887766]
mode = 2560x1440@60000
position = 1920,0
scale = 100
mirror_of = none

[fallback]
# applied to outputs whose EDID hash is not listed above
mode = preferred
position = right_of_primary
scale = 100
```

A separate `[manufacturer_overrides]` section can apply per-vendor scale
defaults (e.g. some HiDPI panels default to 200%).

## QEMU test matrix

Verified working configurations during development:

| Flag combination | Backend | Expected behaviour |
|---|---|---|
| `--virtio --displays 2` | SDL | Two host windows, each scanout independently driven |
| `--virtio --displays 2 --spice` | GTK + SPICE | One GTK window for input, two SPICE windows via `remote-viewer` |
| `--virtio --displays 2 --spice-app` | spice-app | Built-in viewer opens two windows |
| `--virgl --displays 2 --kvm` | SDL+GL | Same as SDL but with GL passthrough |
| `--virtio --displays 4 --kvm` | SDL | Stress test (vgamem auto-scaled to 128 MiB) |

## Phase plan (revised, enterprise alignment)

The phases mirror the layered diagram — each phase produces something
testable in isolation.

1. **Phase 0 — host test infrastructure.** `run.sh --displays N`. ✅ done.
2. **Phase 1 — kernel display subsystem.**
   - 1a. `OutputMode` / `OutputInfo` / `OutputLayout` types in
     `kernel/src/drivers/gpu/output.rs`.
   - 1b. `GpuDriver` trait per-output methods (default = single-output
     fallback). Includes `apply_layout(&OutputLayout)` for atomic
     application.
   - 1c. `virtio_gpu`: `Vec<ScanoutState>`, num_scanouts from device
     config, `events_read` hotplug, mirroring via shared resource_id.
   - 1d. New display syscalls: `SYS_DISPLAY_LIST`, `SYS_DISPLAY_SET_LAYOUT`,
     `SYS_DISPLAY_MAP_FB`, `SYS_DISPLAY_FLUSH`, `SYS_DISPLAY_POLL_EVENT`.
3. **Phase 2 — compositor refactor.**
   - 2a. `Vec<Output>` with virtual-desktop rectangles.
   - 2b. Window position is global; per-output clip is computed.
   - 2c. Per-output damage rings + render passes.
4. **Phase 3 — interactive concerns.**
   - 3a. Cursor traverses output boundaries.
   - 3b. Maximize/snap is per-output (output under titlebar wins).
5. **Phase 4 — IPC + anyui.**
   - 4a. `libcompositor` extended with `CMD_GET_OUTPUTS` /
     `CMD_APPLY_LAYOUT` / `EVT_OUTPUT_CHANGED`.
   - 4b. `libanyui_client` exposes `Screen::list / primary / for_window`.
   - 4c. Deskbar and wallpaper instances per output.
6. **Phase 5 — services + app.**
   - 5a. `services/displayd` daemon: layout owner, hotplug responder,
     persistence to `/System/etc/display.conf`.
   - 5b. `libdisplay_client` for talking to displayd.
   - 5c. `apps/display-settings` GUI: drag-arrange, mode picker,
     mirror toggle.
7. **Phase 6 — DPI/scale per output.** anyui scale-aware widgets.
8. **Phase 7 — SPICE vdagent monitors-config** integration (incoming
   client-driven layout changes, outgoing layout reports).

## Open design questions (decide before the relevant phase)

- **Independent vs. global window numbering**: when a window is dragged
  between outputs, does it keep its window-id? *Working assumption: yes,
  windows are global, only their `virtual_rect` changes.* Revisit before
  Phase 3b.
- **Per-output workspaces**: out of scope for the first cut. Revisit
  after Phase 5 ships.
- **Color management**: out of scope. Document as future work.
