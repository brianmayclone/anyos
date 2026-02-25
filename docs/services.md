# anyOS Service System

The anyOS service system provides a unified mechanism for managing background daemons, centralized logging, and log inspection. It consists of the following components:

| Component | Type | Source | Binary |
|-----------|------|--------|--------|
| **svc** | CLI tool | `bin/svc/` | `/System/bin/svc` |
| **logd** | System daemon | `bin/logd/` | `/System/bin/logd` |
| **crond** | System daemon | `bin/crond/` | `/System/bin/crond` |
| **httpd** | System daemon | `bin/httpd/` | `/System/bin/httpd` |
| **amid** | System daemon | `system/amid/` | `/System/bin/amid` |
| **Event Viewer** | GUI application | `system/eventviewer/` | `/Applications/Event Viewer.app` |

### Managed Services

The following services are configured in `/System/etc/svc/` and started at boot via `svc start-all`:

| Service | Description |
|---------|-------------|
| **logd** | Centralized logging daemon (collects app + kernel messages) |
| **sshd** | SSH server |
| **echoserver** | Echo test server |
| **crond** | Cron job scheduler (periodic task execution) |
| **httpd** | HTTP web server |

### System Daemons (non-svc)

These daemons are started directly by the compositor or init, not managed by `svc`:

| Daemon | Description |
|--------|-------------|
| **amid** | Anywhere Management Interface — system information database with SQL query interface |
| **audiomon** | Audio monitoring service |
| **inputmon** | Input device monitoring |
| **netmon** | Network monitoring |
| **login** | Login manager |
| **permdialog** | Permission dialog daemon |

---

## Table of Contents

- [Service Manager (svc)](#service-manager-svc)
  - [Commands](#commands)
  - [Service Configuration](#service-configuration)
  - [Dependencies](#dependencies)
  - [Thread Detection](#thread-detection)
  - [Boot Integration](#boot-integration)
- [Logging Daemon (logd)](#logging-daemon-logd)
  - [Architecture](#architecture)
  - [Configuration](#logd-configuration)
  - [Log Format](#log-format)
  - [Log Rotation](#log-rotation)
  - [Kernel Messages](#kernel-messages)
- [Logging API (anyos_std::log)](#logging-api-anyos_stdlog)
  - [Macros](#macros)
  - [Wire Protocol](#wire-protocol)
  - [Source Detection](#source-detection)
  - [Lazy Pipe Initialization](#lazy-pipe-initialization)
- [Event Viewer](#event-viewer)
  - [UI Layout](#ui-layout)
  - [Filtering](#filtering)
  - [Auto-Refresh](#auto-refresh)

---

## Service Manager (svc)

The `svc` command manages system services defined by configuration files in `/System/etc/svc/`. It detects running services by querying the kernel thread list (not PID files), making it robust against crashes and restarts.

**Source:** `bin/svc/src/main.rs`

### Commands

```
svc <command> [service] [args...]
```

| Command | Description |
|---------|-------------|
| `svc start <name> [args]` | Start a service (with optional extra arguments) |
| `svc stop <name>` | Stop a running service (sends SIGKILL) |
| `svc status <name>` | Check if a service is running (prints TID) |
| `svc restart <name> [args]` | Stop then start a service |
| `svc list` | List all configured services with status |
| `svc start-all` | Start all configured services that are not already running |

#### Example Output

```
$ svc list
SERVICE          STATUS   EXEC
-------          ------   ----
echoserver       running  /System/bin/echoserver
logd             running  /System/bin/logd
sshd             stopped  /System/bin/sshd

$ svc start sshd
sshd: started (TID 42)

$ svc status sshd
sshd: running (TID 42)
```

### Service Configuration

Each service is defined by a plain-text configuration file in `/System/etc/svc/`. The filename is the service name.

**Sysroot source path:** `sysroot/System/etc/svc/`

#### File Format

```
exec=<path to binary>
args=<optional command-line arguments>
depends=<comma-separated list of dependency service names>
```

All keys are optional except `exec`. If `exec` is missing or empty, the config file is considered invalid and the service is skipped.

#### Example Configurations

**`/System/etc/svc/sshd`**
```
exec=/System/bin/sshd
args=
```

**`/System/etc/svc/echoserver`**
```
exec=/System/bin/echoserver
args=
```

**`/System/etc/svc/logd`**
```
exec=/System/bin/logd
args=
```

**`/System/etc/svc/crond`**
```
exec=/System/bin/crond
args=
```

**`/System/etc/svc/httpd`**
```
exec=/System/bin/httpd
args=
```

**Service with dependencies (example):**
```
exec=/System/bin/webapp
args=--port 8080
depends=logd,sshd
```

### Dependencies

Services can declare dependencies via the `depends=` key. When starting a service (either via `svc start` or `svc start-all`), the dependency resolver:

1. Parses the `depends=` value as a comma-separated list of service names
2. Recursively resolves each dependency's own dependencies (depth-first)
3. Starts any dependency that is not already running
4. Aborts with an error if a dependency fails to start
5. Protects against circular dependencies with a maximum chain depth of **8 levels**

### Thread Detection

Unlike traditional init systems that use PID files, `svc` queries the kernel's live thread list via the `sysinfo` syscall (command 1). Each 60-byte thread entry contains:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | TID (u32 LE) |
| 5 | 1 | State (0=ready, 1=running, 2=blocked, 3=dead) |
| 8 | 23 | Thread name (null-terminated) |

The kernel assigns the thread name from the binary filename (e.g., `/System/bin/sshd` becomes `sshd`). A service is considered **running** if any non-terminated thread (state 0, 1, or 2) matches the service name.

This approach is crash-safe: if a service crashes, there is no stale PID file to clean up. The next `svc status` call will correctly report it as stopped.

### Boot Integration

Services are started at boot via `init.conf`:

**`/System/etc/init/init.conf`**
```
/System/bin/dhcp
/System/bin/svc start-all
```

The `init` program:
1. Runs each line as a command (splitting path from arguments at the first space)
2. Passes the full command line as `args` (including argv[0] for the program name convention)
3. Suffix `&` runs the program in the background (init does not wait for it)
4. Lines starting with `#` are comments

When `svc start-all` runs, it:
1. Reads all service config files from `/System/etc/svc/`
2. Skips services that are already running
3. Resolves and starts dependencies
4. Starts each remaining service
5. Reports how many services were started vs. already running

---

## Logging Daemon (logd)

The `logd` daemon is a central logging service that collects messages from two sources and writes timestamped, structured log entries to disk with automatic rotation.

**Source:** `bin/logd/src/main.rs`
**Binary:** `/System/bin/logd`
**Config:** `/System/etc/logd.conf`
**Log output:** `/System/logs/system.log`

### Architecture

```
+------------------+       +------------------+
| Application      |       | Kernel           |
| anyos_std::log   |       | serial_println!  |
|   log_info!(...)  |       |   dmesg buffer   |
+--------+---------+       +--------+---------+
         |                          |
    pipe_write("log")         sys::dmesg()
         |                          |
         v                          v
+--------+------- logd -------------+---------+
|                                             |
|  Pipe Reader          Dmesg Tracker         |
|  (poll pipe_read)     (poll new offsets)     |
|         |                    |              |
|         +-------> LogWriter <+              |
|                   (buffer + flush)          |
|                      |                      |
+----------------------|----------------------+
                       v
             /System/logs/system.log
             /System/logs/system.log.1
             /System/logs/system.log.2
             ...
```

**Data flow:**
1. **Application messages** arrive via the named pipe `"log"`. Apps write using the `anyos_std::log` macros. Format: `LEVEL|source|message\n`
2. **Kernel messages** are polled from the dmesg ring buffer. A `DmesgTracker` records the last-read offset and only processes new bytes.
3. Both sources are timestamped and written to an in-memory buffer.
4. The buffer is flushed to disk periodically (default: every 5 seconds) or when it exceeds 4 KiB.

### logd Configuration

**Path:** `/System/etc/logd.conf`

| Key | Default | Description |
|-----|---------|-------------|
| `log_dir` | `/System/logs` | Directory for log files (created automatically) |
| `max_size` | `1048576` (1 MiB) | Maximum size of `system.log` before rotation |
| `max_files` | `4` | Number of rotated files to keep |
| `kernel` | `true` | Enable kernel dmesg polling |
| `flush_interval` | `5000` | Milliseconds between disk flushes |

**Example `/System/etc/logd.conf`:**
```
log_dir=/System/logs
max_size=1048576
max_files=4
kernel=true
flush_interval=5000
```

### Log Format

All log entries use a consistent format:

```
[YYYY-MM-DD HH:MM:SS] LEVEL source: message
```

**Timestamp:** Real-time clock (RTC) via `sys::time()`. Persists across reboots.

**Level field** (padded to 5 characters):

| Level | Description | Color (ARGB) |
|-------|-------------|--------------|
| `INFO` | Informational message | `0xFF58D68D` (green) |
| `WARN` | Warning | `0xFFFFAA00` (orange) |
| `ERROR` | Error condition | `0xFFFF4444` (red) |
| `DEBUG` | Debug/diagnostic | `0xFF969696` (grey) |
| `KERN` | Kernel message (from dmesg) | `0xFF5DADE2` (blue) |

**Examples:**
```
[2026-02-22 20:03:12] INFO  logd: logging daemon started
[2026-02-22 20:03:12] INFO  sshd: listening on port 22
[2026-02-22 20:03:15] KERN  E1000: link up 1000 Mbps
[2026-02-22 20:04:01] WARN  webapp: connection pool 80% full
[2026-02-22 20:04:02] ERROR webapp: database connection timeout
```

### Log Rotation

When `system.log` exceeds `max_size`, logd rotates files:

1. Delete `system.log.<max_files-1>` (oldest)
2. Rename `system.log.<N-1>` to `system.log.<N>` (shift existing)
3. Rename `system.log` to `system.log.1`
4. Reset `current_size` to 0 (new `system.log` starts empty)

If `max_files=1`, the file is truncated instead of rotated.

### Kernel Messages

When `kernel=true`, logd polls the dmesg ring buffer using `sys::dmesg()`:

- **Baseline**: On startup, reads the current dmesg buffer and records the offset. Boot messages are NOT re-logged (they are already visible via the `dmesg` command).
- **Polling**: Each loop iteration reads dmesg and compares against `last_offset`. Only new bytes are processed.
- **Wrap detection**: If the new offset is less than `last_offset`, the ring buffer has wrapped. All current data is treated as new.

Each kernel message line is prefixed with `KERN` level.

---

## Logging API (anyos_std::log)

The standard library provides a zero-configuration logging API that sends structured messages to `logd` via the named pipe.

**Source:** `libs/stdlib/src/log.rs`

### Macros

```rust
use anyos_std::*;

// Informational message
log_info!("server started on port {}", port);

// Warning
log_warn!("connection pool {}% full", usage);

// Error
log_error!("failed to open {}: {}", path, err);

// Debug (verbose diagnostics)
log_debug!("packet received: {} bytes", len);
```

All macros use `format_args!()` internally, which means:
- **Zero heap allocation** for the format string
- Full `core::fmt` formatting support (`{}`, `{:?}`, `{:#x}`, etc.)
- Message is built in a 512-byte stack buffer

### Wire Protocol

Messages are sent over the named pipe `"log"` in this format:

```
LEVEL|source|message\n
```

| Field | Description |
|-------|-------------|
| `LEVEL` | One of `INFO`, `WARN`, `ERROR`, `DEBUG` |
| `source` | Program name derived from argv[0] (see below) |
| `message` | Free-form text (max ~460 bytes per message) |

**Pipe delimiter:** `|` (0x7C)
**Line terminator:** `\n` (0x0A)

### Source Detection

The source name is automatically derived from the program's argv[0]:

1. Read raw args via `process::getargs()`
2. Find end of argv[0] (first space or end of string)
3. Extract filename after last `/`
4. Truncate to 31 characters

Example: `/System/bin/echoserver` becomes `echoserver`.

### Lazy Pipe Initialization

The pipe to logd is opened lazily on first use:

1. First `log_*!()` call tries `pipe_open("log")`
2. On success: pipe ID is cached in an `AtomicU32` for subsequent calls
3. On failure (logd not running): `u32::MAX` is stored, preventing retry on every call
4. If logd is not running, log messages are silently dropped (no crash, no error)

To reset the connection after a logd restart, call `anyos_std::log::reset()`.

---

## Event Viewer

The Event Viewer is a GUI application modeled after the Windows Event Viewer (Ereignisanzeige). It reads log files produced by `logd` and displays them in a sortable, filterable table with a detail pane.

**Source:** `apps/eventviewer/src/main.rs`
**Binary:** `/System/bin/eventviewer`
**UI Framework:** libanyui (not uisys)

### UI Layout

```
+----------------------------------------------------------+
| [Refresh]  | Level: [All|Error|Warn|Info|Kern|Debug] | Q  |  <- Toolbar (36px)
+----------------------------------------------------------+
| Time          | Level | Source     | Message              |  <- DataGrid header
|---------------|-------|-----------|----------------------|
| [00:03:12.456]| INFO  | logd      | logging daemon sta.. |  <- DataGrid rows
| [00:03:12.510]| INFO  | sshd      | listening on port 22 |     (DOCK_FILL)
| [00:03:15.023]| KERN  | E1000     | link up 1000 Mbps    |
| [00:04:01.789]| WARN  | webapp    | connection pool 80.. |
| [00:04:02.001]| ERROR | webapp    | database connection. |
+----------------------------------------------------------+
| Details                                                   |  <- Detail pane
| [00:04:02.001] ERROR webapp: database connection timeout  |     (100px, monospace)
+----------------------------------------------------------+
| 5 events                                                  |  <- Status bar (24px)
+----------------------------------------------------------+
```

**Window:** 800 x 520 pixels, dark theme (0xFF1E1E1E background)

**Controls used:**
- `Toolbar` — top bar with Refresh button, level filter, search field
- `SegmentedControl` — level filter tabs (All, Error, Warn, Info, Kern, Debug)
- `SearchField` — full-text search with per-keystroke filtering
- `DataGrid` — main log table (monospace font, 20px row height, color-coded level column)
- `Divider` — separator between grid and detail pane
- `Label` — detail view (monospace), status bar
- `View` — container for detail pane and status bar

### Filtering

The Event Viewer supports two simultaneous filter dimensions:

**Level filter** (via SegmentedControl):
| Index | Filter | Shows |
|-------|--------|-------|
| 0 | All | All entries |
| 1 | Error | `ERROR` entries only |
| 2 | Warn | `WARN` entries only |
| 3 | Info | `INFO` entries only |
| 4 | Kern | `KERN` entries only |
| 5 | Debug | `DEBUG` entries only |

**Text search** (via SearchField):
- Case-insensitive substring match
- Searches across source, message, and level fields
- Filters in real-time on each keystroke

Both filters are combined (AND logic). The status bar shows `"X of Y events"` when filters are active.

### Auto-Refresh

The Event Viewer automatically reloads log files every **10 seconds** via `ui::set_timer()`. Manual refresh is available via the Refresh button.

**Log files read:**
1. `/System/logs/system.log` (current)
2. `/System/logs/system.log.1` through `.8` (rotated, stops at first missing file)

Entries are displayed newest-first. Clicking a row shows the full entry in the detail pane.

### Color Coding

The Level column uses color-coded text:

| Level | Color | Hex |
|-------|-------|-----|
| ERROR | Red | `#FF4444` |
| WARN | Orange | `#FFAA00` |
| INFO | Green | `#58D68D` |
| KERN | Blue | `#5DADE2` |
| DEBUG | Grey | `#969696` |

---

## Adding a New Service

To add a new managed service:

1. **Create the service binary** in `bin/<name>/` (Rust) or `bin/<name>/src/<name>.c` (C)

2. **Create the service config** at `sysroot/System/etc/svc/<name>`:
   ```
   exec=/System/bin/<name>
   args=
   depends=
   ```

3. **Add to build system** — `CMakeLists.txt`:
   ```cmake
   add_rust_user_program(<name> ${CMAKE_SOURCE_DIR}/bin/<name>)
   ```

4. **Add to workspace exclusion** — root `Cargo.toml`:
   ```toml
   exclude = [
     ...
     "bin/<name>",
   ]
   ```

5. The service will be started automatically at boot by `svc start-all`.

## Sending Log Messages from Applications

Any user program can send structured log messages:

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::log_info!("application started");

    // ... application logic ...

    if let Err(e) = do_something() {
        anyos_std::log_error!("operation failed: {}", e);
    }

    anyos_std::log_info!("shutting down gracefully");
}
```

Messages appear in `/System/logs/system.log` and the Event Viewer within the configured flush interval (default 5 seconds).
