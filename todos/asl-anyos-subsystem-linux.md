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
