# lxe - Linux Experience Extension Roadmap

## Ziel

`lxe` ist ein separater Linux-Kompatibilitaetspfad neben ASL. ASL bleibt die
VM-first Linux-Umgebung; `lxe` ist der direkte Linux-ELF64/Kernel-ABI-Pfad fuer
kleine bis mittelgrosse Linux-Programme.

Der erste Erfolg ist nicht "Debian komplett", sondern ein stabiler vertikaler
Slice:

1. Linux-ELF64 starten.
2. Linux-Syscalls nach anyOS-Semantik uebersetzen.
3. Dynamische glibc-Binaries ueber die lxe-Linux-Base ausfuehren.
4. Debian-Pakete in diese Linux-Base installieren.

## Produktgrenzen

- `lxe` ist keine VM und benutzt keinen Linux-Kernel.
- `lxe` implementiert die Linux x86_64 Syscall-ABI auf dem anyOS-Kernel.
- `lxe` ist bewusst opt-in: Prozesse laufen entweder als `AnyOs` oder als
  `LinuxX86_64` ABI-Personality.
- ASL bleibt der Pfad fuer echte Distributionen, Kernelmodule, systemd-nahe
  Workloads und maximale Linux-Kompatibilitaet.

## Aktueller Implementierungsstand

- Kernel-Threads besitzen eine ABI-Personality; Fork/Spawn vererben sie.
- `SYS_LXE_SPAWN` startet ELF64-Prozesse gezielt als `LinuxX86_64`.
- Linux-x86_64 Syscalls werden im gemeinsamen x86_64 Entry anhand der
  Personality in eine separate Linux-Tabelle dispatcht.
- Der Linux-ELF64-Pfad baut einen Linux-konformen Initialstack mit
  `argc`/`argv`/`envp`/`auxv` inklusive `AT_PHDR`, `AT_ENTRY`, `AT_RANDOM`,
  `AT_PLATFORM` und `AT_EXECFN`.
- Der Linux-Loader kann `PT_INTERP` aus der lxe-Linux-Base laden, ET_DYN-Objekte
  mit festem Load-Bias mappen und `AT_BASE` auf den dynamischen Loader setzen.
- Linux-ABI-Prozesse bekommen auf x86_64 eine User-PML4 ohne Low-Identity-
  Mapping, damit klassische Linux-ET_EXECs bei `0x400000` nicht mehr mit
  Kernel-Identity-Mappings kollidieren.
- Tier-0/Startsyscalls sind angebunden: `read`, `write`, `open`, `openat`,
  `close`, `stat`, `lstat`, `fstat`, `newfstatat`, `lseek`, `brk`, `mmap`,
  `munmap`, `getpid`, `getcwd`, `chdir`, `readlink`, `uname`, `getuid`,
  `getgid`, `arch_prctl`, `set_tid_address`, `set_robust_list`, `getrandom`,
  `exit` und `exit_group`.
- `/System/bin/lxe` existiert mit `status`, `init`, `repair`, `run`, `pkg`
  und `apt install`.
- `lxe init` legt die Linux-Base, Apt-Konfiguration, Cache und Paketdatenbank
  an, bootstrapped eine minimale Apt-Basis inklusive `passwd` und versucht
  danach interaktiv `passwd root` zu starten.
- `lxe` prueft vor `run`/`passwd` den Linux-ELF64-Header, zeigt `PT_INTERP`
  und fehlende Interpreter-Pfade sichtbar an und warnt bei fehlendem TTY fuer
  interaktive Linux-Tools.
- `lxe apt install <pkg>` laedt Paketindex und `.deb`-Pakete aus
  `deb.debian.org`, loest einfache `Pre-Depends`/`Depends` rekursiv auf und
  extrahiert `data.tar.gz` in die Linux-Base.
- Der Paketcache verwendet interne, dateisystemfreundliche Namen; Debian-
  Dateinamen mit `+`, `:` oder anderen Sonderzeichen bleiben nur in der URL.
- `libzip` reicht Tar-Metadaten wie Typeflag, Mode, UID/GID und Link-Ziel an
  Clients durch. `lxe` erzeugt Symlinks und wendet chmod/chown best-effort
  an.
- `data.tar.xz` ist im `lxe`/`libzip_client`-Pfad verdrahtet. `libzip`
  entpackt XZ-Container mit einfachem LZMA2-Filter inklusive normaler
  komprimierter Chunks und unkomprimierter LZMA2-Chunks.

Aktuelle Grenze: dynamische ELF64-Binaries kommen jetzt bis zum Interpreter-
Startpfad, brauchen aber weitere Linux-Syscalls (`futex`, `mprotect`, `access`,
`pread64`, TLS-/Thread-Pfade), bevor glibc stabil laeuft. Moderne Debian-
Pakete verwenden haeufig `data.tar.xz`; der Debian-Standardpfad mit einfachem
LZMA2-Filter ist implementiert. XZ-Filterketten mit Delta/BCJ und SHA256-
Check-Verifikation sind noch nicht Ziel des ersten Lxe-Slices.

## Architektur

### Kernel

- `AbiPersonality` am Thread/Prozess:
  - `AnyOs`
  - `LinuxX86_64`
- x86_64 `SYSCALL` Entry bleibt gemeinsam.
- `syscall_dispatch_64` waehlt anhand der aktuellen Personality:
  - `AnyOs`: bestehender anyOS Dispatch.
  - `LinuxX86_64`: Linux-Registerkonvention und Linux-Syscall-Tabelle.
- Linux-ABI verwendet:
  - `rax`: Syscallnummer
  - `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`: Argumente 1-6
  - Return: `>= 0` Erfolg, `-errno` Fehler

### Loader

- Bestehender ELF64 Loader bleibt Grundlage fuer `PT_LOAD`.
- Linux-Modus erweitert:
  - Linux Stacklayout: `argc`, `argv`, `envp`, `auxv`
  - `PT_INTERP` fuer `/lib64/ld-linux-x86-64.so.2`
  - AuxV: `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_ENTRY`, `AT_PAGESZ`,
    `AT_RANDOM`, `AT_PLATFORM`, `AT_EXECFN`
- Phase 1 darf statische ELF64-Binaries ohne dynamischen Loader starten.

### Userland

- `/System/bin/lxe` ist die erste CLI.
- Spaeter optional: `lxed` als Broker fuer Linux-Base, Paketdatenbank,
  `/proc`-Emulation und Downloads.
- Standard-Linux-Base:
  - `/System/var/lxe/rootfs`
  - Cache: `/System/var/lxe/cache`
  - Datenbank: `/System/var/lxe/db`

## Kompatibilitaetsstufen

### Tier 0: Statische Smoke-Binaries

Ziel: handgebaute oder musl-statische ELF64-Binaries mit minimalen Syscalls.

Kernel-Syscalls:

- `exit` / `exit_group`
- `write`
- `read`
- `close`
- `brk`
- `mmap`
- `munmap`
- `getpid`
- `arch_prctl` minimal

CLI:

- `lxe run <path> [args...]`
- `lxe status`

### Tier 1: Kleine dynamische glibc-Tools

Ziel: `/bin/echo`, `/usr/bin/env`, einfache Coreutils aus der lxe-Linux-Base.

Zusatz:

- `openat`
- `newfstatat`, `fstat`, `statx` Stub/Teilmenge
- `pread64`
- `readlink`
- `access` / `faccessat`
- `set_tid_address`, `set_robust_list`, `rseq` als sichere Minimalpfade
- `mprotect` zumindest fuer vorhandene Mappings

### Tier 2: Shell-nahe Workloads

Ziel: `dash`, einfache Skripte, pipes, redirection.

Zusatz:

- `clone` begrenzt
- `wait4`
- `pipe2`
- `dup`, `dup2`, `dup3`
- `fcntl`
- `getcwd`, `chdir`
- Signal-Grundpfad

### Tier 3: Paketinstallation

Ziel: `.deb` lokal installieren und einfache Pakete nutzbar machen.

CLI:

- `lxe init`
- `lxe repair`
- `lxe pkg install <file.deb>`
- `lxe apt install <pkg>`

Implementierung:

- Debian `ar` Container lesen.
- `data.tar.gz` und `data.tar.xz` extrahieren.
- Tar-Metadaten fuer Symlinks, Mode und Ownership auswerten.
- Paketdatenbank minimal pflegen.
- Paketindex aus Debian-Archiv laden und einfache Depends aufloesen.
- Maintainer-Scripts zuerst nicht automatisch ausfuehren.

## Risiken

- `futex`, `clone`, TLS und Signals bestimmen, ab wann moderne glibc-Programme
  stabil laufen.
- `/proc`, `/sys`, `/dev`, PTY und `ioctl` sind breite Oberflaechen.
- Linux-Flags und Structs duerfen nicht blind auf anyOS-Strukturen gecastet
  werden.
- Der Linux-ABI-Pfad muss die bestehenden anyOS-Capabilities respektieren.

## Erste Implementierungsphase

1. Kernel-`AbiPersonality` einfuehren.
2. Linux-x86_64 Dispatch-Skeleton mit Tier-0 Syscalls.
3. Kernel-Syscall `SYS_LXE_SPAWN` fuer kontrolliertes Starten als
   `LinuxX86_64`.
4. `/System/bin/lxe` mit `run`, `status`, `init`, `repair`, `pkg` Skeleton.
5. Tests: Buildbarkeit und Smoke-Pfad fuer statische Linux-ELF64-Binaries.
