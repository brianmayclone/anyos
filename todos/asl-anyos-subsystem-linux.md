# ASL - AnyOS Subsystem for Linux

## Zielbild

ASL steht fuer **AnyOS Subsystem for Linux**.

ASL ist die offizielle Linux-Kompatibilitaetsplattform fuer anyOS auf Basis
einer leichtgewichtigen, vom anyOS-Hypervisor kontrollierten Linux-VM mit
starker Desktop-, Filesystem- und Entwickler-Integration.

Das Produktziel ist explizit **nicht** "eine weitere VM-App", sondern ein
betrieblich sauberes Subsystem mit:

- reproduzierbarer Linux-Runtime
- klarer Sicherheits- und Ressourcenisolation
- enger Integration in Terminal, Editor, Netzwerk und Dateisystem
- service-faehigem Lifecycle
- spaeter optionaler GUI-App-Integration

---

## Executive Summary

ASL ist fuer anyOS ein realistischer Ausbaupfad, weil zentrale Vorleistungen
bereits existieren:

- Kernel-Virtualisierungssyscalls (`vm_create`, `vcpu_run`, `vm_set_memory`)
- x86-Backends fuer VMX/SVM
- virtio-nahe Treiber- und Integrationsbausteine
- bestehendes Service-Modell via `svc`
- bereits vorhandene Host/Guest-Integrationsmuster wie `vdagent`

Der Architekturansatz fuer ASL lautet daher:

- **Linux laeuft in einer gemanagten Utility-VM**
- **anyOS bleibt der Host und Policy-Owner**
- **Integration erfolgt ueber klar definierte Host-Services und paravirtuale Kanaele**
- **Datei-, Terminal- und Netzwerk-UX werden systemisch gelost, nicht app-lokal**

Die bereits festgezogenen Leitentscheidungen sind:

- ASL ist ausschliesslich WSL2-artig
- Linux-Instanzen werden als verwaltete Distributionen modelliert
- Rootfs wird geschichtet aus Base- und Overlay-Layer aufgebaut
- NAT ist der Standard-Netzwerkmodus
- Shared Folders sind explizite, brokered Exportpunkte
- `asl-agent` ist Standardbestandteil offizieller Distros, aber nicht bootkritisch

ASL sollte in drei Produktstufen gebaut werden:

1. **ASL Foundation**
Linux bootet, Terminal funktioniert, Netzwerk vorhanden, Rootfs verwaltbar
2. **ASL Developer Edition**
Shared Folders, Port-Forwarding, Tooling, Session-Resume, bessere DX
3. **ASL Desktop Integration**
Linux-GUI-Apps, Clipboard, Notifications, ggf. Wayland/X11-Bridge

---

## Produktumfang

### In Scope fuer v1

- Verwaltete Linux-VM pro Benutzer oder pro Systeminstanz
- Import und Update von Linux-Rootfs-Images
- Start/Stop/Status ueber Service und CLI
- Interaktive Shell-Sessions aus anyOS Terminal
- TCP/IP-Konnektivitaet fuer den Gast
- Shared Folder zwischen anyOS und Linux
- Port-Forwarding Linux -> anyOS
- Ressourcenlimits fuer CPU, RAM, Disk
- Persistenter Gastzustand auf Volume-Ebene
- Basisobservability: Logs, Status, Exit/Ursachen, Metriken

### Out of Scope fuer v1

- Vollstaendige Linux-Syscall-Kompatibilitaet ohne VM
- Perfekte POSIX-Semantik auf Host-Filesystem-Ebene
- Container-Orchestrierung
- Kubernetes-kompatible Plattformfeatures
- Mehrmandantenfaehigkeit mit harten Enterprise-SLA-Garantien
- GPU-Beschleunigung im Linux-Gast
- Native Linux-GUI-Apps ohne zusaetzliche Integrationsschicht

---

## Architekturprinzipien

1. **Ausschliesslich WSL2-artig**
ASL wird ausschliesslich als kontrollierte Linux-Utility-VM gebaut. Ein
WSL1-artiger Syscall-/ABI-Kompatibilitaetslayer gehoert nicht zur Architektur
und wird auch nicht als spaeterer Alternativpfad verfolgt.

2. **Host owns policy**
Ressourcen, Mounts, Netzfreigaben, Lifecycle und Rechte werden durch anyOS
definiert und nicht durch ad-hoc Gastkonfiguration.

3. **Integration ueber wenige stabile Kanaele**
Statt vieler Spezialpfade bekommt ASL wenige klar versionierte Protokolle:
Control, Console, Filesystem, Network, Clipboard, Metrics.

4. **Default secure, opt-in rich integration**
Shared Folders, Host-Socket-Exposure und GUI-Integration muessen aktiv und
sichtbar freigeschaltet werden.

5. **Serviceability vor Feature-Fuelle**
Bootbarkeit ist nicht gleich Produktreife. Logs, Diagnose, Fehlerzustaende,
Wiederanlauf und Updates sind von Anfang an Teil des Designs.

---

## Zielarchitektur

### Schichtenmodell

```text
+--------------------------------------------------------------+
| anyOS User Experience                                        |
| Terminal | Finder | anyOS Code | Settings | Taskmanager      |
+--------------------------------------------------------------+
| ASL Client Layer                                             |
| aslctl | Terminal connector | Settings panel | Port broker   |
+--------------------------------------------------------------+
| ASL Host Services                                            |
| asld | aslfsd | aslnetd | aslportd | aslconsoled | aslobsd   |
+--------------------------------------------------------------+
| anyOS Platform                                               |
| svc | CoreFS | IPC | SHM | Event Bus | Network Stack | VFS   |
+--------------------------------------------------------------+
| Hypervisor Layer                                             |
| vm_* / vcpu_* syscalls | VMX/SVM backend | virt device model |
+--------------------------------------------------------------+
| Linux Guest                                                  |
| Kernel | init/system manager | agent | shell | userland      |
+--------------------------------------------------------------+
```

### Kernkomponenten

#### 1. `asld` - ASL Control Plane Daemon

Zentrale Steuerinstanz fuer:

- VM-Erzeugung und Lifecycle
- Rootfs-Registrierung
- Ressourcenprofile
- Session-Verwaltung
- Sicherheits-Policy
- Konfigurationsowner ueber `confd`
- Health-State des Subsystems

Verantwortung:

- einzig autoritative Instanz fuer den Zustand einer ASL-Distribution
- Besitzer der VM-ID, vCPU-Konfiguration und Memory-Map
- startet Folgekomponenten oder bindet sie logisch an eine Distribution

Empfohlener Pfad:

- Source: `system/daemons/asld/`
- Binary: `/System/bin/asld`
- Service config: `/System/etc/svc/asld`

#### 2. `aslctl` - Admin- und User-CLI

CLI fuer Verwaltung und Benutzung:

- `aslctl list`
- `aslctl import <rootfs>`
- `aslctl create <name> --profile dev`
- `aslctl start <name>`
- `aslctl stop <name>`
- `aslctl shell <name>`
- `aslctl exec <name> -- <cmd>`
- `aslctl mount add <name> ...`
- `aslctl port list <name>`
- `aslctl logs <name>`
- `aslctl doctor <name>`

Empfohlener Pfad:

- Source: `bin/aslctl/`
- Binary: `/System/bin/aslctl`

#### 3. `asl-agent` - Gastagent in Linux

Leichtgewichtiger Linux-Gastagent fuer:

- Session-/Console-Multiplexing
- Heartbeats und Readiness
- Mount-Metadaten
- saubere Shutdown-/Suspend-Koordination
- einfaches Gastinventar fuer Status und Diagnose

Spaeter optional:

- Clipboard, Notifications, GUI-Hooks
- detaillierteres Port- und Prozessinventar

Der Agent ist Standardbestandteil offizieller ASL-Distributionen, darf aber
nicht die Grundfunktion blockieren. Linux muss auch ohne funktionsfaehigen
Agent noch booten und via Fallback-Konsole erreichbar sein.

#### 4. `aslfsd` - Shared Filesystem Broker

Host-seitiger Dateifreigabedienst zwischen anyOS und Linux-Gast.

Optionen:

- **v1 empfohlen:** FUSE-basierter Hostdienst mit paravirtualem Transport
- **spaeter optional:** 9p oder virtio-fs-aehnlicher nativer Transport

Warum eigener Dienst:

- Policy, Mapping, Caching und Audit gehoeren nicht in `asld`
- Filesystem-Integration braucht eigene Fehler- und Backpressure-Semantik

#### 5. `aslnetd` - Guest Networking Manager

Host-seitiger Netzbroker fuer:

- Guest-NIC-Bereitstellung
- NAT oder Bridge-Modus
- DNS-/Resolver-Policy
- Host-zu-Gast Erreichbarkeit
- Telemetrie zu offenen Ports und Flows

#### 6. `aslconsoled` - Console and Session Broker

Terminalsitzungen fuer:

- PTY-Bridge Host <-> Gast
- persistente Shell-Session
- `exec` und `shell`
- Terminal-Reconnect nach Fenster- oder Sessionverlust

#### 7. `aslobsd` - Observability and Diagnostics

Sammelt:

- Lifecycle-Ereignisse
- Bootzeiten
- Gast-Heartbeat
- Exit Reasons
- Memory-/CPU-Nutzung
- I/O-Statistiken
- Rate-limited Gastlogs

Kann spaeter in bestehende anyOS-Log- und Event-Viewer-Pfade integriert werden.

---

## Laufzeitmodell

### Distributionsmodell

ASL soll Linux-Instanzen als **Distributionen** behandeln, nicht als lose VM-
Images.

Eine Distribution besteht aus:

- Name und ID
- Linux-Rootfs
- Kernel-Kompatibilitaetsprofil
- Ressourcenprofil
- Mount-Definitionen
- Netzwerkregeln
- Benutzerzuordnung
- Persistenzstore
- Status- und Diagnosedaten

Empfohlene Ablage:

```text
/System/var/asl/
  distros/
    ubuntu-dev/
      images/
        base.img
        overlay.img
        state.img
      runtime/
      logs/
      sockets/
```

Autoritative Konfiguration liegt nicht als lose Datei im Distro-Verzeichnis,
sondern in `confd` unter:

```text
system/platform/asl/distros/<name>/...
```

Das Distro-Verzeichnis bleibt fuer Images, Runtime-Artefakte, Logs und Sockets
zustaendig.

### VM-Modell

V1-Empfehlung:

- eine VM pro Distribution
- initial ein Benutzerkontext pro Distribution
- spaeter optional mehrere Benutzer-Sessions auf derselben Distribution

Begruendung:

- deutlich einfacher fuer Policy, Debugging und Recovery
- klare Ressourcen- und Ownership-Grenzen

### Boot-Modell

Bootfolge:

1. `asld` liest Distributionskonfiguration
2. Linux-Kernel und initrd werden geladen
3. VM wird via `vm_create` und `vcpu_create` aufgebaut
4. virtio-/paravirt-Geraete werden exponiert
5. Gast bootet mit minimalem Init
6. `asl-agent` meldet Readiness
7. `aslconsoled`, `aslfsd`, `aslnetd` haengen Integrationskanaele an
8. Distribution wird als `running` markiert

Wichtig:

- der Gast darf auch ohne Agent in einen degradierten, aber erreichbaren Zustand
  booten
- `running` und `agent=ready` sind bewusst nicht dasselbe

State Machine:

- `created`
- `starting`
- `booting`
- `ready`
- `degraded`
- `stopping`
- `stopped`
- `failed`
- `repairing`

---

## Virtualisierungsarchitektur

### Hypervisor-Nutzung

ASL nutzt die bereits vorhandenen anyOS-VM-Syscalls als unterste Schnittstelle.
Die Verantwortung wird wie folgt getrennt:

- Kernel/Hypervisor:
  - VM-Ressourcen
  - vCPU-Execution
  - Guest Memory Mapping
  - Interrupt Injection
  - Hardware-Virtualisierung
- `asld`:
  - VM-Zusammenbau
  - Geraetemodell-Konfiguration
  - Policy und Lifecycle

### Device-Modell

Empfohlene v1-Geraete:

- paravirt-Konsole oder virtio-serial-artiger Kontrollkanal
- paravirt-Blockdevice fuer Rootfs
- paravirt-Netzdevice
- Entropiequelle
- optional Ballooning spaeter

Nicht fuer v1 priorisieren:

- GPU
- Audio
- USB-Passthrough
- komplexe PCI-Topologien

### Memory-Modell

V1:

- fester RAM pro Distribution
- statische Guest-Phys-Memory-Regionen
- kein Memory Overcommit

V2:

- dynamische Ballooning-Strategie
- heuristische Idle-Reclamation
- Hostdruck-basierte Obergrenzen

### Snapshot- und Resume-Strategie

Nicht direkt Vollsnapshot auf v1 zwingen.

Reihenfolge:

1. sauberes Stop/Start
2. Fast Boot durch warmen Page Cache und persistente Rootfs-Layer
3. spaeter VM-Suspend/Resume
4. spaeter konsistente Snapshots

---

## Storage-Architektur

### Rootfs-Format

Empfehlung fuer v1:

- read-only Base Image
- separater writable Overlay-/Diff-Layer
- optional separater State-/Data-Layer
- klare Trennung zwischen:
  - importiertem Linux-Image
  - persistenten Gastaenderungen
  - Benutzerdaten

Moegliche Struktur:

```text
base.img
overlay.img
state.img
```

Ziele:

- Updates des Basisimages ohne Datenverlust
- Rollback-Faehigkeit
- Reparierbarkeit
- kleinere Delta-Backups

Betriebsregeln:

- `base.img` wird nie in-place beschrieben
- `overlay.img` ist exklusiv an eine Distribution gebunden
- `state.img` ist fuer mutable Laufzeit- und Benutzerdaten reservierbar
- Distributionen werden logisch exportiert und geklont, nicht nur als rohe
  Einzeldateien behandelt

### Host <-> Guest Shared Folders

Empfehlung fuer v1:

- explizit deklarierte Mounts
- Standardmount nur nach Benutzerfreigabe
- kein pauschaler Vollzugriff auf Host-Home
- Broker-Modell ueber `aslfsd`
- keine direkte CoreFS-Durchreichung in den Gast

Mount-Objekte enthalten mindestens:

- `host_path`
- `guest_path`
- `mode`
- `metadata_mode`
- `case_mode`
- `exec_policy`
- `watch_policy`

Mount-Modi:

- `readonly`
- `readwrite`

Metadata-Modi:

- `strict`
- `relaxed`

Case-Modi:

- `host-native`
- `case-sensitive`
- `case-folded`

Exec-Policies:

- `inherit`
- `noexec`
- `host-metadata`

Watch-Policies:

- `best-effort`
- `off`

Wichtige Designfragen:

- UID/GID-Mapping
- Dateiattribute und Executable-Bits
- Symlink-Politik
- Locking-Semantik
- Inotify-/Watcher-Aequivalent
- Crash-Recovery bei Host-/Guest-Neustart

Empfehlung:

- v1 klare Dokumentation, dass Host-Shared-Folders nicht 100 Prozent POSIX-
  identisch sind
- Shared Folders als POSIX-nahe Entwicklungsfreigaben positionieren, nicht als
  vollstaendige Linux-Dateisysteme
- `aslfsd` bleibt Sicherheits- und Semantikgrenze
- fuer Build-Systeme und Paketmanager eigene Linux-native Verzeichnisse im
  Gast empfehlen

### Integration mit CoreFS

ASL sollte CoreFS nicht direkt mit Linux-POSIX-Semantik ueberfrachten.

Stattdessen:

- ASL bekommt einen Broker-Layer fuer Semantikuebersetzung
- Hostseitige Freigaben werden als kontrollierte Exportpunkte behandelt
- CoreFS bleibt Host-Filesystem, nicht Linux-kompatible Universal-Abstraktion

---

## Netzwerkarchitektur

### v1-Empfehlung: NAT als Default

Standardmodus:

- Gast erhaelt private IP
- ausgehende Verbindungen erlaubt
- eingehende Verbindungen nur ueber explizite Port-Freigaben
- DNS-Aufloesung ueber den Host-Broker
- Default-Exponierung nur lokal ueber `127.0.0.1`

Vorteile:

- deutlich einfacher und sicherer
- stabile DX fuer Paketmanager, `git`, `curl`, `ssh`

### Port-Forwarding-Modell

`aslportd` oder Teil von `aslnetd`:

- erkennt offene oder freigegebene Gastports
- bietet regelbasiertes Forwarding `host_port -> guest_port`
- kann Ports im anyOS-UI sichtbar machen

Beispiele:

- `localhost:3000` im Gast wird auf Host sichtbar
- IDE/Browser koennen Dev-Server automatisch finden

### Erweiterungen spaeter

- Bridge-Modus
- mDNS/Service Discovery
- Hostname-Aufloesung Host <-> Gast
- per-App Firewall-Regeln

---

## Terminal- und Prozessintegration

### Ziel

Linux soll aus anyOS-Terminal heraus wie ein erstklassiger Arbeitskontext
nutzbar sein.

### Betriebsarten

1. **Interactive shell**
`aslctl shell ubuntu-dev`

Fallback:
`aslctl shell ubuntu-dev --fallback-console`

2. **Single command execution**
`aslctl exec ubuntu-dev -- cargo test`

3. **Persistent named sessions**
vergleichbar mit gemanagtem `tmux`-artigen Verhalten, aber systemisch
kontrolliert, z. B. `aslctl shell ubuntu-dev --session dev`

### Integrationspunkte

- anyOS Terminal startet Shell in einer ASL-Distribution
- anyOS Code kann spaeter Build/Run Tasks im Gast starten
- Finder oder File-Dialogs koennen "Open Linux Shell here" anbieten

### PTY-Modell

`aslconsoled` verwaltet:

- Host-PTY
- Transport in den Gast
- Resize-Events
- UTF-8-/Escape-Sequenz-Passthrough
- Session-Recovery

Wichtig:

- PTY/TTY-Semantik nicht im Terminal-UI verstecken
- Sitzungen muessen bei UI-Absturz erhalten bleiben koennen
- agent-sensitive und agent-unabhaengige Pfade sauber unterscheiden
- degradiertes Shell-Verhalten sichtbar machen statt generisch zu scheitern

---

## Desktop-Integration

### v1

- Terminal
- Port-Freigaben
- Shared Folders
- Logs/Status in UI
- Copy/Paste fuer Text optional, aber nicht kritisch

### v2

- Clipboard-Synchronisation
- Linux-Prozessliste in Taskmanager
- "Open in Linux" aus Finder oder Editor
- Benachrichtigungen aus Gastprozessen

### v3

- Linux-GUI-Apps via Wayland- oder X11-Bridge
- Fenster als erstklassige anyOS-Fenster
- Icon-, Titel- und Lifecycle-Integration
- Audio-/Clipboard-/DPI-/IME-Unterstuetzung

Wichtige Entscheidung:

Linux-GUI-Apps sind ein eigenes Teilprogramm. Nicht in v1 mitschleppen.

---

## Sicherheitsmodell

### Trust Boundaries

ASL fuehrt untrusted oder semi-trusted Linux-Workloads aus. Deshalb gelten
folgende Grenzen:

- Linux-Gast ist kein privilegierter Hostbestandteil
- Gast darf ohne explizite Freigabe nicht auf Host-Dateien zugreifen
- Gastports sind nicht automatisch nach aussen exponiert
- Gastagent ist funktional, aber nicht allmaechtig und kein
  Vertrauensanker fuer Host-Sicherheit

### Berechtigungsmodell

Fuer ASL benoetigt anyOS eine klare Capability-Schicht fuer Hypervisor- und
Subsystemkontrolle.

Empfohlene neue oder geschlossene Host-Rechte:

- `hypervisor`
- `asl.manage`
- `asl.mount`
- `asl.port_forward`
- `asl.inspect`

### Mandatory Controls

- Ressourcenlimits pro Distribution
- begrenzte Host-Mounts
- explizite Portfreigaben
- Auditierbare Konfigurationsaenderungen
- Signatur oder Trust-Metadaten fuer importierte Rootfs-Images

### Threats

- manipuliertes Rootfs-Image
- Escape ueber shared-memory-nahe Kontrollkanaele
- Host-Dateizugriff ueber zu breite Mount-Freigaben
- Port-Leaks
- Denial of Service ueber CPU/RAM/Disk-Flooding

### Security-Maßnahmen

- minimaler paravirt-Angriffsraum
- versionierte Protokolle
- Input-Validierung an allen Broker-Grenzen
- Rate Limits
- per-Distro Quotas
- Read-only Base Images
- Standard-Policy: deny by default

---

## Konfiguration und Management

### Konfigurationsobjekt

Beispiel:

```json
{
  "name": "ubuntu-dev",
  "kernel_profile": "linux-x86_64-generic",
  "memory_mb": 2048,
  "vcpu_count": 2,
  "network_mode": "nat",
  "auto_start": false,
  "mounts": [
    {
      "host_path": "/Users/strati/projects",
      "guest_path": "/mnt/projects",
      "mode": "readwrite"
    }
  ],
  "ports": [
    {
      "host_addr": "127.0.0.1",
      "host_port": 3000,
      "guest_port": 3000,
      "proto": "tcp"
    }
  ]
}
```

### Verwaltungsoberflaechen

- CLI via `aslctl`
- spaeter Settings-Panel
- spaeter Finder-/Taskmanager-Integration

### Enterprise-taugliche Verwaltungsfunktionen

- Import/Export
- Konfig-Validation
- Drift Detection
- Diagnosepaket
- klare Fehlercodes
- menschenlesbare und maschinenlesbare Statusausgaben

---

## Observability und Betrieb

### Mindestanforderungen

- strukturierte Logs pro Komponente
- Event-Timeline pro Distribution
- Health-Probes
- Bootdauer-Metriken
- CPU/RAM/Disk/Netz-Nutzung
- Exit- und Restart-Grund

### Doctor-Workflow

`aslctl doctor <name>` sollte pruefen:

- VT-x/AMD-V verfuegbar
- notwendige Kernel-Syscalls funktionsfaehig
- Rootfs konsistent
- Mount-Pfade erreichbar
- Netzpfad gesund
- Agent-Status sauber zwischen `ready`, `degraded` und `disconnected`
  unterscheidbar
- Port-Broker aktiv

### Restart-Strategie

- abgestufte Restarts pro Komponente
- `asld` darf Gast nicht in inkonsistente Dauerschleifen schicken
- exponentielles Backoff
- `failed` State mit Diagnosehinweisen

---

## API- und Protokolldesign

### Grundsatz

Alle ASL-internen Protokolle muessen:

- versioniert sein
- klar dokumentierte Ownership fuer Buffer und Handles haben
- Timeouts und Fehlercodes definieren
- future-proof erweitert werden koennen

### Benoetigte Protokolle

1. **Control Protocol**
Host-CLI/UI <-> `asld`

2. **Guest Agent Protocol**
`asld`/Broker <-> `asl-agent`

3. **Console Protocol**
`aslconsoled` <-> Gast-PTY/Agent

4. **Filesystem Broker Protocol**
`aslfsd` <-> Gast-FS-Client

5. **Port and Network Protocol**
`aslnetd` <-> Gastagent oder Kernel-Hooks

6. **Metrics Protocol**
Broker/Agent -> `aslobsd`

---

## Empfohlene Repository-Struktur

```text
system/
  daemons/
    asld/
    aslfsd/
    aslnetd/
    aslconsoled/
    aslobsd/
  utilities/
    aslctl/
  asl/
    docs/
    schemas/
    guest-agent/
sysroot/
  System/
    etc/
      svc/
        asld
```

Optional spaeter:

```text
apps/
  asl-settings/
```

---

## Umsetzungsplan

## Phase 0 - Architectural Foundation

Ziel:

- Architekturentscheidungen festziehen
- Scope kontrollieren
- Schnittstellen sauber schneiden

Arbeitspakete:

- Namens- und Produktentscheidung `ASL`
- Distro-Lifecycle und Statusmodell definieren
- geschichtetes Distro-/Rootfs-Modell festlegen
- minimale Geraetematrix fuer v1 festlegen
- Host-zu-Gast Protokollgrenzen definieren
- Security Baseline dokumentieren
- Rolle des `asl-agent` und Fallback-Konsole festziehen

Ergebnisse:

- Architektur-ADR
- Konfigschema v1
- CLI-Kommandomodell
- Status- und Fehlercodekatalog

## Phase 1 - Bootable Linux Utility VM

Ziel:

- Linux bootet stabil unter anyOS

Arbeitspakete:

- `asld` Grundgeruest
- VM-Build-Up ueber vorhandene Hypervisor-Syscalls
- Kernel + initrd Loading
- paravirt-Konsole
- Blockdevice fuer Rootfs
- Gast-Readiness ueber einfachen Heartbeat

Akzeptanzkriterien:

- `aslctl start demo` bootet Linux
- serielle oder paravirt-Konsole zeigt Login-Shell
- `aslctl stop demo` beendet sauber

## Phase 2 - Basic Productization

Ziel:

- benutzbares Subsystem fuer CLI-Workloads

Arbeitspakete:

- `aslctl shell` und `aslctl exec`
- degradierten Fallback fuer Shell-Zugriff
- NAT-Netzwerk
- DNS-Aufloesung
- Import von Rootfs-Tarball oder Image
- Persistenz von Overlay-Layer
- Logging und Statusabfrage

Akzeptanzkriterien:

- `apt`, `git`, `curl`, `ssh` im Gast funktionieren
- Neustart der Distribution behaelt Daten
- Logs und Status sind nachvollziehbar

## Phase 3 - Developer Integration

Ziel:

- gute Entwicklererfahrung

Arbeitspakete:

- Shared Folders
- Port-Forwarding
- Session-Reconnect
- Tooling fuer Diagnose
- konkrete Mount-Policies fuer Metadata/Case/Exec/Watch
- evtl. Editor-Integration

Akzeptanzkriterien:

- Projektordner kann kontrolliert in Gast gemountet werden
- Dev-Server im Gast sind vom Host aus nutzbar
- Shell-Sitzung ueberlebt UI-Neustart

## Phase 4 - Hardening

Ziel:

- robustes, supportbares System

Arbeitspakete:

- Ressourcenquoten
- Fehlerbehandlung und degraded states
- Testmatrix
- Image-Trust und Import-Validierung
- Recovery-Werkzeuge

Akzeptanzkriterien:

- definierte Fehlerfaelle fuehren nicht zu Host-Instabilitaet
- wiederholte Start/Stop-Zyklen sind stabil
- `aslctl doctor` liefert verwertbare Diagnose

## Phase 5 - Desktop Integration

Ziel:

- Linux fuehlt sich als Subsystem statt als isolierte VM an

Arbeitspakete:

- Clipboard
- Notifications
- Finder-/Taskmanager-Integration
- optional GUI-App-Bridge

Akzeptanzkriterien:

- zentrale UX-Flows lassen sich ohne manuelle Spezialkommandos nutzen

---

## Technische Risiken

### 1. Shared Filesystem wird semantisch unsauber

Risiko:

- Linux-Tools erwarten POSIX-Verhalten, Host-FS liefert das nicht vollstaendig

Gegenmassnahme:

- klare Dokumentation
- conservative defaults
- Linux-native Arbeitsverzeichnisse fuer sensible Workloads empfehlen

### 2. Hypervisor-Pfad ist funktional, aber noch nicht produkthaft

Risiko:

- Race Conditions, Memory-Fehler, unvollstaendige Device-Modelle

Gegenmassnahme:

- v1 mit minimalem Device-Set
- aggressive Test- und Fault-Injection-Strategie
- keine voreilige GPU-/USB-Komplexitaet

### 3. Agent wird Single Point of Failure

Risiko:

- Subsystem haengt an zu viel Logik im Gastagenten

Gegenmassnahme:

- Bootpfad ohne Agent funktionsfaehig halten
- Agent nur fuer "rich integration", nicht fuer Grundfunktion

### 4. UX kippt in "nur eine VM"

Risiko:

- Start, Shell, Dateizugriff und Ports fuehlen sich schwergewichtig an

Gegenmassnahme:

- Fokus auf Terminal, Shared Folders und Port-Broker als erste UX-Hebel

---

## Teststrategie

### Testebenen

- Unit-Tests fuer Control- und Konfiglogik
- Integrations-Tests fuer `asld` mit Mock-Hypervisor oder Test-VM
- End-to-End-Tests fuer Start, Shell, Netzwerk, Shared Folder
- Chaos-Tests fuer Restart, Agent-Ausfall, Hostdienst-Ausfall
- Security-Tests fuer Freigaben, Grenzen und Input-Validation

### Pflichtszenarien

- Import einer Distribution
- erster Boot
- Shell-Session
- Dateizugriff ueber Shared Folder
- Port-Forwarding fuer Webserver
- Stop/Start ohne Datenverlust
- kaputtes Rootfs
- fehlender Agent
- Netzwerk nicht verfuegbar

---

## MVP-Empfehlung

Wenn ASL schnell echten Wert liefern soll, sollte das MVP **eng** geschnitten
werden:

- genau eine offiziell unterstuetzte Linux-Distribution
- x86_64 only
- NAT only
- Terminal only
- Shared Folders nur explizit
- keine GUI-Apps
- keine Snapshots
- keine dynamische Memory-Ballooning-Logik

Dieses MVP ist klein genug, um lieferbar zu bleiben, und stark genug, um den
praktischen Nutzen zu beweisen.

---

## Konkrete naechste Schritte

1. `ASL` als offiziellen Namen und ausschliesslich WSL2-artigen Ansatz festschreiben.
2. ADRs fuer Distro-/Rootfs-Modell, Netzwerk-Default, Shared Folders und
   `asl-agent` als Basisdokumente pflegen.
3. `asld` und `aslctl` als neue Systemkomponenten anlegen.
4. Minimalen Linux-Bootpfad ueber bestehende VM-Syscalls implementieren.
5. Paravirt-Konsole plus degradierten Fallback-Pfad bauen.
6. Minimalen `asl-agent` fuer Heartbeat und Session-Komfort aufsetzen.
7. Danach Shared Folders und Port-Forwarding entlang der ADRs umsetzen.

---

## Entscheidungsempfehlung

ASL sollte als **strategisches Subsystem** und nicht als Nebenfeature behandelt
werden.

Die richtige Produktthese lautet:

- anyOS bleibt ein eigenstaendiges OS
- Linux wird als produktiv nutzbares, stark integriertes Subsystem bereitgestellt
- die technische Basis ist eine kontrollierte Utility-VM

Damit bleibt die Architektur ehrlich, lieferbar und erweiterbar.

---

## Dev-Plattform Implementierungsplan (2026-05-07)

Zielbild: ASL muss am Ende fuer einen Entwickler nutzbar sein, um unter ASL
Programme zu entwickeln. Sprachen-Scope: **Java, C/C++, Rust, Node.js**.
Editor-Modell: **1c** — Editor laeuft sowohl auf anyOS (anycode, ueber Shared
Mount + `aslctl run`) als auch spaeter in der Distro (Block F, GUI-Forwarding).
Aussenanbindung: **Inbound Port-Forward, Host↔Distro File-Sharing, Git/HTTPS
outbound**.

Detail-Audit der bestehenden Implementierung gegen die Spezifikationen siehe
[../docs/asl-implementation-audit.md](../docs/asl-implementation-audit.md).

### Block A — Workflow-Grundlage (nicht verhandelbar)

#### A1. Outbound-Netzwerk verlaesslich
- [x] DNS-Broker in `aslnetd` ist implementiert (war beim Audit unklar).
      `dns_reply()` in `system/daemons/aslnetd/src/main.rs` nutzt
      `net::dns()` als Resolver, `gateway.asl`/`host.asl`/`dns.asl` werden
      special-mapped.
- [x] `dns_broker_enabled` Config-Flag respektieren: neuer
      `BrokerState.dns_broker_enabled` (Default `true` aus Schema), neuer
      `SET_DNS_BROKER 0|1` IPC-Command, Drop-Counter
      `dns_disabled_drops` im Status. **6 neue Tests gruen** (Toggle,
      garbage args, drop-counter sticky, status surface,
      `is_dns_query`-Heuristik). 2026-05-07.
- [ ] End-zu-End-Test outbound: `aslctl exec <distro> -- curl https://github.com`,
      `apt update`, `git clone https://...`, `cargo fetch` mit ~20 Crates.
      **Blockiert** auf laufende Distro-VM (haengt damit auch an
      Test-Harness-Bringup).
- [ ] TLS-Stack: pruefen dass die Linux-Gast-CA-Bundle-Pfade funktionieren.

#### A2. Inbound Port-Forward
- [ ] `aslctl port add` end-zu-end testen (TCP + UDP).
- [x] ~~Bug fixen: `aslctl port validate` sendet `NETWORK_VALIDATE`~~ — geklaert
      2026-05-07: `port validate` und `network validate` sind CLI-Aliase fuer
      denselben kombinierten Validator in asld. Kein Bug.
- [ ] Live-Workflow: `npm run dev` in Distro auf :3000, von Surf auf anyOS
      `http://127.0.0.1:3000` oeffnen (zurueckgestellt fuer E2E nach B1/B2).
- [ ] Persistenz: Port-Forwards ueberleben Distro-Stop/Start und asld-Restart
      (zurueckgestellt fuer E2E nach B1/B2).

#### A3. Shared-Mounts (Host↔Distro File-Sharing)
- [x] Policy-Validierung in `aslfsd` getestet — `test = false` entfernt,
      `validate_export_fields` mit Length-Guard gegen Panic, **28 Tests
      gruen** (Path-Safety, ID-Format, alle Mode-/Policy-Werte aus
      ADR-0004, Apply/Replace/Clear/Validate-IPC, Status-Counter).
      2026-05-07.
- [ ] Verifizieren dass `aslfsd`-Mounts in der Gast-VM tatsaechlich ankommen
      — virtio-fs / 9P-Anbindung in `system/daemons/asld/src/vm.rs` pruefen.
      **Blockiert** auf laufende Distro-VM.
- [ ] Bidirektionalitaet: anyOS schreibt → Gast `cat` sieht es ohne Remount.
      Umgekehrt genauso. **Blockiert** auf laufende Distro-VM.
- [ ] Permissions/UID-Mapping: `chmod +x build.sh` im Gast bearbeitbar; Builds
      schreiben Artefakte zurueck die anyOS lesen kann.
- [ ] Performance-Smoke: `cargo build` eines mittelgrossen Crates auf
      shared-mounted Source.
- [ ] Watch-Policy: `case_mode`, `exec_policy`, `metadata_mode` aus dem Schema
      in der Gast-VM durchexerzieren (Pure-Validation gruen, Wirkung im
      Gast steht aus).

#### A4. `aslctl run` mit Stdio + Exit-Code
- [x] Subcommand `run --cwd <path> --env KEY=VALUE -- <cmd>...` ergaenzt
      (`bin/aslctl/src/lib.rs`, 2026-05-07). `run` ist Alias zu `exec`,
      identisches Wire-Format (`EXEC ...`). 7 neue Tests, alle gruen.
- [ ] Stdin-Pipe (Backend-seitig — `exec_command` liefert bereits
      `stdin_pipe_name`, Aufrufer-Anbindung in anycode steht aus).
- [ ] Stdout/Stderr getrennt bis zum Aufrufer (heute nur ein
      `stdout_pipe_name` — `stderr` ggf. ueber separate Pipe ergaenzen).
- [ ] Exit-Code propagieren — `ExecInvocation` hat heute `attached_pid` aber
      kein `exit_code`-Feld. Erweitern fuer IDE-Build-Tasks.
- [ ] Backend in `system/daemons/asld/src/runtime.rs` pruefen: `exec_command`
      ist heute Setup + Spawn ohne Wait. Fuer `run` braucht es entweder einen
      synchronen Wait-Pfad oder einen `EXEC_WAIT <exec-id>` Wire-Command.

#### A5. `aslctl logs` + Diagnose-Endpunkte
- [ ] `GetLogs` API in asld (`system/daemons/asld/src/ipc.rs`) — fehlt komplett.
- [ ] Log-Quellen: asld-eigenes Log + Agent-Output + Konsole
      (aslconsoled-Buffer).
- [ ] `aslctl logs <distro> [--follow]` Subcommand.
- [ ] `ListEvents` API + `aslctl events` an aslobsd-Quelle koppeln (CLI heisst
      aktuell `vm-events` — umbenennen oder Alias).
- [ ] `aslctl --json` global einfuehren — Voraussetzung fuer aslmanager-Backend
      (Block D) und Skripting.

### Block B — Distro-Bringup fuer Devs

#### B1. ImportBaseImage und Image-Trust
- [x] Linux-Stub `runtime.rs:1135` ist beabsichtigt (asld auf Linux nur
      fuer Host-Tests, produktiv laeuft auf anyOS — dort funktioniert der
      Import). Geklaert 2026-05-07.
- [x] `RAW_DISK_MIN_BYTES` von `u32` auf `u64` umgestellt mit
      `STAT_SIZE_OVERFLOW_SENTINEL`-Detection — schuetzt vor stillem
      32-bit-stat-Truncation bei >4 GiB Images. ADR-0011 verweist darauf.
- [x] ADR-0011 angelegt: gestaffelte Image-Trust-Strategie.
      Stufe 1 (TLS + URL-Whitelist + Size + MBR) ist heute aktiv und im
      Installer-Log als `WARN: ... ADR-0011 stage 1` sichtbar.
- [ ] **Stufe 2** (TODO, separater Block): SHA512-Hash-Pinning gegen
      versionsgebundene Konstante. Hash-Berechnung in zweitem Read-Pass
      nach Download. Erfordert Streaming-Hash oder einfaches `read_loop`
      ueber bestehende Datei.
- [ ] **Stufe 3** (TODO, blockiert auf GPG in anyOS): SHA512SUMS+sig
      auto-laden, GPG-verify. Eigener Roadmap-Punkt — nicht in dieser
      Auslieferung.

#### B2. Tests fuer aslmanager-Logik
- [x] `aslmanager_core` Library angelegt (`apps/aslmanager/core/`).
      Pure-Funktionen extrahiert: `is_allowed_debian_url`,
      `is_safe_absolute_dir`, `is_safe_artifact_path`, `split_key_value`,
      `join_path`, `artifact_size_ok`, `raw_disk_header_ok`,
      `should_try_http_fallback`, `official_http_fallback`, `parse_u64`.
      `#![cfg_attr(not(test), no_std)]` — gleiche Quelle fuer anyOS-Build
      und Host-Tests.
- [x] **32 Tests gruen** (`cargo +stable test -p aslmanager_core`):
      URL-Whitelist (5), Path-Safety (5), Config-Parsing (4),
      Artifact-Size (4), MBR-Header (4), HTTP-Fallback-Policy (5),
      `parse_u64` (3), Pfad-Beispiele (2).
- [x] aslmanager-App auf das Core-Crate umgestellt — keine
      Code-Duplikation mehr.

#### B3. Toolchain-Profile (separater Schritt nach erstem Boot)
- [x] Skripte angelegt unter `defaults/System/etc/asl/toolchains/`
      mit CMake-Install-Regel in `cmake/UserPrograms.cmake`:
      - `dev-c.sh` — gcc, g++, clang, gdb, make, cmake, ninja, pkg-config
      - `dev-rust.sh` — rustup + stable toolchain (TLS 1.2 enforced)
      - `dev-node.sh` — nvm v0.40.1 + Node.js LTS
      - `dev-java.sh` — OpenJDK 21 + Maven + Gradle
      Alle Skripte: `set -euo pipefail`, idempotent (re-run ist no-op),
      strukturiertes Logging mit `[asl-toolchain:<profil>]` Tag.
      `README.md` daneben dokumentiert Aufruf und Konventionen.
      Permissions: 755 fuer .sh, 644 fuer README. 2026-05-07.
- [ ] **Live-Verifikation** in einer Distro: ueber `aslctl run` aufrufen
      und sehen dass die Toolchain wirklich installiert wird. **Blockiert**
      auf laufende Distro-VM (haengt mit A1-E2E zusammen).
- [ ] Optional: aslmanager-UI bekommt "Install toolchain"-Buttons die
      diese Skripte triggern und Output ins Logs-Tab streamen.

#### B4. Snapshot / Clone (war frueher B3)
- [ ] `aslctl clone` ist laut Audit IMPLEMENTED — testen mit "Base Dev"-Distro
      als Quelle.
- [ ] Use-Case: `clone base-dev → projektX-dev`, experimentieren, wegwerfen
      kostet nichts.

### Block C — Dev-Komfort und IDE-Integration (1a-Pfad)

#### C1. anycode Build-Task-Profil
- [ ] Build-Task-Definition in anycode die
      `aslctl run --distro <name> --cwd /workspace -- cargo build` aufruft.
- [ ] Output-Parser fuer gcc/cargo/tsc Fehlerformate → klickbare Zeilen.
- [ ] Run-Konfiguration: Programm in Distro starten + Port-Forward.

#### C2. TTY-Qualitaet fuer `aslctl shell`
- [ ] aslconsoled ANSI-Vollstaendigkeit pruefen — reicht fuer tmux/htop/vim?
- [ ] Resize-Forwarding vom anyOS-Terminal-Fenster bis zum PTY in der Distro.
- [ ] UTF-8 + Wide-Chars durchtesten.

#### C3. Language-Server (faellt aus A4)
- [ ] Wenn Stdio sauber durchgeht, kann anycode `rust-analyzer`/`clangd` ueber
      `aslctl run --distro X -- rust-analyzer` als Sub-Prozess starten. Kein
      eigener Block — Validierung in C1.

### Block D — Performance + Robustheit + Manager-Backend

#### D1. FS-Performance-Messung
- [ ] Benchmark-Skript: `cargo build` Cold/Warm, `npm install` mit ~500 Deps,
      `gradle build`. Vergleich shared-mount vs. nativ in Distro-FS.
- [ ] Wenn shared-mount > 2× langsamer: Bottleneck identifizieren
      (virtio-fs vs. 9P-Latenz, Caching-Strategie).

#### D2. aslmanager Performance-Tab mit Daten fuellen
- [ ] Stats-Endpoint in asld: vCPU-Last, RSS, Disk-IO,
      Netzwerk-Throughput pro Distro.
- [ ] aslmanager-Frontend an Endpoint haengen (`apps/aslmanager/src/main.rs`).
- [ ] Logs-Tab an `GetLogs` (aus A5) anbinden.

#### D3. Crash-Recovery
- [ ] asld-Restart-Test mit ≥2 laufenden Distros: Runtime-States rekonstruiert?
      Port-Forwards aktiv? Mounts live?
- [ ] Concurrency: parallele `start`/`stop` aus zwei Shells.

#### D4. Globale CLI-Flags (Rest)
- [ ] `--quiet`, `--verbose`, `--timeout` ergaenzen (aus Audit).
- [ ] `--user` falls Mehrbenutzerkontext relevant wird.

### Block E — Doku- und Aufraeumarbeiten (parallel)
- [ ] `docs/asld-scaffolding-plan.md` archivieren (ueberholt).
- [ ] `seed_image_path` in `docs/asl-config-schema.md` ergaenzen oder als
      intern markieren.
- [ ] Audit-Lueckenliste in den Spec-Doks nachpflegen wenn beim Implementieren
      Spec-Drift auffaellt.
- [ ] Dev-Quickstart-Doku schreiben: "Wie richtest du eine
      C++/Rust/Node/Java-Distro ein?"
- [ ] CLI-Naming: `vm-events` → `events` (Alias), `storage import` ↔
      `import` aus Doc abgleichen.

### Block F — Editor in der Distro (1c-Teil, separater Brocken, spaeter)

Nicht Teil der ersten Auslieferung. Optionen:
- **F1: Wayland-Proxy** — `WAYLAND_DISPLAY` in der Distro,
      Socket-Forwarding zum anyOS-Compositor. Mittlere Komplexitaet, modern.
- **F2: X11-Server in anyOS** — minimaler Xwayland-artiger Server, Distro-Apps
      nutzen `DISPLAY=:0`. Hoher Aufwand, kompatibel mit allem.
- **F3: VNC/RDP-artig** — Distro startet VNC-Server, anyOS hat Client.
      Pragmatisch, schwaechere UX.

Eigenes Roadmap-Item nachdem A-D produktiv ist.

### Reihenfolge

```
A1 (DNS+Outbound) ──┐
A2 (Port-Forward)   ├─→  Workflow-MVP
A3 (Shared-Mount)   │    (Distro manuell entwickelbar)
A4 (aslctl run) ────┘
        ↓
A5 (Logs/Events/--json)  — Debugging-Hilfen, parallel zu B moeglich
        ↓
B1 (ImportBase) → B2 (Bootstrap) → B3 (Clone)
        ↓
C1 (anycode) ── parallel ── C2 (TTY)
        ↓
D1-D4 (Perf, Manager-Backend, Robustheit)
        ↓
F (Editor in Distro)  — separat, spaeter
```

Erstes konkretes Arbeitspaket: **A1 — DNS-Broker in aslnetd**. Ohne das
funktionieren `apt update`/`git clone` nicht.
