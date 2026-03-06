# Compositor Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract Desktop/Menüleiste into a Shell process, CrashDialog into a standalone .app, create a Sessionhost, and strip the compositor to pure window management + compositing.

**Architecture:** Four new/modified binaries: Shell (menubar + desktop icons + wallpaper), CrashDialog.app (standalone crash UI), Sessionhost (process lifecycle), and a slimmed compositor (windows + compositing only). Apps register menus directly with Shell. Compositor notifies Shell of focus changes.

**Tech Stack:** Rust (no_std), anyos_std, libanyui_client, libfont_client, libimage_client, librender_client

---

### Task 1: Create CrashDialog .app scaffold

**Files:**
- Create: `system/crashdialog/Cargo.toml`
- Create: `system/crashdialog/src/main.rs`
- Create: `system/crashdialog/build.rs`
- Create: `system/crashdialog/Info.conf`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "crashdialog"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libfont_client = { path = "../../libs/libfont_client" }
librender_client = { path = "../../libs/librender_client" }
```

**Step 2: Create build.rs**

Copy the pattern from `system/compositor/dock/build.rs` — it sets the linker script path.

**Step 3: Create Info.conf**

```
id=com.anyos.crashdialog
name=CrashDialog
exec=CrashDialog
version=0.3.110
category=System
capabilities=display,shm,event
```

**Step 4: Create main.rs — port crash_dialog.rs**

Port `system/compositor/compositor/src/desktop/crash_dialog.rs` (287 lines) into a standalone app.

The current CrashDialog reads a `CrashReport` struct from the compositor's memory. As a standalone app, it receives crash info via command-line args or a shared memory region.

**Design:**
- Sessionhost passes crash info via SHM: `spawn("/System/CrashDialog.app/CrashDialog", shm_id_as_string)`
- CrashDialog maps the SHM, reads `CrashReport`, renders the dialog window
- On "OK" click → exit

```rust
#![no_std]
#![no_main]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::prelude::*;

// Same CrashReport struct as in compositor/desktop/crash_dialog.rs
#[repr(C)]
struct CrashReport {
    tid: u32,
    signal: u32,
    rip: u64, rsp: u64, rbp: u64,
    rax: u64, rbx: u64, rcx: u64, rdx: u64,
    rsi: u64, rdi: u64,
    r8: u64, r9: u64, r10: u64, r11: u64,
    r12: u64, r13: u64, r14: u64, r15: u64,
    cr2: u64,
    cs: u64, ss: u64, rflags: u64, err_code: u64,
    stack_frames: [u64; 16],
    num_frames: u32,
    name: [u8; 32],
    valid: bool,
}

// Dialog dimensions from existing code
const DIALOG_W: u32 = 420;
const DIALOG_H_COLLAPSED: u32 = 160;
const DIALOG_H_EXPANDED: u32 = 440;

#[no_mangle]
pub extern "C" fn main() {
    // Parse SHM ID from args
    let args = anyos_std::env::args();
    // Map SHM, read CrashReport
    // Create window (borderless, centered)
    // Render dialog (port render() from crash_dialog.rs)
    // Event loop: handle clicks on OK / Details toggle
    // On OK → exit
}
```

Port the rendering logic from `crash_dialog.rs` lines 90-287:
- `signal_name()` — maps signal numbers to names
- `render()` — draws dialog background, red indicator bar, icon, text, details panel, OK button
- `handle_click()` — toggle details expansion, dismiss on OK

**Step 5: Add to workspace**

Add `"system/crashdialog"` to the `[workspace] members` array in the root `Cargo.toml`.

**Step 6: Build and verify**

```bash
ANYOS_VERSION=0.3.110 cargo build --bin crashdialog
```

**Step 7: Commit**

```bash
git add system/crashdialog/ Cargo.toml
git commit -m "feat: add standalone CrashDialog .app"
```

---

### Task 2: Create Sessionhost

**Files:**
- Create: `system/sessionhost/Cargo.toml`
- Create: `system/sessionhost/src/main.rs`
- Create: `system/sessionhost/build.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "sessionhost"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
```

**Step 2: Create build.rs**

Same linker script pattern as dock/compositor.

**Step 3: Create main.rs**

Sessionhost responsibilities:
1. Start Shell process
2. Subscribe to system events (`ipc::evt_sys_subscribe(0)`)
3. Monitor for `EVT_PROCESS_EXITED` events
4. On crash (exit code > 128 = signal): create SHM with CrashReport, spawn CrashDialog
5. On Shell crash: restart Shell

```rust
#![no_std]
#![no_main]

extern crate alloc;
use anyos_std::prelude::*;

const SHELL_PATH: &str = "/System/Shell";
const CRASH_DIALOG_PATH: &str = "/System/CrashDialog.app/CrashDialog";

#[no_mangle]
pub extern "C" fn main() {
    // Spawn Shell
    let shell_tid = anyos_std::process::spawn(SHELL_PATH, "");

    // Subscribe to system events
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);

    // Main loop
    loop {
        let mut evt = [0u32; 5];
        anyos_std::ipc::evt_sys_poll(sys_sub, &mut evt, 5000);

        if evt[0] == 0 { continue; } // timeout

        match evt[0] {
            EVT_PROCESS_EXITED => {
                let tid = evt[1];
                let exit_code = evt[2];

                if exit_code > 128 {
                    // Process crashed — launch CrashDialog
                    // Read crash report from kernel if available
                    // Create SHM, write report, spawn CrashDialog with SHM ID
                    launch_crash_dialog(tid, exit_code);
                }

                if tid == shell_tid {
                    // Shell crashed — restart it
                    shell_tid = anyos_std::process::spawn(SHELL_PATH, "");
                }
            }
            _ => {}
        }
    }
}

fn launch_crash_dialog(tid: u32, exit_code: u32) {
    // Allocate SHM, write crash info
    // Spawn CrashDialog with SHM ID as argument
    let args = /* format shm_id */;
    anyos_std::process::spawn(CRASH_DIALOG_PATH, &args);
}
```

**Note:** Check how `show_crash_dialog()` currently works in `compositor/src/main.rs` (around line 722-762, `EVT_PROCESS_EXITED` handler). The kernel provides crash report data via the system event — port that extraction logic here.

**Step 4: Add to workspace**

Add `"system/sessionhost"` to root `Cargo.toml` workspace members.

**Step 5: Build and verify**

```bash
ANYOS_VERSION=0.3.110 cargo build --bin sessionhost
```

**Step 6: Commit**

```bash
git add system/sessionhost/ Cargo.toml
git commit -m "feat: add Sessionhost for process lifecycle management"
```

---

### Task 3: Create Shell process (Desktop + Menüleiste)

**Files:**
- Create: `system/shell/Cargo.toml`
- Create: `system/shell/src/main.rs`
- Create: `system/shell/src/menubar.rs`
- Create: `system/shell/src/desktop.rs`
- Create: `system/shell/build.rs`
- Modify: `Cargo.toml` (workspace members)

This is the largest task. The Shell takes ownership of:
- Menu bar rendering (from `compositor/src/menu/`)
- Desktop icons (from `compositor/src/desktop/desktop_icons.rs`)
- Wallpaper (from `compositor/src/desktop/mod.rs`)
- Menu registration IPC (apps register menus with Shell, not compositor)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "shell"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libfont_client = { path = "../../libs/libfont_client" }
librender_client = { path = "../../libs/librender_client" }
libimage_client = { path = "../../libs/libimage_client" }
```

**Step 2: Create build.rs**

Same linker script pattern.

**Step 3: Design Shell IPC**

Shell creates its own event channel named `"shell"`. Apps connect to `"shell"` to register menus.

New Shell IPC commands (reuse existing protocol constants where sensible):
- `CMD_SET_MENU (0x4001)` — app registers its menu bar definition via SHM
- `CMD_UPDATE_MENU_ITEM (0x4002)` — app updates a menu item's state
- `CMD_ADD_STATUS_ICON (0x4003)` — app adds a tray icon
- `CMD_REMOVE_STATUS_ICON (0x4004)` — app removes a tray icon

Shell subscribes to compositor's channel to receive:
- `EVT_WINDOW_OPENED (0x0060)` — track active apps
- `EVT_WINDOW_CLOSED (0x0061)` — cleanup menus for exited apps
- A new event: `EVT_FOCUS_CHANGED` — compositor tells Shell which app is now focused (needs to be added to compositor, see Task 5)

**Step 4: Create main.rs**

```rust
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use anyos_std::prelude::*;

mod menubar;
mod desktop;

#[no_mangle]
pub extern "C" fn main() {
    libfont_client::init();

    // Create Shell's IPC channel for menu registration
    let shell_channel = anyos_std::ipc::evt_chan_create("shell");

    // Subscribe to compositor events (focus changes)
    let comp_sub = anyos_std::ipc::evt_chan_subscribe("compositor");

    // Subscribe to system events (for desktop icon updates)
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);

    // Create menubar window (full-width, height ~24px, always on top, borderless)
    // Create desktop window (full-screen, behind all other windows)

    let mut shell = ShellState {
        menubar: menubar::MenuBar::new(/* screen_width */),
        desktop: desktop::Desktop::new(/* screen_width, screen_height */),
        active_app_tid: 0,
        // ...
    };

    // Main event loop
    loop {
        // 1. Poll Shell IPC channel (menu registrations from apps)
        // 2. Poll compositor subscription (focus changes)
        // 3. Poll system events (volume mounts for desktop icons)
        // 4. Handle menubar input (clicks on menus)
        // 5. Handle desktop input (icon clicks, context menus)
        // 6. Redraw if needed
    }
}
```

**Step 5: Create menubar.rs**

Port from `compositor/src/menu/` (4 files, ~1089 lines total):
- `mod.rs` (229 lines) — MenuBar struct, registration logic
- `types.rs` (234 lines) — MenuBarDef, MenuItem, parsing from SHM
- `dropdown.rs` (372 lines) — dropdown rendering and interaction
- `rendering.rs` (254 lines) — menubar background, items, clock, status icons

The MenuBar renders into its own window buffer. Key difference from current code: menus come via Shell's IPC channel, not compositor's.

**Step 6: Create desktop.rs**

Port from:
- `desktop_icons.rs` (991 lines) — DesktopIconManager, volume icons, context menus
- Wallpaper loading/rendering from `desktop/mod.rs`

Desktop renders wallpaper + icons into a full-screen background window.

**Step 7: Add to workspace and build**

```bash
ANYOS_VERSION=0.3.110 cargo build --bin shell
```

**Step 8: Commit**

```bash
git add system/shell/ Cargo.toml
git commit -m "feat: add Shell process (menubar + desktop)"
```

---

### Task 4: Add EVT_FOCUS_CHANGED to compositor

**Files:**
- Modify: `system/compositor/compositor/src/ipc_protocol.rs`
- Modify: `system/compositor/compositor/src/desktop/mod.rs` (focus change logic)
- Modify: `system/compositor/compositor/src/desktop/input.rs` (where focus changes happen)

**Step 1: Add protocol constant**

In `ipc_protocol.rs`, add:
```rust
pub const EVT_FOCUS_CHANGED: u32 = 0x0062;
// Payload: [EVT_FOCUS_CHANGED, new_focused_tid, new_focused_win_id, 0, 0]
```

**Step 2: Emit on focus change**

In `desktop/mod.rs` or `desktop/input.rs`, wherever `focused_window` is updated, emit `EVT_FOCUS_CHANGED` as a broadcast event (same pattern as `EVT_WINDOW_OPENED`). Look for all places where `self.focused_window` is assigned.

Key locations to find:
- `desktop/input.rs` — mouse click focuses window
- `desktop/ipc.rs` — `CMD_FOCUS_BY_TID`, `CMD_CREATE_WINDOW` (auto-focus)
- `desktop/mod.rs` — `CMD_DESTROY_WINDOW` (focus fallback)

Add a helper method:
```rust
fn emit_focus_changed(&mut self, tid: u32, win_id: u32) {
    self.tray_ipc_events.push((None, [EVT_FOCUS_CHANGED, tid, win_id, 0, 0]));
}
```

Call it whenever `self.focused_window` changes.

**Step 3: Build and verify**

```bash
ANYOS_VERSION=0.3.110 cargo build --bin compositor
```

**Step 4: Commit**

```bash
git add system/compositor/compositor/src/ipc_protocol.rs system/compositor/compositor/src/desktop/
git commit -m "feat: add EVT_FOCUS_CHANGED broadcast for Shell"
```

---

### Task 5: Strip compositor — remove desktop/menubar rendering

**Files:**
- Delete: `system/compositor/compositor/src/desktop/crash_dialog.rs`
- Delete: `system/compositor/compositor/src/desktop/desktop_icons.rs`
- Delete: `system/compositor/compositor/src/menu/` (entire directory)
- Modify: `system/compositor/compositor/src/desktop/mod.rs` — remove wallpaper, menu_bar, crash_dialogs, desktop_icons, logo fields
- Modify: `system/compositor/compositor/src/desktop/input.rs` — remove menu interactions, desktop icon interactions
- Modify: `system/compositor/compositor/src/desktop/ipc.rs` — remove `CMD_SET_MENU`, `CMD_ADD_STATUS_ICON`, `CMD_REMOVE_STATUS_ICON`, `CMD_SET_WALLPAPER` handling (these now go to Shell)
- Modify: `system/compositor/compositor/src/main.rs` — remove crash dialog spawning from `EVT_PROCESS_EXITED`, remove menu-bar-related init

**This task is the most delicate — must be done carefully to avoid breaking window management.**

**Step 1: Remove crash_dialog.rs**

Delete the file. Remove `mod crash_dialog;` from `desktop/mod.rs`. Remove `crash_dialogs: Vec<CrashDialog>` field and all references.

**Step 2: Remove desktop_icons.rs**

Delete the file. Remove `mod desktop_icons;` from `desktop/mod.rs`. Remove `desktop_icons` field and all polling/rendering references.

**Step 3: Remove menu/ directory**

Delete `src/menu/` entirely. Remove `mod menu;` from `main.rs` or wherever it's declared. Remove `menu_bar: MenuBar` field from Desktop. Remove menu rendering from the render path. Remove menu interaction from input handling.

**Step 4: Remove wallpaper from Desktop**

Remove `wallpaper_path`, `wallpaper_pixel_cache`, wallpaper loading/rendering. The background layer (`bg_layer_id`) can remain as a solid color fallback, or be removed if Shell's desktop window covers it.

**Step 5: Remove crash dialog handling from main.rs**

In the `EVT_PROCESS_EXITED` handler (~line 722-762), remove the `show_crash_dialog()` call. The Sessionhost now handles this.

**Step 6: Remove menu-related IPC commands from ipc.rs**

Remove handling for `CMD_SET_MENU`, `CMD_UPDATE_MENU_ITEM`, `CMD_ADD_STATUS_ICON`, `CMD_REMOVE_STATUS_ICON`. These now go to Shell's channel.

**Step 7: Clean up Desktop struct**

Remove fields that are no longer needed:
- `menu_bar`, `btn_hover`, `btn_pressed`, `btn_anims` (menu interaction)
- `crash_dialogs`
- `desktop_icons`
- `wallpaper_path`, `wallpaper_path_len`, `wallpaper_pixel_cache`
- `logo_white`, `logo_black`, `logo_w`, `logo_h`
- `menubar_layer_id` (if menubar rendering is fully removed)

Keep:
- All window management fields (`windows`, `focused_window`, `next_window_id`, etc.)
- Input state (`mouse_x/y`, `dragging`, `resizing`, `current_modifiers`)
- Cursor management
- Compositor layers for windows
- Clipboard (stays in compositor as shared state)
- `app_subs`, `tray_ipc_events` (still needed for event routing)
- Volume HUD (can stay or move to Shell — user preference)

**Step 8: Build and verify**

```bash
ANYOS_VERSION=0.3.110 cargo build --bin compositor
```

Expect many compilation errors from removed fields/modules. Fix each one. This is iterative.

**Step 9: Commit**

```bash
git add -A system/compositor/compositor/
git commit -m "refactor: strip desktop/menubar/crashdialog from compositor"
```

---

### Task 6: Update init.conf and sysroot

**Files:**
- Modify: `system/init/src/main.rs` or the init.conf template/default
- Modify: build/install scripts (if any) to install Shell, CrashDialog, Sessionhost to `/System`

**Step 1: Update boot sequence**

The init system needs to start Sessionhost (which starts Shell). Current boot order:
1. Compositor (already started)
2. Sessionhost (new — starts after compositor)
3. Dock (already started, can stay as-is)

Sessionhost starts Shell internally, so Shell doesn't need an init.conf entry.

**Step 2: Install paths**

Ensure the build system copies:
- `shell` binary → `/System/Shell`
- `sessionhost` binary → `/System/Sessionhost`
- `crashdialog` binary + Info.conf → `/System/CrashDialog.app/CrashDialog` + `/System/CrashDialog.app/Info.conf`

**Step 3: Verify full boot works**

Test sequence:
1. Compositor starts → shows background
2. Sessionhost starts → spawns Shell
3. Shell creates menubar window + desktop window
4. Dock starts
5. User logs in
6. Apps register menus with Shell (not compositor)
7. Kill an app → Sessionhost detects → CrashDialog appears

**Step 4: Commit**

```bash
git add system/init/ build/
git commit -m "feat: update boot sequence for Shell/Sessionhost/CrashDialog"
```

---

### Task 7: Update anyos_std / app-side menu API

**Files:**
- Modify: `libs/stdlib/src/` (if there's a menu registration helper that currently targets "compositor" channel)

**Step 1: Check current menu API**

Apps currently call `CMD_SET_MENU` on the compositor's IPC channel. This needs to target the `"shell"` channel instead.

Search for any convenience wrappers in `anyos_std` or `libanyui_client` that hardcode the `"compositor"` channel name for menu operations. Update them to use `"shell"`.

If there's no wrapper and apps send raw IPC, the apps themselves need updating — but the protocol constants stay the same, just the target channel changes.

**Step 2: Build affected apps**

Rebuild apps that use menus to verify they compile.

**Step 3: Commit**

```bash
git commit -m "feat: redirect menu registration from compositor to shell"
```

---

Plan complete and saved to `docs/plans/2026-03-06-compositor-extraction.md`. Two execution options:

**1. Subagent-Driven (this session)** — I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** — Open new session with executing-plans, batch execution with checkpoints

Which approach?