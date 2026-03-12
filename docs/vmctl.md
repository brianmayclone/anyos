# vmctl — AI-friendly CoreVM CLI Controller

`vmctl` ist ein Kommandozeilen-Tool zur headless VM-Steuerung, optimiert fuer Automatisierung durch KI-Agenten und Skripte. Es hostet die VM direkt im eigenen Prozess (ohne Umweg ueber vmd/IPC) und gibt strukturierten, parsebaren Output aus.

## Uebersicht

```
vmctl <command> [options]
```

| Befehl | Beschreibung |
|--------|-------------|
| `run` | VM erstellen, ausfuehren, Serial-Output streamen |
| `serial` | Interaktive serielle Konsole (stdin/stdout) |
| `list` | Konfigurierte VMs auflisten |
| `info <uuid>` | VM-Konfiguration anzeigen |
| `create-disk <path> <size_mb>` | Leeres Disk-Image erstellen |
| `help` | Hilfe anzeigen |

## Subcommands

### `vmctl run`

Erstellt eine VM in-process, startet sie, streamt die serielle Ausgabe live nach stdout und gibt beim Beenden einen strukturierten Exit-Summary aus.

**Optionen:**

| Flag | Beschreibung | Default |
|------|-------------|---------|
| `-u <uuid>` | VM-Config per UUID laden (aus `/System/shared/vmmanager/vms/`) | — |
| `-r <mb>` | RAM-Groesse in MiB | 64 |
| `-d <path>` | Pfad zum Disk-Image (AHCI Port 0, HDD) | — |
| `-i <path>` | Pfad zum ISO-Image (AHCI Port 1, CD-ROM) | — |
| `-b <type>` | BIOS-Typ: `corevm` oder `seabios` | `corevm` |
| `-t <secs>` | Timeout in Sekunden (0 = kein Timeout) | 0 |
| `-s` | VGA-Textscreen (80x25) beim Beenden ausgeben | aus |
| `-g` | CPU-Register beim Beenden ausgeben | aus |
| `-n` | Netzwerk aktivieren (E1000 NIC) | aus |
| `-k <text>` | Text nach Boot via PS/2-Tastatur tippen | — |
| `-w <ms>` | Wartezeit vor `-k` Eingabe in ms | 3000 |

**Beispiele:**

```bash
# VM mit 128 MB RAM und Disk starten, 30s laufen, Screen + Register dumpen
vmctl run -r 128 -d /data/disk.img -t 30 -s -g

# Bestehende VM-Config laden und 60s laufen lassen
vmctl run -u 01234567890abcdef -t 60 -s

# ISO booten mit SeaBIOS
vmctl run -r 64 -i /data/boot.iso -b seabios -t 10 -s

# VM starten, nach 5s "ls\n" tippen, Screen anzeigen
vmctl run -r 64 -d /data/disk.img -k "ls\n" -w 5000 -t 15 -s
```

### `vmctl serial`

Interaktive serielle Konsole: VM laeuft dauerhaft, stdin wird an die VM-Serielle weitergeleitet, stdout zeigt die serielle Ausgabe der VM.

```bash
vmctl serial -r 64 -d /data/disk.img
vmctl serial -r 128 -d /data/disk.img -b seabios
```

Akzeptiert die gleichen Konfigurations-Flags wie `run` (ausser `-t`, `-s`, `-g`, `-k`, `-w`).

### `vmctl list`

Listet alle konfigurierten VMs aus `/System/shared/vmmanager/vms/*.conf` auf.

```bash
vmctl list
```

Ausgabe:
```
--- VM LIST ---
UUID                                  NAME                  RAM  DISK
--------------------------------------------------------------------------------------
a1b2c3d4e5f6789012345678              Ubuntu 20.04        512MB  /data/vms/ubuntu.img
--- END LIST ---
```

### `vmctl info <uuid>`

Zeigt die Konfiguration einer VM an.

```bash
vmctl info a1b2c3d4e5f6789012345678
```

Ausgabe:
```
--- VM INFO ---
uuid: a1b2c3d4e5f6789012345678
name: Ubuntu 20.04
ram_mb: 512
bios: seabios
disk: /data/vms/ubuntu.img
iso: (none)
net_enabled: true
mac: 52:54:00:12:34:56
--- END INFO ---
```

### `vmctl create-disk <path> <size_mb>`

Erstellt ein leeres (nullgefuelltes) Disk-Image.

```bash
vmctl create-disk /data/vms/blank.img 256
```

## Strukturierte Ausgabe

Alle Ausgaben sind mit Delimitern versehen, damit sie programmatisch geparst werden koennen:

| Delimiter | Inhalt |
|-----------|--------|
| `--- SERIAL OUTPUT ---` / `--- END SERIAL OUTPUT ---` | Serielle Ausgabe der VM (live gestreamt) |
| `--- VGA TEXT SCREEN (80x25) ---` / `--- END SCREEN ---` | VGA-Textbuffer als lesbarer Text |
| `--- CPU REGISTERS ---` / `--- END REGISTERS ---` | CPU-Register (RAX-R15, RIP, RFLAGS, CRx, Segmente) |
| `--- VM EXIT SUMMARY ---` / `--- END SUMMARY ---` | Strukturierter Exit-Report |
| `--- VM LIST ---` / `--- END LIST ---` | VM-Liste |
| `--- VM INFO ---` / `--- END INFO ---` | VM-Konfiguration |

### Exit Summary Felder

```
--- VM EXIT SUMMARY ---
exit_reason: timeout|shutdown
runtime_ms: 30000
exit_count: 1500000
serial_bytes: 2048
typed: true
--- END SUMMARY ---
```

| Feld | Beschreibung |
|------|-------------|
| `exit_reason` | `timeout` (Zeitlimit erreicht) oder `shutdown` (VM hat sich beendet / Triple Fault) |
| `runtime_ms` | Laufzeit in Millisekunden |
| `exit_count` | Anzahl der VM-Exits (I/O, MMIO, HLT etc.) |
| `serial_bytes` | Gesamtanzahl empfangener Serial-Bytes |
| `typed` | `true` wenn `-k` Text getippt wurde (nur bei `run_with_typing`) |

## CPU-Register Dump

Ausgabe mit `-g` Flag:

```
--- CPU REGISTERS ---
RAX=0000000000000000  RBX=0000000000000000  RCX=0000000000000000  RDX=0000000000000663
RSI=0000000000000000  RDI=0000000000000000  RBP=0000000000000000  RSP=0000000000007C00
R8 =0000000000000000  R9 =0000000000000000  R10=0000000000000000  R11=0000000000000000
R12=0000000000000000  R13=0000000000000000  R14=0000000000000000  R15=0000000000000000
RIP=0000000000007C00  RFLAGS=0000000000000202
CR0=0000000000000010  CR2=0000000000000000  CR3=0000000000000000  CR4=0000000000000000  EFER=0000000000000000
CS: sel=0000 base=0000000000000000 limit=0000FFFF  DS: sel=0000 base=0000000000000000
SS: sel=0000 base=0000000000000000  ES: sel=0000  FS: sel=0000  GS: sel=0000
--- END REGISTERS ---
```

## VGA Text Screen Dump

Ausgabe mit `-s` Flag (nur nicht-leere Zeilen):

```
--- VGA TEXT SCREEN (80x25) ---
anyOS v0.4.23
Loading kernel...

root@anyos:~#
--- END SCREEN ---
```

## Tastatur-Automation (-k Flag)

Der `-k` Flag konvertiert ASCII-Text in PS/2 Set 1 Scancodes. Unterstuetzte Zeichen:

- Buchstaben: `a-z`, `A-Z` (mit Shift)
- Ziffern: `0-9`
- Sonderzeichen: `` -=[]\;',./`~!@#$%^&*()_+{}|:"<>? ``
- Steuerzeichen: `\n` (Enter), `\t` (Tab)

Nicht unterstuetzt: Funktionstasten, Pfeiltasten, Ctrl/Alt-Kombinationen.

**Timing:**
- `-w <ms>` steuert die Wartezeit nach VM-Start bevor getippt wird (Default: 3000ms)
- Das ist wichtig damit BIOS und Bootloader Zeit haben zu laden

## Architektur

```
┌─────────────────────────────────────────────────┐
│              vmctl (CLI Programm)                │
│  - Parst Argumente                              │
│  - Erstellt VM in-process                       │
│  - Fuehrt VM-Exit-Loop aus                      │
│  - Streamt Serial-Output nach stdout            │
│  - Dumpt Screen/Register bei Exit               │
└───────────────────────┬─────────────────────────┘
                        │ libcorevm_client API
                        ▼
              ┌──────────────────────┐
              │   libcorevm.so       │
              │ - x86 CPU Emulation  │
              │ - 15 Device Models   │
              │ - Software Backend   │
              └──────────────────────┘
```

Im Gegensatz zu `vmd` (Daemon mit IPC-Pipes und SHM) hostet `vmctl` die VM direkt im eigenen Prozess. Das macht es einfacher fuer Automatisierung:
- Kein separater Daemon-Prozess noetig
- Keine IPC-Pipes oder Shared Memory
- Alles ueber stdout/stdin
- Prozess beendet sich nach Timeout/Shutdown

## VM-Konfigurationsdateien

`vmctl run -u <uuid>` laedt die Konfiguration aus `/System/shared/vmmanager/vms/<uuid>.conf`. Das Format ist kompatibel mit vmmanager:

```
name=My VM
ram=256
disk=/path/to/disk.img
iso=/path/to/boot.iso
bios=corevm
net_enabled=1
mac_address=52:54:00:12:34:56
```

CLI-Flags ueberschreiben Werte aus der Config-Datei.

## Typischer KI-Workflow

1. **VM starten und beobachten:**
   ```
   vmctl run -r 64 -d /data/os.img -t 30 -s
   ```
   → KI liest Serial-Output und VGA-Screen, prueft ob OS korrekt bootet

2. **Kommando ausfuehren und Ergebnis pruefen:**
   ```
   vmctl run -r 64 -d /data/os.img -k "uname -a\n" -w 5000 -t 15 -s
   ```
   → KI tippt nach 5s `uname -a`, liest Screen nach 15s, prueft Output

3. **Disk erstellen und OS installieren:**
   ```
   vmctl create-disk /data/new.img 512
   vmctl run -r 128 -d /data/new.img -i /data/installer.iso -b seabios -t 300 -s
   ```

4. **Debugging:**
   ```
   vmctl run -r 64 -d /data/os.img -t 5 -s -g
   ```
   → KI prueft CPU-Register und Screen nach 5s, diagnostiziert Boot-Probleme

## Dateien

- `bin/vmctl/src/main.rs` — Hauptimplementierung
- `bin/vmctl/Cargo.toml` — Paketdefinition
- `cmake/UserPrograms.cmake` — Build-Registrierung (`add_rust_user_program(vmctl)`)
- `libs/libcorevm_client/src/lib.rs` — Client-API die vmctl nutzt
- `docs/corevm-api.md` — Vollstaendige CoreVM API-Referenz
