# `asld` Scaffolding Plan v1

## Ziel

Dieses Dokument beschreibt die erste konkrete Repo-Struktur fuer `asld`.

Es geht bewusst noch nicht um vollstaendige Funktionalitaet, sondern um ein
sauberes Geruest, auf dem wir ASL iterativ aufbauen koennen:

- klar getrennte Module
- fruehe `confd`-Integration
- Runtime-Status getrennt von Konfiguration
- Platz fuer spaetere Hypervisor-, Console-, Network- und Filesystem-Broker

## Designprinzipien

1. `asld` ist fachlicher Owner, nicht Monolith fuer alle Subsystemdetails.
2. Konfiguration kommt aus `confd`, Runtime bleibt lokal in `asld`.
3. IPC nach aussen soll frueh stabil aussehen, auch wenn intern noch Stubs laufen.
4. Hypervisor-, Agent- und Brokerlogik werden sauber in Teilmodule getrennt.
5. Phase 1 braucht Bootbarkeit und Status, nicht Vollintegration.

## Empfohlene Verzeichnisstruktur

```text
system/daemons/asld/
  Cargo.toml
  build.rs
  src/
    main.rs
    config.rs
    schema.rs
    ipc.rs
    model.rs
    status.rs
    store.rs
    runtime.rs
    distro.rs
    image.rs
    vm.rs
    agent.rs
    mounts.rs
    network.rs
    errors.rs
```

## Modulrollen

### `main.rs`

Verantwortung:

- Prozessstart
- `confd`-Manifest registrieren
- Runtime-Strukturen initialisieren
- IPC-Endpunkt oeffnen
- Request-Loop starten

Nicht hier hinein:

- Konfigurationsparsing im Detail
- VM-Zusammenbau
- Business-Logik fuer Mounts oder Ports

### `schema.rs`

Verantwortung:

- ASL-Root-Manifest fuer `confd`
- Namespace `platform/asl`
- Root-Defaults fuer Profile und globale Basiswerte
- `register_manifest()`

Anlehnung:

- aehnlich zu `ntpd/src/config.rs`, aber fuer ASL eher als eigene
  Registry-/Schema-Datei statt reinem Config-Loader

### `config.rs`

Verantwortung:

- Lesen der autoritativen ASL-Konfiguration aus `confd`
- Laden einer Distribution aus `system/platform/asl/distros/<name>/...`
- Schreiben gezielter Konfigaenderungen ueber `confd`
- Materialisierung neuer Distro-Teilbaeume

Wichtig:

- kein Runtime-Zustand
- keine VM-Lifecycle-Entscheidungen

### `model.rs`

Verantwortung:

- zentrale Datentypen fuer:
  - `DistroConfig`
  - `DistroStatus`
  - `MountSpec`
  - `PortForwardSpec`
  - `AgentState`
  - `DistroState`

Ziel:

- eine gemeinsame Typschicht fuer `config.rs`, `ipc.rs`, `runtime.rs`

### `status.rs`

Verantwortung:

- Runtime-Statusobjekte
- Uptime
- letzter Fehler
- Health-Berechnung
- Agentzustand
- Mount-/Netz-Basisstatus

Wichtig:

- hier lebt bewusst nur nichtpersistente Sicht

### `store.rs`

Verantwortung:

- In-Memory-Registry laufender Distributionen
- Abbildung `name -> runtime handle / status`
- Zugriffssynchronisation fuer den Request-Handler

V1-Empfehlung:

- einfacher Hostprozess-interner Store
- spaeter erweiterbar fuer persistente Operation-Logs oder Jobs

### `runtime.rs`

Verantwortung:

- Orchestrierung des laufenden Systems
- Start/Stop/Restart-Operationen
- Laden von Konfiguration + Aufbau eines Runtime-Kontexts
- Aufruf der spezialisierten Teilmodule

`runtime.rs` ist die eigentliche Service-Layer-Datei von `asld`.

### `distro.rs`

Verantwortung:

- Create/Delete/Clone/Import-nahe Distro-Operationen
- Validierung von Namen, Ownership und Basiskonfiguration
- Vorbereitung des Distro-Arbeitsverzeichnisses unter `/System/var/asl/...`

### `image.rs`

Verantwortung:

- Import von Base Images
- Image-Referenzen
- Trust-Metadaten
- Abbildung `base_image_ref -> image metadata`

V1:

- darf intern noch sehr einfach bleiben
- wichtig ist die saubere Trennung von Distro und Image

### `vm.rs`

Verantwortung:

- Schnittstelle zum Hypervisor-Pfad
- Stub oder Adapter fuer:
  - `vm_create`
  - `vcpu_create`
  - Memory-Setup
  - Kernel-/initrd-Loading

V1:

- darf anfangs noch `NotImplemented` zurueckgeben
- Interface aber frueh sauber definieren

### `agent.rs`

Verantwortung:

- Agentstatus
- Heartbeat-Buchhaltung
- Readiness
- Restart-Agent-Befehl
- degraded vs. ready Logik

### `mounts.rs`

Verantwortung:

- Mount-Validierung
- Uebersetzung von `MountSpec` in Broker-Requests
- Plausibilitaetschecks fuer `host_path`, `guest_path`, Policies

V1:

- zunaechst Konfig- und Validierungslogik
- spaeter echte Kopplung an `aslfsd`

### `network.rs`

Verantwortung:

- Port-Forward-Spezifikationen
- NAT-Mode-Validierung
- Kommunikation mit `aslnetd` oder vorbereiteter Stub

### `ipc.rs`

Verantwortung:

- Request-/Reply-Protokoll fuer `aslctl`
- Mapping auf `runtime.rs`-Operationen
- Serialisierung menschenlesbarer oder spaeter strukturierter Antworten

V1-Empfehlung:

- transportnah und duenn halten
- keine Businesslogik hier

### `errors.rs`

Verantwortung:

- zentrale Fehlercodes
- `DISTRO_NOT_FOUND`
- `INVALID_STATE`
- `AGENT_UNAVAILABLE`
- `MOUNT_INVALID`
- `PORT_CONFLICT`

Ziel:

- konsistent mit `docs/asld-control-plane-api.md`

## Phase-1 Scaffolding Scope

Der erste echte Code-Schnitt fuer `asld` sollte nur Folgendes koennen:

- Manifest bei `confd` registrieren
- Distro-Konfiguration aus `confd` lesen
- Runtime-Statusobjekt fuer eine Distro aufbauen
- IPC-Grundgeruest fuer:
  - `list`
  - `status`
  - `create`
  - `start`
  - `stop`
- Hypervisor-/Agent-/Mount-/Network-Pfade als strukturierte Stubs bereitstellen

## Minimaler Kontrollfluss

### Startpfad

1. `main.rs` startet
2. `schema::register_manifest()` wird aufgerufen
3. `store` und `runtime` werden initialisiert
4. `ipc` oeffnet Request-Pfad
5. Requests werden an `runtime` delegiert

### `start <name>`

1. `ipc` parst Request
2. `runtime` laedt `DistroConfig` ueber `config`
3. `runtime` erzeugt Runtime-Kontext im `store`
4. `vm` wird aufgerufen
5. `status` wird auf `starting/booting/running` aktualisiert

## Suggested First Cargo Dependencies

Wahrscheinlich noetig:

- `anyos_std`
- `libconf`
- `libconf_schema`
- optional spaeter `libsvc`

Keine voreilige Zusatzabhaengigkeit fuer komplexe Serialisierung in v1.

## Suggested First IPC Surface

Fuer das erste Scaffolding genuegen:

- `LIST`
- `STATUS <name>`
- `CREATE <name> <image-ref> <owner>`
- `START <name>`
- `STOP <name>`
- `AGENT_STATUS <name>`

Alles andere darf anfangs sauber `ERR not_implemented` liefern.

## Suggested First `confd` Integration

`schema.rs`:

- registriert Root-Manifest `platform/asl`

`config.rs`:

- `load_distro(name) -> Result<DistroConfig, Error>`
- `save_distro(config) -> Result<(), Error>`
- `ensure_distro_tree(config) -> Result<(), Error>`

## Separation of Concerns

Wichtig fuer spaetere Wartbarkeit:

- `asld` ist nicht `aslfsd`
- `asld` ist nicht `aslnetd`
- `asld` ist nicht `aslconsoled`

`asld` orchestriert und besitzt Policy, aber fuehrt nicht jede Detailfunktion
selbst aus.

## First Milestone

Ein sinnvolles erstes Milestone-Ziel fuer echten Code:

- Daemon startet
- Root-Manifest wird in `confd` registriert
- `aslctl` oder Testclient kann `LIST` und `STATUS` aufrufen
- `CREATE` materialisiert einen Distro-Teilbaum in `confd`
- `START` wechselt mindestens den Runtime-Statuspfad, auch wenn der VM-Start
  intern noch gestubbt ist

## Non-Goals

- vollständiger VM-Boot im ersten Scaffolding-Commit
- komplette `aslctl`-Integration
- Port-Forwarding oder Shared-Folder-Implementation
- fertige Agent-Kommunikation
- finaler IPC-Transport

## Follow-up

Wenn dieses Geruest steht, sind die naechsten sinnvollen Code-Schritte:

1. `schema.rs` + `config.rs`
2. `model.rs` + `errors.rs`
3. `ipc.rs` + `runtime.rs`
4. `vm.rs`-Adapter
5. erstes `aslctl`-Geruest
