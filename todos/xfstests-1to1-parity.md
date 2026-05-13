# xfstests 1:1 Parity Plan

Stand: 2026-05-13

Upstream-Snapshot:

- Repository: https://github.com/kdave/xfstests
- Referenz: `master`
- Commit: `57d71a884dd1b3b3c44a27d2d106b3be84ddc5fb`
- Commit-Datum: 2026-03-12
- Commit-Titel: `ltp: add support for FALLOC_FL_WRITE_ZEROES to fsx and fsstress`

Ziel:

- Fuer jeden nummerierten xfstests-Test muss es in anyOS einen expliziten
  Status geben: `native`, `adapted`, `covered`, `blocked`, `unsupported`,
  `not-applicable` oder `todo`.
- "1:1-Paritaet" bedeutet nicht, dass XFS/Btrfs-spezifische Features in
  CoreFS/exFAT erfunden werden. Es bedeutet, dass jeder upstream Test bewusst
  eingeordnet wird und entweder einen anyOS-Test, eine Feature-Anforderung oder
  einen begruendeten Skip besitzt.
- `vfsstress` bleibt der erste native Zielort fuer generische VFS-/FS-Tests.
  Fuer Mount/Recovery/Device-Fehler braucht es zusaetzlich einen Harness,
  z.B. `fsqastress` oder `tests/xfstests-anyos`.

## Upstream-Inventar

Aus dem GitHub-Tree des Snapshots ergeben sich 2.155 nummerierte Tests:

| Suite | Anzahl | Bereich | Parity-Ziel |
| --- | ---: | --- | --- |
| `generic` | 791 | `001..791` | P0/P1: groesstenteils portieren/adaptieren |
| `xfs` | 798 | `001..841` | P2/P3: XFS-Features auf CoreFS-Analoga mappen oder skippen |
| `btrfs` | 347 | `001..347` | P2/P3: Btrfs-Features mappen oder skippen |
| `overlay` | 106 | `001..117` | P3: erst mit Overlay/FUSE/Whiteout relevant |
| `ext4` | 71 | `001..308` | P1/P2: portable Regressionen adaptieren, ext4-only skippen |
| `f2fs` | 23 | `001..023` | P3: f2fs-only skippen, generische Ideen adaptieren |
| `ceph` | 6 | `001..006` | P3: Netzwerk-FS, aktuell not-applicable |
| `selftest` | 7 | `001..007` | P2: Harness-Selbsttests fuer anyOS nachbauen |
| `cifs` | 1 | `001..001` | P3: Netzwerk-FS, aktuell not-applicable |
| `nfs` | 1 | `001..001` | P3: Netzwerk-FS, aktuell not-applicable |
| `ocfs2` | 1 | `001..001` | P3: Cluster-FS, aktuell not-applicable |
| `perf` | 1 | `001..001` | P2: Performance-Harness adaptieren |
| `tmpfs` | 1 | `001..001` | P2: Ramfs/tmpfs-Aequivalent pruefen |
| `udf` | 1 | `102` | P3: UDF-only, aktuell not-applicable |

## Parity-Statusmodell

- `native`: Test ist als anyOS-Test implementiert und prueft dieselbe Semantik.
- `adapted`: Test ist angepasst, weil APIs/Tools anders sind, aber dieselbe
  Fehlerklasse wird abgedeckt.
- `covered`: Ein bestehender anyOS-Test deckt die relevante Semantik bereits ab.
- `blocked`: Feature/API/Harness fehlt, z.B. file-backed mmap, direct IO,
  Hardlinks, fault-injection.
- `unsupported`: Feature ist bewusst nicht Teil von CoreFS/exFAT.
- `not-applicable`: Test gehoert zu einem anderen FS/Protokoll, z.B. Ceph/NFS.
- `todo`: noch nicht eingeordnet oder implementiert.

Akzeptanz fuer echte 1:1-Paritaet:

- Es gibt eine maschinenlesbare Manifest-Datei mit genau einem Eintrag pro
  upstream Test.
- Kein Eintrag bleibt auf `todo`.
- Jeder `blocked` Eintrag referenziert eine konkrete Feature-/Harness-TODO.
- Jeder `unsupported`/`not-applicable` Eintrag enthaelt eine kurze Begruendung.
- Jeder `native`/`adapted`/`covered` Eintrag referenziert den anyOS-Testfall.

## [P0] Manifest und Upstream-Sync

TODO:

- Verzeichnis `tests/xfstests-parity/` anlegen.
- `manifest.csv` oder `manifest.json` generieren mit Spalten:
  - `suite`
  - `id`
  - `upstream_path`
  - `upstream_commit`
  - `groups`
  - `required_features`
  - `status`
  - `anyos_test`
  - `reason`
  - `notes`
- Importer bauen:
  - GitHub tree oder lokaler xfstests checkout als Input.
  - Nummerierte Tests aus `tests/*/[0-9][0-9][0-9]` erkennen.
  - `_begin_fstest` Gruppen extrahieren.
  - `_require_*` Requirements grob extrahieren.
  - Ergebnis deterministisch sortieren.
- `README.md` fuer die Parity-DB schreiben.

Akzeptanz:

- Manifest enthaelt 2.155 Tests fuer Snapshot `57d71a8`.
- Counts pro Suite entsprechen der Tabelle oben.
- Ein erneuter Import kann neue upstream Tests als `todo` markieren, ohne
  bestehende Bewertungen zu verlieren.

## [P0] anyOS xfstests Harness

Fehlt:

- xfstests arbeitet mit `TEST_DEV`, `TEST_DIR`, `SCRATCH_DEV`, `SCRATCH_MNT`,
  `_require_*`, `_scratch_mkfs`, `_scratch_mount`, `_check_test_fs`.
- `vfsstress` allein kann dieses Modell nicht 1:1 abbilden.

TODO:

- Harness-Design fuer anyOS:
  - Test- und Scratch-Device.
  - Scratch-Mountpoint.
  - Format-Backends fuer CoreFS und exFAT.
  - Run/Skip/Fail-Auswertung.
  - Artefakt-Verzeichnis fuer Logs.
  - Feature-Probes.
- CLI:
  - `fsqastress list`
  - `fsqastress run generic/001`
  - `fsqastress run --group quick`
  - `fsqastress run --suite generic --fs corefs`
  - `fsqastress run --suite generic --fs exfat`
  - `fsqastress report --json`
- Bestehendes `vfsstress` als Test-Backend einbinden, nicht duplizieren.

Akzeptanz:

- Harness kann Tests sauber `PASS`, `FAIL`, `SKIP`, `BROKEN` melden.
- Scratch-Modus kann Daten zerstoeren, aber nur nach expliziter Device-Angabe.
- Quick-Gruppe laeuft ohne manuelle Schritte.

## [P0] `generic` Suite komplett einordnen

Scope:

- 791 Tests: `generic/001..generic/791`.
- Hohe Prioritaet, weil `generic` die portable VFS-/Filesystem-Semantik
  enthaelt, die fuer CoreFS/exFAT am relevantesten ist.

Teilbereiche:

- Datenintegritaet: fsx, fsstress, copier, checksums, random IO.
- Metadaten: create, unlink, rename, chmod, chown, timestamps, stat/statx.
- Directories: readdir, grosse Directories, tiefe Directories, Namenslimits.
- Space-Management: ENOSPC, truncate, holes, preallocation, unwritten extents.
- Persistenz: fsync, sync, close, remount, recovery.
- Memory/IO APIs: mmap, direct IO, AIO, splice, writev, uring.
- Feature-APIs: xattr, ACLs, project IDs, immutable flags, reflink, clone,
  dedupe, quotas.
- Fault/Recovery: dm-error, log-writes, shutdown, freeze.

TODO:

- Alle `generic/*` Tests im Manifest priorisieren:
  - P0: portable Datenintegritaet, fsstress/fsx, ENOSPC, fsync/recovery.
  - P1: symlink/link, permissions, names, directories, statfs/stat metadata.
  - P2: mmap/direct IO/AIO/splice/writev, falls API vorhanden oder geplant.
  - P3: reflink/dedupe/quota/xattr/acl/immutable, je nach Feature-Roadmap.
- Fuer jeden P0/P1-Test einen nativen anyOS-Testfall benennen oder bauen.
- Fuer jedes fehlende Feature einen `blocked` Eintrag mit Feature-TODO anlegen.

Akzeptanz:

- Kein `generic/*` bleibt ohne Status.
- P0/P1 `generic` Tests sind entweder implementiert oder mit konkretem Blocker
  verknuepft.

## [P1] `ext4` Suite einordnen

Scope:

- 71 Tests: `ext4/001..ext4/308`.

TODO:

- Portable Regressionen aus ext4 adaptieren:
  - delayed allocation/writeback Korruptionen.
  - fsync/recovery Muster.
  - extent/hole/truncate Muster, soweit FS-neutral.
  - directory/index/name Regressionen, soweit FS-neutral.
- ext4-only Features als `unsupported` markieren:
  - ext4-spezifische ioctls.
  - journal internals.
  - inline data, DAX, orphan-list Details, soweit nicht in CoreFS vorhanden.

Akzeptanz:

- Jede ext4-Regression ist entweder auf CoreFS/exFAT-Semantik gemappt oder
  begruendet als ext4-only markiert.

## [P2] `xfs` Suite einordnen

Scope:

- 798 Tests: `xfs/001..xfs/841`.
- Viele Tests sind XFS-intern oder xfs_io/xfs_db/xfs_repair-spezifisch.

TODO:

- Portable Ideen extrahieren:
  - repair/fsck Workflows.
  - metadata scrub/repair Analoga.
  - quota/project-id Aehnlichkeiten, falls CoreFS sowas bekommt.
  - ENOSPC und allocation group Stress als CoreFS allocator stress.
  - reflink/dedupe nur, falls Feature geplant.
- XFS-only Tests markieren:
  - xfs_db, xfs_repair internals.
  - realtime volumes, AG internals, inode btrees.
  - XFS ioctls und geometry.
- CoreFS-spezifische Analoga definieren:
  - `corefs-scrub`.
  - `fsck.corefs`.
  - block allocator integrity.
  - snapshot/defrag/resize/tier Tools.

Akzeptanz:

- Jeder `xfs/*` Eintrag hat Status.
- XFS-only Skips sind nicht still, sondern begruendet.
- Portable XFS-Regressionsideen erscheinen als CoreFS-native Tests.

## [P2] `btrfs` Suite einordnen

Scope:

- 347 Tests: `btrfs/001..btrfs/347`.

TODO:

- Portable Ideen extrahieren:
  - checksummed data integrity.
  - send/receive nur als not-applicable, falls kein Aequivalent.
  - snapshot semantics nur, falls CoreFS snapshots relevant werden.
  - subvolume semantics als not-applicable, solange keine Subvolumes.
  - compression/encryption nur, falls CoreFS Features aktiv.
  - scrub/device error behavior auf CoreFS-scrub mappen.
- Btrfs-only Features markieren:
  - subvolumes.
  - qgroups.
  - send/receive.
  - balance, raid profiles, zoned specifics.

Akzeptanz:

- Jede Btrfs-Semantik ist entweder gemappt, blocked oder not-applicable.

## [P3] `overlay` Suite einordnen

Scope:

- 106 Tests: `overlay/001..overlay/117`.

TODO:

- Nur aktivieren, wenn anyOS Overlay/FUSE/Whiteout-Semantik bekommt.
- Bis dahin:
  - Whiteout/redirect/metacopy/index Features als `not-applicable`.
  - Portable rename/unlink/readdir Deadlock-Ideen in `generic`/`vfsstress`
    uebernehmen.

Akzeptanz:

- Overlay-Suite bleibt im Manifest sichtbar, aber blockiert nicht
  CoreFS/exFAT-Basisfreigabe.

## [P3] Weitere FS-Suites einordnen

### `f2fs`

- 23 Tests.
- f2fs-only Features als `not-applicable`.
- Portable ENOSPC/recovery/checkpoint Ideen adaptieren, wenn sinnvoll.

### `ceph`, `cifs`, `nfs`

- Netzwerk-FS-Suites.
- Aktuell `not-applicable`, bis anyOS entsprechende Clients produktiv testet.
- Portable Client-Cache/rename/open-unlink Ideen koennen spaeter adaptiert
  werden.

### `ocfs2`

- Cluster-FS, aktuell `not-applicable`.

### `tmpfs`

- Auf `ramfs`/tmpfs-Aequivalent mappen, falls vorhanden.

### `udf`

- UDF-only, aktuell `not-applicable`.

### `perf`

- Performance-Harness-Idee fuer CoreFS/exFAT uebernehmen.

### `selftest`

- Harness-Selbsttests fuer anyOS nachbauen:
  - skip handling.
  - fail handling.
  - group filtering.
  - scratch-device protection.

## [P0] xfstests-Tooling-Paritaet

Upstream `src/` enthaelt Hilfsprogramme, die viele Tests antreiben. Relevante
Tool-Familien fuer anyOS:

- `fsx`: Random read/write/truncate/fallocate/mmap/direct-io.
- `fsstress`: paralleler Metadaten- und Datenstress.
- `fill`, `fill2fs`, `enospc_unlink`, `t_enospc`: ENOSPC.
- `fsync-tester`, `fsync-err`, `unlink-fsync`: Persistenz.
- `dirstress`, `dirperf`, `nametest`, `readdir-while-renames`: Directories.
- `holes`, `holetest`, `seek_sanity_test`, `t_holes`: Sparse/Holes.
- `mmap-*`, `t_mmap_*`: file-backed mmap.
- `dio-*`, `min_dio_alignment`: direct IO.
- `aio-*`, `uring_*`: async IO.
- `attr*`, `listxattr`, `fs_perms`, `t_immutable`: attributes/permissions.
- `rename`, `renameat2`, `multi_open_unlink`: rename/open lifetime.
- `dmerror`, `log-writes`, `fault`: Fehler-Injektion und Recovery.
- `scaleread`, `metaperf`, `dirperf`: Performance.

TODO:

- Native anyOS-Port oder Ersatz fuer jede Tool-Familie definieren.
- Prioritaet:
  - P0: `fsx`, `fsstress`, ENOSPC, fsync, directory/name, fault basics.
  - P1: permissions, symlink/link, open-unlink/rename, holes.
  - P2: mmap, direct IO, async IO, performance.
  - P3: xattr, reflink/dedupe, quotas, FS-specific tools.

Akzeptanz:

- Kein upstream Tool bleibt "unbemerkt"; jede Tool-Familie hat Status.
- Tests duerfen mehrere upstream Tools durch einen anyOS-nativen Test ersetzen,
  solange das Manifest `covered` korrekt referenziert.

## [P0] Feature-Gates / `_require_*` Paritaet

TODO:

- anyOS-Feature-Probes definieren:
  - `require_symlink`
  - `require_hardlink`
  - `require_chmod`
  - `require_chown`
  - `require_xattr`
  - `require_acl`
  - `require_mmap_file`
  - `require_direct_io`
  - `require_async_io`
  - `require_fallocate`
  - `require_seek_data_hole`
  - `require_sparse`
  - `require_quota`
  - `require_reflink`
  - `require_dedupe`
  - `require_freeze`
  - `require_fault_injection`
  - `require_scratch`
  - `require_remount`
  - `require_fsck`
- Feature-Probes muessen `SKIP` liefern, nicht `FAIL`, wenn eine optionale
  Semantik nicht existiert.

Akzeptanz:

- Alle upstream `_require_*`-Muster koennen mindestens auf ein anyOS Feature
  Gate, `unsupported` oder `not-applicable` gemappt werden.

## [P0] Gruppen-Paritaet

TODO:

- `_begin_fstest` Gruppen aus upstream Tests extrahieren und in Manifest
  speichern.
- anyOS-Gruppen definieren:
  - `quick`
  - `auto`
  - `stress`
  - `soak`
  - `rw`
  - `metadata`
  - `enospc`
  - `recovery`
  - `mmap`
  - `directio`
  - `aio`
  - `perf`
  - `dangerous`
  - `fs-specific`
  - `unsupported`
- Gruppenselektion im Harness implementieren.

Akzeptanz:

- `fsqastress run --group quick` entspricht dem xfstests-Konzept.
- Dangerous/Scratch Tests laufen nie ohne explizite Freigabe.

## [P0] Umsetzungspfad

1. Manifest-Importer bauen und 2.155 Tests erfassen.
2. Feature-Gates und Statusmodell implementieren.
3. `generic` Suite automatisch einordnen: Startstatus `todo`, bekannte
   CoreFS/exFAT-unrelevante Features als `blocked`/`unsupported`.
4. `vfsstress` um P0/P1 `generic` Kern erweitern:
   - parallel fsstress.
   - ENOSPC.
   - symlink/hardlink status.
   - open-unlink/rename.
   - readdir mutation.
   - long names/large dirs.
   - fsync ordering.
5. Scratch-Harness fuer CoreFS/exFAT bauen.
6. Recovery/Fault-Injection bauen.
7. `ext4`, `xfs`, `btrfs` portable Regressionen mappen.
8. Netzwerk/Cluster/Overlay-Suites als `not-applicable` oder `blocked`
   klassifizieren.
9. Performance/Soak/CI-JSON komplettieren.
10. Upstream-Sync als wiederholbaren Prozess dokumentieren.

## Minimaler Parity-Meilenstein

M1:

- Manifest vollstaendig.
- Alle 791 `generic` Tests mit Status.
- P0/P1 `generic` Tests nativ/adaptiert oder mit konkretem Blocker.

M2:

- Alle 2.155 Tests mit Status.
- `xfs`, `btrfs`, `ext4`, `overlay` portable Regressionen extrahiert.
- Alle not-applicable/unsupported Begruendungen dokumentiert.

M3:

- Quick-Gruppe fuer CoreFS und exFAT in anyOS CI.
- Heavy/Soak-Gruppe fuer Pre-Release.
- JSON-Reports und Performance-Baselines.

## Implementierungsstand

2026-05-13:

- Manifest-Importer umgesetzt:
  - `tools/xfstests_parity_import.py`
  - `tests/xfstests-parity/manifest.csv`
  - `tests/xfstests-parity/summary.json`
- Manifest-Validator umgesetzt:
  - `tools/xfstests_parity_check.py`
- Erste P0-`vfsstress`-Tests umgesetzt:
  - `parallel_fsstress_case` als fsstress-artiger Multi-Prozess-Workload.
  - `enospc_accounting_case` als bounded ENOSPC/`statfs`-Accounting-Probe.
  - `scratch_lifecycle_case` fuer CoreFS/exFAT mit mkfs, mount, Workload,
    unmount, fsck, remount und Readback-Verify.
- Fehlende anyOS-API fuer exFAT-Scratch umgesetzt:
  - `/System/sbin/mkfs.exfat`
  - `mkfs_exfat` als no_std Userland-Formatter.
  - `/System/sbin/fsck.exfat`
  - `fsck_exfat` als read-only Strukturchecker fuer exFAT.
  - CMake-Integration fuer das Sysroot.

Noch offen fuer P0:

- Kleines deterministisches Scratch-Testdevice fuer harte ENOSPC-Faelle.
- Recovery/EIO/Fault-Injection API und Harness.
- Manifest-Status fuer betroffene upstream Tests auf `adapted`/`covered` setzen.
