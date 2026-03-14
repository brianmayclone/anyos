# anyOS nogui Mode — Text Console Boot

The **nogui** boot mode starts anyOS without the compositor and desktop environment, presenting a full-screen interactive text console directly on the framebuffer. It is designed for headless-style use, recovery scenarios, server workloads, and low-resource environments.

---

## Table of Contents

- [Activating nogui Mode](#activating-nogui-mode)
- [Boot Sequence](#boot-sequence)
- [Text Console (textcon)](#text-console-textcon)
  - [Display Modes](#display-modes)
  - [ANSI / VT100 Support](#ansi--vt100-support)
  - [Scrollback Buffer](#scrollback-buffer)
  - [Cursor Blinking](#cursor-blinking)
- [Login & Session](#login--session)
  - [Keyboard Layout](#keyboard-layout)
  - [System Environment](#system-environment)
  - [User Environment](#user-environment)
- [Shell](#shell)
  - [Built-in Commands](#built-in-commands)
  - [readline Features](#readline-features)
  - [Tab Completion](#tab-completion)
  - [Pipelines & Redirection](#pipelines--redirection)
  - [Background Execution](#background-execution)
- [Console Control — the `mode` Command](#console-control--the-mode-command)
- [Boot Configuration Editor — `bcedit`](#boot-configuration-editor--bcedit)
- [Session Store](#session-store)
- [Differences from GUI Mode](#differences-from-gui-mode)

---

## Activating nogui Mode

Add `params=nogui` to a boot entry in `/boot/boot.cfg`:

```ini
[anyOS (Textmode)]
kernel=0
params=nogui
description=anyOS without compositor (text console login)
```

Or use `bcedit` at runtime to set it:

```
bcedit set "anyOS (Textmode)" params nogui
```

The kernel detects `nogui` in the boot parameters and skips the compositor entirely. Instead of `/System/init` → compositor → desktop, it directly launches `/System/bin/textmode_console`.

---

## Boot Sequence

```
Bootloader (selects nogui entry)
    │
    ▼
Kernel init (PIT, PCI, GPU, APIC, scheduler, FS, networking, ...)
    │
    ▼
Kernel reads params="nogui"
    │
    ▼
Skips compositor, skips session host, skips desktop
    │
    ▼
Spawns /System/bin/textmode_console
    │
    ├── Reads /System/etc/inputmon.conf  → loads keyboard layout
    ├── Outputs neofetch system banner
    ├── Sets console mode (80×25 default)
    └── login_loop():
          ├── Prompts "Username:" / "Password:"
          ├── sys_authenticate() → verifies credentials
          ├── setup_environment() → sets UID, HOME, env vars
          │     ├── Loads /System/env  (system-wide, always)
          │     └── Loads ~/.env       (user-specific, optional)
          └── shell_loop() → interactive command line
```

---

## Text Console (textcon)

The kernel text console (`kernel/src/drivers/textcon.rs`) renders directly to the VESA/VirtIO framebuffer using a built-in 8×16 bitmap font. It is the single display surface in nogui mode — no window manager or compositor is involved.

### Display Modes

Three predefined console sizes are supported, selectable with the `mode` command:

| Mode | Columns × Rows | Typical resolution |
|------|----------------|-------------------|
| 1    | 80 × 25        | 1024×768 and above (default) |
| 2    | 120 × 37       | 1280×720 and above |
| 3    | 160 × 50       | 1920×1080 and above |

Cell pixel dimensions are computed as `fb_width / cols` × `fb_height / rows`, so the font scales to fill the screen exactly regardless of resolution.

Changing the mode clears the scrollback buffer and resets the display. The `SYS_CON_RESIZE` syscall (295) performs the resize at runtime.

### ANSI / VT100 Support

The text console implements a VT100/ANSI escape sequence parser:

| Sequence | Effect |
|----------|--------|
| `ESC[H` / `ESC[{r};{c}H` | Cursor home / move to row,col |
| `ESC[A/B/C/D` | Cursor up/down/right/left |
| `ESC[2J` | Clear screen |
| `ESC[K` | Erase to end of line |
| `ESC[0K` / `ESC[1K` / `ESC[2K` | Erase line (to EOL / from BOL / whole) |
| `ESC[0J` / `ESC[1J` / `ESC[2J` | Erase display (to end / from start / whole) |
| `ESC[{n}m` (SGR) | Set colors and attributes (bold, underline, reverse) |
| `ESC[?25l` / `ESC[?25h` | Hide / show cursor |
| `ESC[?7h` / `ESC[?7l` | Enable / disable auto-scroll |

**SGR color support:**

- Standard 8 foreground colors (`30–37`) and background colors (`40–47`)
- Bright variants (`90–97` / `100–107`)
- 256-color palette (`ESC[38;5;{n}m` / `ESC[48;5;{n}m`)
- True color 24-bit RGB (`ESC[38;2;r;g;bm` / `ESC[48;2;r;g;bm`)
- Reset (`ESC[0m`), Bold (`ESC[1m`), Underline (`ESC[4m`), Reverse (`ESC[7m`)

The `$TERM` environment variable is set to `ansi` so programs that check it (e.g. `ls`) automatically enable color output.

### Scrollback Buffer

The console maintains a 200-row ring buffer of off-screen history, plus a shadow buffer of the current visible screen (up to 50 rows). This allows viewport scrolling without re-running ANSI sequences.

| Key | Action |
|-----|--------|
| **Shift + ↑** | Scroll viewport up one line |
| **Shift + ↓** | Scroll viewport down one line |

Shift+Up/Down are intercepted entirely in the kernel (`sys_con_poll_key`) and never reach userspace. Any new terminal output automatically snaps the viewport back to the live view.

Scrollback is cleared when:
- The console mode changes (`mode` command)
- The screen is cleared (`clear` builtin, `ESC[2J`)
- Cursor visibility or auto-scroll flags are toggled via `SYS_CON_SET_MODE`

### Cursor Blinking

The cursor is an underline block at the bottom of the current cell. It blinks at 1 Hz (500 ms on, 500 ms off), driven by the PIT IRQ handler at 1000 Hz via `tick_blink()`.

- Blinking stops when the cursor is hidden (`ESC[?25l` or mode bit 0 set)
- Blinking stops when the viewport is scrolled back (to avoid confusion)
- Any new character output resets the blink phase (cursor stays solid for 500 ms after keystrokes)
- `tick_blink()` uses a non-blocking `try_lock` on the GPU mutex — if the GPU is busy, the blink cycle is silently skipped, preventing deadlock

---

## Login & Session

### Keyboard Layout

Before displaying the login prompt, `textmode_console` reads `/System/etc/inputmon.conf` and extracts the `layout=` key:

```ini
layout=de
```

The layout is applied via `SYS_KBD_SET_LAYOUT`. If the file is absent or the key is missing, the default layout (typically `us`) remains active.

### System Environment

After a successful login, the system-wide environment file `/System/env` is always loaded, regardless of which user logged in. Format: one `KEY=VALUE` per line, `#` for comments.

```sh
# /System/env
PATH=/System/bin:/System/sbin
HOSTNAME=anyos
TERM=ansi
```

### User Environment

After `/System/env`, the user's own `~/.env` is loaded if it exists. For root, `HOME=/`; for other users, `HOME=/Users/<username>`.

```sh
# /Users/alice/.env
EDITOR=nano
PS1=\u@\h:\w$
```

Variables from `.env` override `/System/env` values of the same name.

The process adopts the authenticated user's identity via `sys_set_identity(uid)` before the shell loop starts.

---

## Shell

`textmode_console` includes a built-in interactive shell. External programs are executed via `sys_spawn()`.

### Built-in Commands

These commands run within the shell process itself (they cannot work as external programs because they need to modify the shell's own state):

| Command | Description |
|---------|-------------|
| `cd [dir]` | Change directory; syncs kernel thread CWD so spawned children inherit it |
| `exit` / `logout` | Exit the shell; returns to the login prompt |
| `clear` | Clear the screen (`ESC[2J ESC[H`) |
| `export [KEY=VALUE]` | Set an environment variable; without args, lists all env vars |
| `set [KEY=VALUE]` | Alias for `export` |
| `unset KEY` | Remove an environment variable |

### readline Features

The shell input line supports:

| Key | Action |
|-----|--------|
| **←** / **→** | Move cursor left/right within the line |
| **↑** / **↓** | Navigate command history (last 64 entries) |
| **Backspace** | Delete character before cursor |
| **Delete** (`ESC[3~`) | Delete character at cursor |
| **Home** / **End** | Jump to start/end of line |
| **Tab** | Trigger tab completion |
| **Ctrl+C** | Cancel current input line |

### Tab Completion

Pressing **Tab** triggers context-sensitive completion:

- **Command completion**: If the cursor is on the first word, searches built-in commands, then `$PATH` directories, then `/System/bin/`
- **Path completion**: If the cursor is on a subsequent word (argument), expands the partial path; directories are shown with a trailing `/`
- **Disambiguation**: If multiple completions exist, they are listed and the common prefix is applied

### Pipelines & Redirection

The shell supports single pipelines and standard I/O redirection:

```sh
ls -la | grep txt        # anonymous pipe between two processes
cat file.txt > out.txt   # stdout redirection (create/truncate)
cat file.txt >> out.txt  # stdout append
cmd < input.txt          # stdin redirection
```

### Background Execution

```sh
httpd &           # run in background, shell continues immediately
nohup httpd       # run in background, detached (immune to logout)
```

`&` at the end of a command runs it with `sys_spawn()` (fire-and-forget). The shell prints `[bg] pid=<tid>` and returns to the prompt immediately.

`nohup` is equivalent: it prepends the command to a background spawn without waiting.

---

## Console Control — the `mode` Command

The `mode` binary (`/System/bin/mode`) resizes the console at runtime:

```
mode           — show current size and available modes
mode 1         — 80×25  (standard)
mode 2         — 120×37 (wide)
mode 3         — 160×50 (full HD)
```

Internally it calls `anyos_std::sys::con_resize(cols, rows)` → `SYS_CON_RESIZE` (295).

Changing the mode resets the scrollback buffer and redraws the screen.

---

## Boot Configuration Editor — `bcedit`

`bcedit` (`/System/bin/bcedit`) edits `/boot/boot.cfg` semantically from within the running system. Changes take effect on the next reboot.

```
bcedit                           List all entries (compact)
bcedit list                      List all entries with keys
bcedit list-flags                Show global flags (timeout, default)
bcedit show <name>               Show a single entry
bcedit check                     Validate config (reports errors/warnings)

bcedit set-flag timeout 3        Set auto-boot timeout to 3 seconds
bcedit set-flag default 2        Set default entry to index 2

bcedit add "My Entry"            Add a new boot entry
bcedit remove "My Entry"         Remove an entry (refuses if it is the last one)
bcedit rename <name> <new>       Rename an entry
bcedit duplicate <name> <new>    Clone an entry under a new name

bcedit set <name> params nogui   Set a key in an entry
bcedit del <name> params         Remove a key from an entry

bcedit init                      Restore factory default boot.cfg
```

`bcedit check` validates: numeric flags, default index in range, missing `kernel` key, missing `description`, duplicate entry names, and duplicate keys within an entry.

See [bootloader.md](bootloader.md) for the `boot.cfg` format reference.

---

## Session Store

The session store provides simple key-value persistence for shell scripts across commands, without modifying the process environment.

Backing file: `/tmp/.sstore` (plain text, `KEY=VALUE\n` per line).

| Command | Usage | Description |
|---------|-------|-------------|
| `sstore` | `sstore KEY VALUE` | Store or update a value |
| `sstore` | `sstore` (no args) | List all stored keys and values |
| `sget` | `sget KEY` | Print value for key (silent if missing) |
| `sdel` | `sdel KEY` | Delete a key |
| `sdel` | `sdel --all` | Wipe the entire store |

**Example use in a script:**

```sh
sstore token $(cat /tmp/auth_response)
TOKEN=$(sget token)
echo $TOKEN
sdel token
```

The store is not a replacement for environment variables — environment variables are per-process (inherited by children). The session store is a shared file visible to all processes in the session.

---

## Differences from GUI Mode

| Aspect | GUI mode | nogui mode |
|--------|----------|------------|
| Display | Compositor + windows | Kernel framebuffer textcon |
| Desktop | anyOS desktop shell | None |
| Login | GUI login dialog | Text login prompt |
| Terminal | Terminal.app | textmode_console (built-in shell) |
| Graphics | Full 2D/3D GPU | No UI; framebuffer is text-only |
| Services | All (via init.conf + svc) | Only if launched from the shell |
| ANSI colors | Full (Terminal.app) | Full (textcon ANSI parser) |
| Scrollback | Terminal.app history | Kernel ring buffer (200 rows) |
| `$TERM` | `ansi` | `ansi` |
| Cursor | Software cursor | Kernel blink via PIT IRQ |
