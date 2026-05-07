# SPICE vdagent — Clipboard Sync zwischen Host und Gast

`vdagent` ist der anyOS-Gast-Agent für SPICE/QEMU. Er synchronisiert die
Zwischenablage bidirektional zwischen Host und anyOS-Gast über einen
benannten `virtio-serial` Port (Standard: `com.redhat.spice.0`).

## Architektur

```
┌────────────────┐  spicevmc /          ┌─────────────────┐  evt_chan      ┌──────────────┐
│ Host (QEMU)    │  qemu-vdagent        │ vdagent (Guest) │  CMD_SET/      │ Compositor   │
│  Clipboard     │ ───── virtio ──────▶ │  /System/bin/   │  GET_CLIPBOARD │  Clipboard   │
│ (X11/Wayland/  │ ◀──── serial ─────── │  vdagent        │ ◀────────────▶ │              │
│  Win32)        │      (named port)    │                 │                │              │
└────────────────┘                      └─────────────────┘                └──────────────┘
                  │                                       │
                  │ Kernel: virtio-console multiport      │
                  │  /dev/virtio-ports/com.redhat.spice.0 │
                  │  /dev/vport-spice  (kurzer Alias)     │
```

### Komponenten

| Komponente | Pfad | Zweck |
|---|---|---|
| Kernel-Treiber | [kernel/src/drivers/virtio/serial.rs](../kernel/src/drivers/virtio/serial.rs) | virtio-serial multiport, Char-Devices pro Port |
| Userspace-Daemon | [system/daemons/vdagent/](../system/daemons/vdagent/) | SPICE VDAgent-Protokoll, Clipboard-Bridge |
| Konfig-Schema | [system/daemons/vdagent/src/config.rs](../system/daemons/vdagent/src/config.rs) | confd-Manifest unter `services/vdagent` |
| Service-Definition | [system/svc/src/main.rs](../system/svc/src/main.rs) | Eintrag in `services/vdagent/config` |
| Default-Conf | [defaults/System/etc/vdagent.conf](../defaults/System/etc/vdagent.conf) | Referenz-Doku der Schlüssel |
| virtio-input | [kernel/src/drivers/virtio/input.rs](../kernel/src/drivers/virtio/input.rs) | virtio-mouse-pci / virtio-keyboard-pci → Maus-/Tastatur-Events für SPICE-Viewer |

## Kernel: virtio-input (SPICE Maus & Tastatur)

SPICE-Viewer leitet Maus- und Tastatureingaben **nicht** über emuliertes PS/2,
sondern über QEMUs Input-Subsystem (`virtio-mouse-pci`, `virtio-keyboard-pci`).
Ohne den virtio-input-Treiber bleibt der Gast-Cursor im SPICE-Viewer
eingefroren — die GTK-Anzeige funktioniert weiterhin, weil deren Events durch
die PS/2-/vmmouse-Pfade laufen.

PCI IDs: `1AF4:1052` (modern, bevorzugt) und `1AF4:1040` (transitional).

Der Treiber

- richtet die `eventq` (Index 0, device-writable) und `statusq` (Index 1,
  device-readable, aktuell Stub) ein,
- klassifiziert das Gerät beim Probe via `VIRTIO_INPUT_CFG_EV_BITS` (BTN_LEFT
  + REL → Mouse, BTN_LEFT + ABS → Tablet, sonst → Keyboard),
- akkumuliert `EV_REL` (REL_X / REL_Y / REL_WHEEL) und Button-Änderungen
  (`BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`) bis `EV_SYN` und schiebt **eine**
  `MouseEvent`-Sequenz pro Frame in `crate::drivers::input::mouse::MOUSE_BUFFER`,
- übersetzt Tastatur-`EV_KEY` Linux-Keycodes in PS/2 Set‑1 Scancodes (mit
  optionalem `0xE0`-Präfix für extended Keys) und ruft
  `keyboard::handle_scancode()` auf — der Compositor sieht keine Unterschiede
  zur PS/2-/USB-HID-Eingabe.

Aktivierung in `scripts/run.sh --spice` (zusätzlich zu `-spice` und
`virtserialport` für vdagent):

```
-device virtio-mouse-pci -device virtio-keyboard-pci
```

Limitierungen / TODOs

- Tablet (absolute Positionierung über `EV_ABS`) ist Stub. Erfordert das Lesen
  der `VIRTIO_INPUT_CFG_ABS_INFO`-Min/Max-Werte und Skalierung wie im
  USB-Tablet-Pfad.
- Keine Force-Feedback / LED-Status-Updates über die `statusq`.
- Keycode-Tabelle deckt Standardlayout (alphanumerisch, F1-F12, Navigation,
  Modifier, Numpad, Super/Menü) ab. Multimedia-/Browser-Keys nicht abgebildet.

## Kernel: virtio-serial Multiport

Der Treiber handelt `VIRTIO_CONSOLE_F_MULTIPORT` aus, richtet die
Kontroll-Queues 2/3 ein und legt pro Port eigene RX/TX-Queues an
(Port 0: Queues 0/1; Port N≥1: Queues 2(N+1)/2(N+1)+1).

Für jede vom Host gemeldete `PORT_NAME`-Nachricht werden zusätzliche
Aliase im VFS registriert:

| Pfad | Wann verfügbar |
|---|---|
| `/dev/vport0`, `/dev/vport1` ... | Immer (pro vom Host annoncierter Port) |
| `/dev/virtio-ports/<name>` | Sobald PORT_NAME ankommt — kanonischer Pfad |
| `/dev/vport-spice` | Wenn `name == com.redhat.spice.0` |
| `/dev/vport-webdav` | Wenn `name == org.spice-space.webdav.0` |
| `/dev/vport-qga` | Wenn `name == org.qemu.guest_agent.0` |

## Userspace: vdagent

Der Daemon liest die Konfiguration aus confd (`services/vdagent/config`),
öffnet den konfigurierten Port mit dieser Resolution-Reihenfolge:

1. `device_path` (falls explizit gesetzt)
2. `/dev/virtio-ports/<port_name>` — kanonisch
3. `/dev/vport-<alias>` — Kurzform (nur für well-known Namen)
4. `/dev/vport0` — letzter Fallback (Legacy-Single-Port)

Anschließend werden `VD_AGENT_ANNOUNCE_CAPABILITIES` an den Host gesendet
(Capabilities: `CLIPBOARD_BY_DEMAND`, `CLIPBOARD_SELECTION`) und in einer
Schleife eingehende SPICE-Nachrichten verarbeitet.

### Unterstützte Nachrichten

| Type | Richtung | Aktion |
|---|---|---|
| `ANNOUNCE_CAPABILITIES` | beide | Capabilities-Austausch |
| `CLIPBOARD_GRAB` | Host → Gast | Host hat neue Daten — wir fordern an |
| `CLIPBOARD_REQUEST` | Host → Gast | Host will unsere Daten — Compositor-Clipboard senden |
| `CLIPBOARD` | beide | Eigentliche UTF-8-Daten |
| `CLIPBOARD_RELEASE` | Host → Gast | Host hat Clipboard freigegeben |

Lokal überwacht ein 500-ms-Polling-Loop das Compositor-Clipboard und sendet
bei Änderung `CLIPBOARD_GRAB` an den Host. Der Intervall ist konfigurierbar.

## Konfiguration via confd

Alle Werte unter `services/vdagent/config` (System-Scope):

| Schlüssel | Typ | Default | Bedeutung |
|---|---|---|---|
| `enabled` | bool | `true` | Master-Schalter — `false` beendet den Daemon sauber |
| `port_name` | string | `com.redhat.spice.0` | virtio-serial-Portname auf Host-Seite |
| `device_path` | string | `""` | Override für den Geräte-Pfad (sonst aus `port_name`) |
| `clipboard_poll_ms` | int | `500` | Compositor-Clipboard-Polling-Intervall (50–60000) |
| `idle_sleep_ms` | int | `50` | Hauptschleifen-Schlaf wenn idle (1–5000) |
| `log_level` | string | `info` | `error` \| `warn` \| `info` \| `debug` \| `trace` |
| `max_clipboard_bytes` | int | `65536` | Sanity-Cap für Clipboard-Payloads (256–4 MiB) |

### Beispiele

```bash
# Aktuellen Wert lesen
confctl get /services/vdagent/config/log_level

# Debug-Logging aktivieren ohne Neustart (greift beim nächsten vdagent-Start)
confctl set /services/vdagent/config/log_level debug

# vdagent komplett deaktivieren
confctl set /services/vdagent/config/enabled false
svc restart vdagent

# Anderen Portnamen verwenden (z.B. eigener virtserialport in QEMU)
confctl set /services/vdagent/config/port_name com.example.clip.0
svc restart vdagent
```

## QEMU-Setup

Aus `scripts/run.sh`:

```bash
# Voller SPICE-Display + Clipboard
./scripts/run.sh --spice
# entspricht:
qemu-system-x86_64 ... \
  -spice port=5930,disable-ticketing=on \
  -device virtio-serial \
  -chardev spicevmc,id=vdagent,debug=0,name=vdagent \
  -device virtserialport,chardev=vdagent,name=com.redhat.spice.0
# Verbinden: remote-viewer spice://localhost:5930

# SPICE-Only Modus (built-in viewer als einziges Display)
./scripts/run.sh --spice-app
# entspricht zusätzlich:
qemu-system-x86_64 ... \
  -spice port=5930,disable-ticketing=on \
  -device virtio-keyboard-pci \
  -display spice-app
# Maus läuft über das vdagent-Protokoll (VD_AGENT_MOUSE_STATE), absolute
# Koordinaten direkt vom SPICE-Client; vdagent ruft CMD_INJECT_POINTER auf.
# Tastatur ist nicht Teil des vdagent-Protokolls und läuft ausschliesslich
# über virtio-keyboard-pci (Linux-Keycodes → PS/2 Set 1 in
# kernel/src/drivers/virtio/input.rs).

# Nur Clipboard-Sync (GTK-Display bleibt). QEMU 6.1+
./scripts/run.sh --clipboard
# nutzt -chardev qemu-vdagent,clipboard=on
```

## Diagnose

### Daemon läuft?

```
svc status vdagent
ps | grep vdagent
```

Health-States: `starting` → `ready` (oder `failed:virtio_serial_missing`,
`disabled`).

### Kernel sieht den Port?

```
ls /dev/vport*
ls /dev/virtio-ports/
```

Erwartet: `/dev/vport0`, `/dev/virtio-ports/com.redhat.spice.0`,
`/dev/vport-spice`.

Im Kernel-Log (`dmesg`) bei aktivem `serial_verbose`:

```
VirtIO Serial: probing PCI 1af4:1043
  virtio-serial: multiport=true max_nr_ports=31 (using 8)
  virtio-serial: registered /dev/vport0
  virtio-serial: PORT_ADD port=1 ack
  virtio-serial: PORT_NAME port=1 name=com.redhat.spice.0
  virtio-serial: aliased port 1 as /dev/virtio-ports/com.redhat.spice.0
  virtio-serial: aliased port 1 as /dev/vport-spice
```

### Vom Host kommt nichts an?

Mit `log_level=debug` zeigt vdagent jeden Protokollschritt:

```
confctl set /services/vdagent/config/log_level debug
svc restart vdagent
dmesg | grep vdagent
```

Erwartet bei einer Host-→-Gast-Kopie:

```
vdagent[DEBUG]: host grabbed clipboard
vdagent[DEBUG]: received clipboard data (42 bytes) ← host
```

## Bekannte Einschränkungen

- **Nur UTF-8-Text** — Bilder/HTML/RTF werden noch nicht übersetzt
  (entspricht Phase 5 im SPICE-Implementierungsplan).
- **Mausmodus absolut** ist nicht implementiert — der Cursor folgt PS/2-relativ.
- **Display-Resolution-Sync** (`MONITORS_CONFIG`) noch nicht angebunden.
- **Datei-Transfer (Drag&Drop)** noch nicht implementiert.

Die offenen Punkte sind in `todos/github-issue-import.md` als SPICE-/virtio-
input-Folgeaufgaben zusammengefasst.

## Referenzen

- [SPICE Protocol Specification — VDAgent](https://www.spice-space.org/spice-protocol.html)
- [virtio-v1.2 §5.3 Console Device](https://docs.oasis-open.org/virtio/virtio/v1.2/csd01/virtio-v1.2-csd01.html#x1-2900003)
- [linux-vdagent (Reference Implementation)](https://gitlab.freedesktop.org/spice/linux/vd_agent)
