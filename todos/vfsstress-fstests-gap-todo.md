# vfsstress / fstests Gap TODO

Stand: 2026-05-13

Kontext:

- `vfsstress` ist der anyOS-Userland-Stresstest fuer VFS, CoreFS und exFAT.
- Die aktuelle Suite deckt bereits sequentielle Datenintegritaet, Overwrites,
  Append/Truncate, fsx-artige Random-IO, Sparse-Gaps, Metadaten-Churn,
  Readdir, FD-Offsets, Usercopy-Grenzen und Writeback-Readback ab.
- Im Vergleich zu `xfstests` fehlen vor allem parallele Workloads,
  ENOSPC-Szenarien, Scratch-Filesystem-Lebenszyklen, Recovery/Error-Injection,
  Symlink/Permission/Metadata-Semantik und reproduzierbare Performance-Messung.
- Ziel: ein kleiner, anyOS-nativer fstests-Kern, der CoreFS und exFAT stabiler
  und messbar performanter macht.

## Prio-Legende

- P0: Kritisch fuer Datenintegritaet oder realistische CoreFS/exFAT-Freigabe.
- P1: Hoher Bug-Find-Wert; sollte vor breiter Nutzung dauerhaft laufen.
- P2: Wichtige Semantik-/Performance-Abdeckung, aber weniger akut.
- P3: Langfristige/erweiterte fstests-Paritaet oder Spezialfaelle.

## Aktuelle Abdeckung

- Schreib-/Readback-Matrix mit 4 KiB, 16 KiB, 64 KiB und 256 KiB Blocks.
- Kleine IOs mit ungeraden Laengen.
- Overwrites innerhalb eines Blocks, ueber Blockgrenzen und ueber mehrere
  Blocks.
- Seek-Overwrite plus Verify gegen deterministisches Pattern.
- Append und Truncate auf 0.
- fsx-artiges Random-Modell mit Write/Append/Read/Truncate.
- Sparse/EOF-Erweiterung mit Null-Gap-Verifikation.
- Metadaten-Churn mit Create/Rename/Unlink/Readdir/Stat.
- Close/Reopen/Sync-Readback.
- Separate FD-Offsets.
- Usercopy-Grenzen und misaligned 32K/64K Userbuffer.
- Close-ohne-fsync Writeback-Readback vor und nach globalem `sync`.

## 10-Kategorien-Abgleich gegen xfstests

Diese Matrix ist die Kontrollliste aus dem erneuten Vergleich mit `xfstests`.
Jede Kategorie hat mehrere konkrete Testpunkte in den Detailabschnitten unten.

| Kategorie | Prio | Status in dieser TODO | xfstests-Bezug |
| --- | --- | --- | --- |
| 1. Echte Parallelitaet / fsstress | P0 | `parallel_fsstress_case`, Worker, Hot-Directories, Manifest-Verify | `generic/013`, `generic/476`, `fsstress` |
| 2. ENOSPC / volles FS | P0 | `enospc_accounting_case`, Full-Write, Delete/Reclaim, `statfs` | `generic/083`, `generic/269`, `fill`, `fill2fs` |
| 3. Scratch-FS Lifecycle | P0 | `--scratch`, mkfs, mount, unmount, remount, verify, fsck/scrub | xfstests `TEST_DEV`/`SCRATCH_DEV` Modell |
| 4. Crash/Recovery/EIO | P0 | `recovery_loop_case`, Device-Fail, Flush-Fail, fsck/scrub | `generic/475`, log-writes/dm-error Tests |
| 5. Symlinks/Links/ELOOP | P1 | `symlink_eloop_case`, Readlink, Loops, Hardlink-API-Check | `generic/005`, fsstress `link` Operation |
| 6. Permissions/Ownership/Metadata | P1 | `permission_metadata_case`, chmod/chown/stat/sync/remount | `generic/313`, `generic/317`, `generic/355` |
| 7. Open-Unlink/Rename/FD Lifetime | P1 | `open_unlink_rename_case`, overwrite rename, FD after unlink | `multi_open_unlink`, `rename`, rename regression tests |
| 8. Readdir unter Mutation | P1 | `readdir_while_mutating_case`, Sentinel-Snapshots, Cursor-Safety | `generic/310`, `readdir-while-renames` |
| 9. Grosse Directories / lange Namen | P1 | `large_directory_case`, `long_name_case`, case/LFN Matrix | `dirstress`, `dirperf`, `nametest`, create-long-dirs |
| 10. Performance-Baselines | P2 | `--perf`/`--json`, ops/s, KB/s, sync/fsync latency | `dirperf`, `metaperf`, `scaleread`, perf reports |

## [P0] Echte Parallelitaet wie fsstress

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `parallel_fsstress_case`
  - versteckter Worker-Modus `--worker`
  - CLI-Optionen `--workers`, `--ops`, `--seed`
  - parallele Prozesse via `process::spawn`/`waitpid`
  - per-Worker Manifest-Datei `done.txt`
  - gemeinsame Shared-Hot-Directory-Namen mit Cross-Worker
    Create/Rename/Unlink-Kollisionen
  - Shared-Directory-Scan nach Worker-Ende mit Stat/Read-Sanity-Check
  - Worker-Zwischenwrites verwenden relaxed close/write; strikte fsync-Semantik
    wird separat in `fsync_ordering_case` und `sync_latency_perf_case` getestet

Fehlt:

- Optional echte Thread-Variante, falls wir In-Process VFS-Races testen wollen.

TODO:

- `parallel_fsstress_case` in `vfsstress` ergaenzen.
- CLI-Optionen:
  - `--workers N` fuer Worker-Anzahl.
  - `--ops N` fuer Operationen pro Worker.
  - `--seed N` fuer reproduzierbare Fehler.
- Jeder Worker schreibt deterministische Inhalte und legt ein kleines Manifest
  pro Worker ab.
- Nach Worker-Ende alle uebrigen Dateien per Manifest oder Readdir scannen und
  Inhalt/Stat pruefen.
- Bei Fehlern Seed, Worker, Operation, Pfad und erwartete/gelesene Werte
  ausgeben.

Akzeptanz:

- `vfsstress --profile quick --workers 2` laeuft stabil.
- `vfsstress --profile heavy --workers 8` laeuft ohne Korruption auf exFAT.
- Derselbe Workload laeuft auf CoreFS.
- Fehler sind deterministisch reproduzierbar mit `--seed`.

## [P0] ENOSPC und Space Accounting

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `enospc_accounting_case`
  - `enospc_directory_growth_case`
  - CLI-Option `--enospc-kb`
  - bounded Fill-Workload
  - `statfs`-Vergleich vor Fill, nach Fill, nach Delete
  - `WARN`, wenn innerhalb des Limits kein echtes ENOSPC erreicht wird
  - Delete/Reclaim/Refill-Pfad mit Inhaltsverifikation nach erneutem Schreiben
  - Directory-Wachstum mit vielen kleinen Dateien, Rename/Unlink/Reclaim und
    Refill-Pfad nahe dem gesetzten ENOSPC-Limit

Fehlt:

- Erzwingbarer kleiner Scratch-Datentraeger, damit ENOSPC deterministisch
  erreicht wird.

TODO:

- `enospc_accounting_case` ergaenzen.
- Testverzeichnis kontrolliert bis nahe voll schreiben.
- Danach:
  - grosse Datei bis Write-Fehler,
  - viele kleine Dateien bis Create/Write-Fehler,
  - Rename/Unlink unter Speicherknappheit,
  - `sync`,
  - Teil loeschen,
  - erneut schreiben.
- `statfs` in Summary ausgeben: total, free vorher, free voll, free nach
  Delete, free nach Refill.
- Optional `--enospc-max-kb N`, damit der Test in normalen Images begrenzt
  bleibt.

Akzeptanz:

- Kein Panic/Freeze bei vollem exFAT/CoreFS.
- Fehler kommen sauber zurueck, keine stillen Short-Writes als Erfolg.
- Nach `unlink` kann wieder mindestens die geloeschte Datenmenge geschrieben
  werden.
- `fs::sync()` nach ENOSPC bleibt stabil.

## [P0] Scratch-Filesystem Lifecycle

Status:

- Teilweise umgesetzt in `vfsstress`:
  - CLI-Optionen `--scratch-device`, `--scratch-fs`, `--scratch-mount`
  - `scratch_lifecycle_case`
  - `mkfs.corefs`/`mkfs.exfat`
  - mount, Sentinel-Write/Verify, paralleler Mini-Workload, sync, unmount,
    fsck, remount, Readback-Verify
  - Metadata-Persistenz fuer chmod/chown, falls vom FS unterstuetzt
  - Rename-Persistenz ueber unmount/fsck/remount mit Old-Path-Check
- Neu umgesetzt als anyOS-API/Tool:
  - `/System/sbin/mkfs.exfat`
  - no_std exFAT-Formatter fuer leere Scratch-Volumes
  - automatische Groessenermittlung via `sys::disk_list`
  - Boot-Region, FAT, Allocation Bitmap, Upcase Table und Root Directory
  - `/System/sbin/fsck.exfat`
  - read-only exFAT-Strukturcheck fuer Boot-Region, FAT-Ketten, Root Directory
    und Allocation Bitmap
  - optionaler `/System/sbin/corefs-scrub --mode read-only` nach CoreFS
    Remount-Verify

Fehlt:

- Deterministisch kleines Testdevice fuer ENOSPC/Recovery.
- Separater Harness mit `TEST_DEV`/`SCRATCH_DEV` Sicherheitsmodell.
- Manifest-Mapping fuer alle Scratch-basierten upstream Tests.

TODO:

- Schutzabfrage/Harness fuer destruktive Scratch-Devices zentralisieren.
- exFAT-Formatter und `fsck.exfat` bei groesseren Volumes gegen externe Tools
  validieren.
- Separate Quick/Normal/Heavy Scratch-Profile definieren.

Akzeptanz:

- Scratch-Modus veraendert nie unbeabsichtigt das System-FS.
- Remount-Verify findet verlorene Daten/Metadaten.
- CoreFS-Check-Tool kann optional in denselben Report aufgenommen werden.

## [P0] Crash-/Recovery-/EIO-Simulation

Status:

- Explizit als blocked/unsupported in `vfsstress`:
  - `recovery_injection_case` reportet `SKIP blocked-api`
  - `feature_gate_case` berichtet `error-injection=unsupported`

Fehlt:

- Harte Unterbrechung waehrend Write/Metadata-Workload.
- Simulierter Device-Fehler oder gezielter Write-Fail.
- Recovery-Check nach Remount.

TODO:

- Kernel/Test-Device-Schicht fuer Fehler-Injektion entwerfen:
  - Write nach N Sektoren fehlschlagen lassen.
  - Read nach N Sektoren fehlschlagen lassen.
  - Flush/Fsync fehlschlagen lassen.
  - Device "weg" simulieren.
- `recovery_loop_case` bauen:
  - Workload starten.
  - Fehler injizieren.
  - Prozess/FS sauber abbrechen lassen.
  - remount/reopen.
  - fsck/scrub/readback.
- Fuer CoreFS Journaling/Atomicitaet separat dokumentieren:
  - Was darf nach Crash garantiert sein?
  - Was darf verloren gehen?
  - Was darf nie passieren?

Akzeptanz:

- Kein dauerhaft unmountbares CoreFS nach injiziertem Fehler.
- fsck/scrub meldet reproduzierbare, erklaerbare Ergebnisse.
- exFAT bleibt mindestens readbar oder liefert saubere Fehler.

## [P1] Symlinks, Hardlinks, Readlink und ELOOP

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `symlink_eloop_case`
  - normaler Symlink auf Datei mit Read-through-Verify
  - Symlink-auf-Symlink-Kette
  - dangling Symlink mit `readlink`-Verify und fehlgeschlagenem Open
  - Self-Loop und Zwei-Element-Zyklus muessen sauber fehlschlagen
  - langer Link-Target-String mit exaktem `readlink`
  - tiefe endliche Symlink-Kette und sehr tiefe Kette mit sauberem
    Readback-oder-Fehler-Verhalten
  - `lstat`-Flag fuer Symlink
  - WARN/SKIP, falls das Filesystem keine Symlinks unterstuetzt

Fehlt:

- Hardlink-/Linkcount-Semantik, falls anyOS eine `link`-API bekommt.

TODO:

- `symlink_eloop_case` ergaenzen.
- Testfaelle:
  - normaler Symlink auf Datei.
  - Symlink auf Symlink-Kette.
  - Self-Loop.
  - Zwei-Element-Zyklus.
  - dangling Symlink.
  - langer Link-Target-String.
  - tiefe Symlink-Ketten. [umgesetzt]
- `readlink` gegen erwartete Targets pruefen.
- Hardlink-Status klaeren:
  - aktuell gibt es in `anyos_std::fs` Symlink/Readlink, aber keine erkennbare
    Hardlink-API.
  - falls Hardlinks geplant sind: `link_count_case` und
    `hardlink_unlink_lifetime_case` ergaenzen.
  - falls nicht geplant: `SKIP hardlink unsupported` reporten.

Akzeptanz:

- Normale Symlinks lesen korrekte Inhalte.
- `readlink` liefert exakt das Target.
- Loops fuehren zu sauberem Fehler, nicht zu Stackoverflow/Hang.
- Hardlinks sind entweder getestet oder explizit als unsupported dokumentiert.

## [P1] Open-Unlink, Rename-Overwrite und FD-Lifetime

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `open_unlink_rename_case`
  - offener FD bleibt nach `unlink` lesbar, waehrend der Pfad verschwindet
  - derselbe Pfad kann nach `unlink` neu angelegt und separat verifiziert werden
  - offener FD bleibt nach `rename` lesbar, neuer Pfad enthaelt denselben Inhalt
  - Rename ueber existierende Datei ersetzt den Zielinhalt
  - Rename auf sich selbst bleibt gueltig und erhaelt Inhalt
  - Rename ueber Directory-Grenzen
  - Fehlerfaelle fuer fehlende Quelle und Ziel in fehlendem Directory

Fehlt:

- Weiteres Schreiben ueber alten FD nach Unlink/Rename, sobald eine passende
  Read/Write-Open-Flag-Kombination oder FD-dual-mode Semantik festgelegt ist.
- Directory-FD/fsync-Reihenfolgen fuer Rename-Persistenz.

TODO:

- `open_unlink_rename_case` ergaenzen.
- Szenarien:
  - open file, unlink path, FD readback muss stabil bleiben.
  - open file, rename path, alter FD bleibt gueltig.
  - rename `a` ueber existierende Datei `b`, danach `b` verify.
  - rename `a` nach `dir2/a`.
  - rename Fehlerfaelle pruefen: fehlende Quelle, Ziel im nicht vorhandenen
    Directory.

Akzeptanz:

- Keine verlorenen oder vermischten Inhalte.
- Directory-Eintraege entsprechen nach jedem Schritt der erwarteten Sicht.
- Fehlerfaelle liefern Fehler statt stiller Teilerfolge.

## [P1] Readdir unter Mutation

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `readdir_while_mutating_case`
  - `readdir_parallel_mutation_case`
  - wiederholte Snapshot-Readdir-Pruefung waehrend Create/Rename/Unlink-Zyklen
  - parallele Worker-Prozesse mutieren Shared-Hot-Directory, waehrend der
    Parent Readdir-Snapshots validiert
  - Sentinel-Dateien bleiben ueber Mutationen sichtbar
  - Readdir-Namen werden auf Laenge und UTF-8-Gueltigkeit validiert

Fehlt:

- Directory-Cursor-/Offset-Stabilitaet unter Mutation.

TODO:

- `readdir_while_mutating_case` ergaenzen.
- Worker A: dauernd `readdir`.
- Worker B/C: create/rename/unlink in demselben Directory.
- Optional: bekannte Sentinel-Dateien, die nie geloescht werden, muessen in
  ausreichend vielen Snapshots sichtbar bleiben.
- Falls anyOS nur path-basiertes `readdir` hat, Test als repeated snapshot
  bauen; spaeter FD-basiertes Readdir ergaenzen.

Akzeptanz:

- Kein Panic/Hang.
- `readdir` liefert nie ungueltige Namen/Laengen.
- Sentinel-Dateien verschwinden nicht dauerhaft aus Snapshots.

## [P1] Grosse Directories, lange Namen und exFAT-Namenssemantik

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `large_directory_case`
  - Quick/Normal/Heavy-Directory mit 128/512/1536 Eintraegen
  - `readdir`-Count, Stichproben-Readback und einfache Zeitmessung
  - `long_name_case`
  - lange Pfade bis zum aktuellen anyOS-Path-Budget via `stat/open/unlink`
  - Long-Name-Readdir-Verify via `SYS_READDIR_LONG`/`fs::readdir_long`
  - Case-Matrix dokumentiert beobachtetes case-sensitive/case-folded/collision
  - anyOS-API erweitert:
    - `SYS_READDIR_LONG`
    - `anyos_std::fs::readdir_long`
    - `anyos_std::fs::read_dir` nutzt automatisch das Long-Name-ABI

Fehlt:

- Nicht-ASCII/UTF-8/UTF-16-Grenzen, soweit anyOS-Userland sie unterstuetzt.
- Direkte Raw-`SYS_READDIR`-Nutzer koennen weiter das alte kompatible
  64-Byte-Format verwenden; sichtbare Tools sollen auf `read_dir` oder
  `readdir_long` migriert werden.

TODO:

- `large_directory_case` ergaenzen.
- `long_name_case` ergaenzen.
- Namensmatrix:
  - kurze Namen.
  - 8.3-aehnliche Namen.
  - 63/127/255 Byte Namen.
  - gleiche Praefixe mit unterschiedlichem Suffix.
  - Case-Varianten wie `Foo`, `foo`, `FOO`.
- Fuer exFAT explizit dokumentieren, ob case-insensitive Semantik erwartet
  wird oder aktuell nicht unterstuetzt ist.

Akzeptanz:

- `readdir` findet alle erwarteten Namen.
- `stat/open/unlink` funktionieren fuer lange Namen.
- Case-Verhalten ist konsistent und dokumentiert.
- Performance-Metrik fuer create/stat/readdir/unlink wird ausgegeben.

## [P1] Permissions, Ownership und Metadata Persistenz

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `permission_metadata_case`
  - `chmod` mit `stat`-Verify direkt und nach `sync`
  - `chown` mit uid/gid-Verify direkt und nach `sync`, falls unterstuetzt
  - im Scratch-Lifecycle chmod/chown-Verify nach Remount, falls Scratch aktiv
  - Content-Verify nach Metadata-Aenderungen
  - Groessen- und mtime-Plausibilitaet nach Append
  - WARN, falls chmod/chown vom getesteten FS nicht unterstuetzt werden

Fehlt:

- Fehlerfaelle fuer unzulaessige Zugriffe, soweit anyOS Permissions enforced.
- Detaillierte Timestamps/ctime/mtime bei Truncate und Rename.

TODO:

- `permission_metadata_case` ergaenzen.
- Testfaelle:
  - `chmod` verschiedene Modi.
  - `chown` verschiedene uid/gid.
  - `stat` direkt danach.
  - `sync`, close/reopen, erneut `stat`.
  - im Scratch-Modus nach Remount erneut `stat`. [umgesetzt im Scratch-Lifecycle]
  - truncate muss Metadata-Version/Timestamps aktualisieren, sobald im stat API
    sichtbar.

Akzeptanz:

- Mode/uid/gid gehen nicht nach Sync/Remount verloren.
- Unsupported Semantik wird klar als `SKIP` statt `PASS` gemeldet.

## [P1] Fsync, Close und Writeback-Semantik

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `fsync_ordering_case`
  - write + close ohne fsync mit Readback vor/nach globalem `sync`
  - write + Datei-`fsync` + close
  - write + globaler `sync` + close
  - create + Datei-`fsync` + rename + Readback/Old-Path-Verify
  - create + rename + globaler `sync` + Readback/Old-Path-Verify
  - Scratch-Lifecycle prueft Rename-Persistenz nach unmount/fsck/remount
  - Directory-`fsync` wird als `ok` oder `unsupported` berichtet

Fehlt:

- Vollstaendige Remount-Verifikation jeder einzelnen fsync-Matrix-Zelle.
- Klare Persistenz-Dokumentation pro FS nach Crash/Powerloss.

TODO:

- `fsync_ordering_case` ergaenzen. [teilweise umgesetzt]
- Matrix:
  - write + close ohne fsync. [umgesetzt]
  - write + fsync + close. [umgesetzt]
  - write + global sync + close. [umgesetzt]
  - create + rename + fsync file. [umgesetzt]
  - create + rename + global sync. [umgesetzt]
  - optional directory fsync. [umgesetzt als ok/unsupported]
- In Scratch-Modus jeweils remount und verify.

Akzeptanz:

- Persistenzregeln pro FS sind dokumentiert.
- CoreFS liefert die staerkeren Garantien, die wir festlegen.
- exFAT-Verhalten wird realistisch, aber deterministisch getestet.

## [P2] Sparse/Hole/Seek-Semantik

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `sparse_hole_matrix_case`
  - Head/Tail-Write mit grossem Hole
  - Null-Verify vor, zwischen und nach Datenbereichen
  - Teil-Overwrite mitten im Hole
  - Truncate auf 0 und Regrow mit erneuter Nullbereich-Pruefung

Fehlt:

- SEEK_DATA/SEEK_HOLE, falls irgendwann unterstuetzt.
- Truncate auf beliebige Groessen, sobald die API mehr als Truncate-to-zero
  unterstuetzt.

TODO:

- Bestehenden `sparse_eof_gap_case` zu `sparse_hole_matrix_case` erweitern. [teilweise umgesetzt]
- Matrix:
  - write at 0, write far offset. [umgesetzt]
  - read before, across and after data. [umgesetzt]
  - overwrite mitten im Hole. [umgesetzt]
  - truncate kleiner als Hole-Ende. [blocked: API nur truncate-to-zero]
  - truncate groesser und Nullbereich pruefen. [blocked: API nur truncate-to-zero]

Akzeptanz:

- Holes lesen als Nullbytes.
- Dateigroesse ist nach jedem Schritt korrekt.
- Keine alten Daten werden durch Hole-Reads sichtbar.

## [P2] Mmap-/Mapped-IO-Paritaet

Status:

- Explizit als unsupported in `vfsstress`:
  - `feature_gate_case` berichtet `file-mmap=unsupported`

Fehlt:

- xfstests hat viele mmap/fsx-Varianten. `vfsstress` nutzt nur read/write
  Syscalls.
- anyOS stdlib hat Memory-Mapping fuer anonymen Speicher, aber keine klare
  file-backed mmap API im `fs`-Modul.

TODO:

- Klaeren, ob file-backed mmap in anyOS existiert oder geplant ist.
- Wenn vorhanden:
  - `mmap_write_read_case` ergaenzen.
  - mmap-write + read syscall verify.
  - syscall write + mmap-read verify.
  - mmap-write + fsync/sync + remount verify.
- Wenn nicht vorhanden:
  - TODO als blocked markieren und API-Anforderung dokumentieren. [umgesetzt via SKIP/Feature-Gate]

Akzeptanz:

- File-backed mmap wird entweder getestet oder explizit als nicht unterstuetzt
  reportet.

## [P2] Direct-IO/AIO-Analoga

Status:

- Explizit als unsupported in `vfsstress`:
  - `feature_gate_case` berichtet `direct-io=unsupported`

Fehlt:

- xfstests testet O_DIRECT und AIO stark. anyOS hat dafuer aktuell keine
  offensichtliche Userland-API in `anyos_std::fs`.

TODO:

- Klaeren, ob direct IO, uncached IO oder async IO existieren sollen.
- Falls ja:
  - Flags in `anyos_std::fs` ergaenzen.
  - Alignment-Matrix bauen: unaligned muss sauber fehlschlagen, aligned muss
    korrekt lesen/schreiben.
  - Parallel direct/buffered IO testen.
- Falls nein:
  - In `vfsstress` als `SKIP direct-io unsupported` reporten. [umgesetzt]

Akzeptanz:

- Keine stille Vermischung von cached/uncached Daten.
- Unsupported Pfade sind transparent.

## [P2] Statfs/Quota/Accounting-Metriken

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `statfs_accounting_case`
  - Probe-Pfad-Fallback fuer `cfg.dir`, `/tmp`, `/`
  - free/used Plausibilitaet nach write, truncate, unlink und sync
  - WARN statt FAIL, wenn `statfs` nicht verfuegbar oder ohne sichtbare Deltas ist

Fehlt:

- Optional Quota oder per-user Accounting, falls anyOS das spaeter bekommt.

TODO:

- `statfs_accounting_case` ergaenzen. [teilweise umgesetzt]
- Vorher/nachher Werte fuer create/write/unlink/truncate/sync pruefen. [umgesetzt]
- Negative Deltas und "free wird nach delete nie groesser" als Warn/Fail
  klassifizieren. [umgesetzt]
- Performance-Report um FS-Free/Used Spalten erweitern.

Akzeptanz:

- `statfs` ist monoton plausibel fuer Writes und Deletes.
- Test toleriert FS-Metadaten-Overhead mit definierter Toleranz.

## [P2] Pfadnormalisierung und Path-Limits

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `path_resolution_case`
  - absolute Pfade mit `.`, `..` und doppelten Slashes
  - trailing slash: Datei mit Slash muss fehlschlagen, Directory mit Slash
    muss statbar bleiben
  - relative Pfade aus wechselndem CWD via `chdir`
  - tiefe Verzeichniskette
  - Pfad knapp unter dem aktuellen Budget
  - Ueberlang-Pfad darf nicht auf Near-Limit-Datei aliasen

Fehlt:

- Ueberlang-Pfad sollte idealerweise sauber mit ENAMETOOLONG fehlschlagen
  statt nur Nicht-Aliasing zu garantieren.

TODO:

- `path_resolution_case` ergaenzen. [teilweise umgesetzt]
- Matrix:
  - relative Pfade aus wechselndem CWD, falls chdir verfuegbar. [umgesetzt]
  - `./a`, `a/../a`, `a//b`, `a/b/`. [umgesetzt]
  - tiefe Verzeichnisketten. [umgesetzt]
  - Pfadlaengen knapp unter/ueber Limit. [teilweise umgesetzt]

Akzeptanz:

- Normale Pfade werden konsistent aufgeloest.
- Ueberlange Pfade schlagen sauber fehl.
- Keine stillen Truncations auf falsche Dateien.

## [P2] Performance-Baselines

Status:

- Teilweise umgesetzt in `vfsstress`:
  - `metadata_perf_case`
  - create/stat/rename/readdir/unlink Durchsatz als ops/s bzw. entries/s
  - `sequential_io_perf_case`
  - sequentielle Write-/Read-Datenrate mit 64K Chunks
  - `random_overwrite_perf_case`
  - 4K Random-Overwrite-Durchsatz
  - `sync_latency_perf_case`
  - fsync/global-sync min/avg/p50/p95/max Latenzen
  - Profilabhaengige Eintragszahlen fuer quick/normal/heavy
  - JSONL-Performance-Zeilen fuer Metadata, Sequential-IO, Random-Overwrite
    und Sync-Latenz

Fehlt:

- Vergleichbare Ausgabe fuer CoreFS vs exFAT.
- Regressionsschwellen.

TODO:

- `--perf` oder `--json` Report-Modus ergaenzen. [teilweise: `--json` Summary umgesetzt]
- Metriken:
  - create/stat/rename/unlink ops/s. [umgesetzt]
  - readdir entries/s. [umgesetzt]
  - sequential write/read KB/s. [umgesetzt]
  - random overwrite ops/s. [umgesetzt]
  - sync latency. [umgesetzt als min/avg/p50/p95/max]
  - fsync latency p50/p95/max, soweit ohne Heap-heavy Statistik machbar. [umgesetzt]
- Output stabil maschinenlesbar machen. [teilweise: Summary-JSON plus Perf-JSONL umgesetzt]

Akzeptanz:

- Ein CI/Runner kann CoreFS vs exFAT vergleichen.
- Regressionsschwellen koennen spaeter definiert werden.

## [P3] Xattrs, Special Files und erweiterte Attribute

Status:

- Explizit als unsupported in `vfsstress`:
  - `feature_gate_case` berichtet `xattr=unsupported`
  - `feature_gate_case` berichtet `special-files=unsupported`

Fehlt:

- xfstests prueft viele xattr/attr-Faelle.
- anyOS-Unterstuetzung ist unklar.

TODO:

- Klaeren, ob xattrs geplant sind.
- Falls ja:
  - set/get/list/remove xattr Tests.
  - grosse Werte.
  - viele Attribute.
  - Persistenz nach sync/remount.
- Special Files nur testen, falls anyOS mknod/devfs-Semantik fuer normale FS
  vorsieht.

Akzeptanz:

- Unsupported Features werden als `SKIP` gefuehrt.
- Implementierte Features haben Persistenz- und Fehlerpfadtests.

## [P3] Whiteout/Overlay/FUSE/Namespace-Paritaet

Status:

- Explizit als P4/blocked Feature-Gate dokumentiert:
  - Overlay/Whiteout/Namespace werden nicht in die CoreFS/exFAT-Basis-Suite
    gemischt.
  - `feature_gate_case` berichtet `overlay=unsupported`,
    `whiteout=unsupported`, `namespace=unsupported`

Fehlt:

- xfstests enthaelt Overlay-/Whiteout-/Namespace-Spezialfaelle.
- Fuer CoreFS/exFAT ist das aktuell nachrangig.

TODO:

- Erst anfassen, wenn overlayfs/fuse/idmapped/userns-Features in anyOS
  produktiv relevant werden.
- Dann eigene Testgruppe `feature-overlay` oder separates Tool bauen.

Akzeptanz:

- Keine Vermischung mit CoreFS/exFAT-Basisfreigabe.

## [P3] Lange Soak-/Dauerlaeufe

Status:

- Umgesetzt in `vfsstress`:
  - `--seconds N`
  - `--soak` als Alias fuer Profil `soak` und default 3600 Sekunden
  - zyklischer Mix aus fsx, Metadata, Parallel-Fsstress, ENOSPC, Sequential-IO
    und Sync-Latenz
  - Fortschrittsausgabe pro Runde mit Seed
  - Scratch-Lifecycle/Remount/fsck wird im Soak-Mix ausgefuehrt, wenn
    `--scratch-device` konfiguriert ist

Fehlt:

- Kein automatischer Scratch-Device-Pool fuer mehrere FS-Typen.

TODO:

- `--seconds N` wie bei `kstress` ergaenzen. [umgesetzt]
- `--soak` Profil ergaenzen. [umgesetzt]
- Workloads zyklisch mischen:
  - parallel fsstress.
  - fsx random.
  - enospc light.
  - metadata churn.
  - sync/remount, falls Scratch-Modus aktiv. [umgesetzt]

Akzeptanz:

- `vfsstress --seconds 3600 --profile heavy` laeuft ohne Speicherdrift.
- Fortschritt wird periodisch ausgegeben.
- Letzter Seed/Checkpoint ist bei Crash sichtbar.

## Report-/Harness-Aufgaben

### [P1] SKIP/WARN/PASS/FAIL Modell

TODO:

- `Summary` um `skips` erweitern. [umgesetzt]
- Feature-Erkennung einbauen:
  - symlink supported. [umgesetzt]
  - chmod/chown supported. [umgesetzt]
  - mount/remount supported. [teilweise: scratch configured/no-device]
  - direct IO supported. [umgesetzt als unsupported]
  - file mmap supported. [umgesetzt als unsupported]
- Unsupported Features als `SKIP`, nicht als `FAIL`. [umgesetzt]

Akzeptanz:

- Quick-Run auf minimalem System bleibt nuetzlich.
- Fehlende optionale Features verbergen keine echten Datenfehler.

### [P1] Reproduzierbare Seeds und Fehlerprotokoll

TODO:

- Globalen Seed im Header ausgeben. [umgesetzt]
- Pro Testfall abgeleitete Seeds ausgeben.
- Bei Failure Operation-Index, Worker-ID und Pfad ausgeben.
- Optional `--keep` automatisch empfehlen, wenn Artefakte vorhanden bleiben. [umgesetzt als Repro-Zeile bei FAIL]

Akzeptanz:

- Ein Fail kann mit einem einzelnen Kommando reproduziert werden.

### [P2] JSON-Ausgabe fuer CI

TODO:

- `--json` ergaenzen. [teilweise umgesetzt]
- Report:
  - Version. [umgesetzt]
  - FS/Pfad. [teilweise: Pfad umgesetzt]
  - Profil. [umgesetzt]
  - Tests mit Status, Dauer, Details. [umgesetzt als JSONL-Testevents]
  - Performance-Metriken. [teilweise: JSONL fuer Perf-Testfamilien]
  - Seeds. [umgesetzt]

Akzeptanz:

- CI kann Failures und Performance-Regressions maschinenlesbar auswerten.

## Vorgeschlagene Umsetzungsreihenfolge

1. P0 Parallelitaet: `parallel_fsstress_case`. [umgesetzt]
2. P1 Harness-Basis: `SKIP/WARN/PASS/FAIL` und Seed-Protokoll. [teilweise umgesetzt in `vfsstress`]
3. P1 Open-Unlink/Rename und Symlink/ELOOP.
4. P0 ENOSPC + `statfs`-Accounting. [teilweise umgesetzt]
5. P1 Readdir unter Mutation + grosse Directories/lange Namen.
6. P0 Scratch-FS Lifecycle fuer CoreFS/exFAT. [teilweise umgesetzt]
7. P1 Fsync/Close/Remount-Ordering.
8. P0 Recovery/Error-Injection Design und erste CoreFS-Variante.
9. P2 Performance-JSON und Baselines.
10. P2/P3 mmap/direct-io/xattr/overlay nur nach Feature-Verfuegbarkeit.

## [P4] 1:1-Upstream-Paritaet und Spezial-Mapping

Status:

- Langfristig/blockiert:
  - Vollstaendige 1:1-Abdeckung aller upstream `xfstests` erfordert ein
    separates Manifest/Harness und mehrere anyOS-Features, die aktuell bewusst
    als unsupported/blocked gemeldet werden.
  - `vfsstress` deckt den CoreFS/exFAT-nativen Kern ab und meldet fehlende
    Feature-Familien per SKIP/Feature-Gate.

TODO:

- Upstream-Testnummern in ein Manifest mappen.
- Fuer jeden Test `covered`, `adapted`, `skip-unsupported` oder `blocked-api`
  pflegen.
- Feature-Familien erst in P4-Portierung aufnehmen, wenn anyOS sie produktiv
  anbietet: Overlay/Whiteout, Namespace/idmapped mounts, xattrs, file mmap,
  direct IO/AIO, Quotas.

Akzeptanz:

- P4 darf die CoreFS/exFAT-Freigabe nicht blockieren.
- P4-Eintraege muessen begruenden, warum ein Test nativ, adaptiert, skipped
  oder blockiert ist.

## Mindestziel fuer CoreFS/exFAT Freigabe

- P0 komplett gruen auf CoreFS und exFAT.
- P1 bis einschliesslich Open-Unlink, Symlink, Readdir-Mutation, lange Namen
  und Fsync/Close gruen oder sauber als unsupported dokumentiert.
- Quick-Profil unter 2 Minuten.
- Normal-Profil fuer lokale Entwicklung.
- Heavy/Soak fuer Nachtlauf oder Vor-Release-Check.
