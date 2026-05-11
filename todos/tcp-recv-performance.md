# TCP Receive Performance Follow-ups

Stand: 2026-05-12

Kontext:

- `sys_tcp_recv` nutzt nach der wget-Korruptionsanalyse einen sicheren
  Kernel-Bouncebuffer und kopiert danach per `copy_to_user_bytes` in den
  Userbuffer.
- Der Pfad ist robuster, kostet aber eine Extra-Kopie und eine Allocation pro
  Receive-Syscall.
- `tcpstress` testet jetzt Receive-Puffer bis 256 KiB; diese Matrix soll bei
  allen Optimierungen gruen bleiben.

## Offene Punkte

### [P1] Fastpath fuer validierte Userbuffer

Wenn der komplette Userbereich seitenweise als schreibbar und stabil validiert
werden kann, darf `sys_tcp_recv` direkt in den Userbuffer empfangen:

```text
TCP recv_buf -> Userbuffer
```

Fallback bleibt der sichere Bouncebuffer-Pfad:

```text
TCP recv_buf -> Kernelbuffer -> copy_to_user -> Userbuffer
```

Akzeptanz:

- Fastpath nur nach starker User-Range-Pruefung.
- Fallback fuer unsichere/ungueltige Ranges bleibt erhalten.
- `tcpstress` Receive-Buffer-Matrix 4K..256K PASS.
- `httpstress` inklusive wget-Runden PASS.

### [P2] Bouncebuffer ohne per-call Allocation

Aktuell allokiert `sys_tcp_recv` fuer den sicheren Pfad pro Call einen `Vec`.
Moegliche Verbesserungen:

- per-CPU Scratchbuffer,
- per-Task Scratchbuffer,
- kleiner statischer Chunkbuffer mit Schleife.

Akzeptanz:

- Keine Allocation im haeufigen Receive-Pfad.
- Keine Datenlecks zwischen Tasks/Sockets.
- Keine Regression bei parallelen Downloads.

### [P2] Page-by-page Usercopy fuer grosse Buffers

Wenn direkter Fastpath nicht moeglich ist, kann der sichere Pfad grosse
Userbuffer in validierten Seiten-/Chunk-Grenzen abarbeiten. Das reduziert Peak-
Allocation und macht Fehlerstellen genauer diagnostizierbar.

Akzeptanz:

- Grosse Userbuffer bis mindestens 256 KiB funktionieren stabil.
- Fehler bei ungueltiger Folgeseite brechen sauber ab.
- Keine Teilkopien ohne korrekten Rueckgabewert.

### [P3] Vectored/User-page Receive-API pruefen

Langfristig koennte TCP direkt in validierte Userpages oder eine kleine
I/O-Vector-Struktur kopieren. Das waere sauberer als ein roher Pointer-
Fastpath, braucht aber klare Regeln fuer Page-Pinning oder Scheduler-
Interaktionen.

Akzeptanz:

- Designnotiz fuer Userpage-Pinning oder stabil validierte User-Ranges.
- Keine Annahme, dass virtuell zusammenhaengender Userspace physisch
  zusammenhaengend ist.
- Benchmark gegen Bouncebuffer- und Fastpath-Variante.

## Tests, die erhalten bleiben sollen

- `tcpstress` Receive-Buffer-Matrix: 4K, 16K, 32K, 64K, 128K, 256K.
- `httpstress` mit Debian `Packages.gz` Default-URL.
- `httpstress` wget-Runden mit CRC32/MD5/gzip/first-diff Vergleich.
- `licof init` Bootstrap-Download als realer End-to-End-Smoke-Test.
