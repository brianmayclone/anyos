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

## Phase status

The phases mirror the layered diagram — each phase produces something
testable in isolation. Phases marked ✅ landed in main; the merge
commit lists every behavioural change.

### ✅ Phase 0 — host test infrastructure
`scripts/run.sh --displays N` (integer, 1..16) and `--displays
WIDTHxHEIGHT,WIDTHxHEIGHT,...` (per-monitor). Writes
`/System/etc/displayd-seed.conf` for the latter form so displayd can
seed confd at first boot.

### ✅ Phase 1 — kernel display subsystem
- 1a. `OutputMode` / `OutputInfo` / `OutputLayout` / `LayoutError` /
  `DisplayEvent` + EDID CRC-64 hash in
  `kernel/src/drivers/gpu/output.rs`.
- 1b. `GpuDriver` trait per-output methods with single-output fallback:
  `set_mode_for_output`, `mode_for_output`, `transfer_rect_for_output`,
  `flush_for_output`, `update_rect_for_output`, `output_info`,
  `set_output_mirror`, `apply_layout` (atomic 3-pass),
  `poll_display_event`.
- 1c. `virtio_gpu`: `Vec<ScanoutState>`, num_scanouts from
  `virtio_gpu_config` offset 8, mirroring via shared `resource_id`,
  hot-plug observation via `events_read` polling. Boot-time activation
  of every advertised secondary scanout dodges the user-CR3 64-MiB
  identity-map fault that hits when `alloc_contiguous` returns physmem
  above that boundary.
- 1d. Six display syscalls (700–705): `SYS_DISPLAY_LIST`,
  `SYS_DISPLAY_SET_LAYOUT`, `SYS_DISPLAY_MAP_FB`, `SYS_DISPLAY_FLUSH`,
  `SYS_DISPLAY_POLL_EVENT`, `SYS_REGISTER_DISPLAY_OWNER`.
  `COMPOSITOR_PD` + `DISPLAY_OWNER_PD` two-PD privilege model so
  displayd can apply layouts atomically without being the compositor.

### ✅ Phase 2 — compositor refactor
- 2a. `Vec<Output>` with virtual-desktop rectangles, lazy
  `init_secondary_outputs()` after `Compositor::new`.
- 2b/2c. Per-output render pass: scaled wallpaper, layer blits with
  ARGB blend, drop shadows (linear falloff approximating the primary's
  cache-baked shadows), rounded corners with anti-aliased mask, blur-
  behind (uses the existing parameterised `blur_back_buffer_region`),
  software cursor. HW cursor stays primary-only (driver-side).

### ✅ Phase 3 — interactive concerns
- 3a. Cursor clamps to `virtual_desktop_bounds()` so it traverses
  output edges. HW cursor parks at the primary edge when on a
  secondary; software-cursor layer renders on the secondary.
- 3b. Maximize uses `output_at(titlebar_centre)` so the window
  expands to the output it visually lives on. Menu bar exception
  applies on the primary only.

### ✅ Phase 4 — IPC + anyui
- 4a. SYS_DISPLAY_LIST is open to every process — apps don't need
  privileged IPC for read access. `libcompositor` extension turned
  out to be redundant.
- 4b. `libanyui_client::Screen` — `list`, `primary`, `at`,
  `for_window`, `scale_px` helpers.
- 4c. Wallpaper extends to secondaries (nearest-neighbour scaled).
  Deskbar/menubar stays primary-only (matches modern macOS / Windows
  defaults). Per-output deskbar mirroring tracked as a follow-up.

### ✅ Phase 5 — services + GUI
- 5a. `services/displayd` daemon — registers as the layout owner
  via `SYS_REGISTER_DISPLAY_OWNER`, pushes layouts via
  `SYS_DISPLAY_SET_LAYOUT`, polls hot-plug events on a 1 s cadence.
- 5b. `libs/libdisplay_client` — `DisplaydClient::list_outputs /
  reapply_layout / probe_hotplug / push_layout / set_output_config /
  set_global_config / set_setup_name`. SHM-backed marshalling for
  the larger payloads (`OutputConfig` 96 B, `GlobalConfig` 32 B).
- 5c. `apps/display-settings` GUI — GNOME-style toolbar
  (Erweitern / Spiegeln) + output list + per-output detail
  (resolution combo, orientation, scale segmented control,
  fractional toggle, enabled toggle, Apply button).
- 5-spawn. Compositor spawns `/System/displayd` after fontd in the
  bootstrap sequence.
- All persistence goes through confd (`services/displayd/config/...`).
  No separate `display.conf` file is created or read.

### ✅ Phase 8 — titlebar + window-switcher integration
- Per-other-output "send to monitor" buttons on every title bar.
  `move_window_to_output` translates window position from source
  output to target output, clamping into the target rect when the
  target is smaller than the source.
- Window-switcher overlay (Strg+F1..F12): coloured M{N} badge per
  card showing the current monitor; right-click cycles the window
  to the next other monitor.

### ✅ Phase 9 — window reflow + run.sh per-monitor + GUI overhaul
- 9a–b. confd schema for displayd lives in
  `system/daemons/displayd/src/schema.rs`. Replaces the planned
  `/System/etc/display.conf` entirely.
- 9c. GUI overhaul as described in Phase 5c — drag-arrange canvas
  is its own follow-up, see open items below.
- 9d. `run.sh --displays WIDTHxHEIGHT,WIDTHxHEIGHT,...` writes
  `/System/etc/displayd-seed.conf`; displayd applies the seed
  values to confd at first boot, idempotent on subsequent boots.
- 9e (later replaced by Phase 10). Named profiles ("home" /
  "office" / "mobile") with hot-plug auto-detection. Superseded
  by the auto-keyed model in Phase 10 — the named-profile IPC is
  kept as a thin compatibility layer over the new mechanism.
- 9f. Compositor window reflow: management thread polls display
  events at 500 ms; on hot-plug or layout-change the compositor
  drops vanished outputs from `outputs[]`, adds new ones, and
  moves any window whose old output disappeared back onto the
  primary (reusing the Phase 8 `move_window_to_output` helper).

### ✅ Phase 10 — auto-keyed monitor setups
Replaces the named-profile model. The display layout is
identified by a deterministic hash of the connected EDID set —
the user no longer presses a "save" button at all. Reconnecting
the same monitor combination anywhere always produces the same
hash and therefore restores the same layout automatically.

- **Hash** = CRC-64 (ECMA polynomial) of the sorted EDID hashes
  concatenated as 8-byte big-endian buffers, rendered as
  16 lower-case hex chars. Sort gives set semantics — plug
  order doesn't matter.
- **Storage** in confd:

  ```
  config/setups/<setup_hash>/edids               (canonical EDID list)
  config/setups/<setup_hash>/friendly_name       (optional cosmetic
                                                  label — "home", …)
  config/setups/<setup_hash>/output/<edid>/...   (per-output config:
                                                  same keys as the
                                                  live config/output)
  config/active_setup                             (current setup hash)
  ```

- **Auto-load** at boot and on every `HotplugChanged`:
  `activate_current_setup()` computes the hash, copies the saved
  per-output values onto the live `config/output/<edid>/*` keys,
  then `apply_persisted_layout()` runs the regular pipeline.
- **Auto-save**: every `CMD_SET_OUTPUT_CONFIG` writes both the
  live keys and the active setup's per-output keys via
  `write_output_to_live_and_active_setup()`. So re-plugging
  this exact set later restores the change without any user
  action.
- **Fresh setups** (a combination never seen before) seed the
  setup keys from whatever's currently in the live config —
  effectively a snapshot of the kernel-reported defaults. The
  next edit accumulates real values.
- **Friendly-name** API (`CMD_SET_SETUP_NAME`,
  `client.set_setup_name("home")`) attaches a cosmetic label
  to the current setup hash — purely for the GUI; the layout
  identity is the hash itself.

Compatibility shims keep `CMD_SAVE_PROFILE` / `CMD_LOAD_PROFILE` /
`CMD_DELETE_PROFILE` callable: `SAVE` becomes "name the current
setup", `LOAD` re-applies the layout for the connected EDID set
(no-op in the new model), `DELETE` is a no-op success.

**Bug surfaced during Phase 10 validation**: an `LayoutApplied`
display event must NOT trigger `apply_persisted_layout()`. The
prior code did, which combined with the new auto-save semantics
caused an infinite write loop (apply → set_layout →
LayoutApplied → apply → …). Confd would crash within ~25 s under
the resulting write storm. Fixed by limiting re-apply triggers
to `HotplugChanged` and `PreferredModeChanged`; `LayoutApplied`
just emits `EVT_LAYOUT_CHANGED` for subscribers now.

### ✅ Phase 7 — SPICE vdagent monitors-config
vdagent parses `VD_AGENT_MONITORS_CONFIG`, builds a `LayoutEntry`
list, forwards via `libdisplay_client::push_layout`. Resize the SPICE
client window → resize chain reaches the kernel atomically.

### ✅ Phase 2c-ext-2 — HW-cursor cross-output
GpuDriver gains `move_cursor_for_output` / `show_cursor_for_output`
(default delegates to single-cursor methods for output 0). virtio_gpu
implements both via `MOVE_CURSOR` / `UPDATE_CURSOR` with per-scanout
`scanout_id`. Compositor's `apply_mouse_move` tracks
`Desktop::last_cursor_output` and on every cursor move:

  1. Locates the output whose virtual rect contains
     `(mouse_x, mouse_y)` via `Compositor::output_at`.
  2. If the cursor crossed an output boundary, hides the cursor on
     the previous output and shows it on the new one.
  3. Sends `MOVE_CURSOR_OUTPUT(target, local_x, local_y)` with
     coordinates translated into the target output's local frame.

Two new opcodes in `sys_gpu_command`:
  `11 = CURSOR_MOVE_OUTPUT(output_id, x, y)`
  `12 = CURSOR_SHOW_OUTPUT(output_id, visible)`

Single-output setups continue to use the legacy `move_hw_cursor`
path so behaviour is identical there.

### ✅ Phase 9c-extended — drag-arrange canvas
Layout preview in `display-settings` is now an interactive Canvas:
each connected output is a draggable rectangle, mouse-up commits
the new `virtual_x` to displayd via `CMD_SET_OUTPUT_CONFIG`.

Drag flow:
  * `on_mouse_down` — `output_at_canvas` hit-test, store
    `(idx, drag_offset_in_virt_px)` in `AppState.dragging`.
  * `on_mouse_move` — update `layout_x[idx]` live, re-render canvas.
  * `on_mouse_up` — snap to nearest other-output edge within
    32 virtual px (right-of-other / left-of-other tidy alignment),
    push `OutputConfig` with the new `virtual_x` to displayd.

Vertical stacking (`virtual_y`) is still horizontal-only in the
GUI — the underlying API accepts it but a y-drag UX (top-aligned
vs centre-aligned) is its own decision and tracked separately.

### Multi-monitor input fixes (along the way)
`scripts/run.sh` sets `-machine pc,vmport=off` for `--displays >1`
so QEMU disables the VMware backdoor — its absolute pointer reports
coords scoped to the primary scanout regardless of which SDL window
the click came from, which mis-routes every secondary click back to
the primary. Two safety nets:

  * `kernel/drivers/input/vmmouse::force_disable()` is called from
    `sys_register_compositor` whenever `display_count() > 1`. This
    is authoritative — `vmmouse::is_active()` returns false
    unconditionally after that, so IRQ12 falls through to PS/2
    (relative dx/dy) on every event.
  * `desktop/input::apply_mouse_move_absolute` derives a delta
    against the previous absolute coord and re-enters the relative
    path on multi-monitor. Belts-and-suspenders for any future path
    that re-engages absolute (USB tablet, SPICE absolute mode).

### ⏳ Phase 6 — DPI-aware widget pipeline (open, larger refactor)
`Screen.scale_percent` exists and per-output scale is persisted, but
the anyui widget pipeline (font sizes, padding, hit-test geometry)
still uses a single `theme::scale_factor` global. Making it
per-window scale-aware requires either:

  1. A render-time override (compositor sets the scale before
     `render_window`, restores after) — works for single-threaded
     render but is fragile under future parallelism, or
  2. A per-window scale field threaded through every widget's
     measure/paint path — clean, but touches ~50 widgets.

Approach 2 is the right answer; deferred to a focused refactor session.

### ✅ Phase 11 — per-output absolute pointer
Multi-instance virtio-input with output-id-tagged events. The first
virtio-input mouse device probed binds to scanout 0, the second to
scanout 1, and so on; further mice past the advertised scanout
count fall back to `OUTPUT_AGNOSTIC` (legacy cross-output relative
path). Each bound device:

- Reads its EV_BITS bitmap; if ABS_X / ABS_Y are exposed, reads
  the per-axis `VIRTIO_INPUT_CFG_ABS_INFO` (min, max) once and
  caches it.
- On every EV_ABS event scales raw value × output dims / range,
  storing the result in `acc_dx` / `acc_dy`.
- Emits MouseEvent with `event_type = MoveAbsolute` and
  `output_id = bound_scanout_id`.

Wire format: `sys_input_poll` puts the output id in arg3 of every
mouse event (5-u32 packet). The compositor's
`INPUT_MOUSE_MOVE_ABSOLUTE` handler:
- `arg3 = 0xFF` (OUTPUT_AGNOSTIC) — legacy path: vmmouse / VMMDev,
  primary-fb-scoped coords. Goes through the existing
  `apply_mouse_move_absolute` which derives relative deltas in
  multi-monitor as a safety net.
- `arg3 = 0..MAX_OUTPUTS` — translates raw x/y by adding the
  bound output's `virtual_x`/`virtual_y` and routes through the
  new `apply_mouse_move_absolute_virtual`, which clamps to the
  virtual desktop and re-enters the relative path so drag /
  resize / hover state stays consistent.

Boot-validated with two `virtio-tablet-pci` devices and
`--displays 2`:

```
[virtio-input] mouse #0 -> output 0 (1024x768, absolute)
[virtio-input] mouse #1 -> output 1 (1280x800, absolute)
```

Open follow-up: USB-HID tablet path (`drivers/usb/hid.rs`) doesn't
yet emit `output_id`. Same plumbing as the virtio-input path —
straightforward but separate.

## Open design questions (decide before the relevant phase)

- **Independent vs. global window numbering**: windows keep their
  global id, only the `virtual_rect` changes when they move
  between outputs. (Decided + implemented.)
- **Per-output workspaces**: out of scope for the first cut. Revisit
  if multi-workspace lands.
- **Color management**: out of scope. Document as future work.
- **Per-output deskbar / menubar**: macOS-style "menubar on primary
  only" is the current default. A `mirror_menubar` toggle in
  display-settings could expose Windows-style "menubar everywhere";
  decide based on user demand.
