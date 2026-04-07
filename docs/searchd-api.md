# anyOS Search Daemon (searchd) API Reference

The Search Daemon (`searchd`) provides system-wide file indexing and full-text search for anyOS. It periodically crawls standard directories, classifies files by type, indexes text content, and exposes a search API via named pipe IPC.

**Binary:** `/System/bin/searchd`
**Source:** `system/searchd/`
**Database:** `/System/sysdb/search.db`
**Configuration:** `/System/etc/searchd.conf`
**IPC Pipe:** `searchd`
**Dependencies:** `anyos_std`, `libdb_client`

## Table of Contents

- [Overview](#overview)
- [Configuration](#configuration)
  - [Main Section](#main-section)
  - [Folders Section](#folders-section)
  - [Example Configuration](#example-configuration)
- [Architecture](#architecture)
  - [Indexing Pipeline](#indexing-pipeline)
  - [File Classification](#file-classification)
  - [Content Indexing](#content-indexing)
  - [Indexed Directories](#indexed-directories)
- [Database Schema](#database-schema)
  - [files Table](#files-table)
  - [content Table](#content-table)
  - [state Table](#state-table)
- [IPC Search API](#ipc-search-api)
  - [Protocol](#protocol)
  - [Commands](#commands)
    - [SEARCH — Freetext Search](#search--freetext-search)
    - [FIND — Filename Search](#find--filename-search)
    - [KIND — Type Filter](#kind--type-filter)
    - [RECENT — Recently Indexed](#recent--recently-indexed)
    - [STATS — Index Statistics](#stats--index-statistics)
    - [REINDEX — Trigger Re-Index](#reindex--trigger-re-index)
  - [Response Format](#response-format)
- [Client Usage Example](#client-usage-example)
- [Timing & Scheduling](#timing--scheduling)
- [Limitations](#limitations)

---

## Overview

`searchd` runs as a background daemon and performs two main tasks:

1. **Indexing** — Crawls the filesystem, classifies every file/directory by type, and stores metadata in a libdb database. For text-based files (source code, config files, documents), the content is also read and stored in searchable chunks.

2. **Search** — Accepts search queries via the `searchd` named pipe and returns matching files with path, name, type, and size.

The daemon waits for a configurable idle period after boot before starting the initial index, avoiding load during system startup. After the initial pass, incremental re-indexing runs periodically to pick up filesystem changes.

## Configuration

Configuration is read from `/System/etc/searchd.conf` at daemon startup. The file uses INI format. Missing keys fall back to defaults.

### Main Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `idleTimeout` | Integer | `10000` | Milliseconds to wait after boot before starting the initial index pass. Prevents indexing from competing with boot-time services. |
| `maxEntries` | Integer | `1000000` | Maximum number of entries in the `files` table. Indexing stops when this limit is reached. Prevents unbounded database growth on large filesystems. |

### Folders Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `excludes` | Comma-separated list | `/proc,/dev,/sys` | Directories to exclude from indexing. Paths are matched as prefixes — excluding `/tmp` also excludes `/tmp/foo/bar`. |

### Example Configuration

```ini
[Main]
idleTimeout=10000
maxEntries=1000000

[Folders]
excludes=/proc,/dev,/sys,/tmp
```

**Note —** Changes to `searchd.conf` require a daemon restart to take effect. The configuration is read once at startup.

## Architecture

### Indexing Pipeline

```
Startup
  |
  v
Load /System/etc/searchd.conf
  |
  v
Open /System/sysdb/search.db (libdb)
  |
  v
Create IPC pipe "searchd"
  |
  v
Wait idleTimeout ms
  |
  v
Initial full index
  |       \
  |        +-- For each directory in INDEX_DIRS:
  |        |     Skip if in excludes list
  |        |     Recursively walk (max depth 16)
  |        |     For each entry:
  |        |       Classify by extension/name -> kind
  |        |       INSERT into files table
  |        |       If text-like and <= 64 KiB:
  |        |         Read content, normalize, chunk
  |        |         INSERT chunks into content table
  |        +-- Stop if maxEntries reached
  |
  v
Main loop:
  - Handle IPC search requests (non-blocking)
  - Every 5 min: incremental re-index (stale dirs only)
```

### File Classification

The indexer automatically classifies every file into one of 16 types based on file extension and well-known filenames:

| Kind | Description | Extensions / Patterns |
|------|-------------|----------------------|
| `directory` | Filesystem directory | (detected by `is_dir()`) |
| `document` | Text documents | `.txt`, `.md`, `.markdown`, `.rst`, `.org`, `.tex`, `.rtf`, `.csv`, `.log`, `README`, `LICENSE`, `CHANGELOG` |
| `script` | Source code, scripts | `.rs`, `.c`, `.h`, `.cpp`, `.js`, `.ts`, `.py`, `.go`, `.java`, `.html`, `.css`, `.sh`, `.sql`, `.php`, ... |
| `config` | Configuration files | `.conf`, `.cfg`, `.ini`, `.toml`, `.yaml`, `.yml`, `.json`, `.env`, `.properties` |
| `image` | Image files | `.png`, `.jpg`, `.jpeg`, `.gif`, `.bmp`, `.ico`, `.svg`, `.webp`, `.tiff`, `.psd` |
| `audio` | Audio files | `.mp3`, `.wav`, `.flac`, `.ogg`, `.aac`, `.wma`, `.m4a`, `.opus` |
| `video` | Video files | `.mp4`, `.avi`, `.mkv`, `.mov`, `.wmv`, `.flv`, `.webm` |
| `archive` | Archive/compressed | `.zip`, `.tar`, `.gz`, `.bz2`, `.xz`, `.7z`, `.rar`, `.iso` |
| `font` | Font files | `.ttf`, `.otf`, `.woff`, `.woff2`, `.bdf` |
| `library` | Shared/static libraries | `.so`, `.dll`, `.dylib`, `.a`, `.lib` |
| `database` | Database files | `.db`, `.sqlite`, `.sqlite3` |
| `executable` | Executables | `.elf`, `.exe`, `.app`, `.bin` or no extension |
| `url` | Bookmarks/URLs | `.url`, `.webloc`, `.desktop` |
| `link` | Symlinks/shortcuts | `.lnk` |
| `binary` | Other binary files | (fallback for unrecognized extensions) |

### Content Indexing

Content is indexed for text-based file types: `document`, `script`, `config`, and `url`.

Constraints:
- Only files up to **64 KiB** are read
- Content is validated as UTF-8; binary files (containing null bytes in the first 512 bytes) are skipped
- Text is **normalized**: whitespace collapsed, converted to lowercase
- Stored in **240-byte chunks** (libdb TEXT column limit is 255 bytes)
- Chunks are split at word boundaries when possible

### Indexed Directories

The following directories are crawled by default:

| Directory | Content |
|-----------|---------|
| `/Users` | User home directories and files |
| `/Applications` | Installed GUI applications |
| `/System/bin` | CLI programs and system tools |
| `/System/etc` | System configuration files |
| `/System/lib` | Shared libraries |
| `/System/fonts` | Installed fonts |
| `/System/share` | Shared data files |
| `/Documents` | User documents |
| `/Desktop` | Desktop files |
| `/Downloads` | Downloaded files |
| `/tmp` | Temporary files |

**Note —** Directories that don't exist are silently skipped. The crawl depth is limited to 16 levels to prevent infinite recursion from symlink loops.

## Database Schema

The search index is stored in `/System/sysdb/search.db` using libdb. Three tables are used:

### files Table

One row per indexed filesystem entry.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT | Absolute file path (unique key) |
| `name` | TEXT | Filename component (without directory) |
| `kind` | TEXT | Classification string (see File Classification) |
| `size` | INTEGER | File size in bytes (0 for directories) |
| `modified` | INTEGER | `uptime_ms()` timestamp when entry was indexed |
| `parent` | TEXT | Parent directory path |

### content Table

Text content chunks for searchable files. A single file may have multiple rows.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT | File path (references `files.path`) |
| `chunk` | INTEGER | Chunk index (0, 1, 2, ...) |
| `body` | TEXT | Normalized text content (up to 240 bytes) |

### state Table

Crawler bookkeeping for incremental re-indexing.

| Column | Type | Description |
|--------|------|-------------|
| `dir` | TEXT | Directory path |
| `last_scan` | INTEGER | `uptime_ms()` timestamp of last completed scan |

## IPC Search API

### Protocol

`searchd` uses the same pipe-based IPC pattern as `amid`:

1. Client creates a response pipe named `searchd-{tid}` (where `tid` is the client's thread ID)
2. Client sends a request to the `searchd` pipe: `{tid}\t{command}\n`
3. `searchd` parses the command, executes the search, and writes the response to `searchd-{tid}`

### Commands

#### SEARCH — Freetext Search

```
SEARCH {query}
```

Searches both filenames and file content for the given query string. Matching is case-insensitive substring matching. Returns up to 100 results.

**Search order:**
1. Filename matches are returned first
2. Content matches follow (duplicates are deduplicated)

**Example request:**
```
42\tSEARCH network config
```

#### FIND — Filename Search

```
FIND {pattern}
```

Searches only filenames (not content) for a substring match. Case-insensitive. Returns up to 100 results.

**Example request:**
```
42\tFIND main.rs
```

#### KIND — Type Filter

```
KIND {kind}
```

Returns all files of a specific classification type. Uses exact match on the `kind` column. Valid values: `directory`, `document`, `script`, `config`, `image`, `audio`, `video`, `archive`, `font`, `library`, `database`, `executable`, `url`, `link`, `binary`.

**Example request:**
```
42\tKIND image
```

#### RECENT — Recently Indexed

```
RECENT {count}
```

Returns the most recently indexed files, sorted by index timestamp descending. Default count is 20, maximum is 100.

**Example request:**
```
42\tRECENT 10
```

#### STATS — Index Statistics

```
STATS
```

Returns index statistics: total file count, directory count, and number of content chunks.

**Example response:**
```
OK	3
files	4521
directories	312
content_chunks	18764

```

#### REINDEX — Trigger Re-Index

```
REINDEX
```

Schedules a full re-index. The index is rebuilt on the next main loop iteration (within ~1 second). During re-indexing, search queries continue to work against the existing data.

### Response Format

**Success (search results):**
```
OK\t{row_count}\n
{path}\t{name}\t{kind}\t{size}\n
{path}\t{name}\t{kind}\t{size}\n
...
\n
```

Each result row contains four tab-separated fields:

| Field | Description |
|-------|-------------|
| `path` | Absolute file path |
| `name` | Filename |
| `kind` | File classification |
| `size` | File size in bytes |

**Success (stats):**
```
OK\t{stat_count}\n
{key}\t{value}\n
...
\n
```

**Error:**
```
ERR\t{message}\n\n
```

## Client Usage Example

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let tid = anyos_std::process::gettid();

    // Create response pipe
    let mut name_buf = [0u8; 32];
    let reply_name = format_pipe_name(tid, &mut name_buf);
    let reply_pipe = anyos_std::ipc::pipe_create(reply_name);

    // Open searchd pipe
    let searchd_pipe = anyos_std::ipc::pipe_open("searchd");
    if searchd_pipe == 0 {
        anyos_std::println!("searchd not running");
        return;
    }

    // Send search request
    let query = anyos_std::format!("{}\tSEARCH network\n", tid);
    anyos_std::ipc::pipe_write(searchd_pipe, query.as_bytes());

    // Read response
    anyos_std::process::sleep(200); // Give searchd time to respond
    let mut buf = [0u8; 4096];
    let n = anyos_std::ipc::pipe_read(reply_pipe, &mut buf);
    if n > 0 {
        if let Ok(response) = core::str::from_utf8(&buf[..n as usize]) {
            anyos_std::println!("{}", response);
        }
    }

    anyos_std::ipc::pipe_close(reply_pipe);
}

fn format_pipe_name<'a>(tid: u32, buf: &'a mut [u8]) -> &'a str {
    let prefix = b"searchd-";
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut pos = prefix.len();
    let mut digits = [0u8; 10];
    let mut dpos = 10;
    let mut val = tid;
    if val == 0 {
        dpos -= 1;
        digits[dpos] = b'0';
    } else {
        while val > 0 {
            dpos -= 1;
            digits[dpos] = b'0' + (val % 10) as u8;
            val /= 10;
        }
    }
    let dlen = 10 - dpos;
    buf[pos..pos + dlen].copy_from_slice(&digits[dpos..]);
    pos += dlen;
    core::str::from_utf8(&buf[..pos]).unwrap_or("searchd-0")
}
```

## Timing & Scheduling

| Event | Interval | Description |
|-------|----------|-------------|
| Initial index | `idleTimeout` ms after boot | Full index of all configured directories |
| Incremental re-index | Every 5 minutes | Only re-scans directories whose `last_scan` is older than 5 minutes |
| IPC polling | 100 ms (active) / 1000 ms (idle) | Checks for incoming search requests |
| Manual re-index | On `REINDEX` command | Full index rebuild within ~1 second |

## Limitations

- **libdb TEXT limit**: Text values are limited to 255 bytes. Content is chunked into 240-byte pieces, which means very long lines or words may be split across chunks.
- **No LIKE/regex**: libdb does not support `LIKE` or regular expression queries. All searches use in-memory substring matching after loading results.
- **No ORDER BY/LIMIT**: libdb does not support `ORDER BY` or `LIMIT`. Sorting (for `RECENT`) is done in memory after fetching all rows.
- **Case-insensitive ASCII only**: Lowercase conversion covers ASCII A-Z only, not Unicode characters.
- **Max 100 results**: Search commands return at most 100 matching entries to keep response sizes bounded.
- **Content limit**: Only files up to 64 KiB have their content indexed. Larger files are indexed by name/metadata only.
- **No real-time updates**: Filesystem changes are picked up at the next incremental re-index (every 5 minutes) or on manual `REINDEX`.
- **Max 8 columns per table**: libdb schema constraint. The current schema uses 6 columns maximum, well within limits.
