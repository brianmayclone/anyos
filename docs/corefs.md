# CoreFS in AnyOS

**CoreFS** ist das native Dateisystem von AnyOS. Es wird als externes Rust-Crate (`corefs-core`) eingebunden, im Kernel als VFS-Treiber registriert und im User-Space durch eine Sammlung von Verwaltungstools ergänzt.

Quelle des Dateisystems: [../../corefs/](../../corefs/) — eigenständiges Workspace-Projekt. Die hier beschriebene Integration ist die AnyOS-Seite. Die umgekehrte Sicht steht in [../../corefs/doc/anyos-integration.md](../../corefs/doc/anyos-integration.md).

## Komponenten im Überblick

| Komponente | Typ | Quelle | Binär |
|---|---|---|---|
| **CoreFS-Treiber** | Kernel-Modul | `kernel/src/fs/corefs/` | Teil des Kernels |
| **mkfs.corefs** | CLI-Tool | `bin/mkfs.corefs/` | `/System/sbin/mkfs.corefs` |
| **fsck.corefs** | CLI-Tool | `bin/fsck.corefs/` | `/System/sbin/fsck.corefs` |
| **corefs-dump** | CLI-Tool | `bin/corefs-dump/` | `/System/sbin/corefs-dump` |
| **corefs-tier** | CLI-Tool | `bin/corefs-tier/` | `/System/sbin/corefs-tier` |
| **corefs-snapshot** | CLI-Tool | `bin/corefs-snapshot/` | `/System/sbin/corefs-snapshot` |
| **corefs-resize** | CLI-Tool | `bin/corefs-resize/` | `/System/sbin/corefs-resize` |
| **corefs-defrag** | CLI-Tool | `bin/corefs-defrag/` | `/System/sbin/corefs-defrag` |
| **corefs-scrub** | CLI-Tool | `bin/corefs-scrub/` | `/System/sbin/corefs-scrub` |
| **libcorefs-tools** | Shared Lib | `libs/libcorefs-tools/` | statisch gelinkt |

## Einbindung in den Kernel

### Crate-Dependency

```toml
# kernel/Cargo.toml
corefs-core = { path = "../../corefs/corefs-core", default-features = false }
```

`default-features = false` schaltet das `crypto`-Feature ab (Poly1305-SIMD ist mit dem Soft-Float-Target `x86_64-anyos` inkompatibel). Der Kernel arbeitet strikt `no_std + alloc`.

### Treiberstruktur

[`kernel/src/fs/corefs/`](../kernel/src/fs/corefs/) besteht aus vier Modulen:

- [`mod.rs`](../kernel/src/fs/corefs/mod.rs) — Plattform-Traits für CoreFS: `KernelClock` (Zeit), `KernelRng` (Zufall), `BlockDeviceAdapter` (Byte→Sektor-Mapping). Enthält außerdem `try_auto_mount_corefs()` für das Boot-Auto-Mount.
- [`probe.rs`](../kernel/src/fs/corefs/probe.rs) — Superblock-Erkennung. Liest 8 Sektoren ab der Partitions-LBA und prüft auf `ODF_MAGIC` (`"COREFSDF"`).
- [`block_device.rs`](../kernel/src/fs/corefs/block_device.rs) — Implementiert `corefs_core::storage::block_device::BlockDevice` auf Basis von `crate::drivers::storage` (ATA/AHCI/NVMe). Sektorgröße 512 B, Partition-Offset, TRIM aktuell No-Op.
- [`driver.rs`](../kernel/src/fs/corefs/driver.rs) — VFS-Treiber. Hydriert beim Mount einen `PersistedState` (`load_state_native`), bearbeitet Lookup/Read/Write/Readdir/Create/Delete in-memory und persistiert per `flush()` über `save_state_native`. Fehler werden über `corefs_to_fs_error()` auf `FsError` abgebildet.

### VFS-Registrierung und Boot

- VFS-Mount-Pfad: [`kernel/src/fs/vfs/mod.rs:344`](../kernel/src/fs/vfs/mod.rs#L344) — `mount_corefs()` baut den `BlockDeviceAdapter`, erzeugt einen `CoreFsDriver` (aktuell read-only) und registriert ihn im Mount-Table unter `FsType::CoreFs`.
- Boot-Auto-Mount: [`kernel/src/boot/x86/storage.rs:115`](../kernel/src/boot/x86/storage.rs#L115) — nach MBR/GPT-Partitions-Enumeration wird für jede Partition `try_auto_mount_corefs()` aufgerufen. Erkannte Volumes landen unter `/mnt/corefs` bzw. `/mnt/corefs{n}`.

## Userland-Tools

Alle Tools binden `corefs-core` **mit** `crypto`-Feature ein (User-Target `x86_64-anyos-user` unterstützt SIMD) und nutzen gemeinsame Infrastruktur aus `libcorefs-tools`.

| Tool | Zweck | Nutzt aus `corefs-core` |
|---|---|---|
| `mkfs.corefs` | Volume formatieren | `storage::ondisk::volume::FormatOptions` |
| `fsck.corefs` | Strukturprüfung / Reparatur | `storage::ondisk::fsck::{fsck, fsck_repair}` |
| `corefs-dump` | Read-only Inspektion | `storage::ondisk::reader::OdfReader` |
| `corefs-tier` | Tiering-Status (Hot/Cold) | `storage::ondisk::tier::tier_status_from_state` |
| `corefs-snapshot` | Snapshots verwalten | `domain::snapshot`, Restore-Helfer |
| `corefs-resize` | Volume vergrößern | `storage::ondisk::resize::grow_device` |
| `corefs-defrag` | Blockstore kompaktieren | `storage::block_store::BlockStore::defragment` |
| `corefs-scrub` | Scrub-Lauf | `storage::ondisk::scrub::{ScrubPlan, ScrubReport}` |

### `libcorefs-tools`

Die Bibliothek unter [`libs/libcorefs-tools/`](../libs/libcorefs-tools/) liefert:

- **`block_device`** — `AnyOsBlockDevice<B: DiskBackend>` als Adapter zwischen `anyos_std::sys::disk_read/disk_write` und dem CoreFS-`BlockDevice`-Trait. Inklusive `MockBackend` für Unit-Tests.
- **`args`** — allokations-bewusster GNU-Style-Parser (`--device 0`, `--json`).
- **`report`** — Text/JSON-Dual-Output via `Report`-Trait und eigenem `JsonBuilder` (kein `serde_json` auf `x86_64-anyos-user`).
- **`error`** — Abbildung `CoreFsError` → stabile Exit-Codes (`NotFound=44`, `Corruption=74`, `InvalidArgument=22`, `IoError=5`, `Generic=1`).

## Build-Integration

Der Build erfolgt aus dem AnyOS-Workspace — alle User-Binaries werden in einem Schritt kompiliert.

CMake-Definition: [`cmake/UserPrograms.cmake`](../cmake/UserPrograms.cmake) (ab Zeile ~950). `mkfs.corefs` und `fsck.corefs` nutzen eigene Custom-Commands, um den Punkt im Dateinamen zu erhalten; die übrigen `corefs-*`-Tools werden über das Standard-`add_rust_sbin_program`-Makro eingebunden. ELF-Binaries werden per `anyelf bin` in Flat-Binaries umgesetzt und nach `/System/sbin/` installiert. Abhängigkeit: `WORKSPACE_STAMP`.

## Verwendung

### Volume anlegen

```sh
mkfs.corefs --device 0 --capacity 16777216
```

### Zustand prüfen

```sh
fsck.corefs --device 0
corefs-dump --device 0 --json
```

### Snapshot erstellen

```sh
corefs-snapshot --device 0 create --label "vor-update"
corefs-snapshot --device 0 list --json
```

### Wartung

```sh
corefs-scrub   --device 0 --mode structural
corefs-defrag  --device 0
corefs-resize  --device 0 --capacity 33554432
corefs-tier    --device 0 --json
```

### Mount

Mounts werden aktuell beim Boot automatisch durchgeführt (read-only) unter `/mnt/corefs`. Ein expliziter Mount-CLI-Befehl ist noch nicht vorhanden.

## Aktuelle Einschränkungen

- Mount im Kernel derzeit **read-only** (Write-Pfad in `driver.rs` vorhanden, noch nicht im VFS freigeschaltet).
- **Verschlüsselung** im Kernel deaktiviert (SIMD-Inkompatibilität); Userland-Tools unterstützen sie.
- `corefs-snapshot` restore ist metadaten-orientiert; vollständige Datei-Body-Snapshots benötigen den `std`-gebundenen `CoreFsService`.
- `corefs-resize` unterstützt nur Wachsen — Schrumpfen erfordert Datenmigration und ist bewusst nicht implementiert.
- TRIM/Discard wird vom `BlockDeviceAdapter` noch nicht an den Storage-Stack weitergereicht.

## Weiterführend

- [../../corefs/doc/overview.md](../../corefs/doc/overview.md) — CoreFS-Projektübersicht
- [../../corefs/doc/architecture.md](../../corefs/doc/architecture.md) — Schichtenmodell
- [../../corefs/doc/persistence-format.md](../../corefs/doc/persistence-format.md) — On-Disk-Format
- [../../corefs/doc/anyos-integration.md](../../corefs/doc/anyos-integration.md) — Integrationsdoku aus Sicht von CoreFS
