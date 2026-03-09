# Release Notes

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
