# mkfs-corefs-host

Host-side CoreFS formatter.  Creates a CoreFS volume inside a regular
file, loopback device or raw `/dev/sdX` — the same on-disk format the
anyOS kernel mounts at runtime.

This is the **host** counterpart of the anyOS-side [`bin/mkfs.corefs`](../../bin/mkfs.corefs).
While the anyOS binary runs inside the OS and talks to the kernel via
`disk_read`/`disk_write` syscalls, `mkfs-corefs-host` runs on Linux /
macOS and operates on a byte-addressable file.  Both share the actual
format logic via `libs/libcorefs-tools` and `corefs-core`.

## Why a separate crate?

1. **Independent toolchain** — builds with stable `cargo`, not the
   anyOS kernel target.  Excluded from the top-level workspace.
2. **Smaller dependency surface** — `libcorefs-tools` exposes a
   `host` feature that drops `anyos_std`, so the host build is
   pure Rust + `std`.
3. **Different I/O backend** — anyOS uses `SyscallBackend`, host
   uses the new `FileBackend` defined in [`src/backend.rs`](src/backend.rs).

## Build & test

```bash
./build.sh              # cargo +stable build --release
./build.sh test         # run unit + integration smoke tests
./build.sh run --help   # show CLI usage
```

The resulting binary lands at:

```
target/x86_64-unknown-linux-gnu/release/mkfs-corefs-host
```

## CLI

```text
mkfs-corefs-host --output <path>
                 [--offset <bytes>]   (default: 0)
                 [--size   <bytes>]   (default: file length - offset)
                 [--label  <name>]    (default: "corefs", max 16 bytes)
                 [--inodes <count>]
                 [--journal-blocks <n>]
                 [--json]
                 [--help]
```

Numeric values accept `k`/`K`, `m`/`M`, `g`/`G`, `t`/`T` suffixes
(powers of 1024) via the shared parser in
[`libs/libcorefs-tools/src/args.rs`](../../libs/libcorefs-tools/src/args.rs).

### Examples

Format a fresh 256 MiB image as a single CoreFS volume:

```bash
truncate -s 256M disk.img
mkfs-corefs-host --output disk.img --label "mydata"
```

Format a CoreFS partition inside a larger multi-partition image
(used by the anyOS image-build pipeline in Phase 3):

```bash
# disk.img has a FAT boot partition at bytes [0..128M) and a
# CoreFS system partition at bytes [128M..512M).
mkfs-corefs-host --output disk.img \
                 --offset 128M \
                 --size   384M \
                 --label  "system"
```

Machine-readable JSON output:

```bash
mkfs-corefs-host --output disk.img --size 64M --json
# → {"output":"disk.img","offset_bytes":0,"capacity_bytes":67108864,...}
```

## Architecture

```
┌──────────────────────────────────┐
│ mkfs-corefs-host (this crate)    │
│   ┌──────────┐   ┌──────────┐    │
│   │ main.rs  │   │ backend  │    │  FileBackend: std::fs::File
│   │ CLI +    │   │ .rs      │    │      + seek/read/write
│   │ report   │   └─────┬────┘    │      + Mutex for Send
│   └────┬─────┘         │         │
│        │       ┌───────▼──────┐  │
│        │       │ format.rs    │  │  wires FileBackend into
│        │       │ (format_     │  │  AnyOsBlockDevice and calls
│        └──────►│  volume)     │  │  OdfDeviceSession::format_new_at
│                └───────┬──────┘  │
└────────────────────────┼─────────┘
                         │
                         ▼
┌──────────────────────────────────┐
│ libs/libcorefs-tools (host mode) │
│   AnyOsBlockDevice<DiskBackend>  │  generic over backend
│   args / error / report          │
└────────────────┬─────────────────┘
                 │
                 ▼
┌──────────────────────────────────┐
│ corefs-core                      │
│   OdfDeviceSession::format_new_at│  writes superblock, bitmaps,
│   volume::inspect                │  inode table, journal, state
└──────────────────────────────────┘
```

## Tests

Two test scopes:

- **Unit tests** in [`src/backend.rs`](src/backend.rs) — exercise the
  `FileBackend` with tempfiles, verifying sector-offset translation,
  short-buffer rejection, and round-trip write/read.
- **Integration smoke tests** in [`tests/smoke.rs`](tests/smoke.rs) —
  format a fresh image, re-open it via the same adapter chain, and
  assert that `corefs_core::volume::inspect` reports a valid
  superblock.  Covers both a single-partition image and a partition
  at a non-zero offset (Dual-Partition-Layout dry-run).

Run with `./build.sh test` or `cargo +stable test --release`.
