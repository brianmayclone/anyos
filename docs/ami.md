# ami & amid — System Information Query System

## Overview

**amid** is a background daemon that maintains a system information database and provides read-only SQL query access via named pipe IPC.
**ami** is an interactive REPL client that connects to amid to query this database.

```
ami (client)                     amid (daemon)
    |                                |
    |-- pipe "ami" ---- request ---->|--- libdb ---> /System/sysdb/ami.db
    |                                |
    |<-- pipe "ami-{tid}" - reply ---|
```

## amid — System Information Daemon

### Start / Location

| Property | Value |
|----------|-------|
| Binary | `/system/amid` |
| Start | `svc start amid` |
| Database | `/System/sysdb/ami.db` |
| Pipe | `"ami"` (named pipe) |

### Initialization

1. Load `libdb.so` client library
2. Ensure `/System/sysdb/` directory exists
3. Create/open database file `/System/sysdb/ami.db`
4. Create all 8 tables (idempotent)
5. Perform initial hardware info collection
6. Create named pipe `"ami"`
7. Enter main event loop

### Refresh Intervals

| Interval | Tables |
|----------|--------|
| 2 s | `mem`, `cpu`, `threads` |
| 10 s | `devices`, `disks`, `net`, `svc` |

### Database Schema (8 Tables)

#### `hw` — Hardware Info (static, populated once)

```sql
CREATE TABLE hw (key TEXT, value TEXT)
```

| key | Source | Example |
|-----|--------|---------|
| `cpu_brand` | sysinfo cmd 4 | `"Intel Core i7-..."` |
| `cpu_vendor` | sysinfo cmd 4 | `"GenuineIntel"` |
| `tsc_mhz` | sysinfo cmd 4 | `"3400"` |
| `cpu_count` | sysinfo cmd 4 | `"4"` |
| `boot_mode` | sysinfo cmd 4 | `"BIOS"` or `"UEFI"` |
| `total_mem_mib` | sysinfo cmd 4 | `"512"` |
| `fb_width` | sysinfo cmd 4 | `"1024"` |
| `fb_height` | sysinfo cmd 4 | `"768"` |
| `fb_bpp` | sysinfo cmd 4 | `"32"` |

#### `mem` — Memory Statistics (fast refresh)

```sql
CREATE TABLE mem (key TEXT, value INTEGER)
```

| key | Source |
|-----|--------|
| `total_frames` | sysinfo cmd 0 |
| `free_frames` | sysinfo cmd 0 |
| `heap_used` | sysinfo cmd 0 |
| `heap_total` | sysinfo cmd 0 |
| `free_mem_mib` | computed: `free_frames / 256` |

#### `cpu` — Per-Core CPU Load (fast refresh)

```sql
CREATE TABLE cpu (core INTEGER, load_pct INTEGER)
```

- `core = -1` — Overall system load percentage
- `core = 0..N` — Per-core load (0-15 cores supported)

Load calculation uses delta-based tracking: `load% = 100 - (idle_delta / total_delta * 100)`.

#### `threads` — Thread List (fast refresh)

```sql
CREATE TABLE threads (
    tid INTEGER, name TEXT, state INTEGER,
    prio INTEGER, arch INTEGER, uid INTEGER,
    pages INTEGER, ticks INTEGER
)
```

| Column | Description |
|--------|-------------|
| `tid` | Thread ID |
| `name` | Thread name (max 24 chars) |
| `state` | 0=ready, 1=running, 2=blocked, 3=dead |
| `prio` | Priority 0-127 |
| `arch` | Architecture indicator |
| `uid` | User ID |
| `pages` | User-space pages allocated |
| `ticks` | CPU ticks since creation |

#### `devices` — Device List (slow refresh)

```sql
CREATE TABLE devices (path TEXT, driver TEXT, dtype INTEGER)
```

Source: `SYS_DEVLIST` syscall, 64-byte entries.

#### `disks` — Block Devices (slow refresh)

```sql
CREATE TABLE disks (
    id INTEGER, disk_id INTEGER, part INTEGER,
    start_lba INTEGER, size_sect INTEGER
)
```

- `part = -1` (0xFF) — Whole disk entry
- `part >= 0` — Partition index

Source: `SYS_DISK_LIST` syscall, 32-byte entries.

#### `net` — Network Configuration (slow refresh)

```sql
CREATE TABLE net (key TEXT, value TEXT)
```

| key | Example |
|-----|---------|
| `ip` | `"10.0.2.15"` |
| `mask` | `"255.255.255.0"` |
| `gateway` | `"10.0.2.2"` |
| `dns` | `"10.0.2.3"` |
| `mac` | `"52:54:00:12:34:56"` |
| `link` | `"up"` or `"down"` |
| `nic_enabled` | `"true"` or `"false"` |
| `nic_available` | `"true"` or `"false"` |

Source: `SYS_NET_CONFIG` cmd 0.

#### `svc` — Service Status (slow refresh)

```sql
CREATE TABLE svc (name TEXT, status TEXT, tid INTEGER)
```

- Scans `/System/etc/svc/` for service config files
- Matches service names against thread list
- Thread state 0-2 = `"running"`, state 3 = `"stopped"`
- `tid = 0` if stopped

### IPC Protocol

**Request** (client -> amid):
```
{tid}\t{sql}\n
```

**Success Response** (amid -> client):
```
OK\t{col_count}\t{row_count}\n
{col1}\t{col2}\t...\n
{val1}\t{val2}\t...\n
...
\n
```

**Error Response**:
```
ERR\t{error_message}\n
\n
```

- Only SELECT queries are allowed (INSERT/UPDATE/DELETE rejected)
- Text values: tabs escaped as `\t`, newlines as `\n`
- NULL values rendered as `"NULL"`
- Response ends with an empty line (double newline)

---

## ami — REPL Client

### Usage

```
ami                     # Interactive REPL mode
ami "SELECT * FROM hw"  # Single-shot query, then exit
```

### Connection

1. Get own TID via `getpid()`
2. Open amid's pipe: `pipe_open("ami")`
3. Create response pipe: `pipe_create("ami-{tid}")`
4. If amid not running, prints error and exits

### Interactive Commands

| Command | Description |
|---------|-------------|
| `help`, `?` | Show available commands |
| `tables`, `list tables`, `show tables`, `\dt` | List all table names |
| `exit`, `quit`, `\q` | Quit REPL |
| *any SQL* | Execute SELECT query and display results |

### Query Examples

```sql
SELECT * FROM hw
SELECT * FROM cpu
SELECT tid, name, state FROM threads
SELECT * FROM net WHERE key = 'ip'
SELECT * FROM mem ORDER BY key
SELECT COUNT(*) FROM threads WHERE state = 1
```

### Output Format

Auto-width table with column headers and separator:

```
tid | name         | state | prio
----+--------------+-------+-----
1   | idle         | 0     | 0
2   | compositor   | 1     | 100
(2 rows)
```

### Error Messages

| Error | Cause |
|-------|-------|
| `amid daemon is not running` | amid pipe not found |
| `failed to create response pipe` | pipe_create() failed |
| `amid pipe disconnected` | pipe_write() failed |
| `timeout waiting for amid response` | No response within 3s |
| `Error: {msg}` | Database error in amid |

### Constants

| Constant | Value |
|----------|-------|
| Max input line | 1024 bytes |
| Max response | 65536 bytes (64 KiB) |
| Response timeout | 3 seconds |
