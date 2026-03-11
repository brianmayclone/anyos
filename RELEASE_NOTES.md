# Release Notes

## v0.4.0 — 2026-03-11

### Features

#### Fullscreen Mode Support
- **Alt+Enter toggle**: Windows registered as fullscreen-capable can enter/exit fullscreen via Alt+Enter
- **Ctrl+Alt+Del escape**: System-wide hotkey to force-exit fullscreen mode
- **Compositor integration**: Full-screen window hides menubar, background, and other windows; `damage_all()` ensures correct redraw on exit
- **Automatic SHM resize**: anyui automatically resizes window SHM buffer to screen dimensions on enter and restores original size on exit — no per-app resize code needed
- **Direct framebuffer access** (kernel syscalls `sys_grant_framebuffer` / `sys_revoke_framebuffer`): Maps GPU framebuffer at VA 0x19000000 into fullscreen app for zero-copy rendering
- **LibGL fullscreen rendering**: `gl_init_fullscreen()` / `gl_swap_buffers_fullscreen()` copy rendered frames directly to the mapped framebuffer, bypassing SHM compositing
- **IPC protocol**: New commands CMD_SET_FULLSCREEN_CAP (0x1030), CMD_REQUEST_FULLSCREEN (0x1031), CMD_EXIT_FULLSCREEN (0x1032) with corresponding responses and events
- **GLDemo**: First fullscreen-capable app — Phong-shaded 3D scene renders at native screen resolution in fullscreen

#### Stack: Kernel -> Compositor -> libcompositor -> libanyui -> App
- Kernel: `sys_grant_framebuffer` / `sys_revoke_framebuffer` syscalls (259/261), `unmap_page_in_pd` for foreign address space unmapping
- Compositor: `enter_fullscreen()` / `exit_fullscreen()` with layer visibility management, `FullscreenCapability` per window
- libcompositor: 3 new DLL exports (set_fullscreen_capable, request_fullscreen, exit_fullscreen)
- libanyui: Fullscreen events (EVENT_FULLSCREEN_ENTER/EXIT), automatic window resize on transitions
- libanyui_client: `Window::set_fullscreen_capable()`, `on_fullscreen_enter()`, `on_fullscreen_exit()`, `get_fullscreen_info()`
- libgl: `gl_init_fullscreen()`, `gl_exit_fullscreen()`, `gl_swap_buffers_fullscreen()` with FXAA support

### Statistics
- **26 files changed**, ~1,087 lines added
- Full-stack feature spanning kernel, compositor, 6 libraries, and 1 app

---

## v0.3.164 — 2026-03-11

### Performance

#### libgl Rasterizer — Heap Allocation Fix
- Eliminated per-frame heap allocations in `draw()` and `draw_elements()` that caused gradual FPS degradation (60 FPS → 4 FPS over time)
- Replaced `Vec` clones with raw pointers, stack-allocated arrays, and reusable static buffers
- Prevents heap fragmentation in long-running 3D applications

#### CoreVM Optimizations
- Enhanced control flow and memory management in the x86 virtual machine engine

### Features

#### ARM64 Architecture (v0.3.164)
- Improved exception handling with better fault diagnostics
- Enhanced SMP initialization sequence
- Improved syscall management for AArch64

#### ACPI & Interrupt Routing
- Implemented DSDT with `_SB.PCI0._PRT` for proper PCI interrupt routing
- Direct GSI (Global System Interrupt) routing for PCI devices
- Enhanced IOAPIC and LAPIC debugging output
- LAPIC LINT0/LINT1 properly unmasked for PIC interrupt acceptance
- Improved IRQ vector mapping with better LAPIC priority handling
- Refactored LAPIC timer handling with proper IRQ fallback logic

#### CoreVM x86 Emulation
- Added `RDFSBASE`, `RDGSBASE`, `WRFSBASE`, `WRGSBASE` instruction support (FSGSBASE)

#### Clock App
- Migrated to modern anyui framework
- Added countdown timer with desktop notifications

#### Physics Engine (libgl)
- Added `linear_damping` property for rigid bodies

#### Settings App
- Added toggle for serial verbose output

### Bug Fixes
- Fixed PIC delivery logic with proper fallback to LAPIC
- Improved terminal rendering and input handling
- Fixed compositor window input handling edge case

### Statistics
- **50 files changed**, ~3,100 lines added, ~1,000 lines removed
- **18 commits** since v0.3.161

---

## v0.3.161 — 2026-03-09

### Major Features

#### Terminal Emulator Rewrite
- Complete rewrite from legacy UI to the modern **libanyui** framework
- **7 color themes**: Default, Dracula, Solarized Dark/Light, Monokai, Nord, Gruvbox, One Dark
- **Unicode support**: CJK wide characters (2-column rendering), combining/diacritical characters
- **Hyperlinks**: OSC 8 protocol with Ctrl+Click to open
- **5000 line scrollback** (up from 500) with visual scrollbar and regex search
- Full **ANSI escape sequence** support: 256-color, true-color, bold, italic, underline, strikethrough, blink, reverse video
- **Alternate screen buffer** for vi/nano/htop
- **Mouse tracking** (modes 1000, 1002, 1003, 1006 SGR)
- **Bracketed paste mode**
- **Shell features**: Job control (bg/fg/jobs/Ctrl+Z), logical operators (&&, ||, ;), pipes, redirection (>, >>, 2>, <), variable expansion ($VAR), command substitution ($(cmd)), here-documents (<< EOF)

#### Physics Engine (libgl)
- New **rigid body dynamics engine** integrated into libgl.so
- Collider types: sphere, infinite plane, axis-aligned box
- Configurable gravity, mass, restitution (bounciness)
- Semi-implicit Euler integration for stable time-stepping
- Per-body forces, impulses, and angular velocity
- **20 new C FFI exports** (libgl total: 105 exports)

#### Signal Handling & Job Control
- Enhanced `kill()` syscall: now accepts signal parameter (SIGKILL, SIGTSTP, SIGCONT, etc.)
- **13 signal numbers** documented and implemented (SIGHUP through SIGTTOU)
- New `ThreadState::Stopped` for proper job control
- `try_waitpid()` returns STOPPED status (0xFFFFFFFD)
- SIGCONT properly clears pending stop signals
- New `send_signal()` helper in anyos_std

#### Window Management
- **Edge snapping**: Drag windows to screen edges for half-screen tiling
- **Window tiling**: Auto-arrange all windows in a grid layout
- Improved modal dialog z-ordering above popup menus

### New Applications

- **Notifications** — Desktop notification history viewer with DataGrid UI and JSON persistence
- **Forger** — 3D voxel world with Minecraft-style block terrain (uses libgl physics + software rasterizer)

### Improvements

#### FTP Client (`ftp`)
- 5 new local commands: `lpwd`, `lcd`, `lls`, `lmkdir`, `lrm`
- Tab completion for remote files
- Improved help with separated remote/local command sections

#### `ls` Command
- `--color` flag for colorized output (blue=directories, cyan=symlinks, green=executables)

#### libanyui
- **Tooltip system** with 500ms hover delay
- `anyui_canvas_draw_text()` for text rendering on Canvas widgets
- Font rendering to raw pixel buffers

#### Kernel
- New `SYS_SET_SERIAL_VERBOSE` syscall (283) for runtime serial debug output control
- `serial_verbose_println!()` / `serial_verbose_print!()` macros for conditional driver logging
- Improved signal delivery and scheduler re-entry after SIGCONT

#### Forger (3D Demo)
- Half-resolution rendering for performance (4x pixel reduction)
- Reduced view distance and chunk generation for software rasterizer stability
- Backface culling enabled

#### GL Demo
- Physics integration demo with bouncing spheres
- Skybox and improved scene rendering

### Bug Fixes
- `glUniform1f()` now correctly broadcasts value to all 4 components
- Fixed window drag accidental snapping (only snaps when mouse actually moved)
- Fixed modal dialog z-order above popup menus
- Improved VNC keyboard input handling

### Statistics
- **184 syscalls** (was 183)
- **105 libgl exports** (was 85)
- **29 GUI applications** (was 27)
- **152 files changed**, ~9,700 lines added, ~2,700 lines removed

---

## Previous Releases

See git log for earlier release history.
