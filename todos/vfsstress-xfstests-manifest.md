# vfsstress xfstests Manifest

Stand: 2026-05-13

Dieses Manifest mappt upstream-`xfstests`-Familien auf anyOS-native
`vfsstress`-Abdeckung. Statuswerte:

- `covered`: nativ in `vfsstress` umgesetzt.
- `adapted`: an anyOS/VFS/CoreFS/exFAT-Semantik angepasst umgesetzt.
- `skip-unsupported`: optionales Feature wird im Testlauf als SKIP gemeldet.
- `blocked-api`: benoetigt erst neue anyOS-API oder Testdevice-Infrastruktur.

| xfstests-Familie | anyOS/vfsstress-Abdeckung | Status | Notiz |
| --- | --- | --- | --- |
| `generic/013`, `generic/476`, `fsstress` | `parallel_fsstress_case`, Worker, Shared-Hot-Dir, Manifest-Verify | adapted | Prozessparallelitaet statt Linux-fsstress-Tooling |
| `generic/083`, `generic/269`, `fill`, `fill2fs` | `enospc_accounting_case`, `enospc_directory_growth_case` | adapted | Deterministisch via `--enospc-kb`, echtes ENOSPC nur mit kleinem Scratch-Device |
| Scratch `TEST_DEV`/`SCRATCH_DEV` Modell | `scratch_lifecycle_case`, mkfs, mount, unmount, fsck, remount | adapted | Destruktiv nur bei explizitem `--scratch-device` |
| Crash/log-writes/dm-error | `recovery_injection_case` | blocked-api | Device-Fehlerinjektion und Crash-Cut-Harness fehlen |
| Symlink/ELOOP/readlink | `symlink_eloop_case` | covered | Normale Links, Ketten, Dangling, Loops, tiefe Ketten |
| Hardlinks/link count | `feature_gate_case` | skip-unsupported | Keine anyOS-Hardlink-API im `fs`-Modul |
| chmod/chown/stat metadata | `permission_metadata_case`, Scratch-Remount-Verify | adapted | Enforcement-Fehlerpfade bleiben abhaengig von Permission-Modell |
| open-unlink/rename lifetime | `open_unlink_rename_case` | covered | FD nach unlink/rename, overwrite, cross-dir und Fehlerpfade |
| readdir while mutating | `readdir_while_mutating_case`, `readdir_parallel_mutation_case` | adapted | Snapshot-Readdir; FD-Cursor-Semantik bleibt blocked bis API existiert |
| dirstress/dirperf/metaperf | `large_directory_case`, `metadata_perf_case` | adapted | Profilabhaengige Groessen fuer quick/normal/heavy/soak |
| nametest/create-long-dirs | `long_name_case`, `path_resolution_case` | adapted | Long-Name-ABI, UTF-8-Namen, Case-Matrix, Path-Budget |
| sparse/hole/seek | `sparse_eof_gap_case`, `sparse_hole_matrix_case` | adapted | SEEK_DATA/SEEK_HOLE und partial truncate blocked |
| fsync/writeback ordering | `fsync_ordering_case`, `writeback_stream_case`, Scratch rename verify | adapted | Crash-Persistenz bleibt blocked bis Error-Injection existiert |
| statfs/accounting | `statfs_accounting_case`, Perf-JSON FS-Felder | covered | Quota/per-user Accounting nicht vorhanden |
| mmap/fsx-mmap | `feature_gate_case` | skip-unsupported | Keine file-backed mmap API |
| O_DIRECT/AIO | `feature_gate_case` | skip-unsupported | Keine Direct-IO/AIO API |
| xattrs/attrs | `feature_gate_case` | skip-unsupported | Keine xattr API |
| special files/mknod | `feature_gate_case` | skip-unsupported | Keine normale-FS-mknod Semantik |
| overlay/whiteout/FUSE/namespace/idmapped | `feature_gate_case` | skip-unsupported | Nicht Teil der CoreFS/exFAT-Basisfreigabe |

Pflege-Regel: neue upstream-Familien werden hier zuerst als `blocked-api` oder
`skip-unsupported` eingetragen und erst dann in `vfsstress` aufgenommen, wenn
die anyOS-Semantik klar ist.
