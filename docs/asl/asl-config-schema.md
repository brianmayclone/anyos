# ASL Config Schema v1

## Ziel

Dieses Dokument definiert das erste konkrete Konfigurationsschema fuer eine
ASL-Distribution. Es dient als gemeinsame Grundlage fuer:

- `asld`
- `aslctl`
- spaetere UI-Editoren
- Export-/Import-Workflows
- Diagnose und Validierung

Das Schema ist bewusst konservativ und bildet nur die in den ADRs
festgezogenen Kernentscheidungen ab.

## Format

V1 verwendet **kein loses `config.json` als autoritative Quelle**. Die
autoritative ASL-Konfiguration liegt in `confd`.

ASL registriert seine Konfigurationsvertraege ueber `asld` in einem eigenen
System-Namespace:

```text
system/platform/asl/images/<image-ref>/...
system/platform/asl/distros/<name>/...
```

Die JSON-Strukturen in diesem Dokument sind logische Objektdarstellungen fuer:

- `confd`-Teilbaeume
- `asld`-Konfigobjekte
- Import-/Export-Formate
- `aslctl --json`-Ausgaben

## Logical Top-Level Object

```json
{
  "schema_version": 1,
  "id": "distro-01HZY3B4Y5R6",
  "name": "ubuntu-dev",
  "owner": "strati",
  "base_image_ref": "ubuntu-24.04-x86_64-v1",
  "kernel_profile": "linux-x86_64-generic",
  "resources": {},
  "storage": {},
  "network": {},
  "mounts": [],
  "port_forwards": [],
  "agent": {},
  "lifecycle": {},
  "metadata": {}
}
```

## Namespace Mapping

Das logische Top-Level-Objekt wird in `confd` auf einen Teilbaum abgebildet.

Beispiel fuer `ubuntu-dev`:

```text
system/platform/asl/distros/ubuntu-dev/schema_version
system/platform/asl/distros/ubuntu-dev/id
system/platform/asl/distros/ubuntu-dev/name
system/platform/asl/distros/ubuntu-dev/owner
system/platform/asl/distros/ubuntu-dev/base_image_ref
system/platform/asl/distros/ubuntu-dev/kernel_profile
system/platform/asl/distros/ubuntu-dev/resources/memory_mb
system/platform/asl/distros/ubuntu-dev/resources/vcpu_count
system/platform/asl/distros/ubuntu-dev/network/mode
...
```

`asld` soll beim Start seinen Manifest-Vertrag fuer `system/platform/asl/...`
bei `confd` registrieren und Defaults sowie spaetere Migrationen ueber `confd`
deklarieren.

## Felder

### `schema_version`

- Typ: `u32`
- Pflicht: ja
- V1-Wert: `1`

### `id`

- Typ: `string`
- Pflicht: ja
- Stabiler, nicht benutzereditierbarer interner Identifier

### `name`

- Typ: `string`
- Pflicht: ja
- Benutzerlesbarer Name der Distribution
- Muss innerhalb des Hosts eindeutig sein

### `owner`

- Typ: `string`
- Pflicht: ja
- Benutzer oder logischer Owner der Distribution

### `base_image_ref`

- Typ: `string`
- Pflicht: ja
- Referenz auf das importierte Basisimage

### `kernel_profile`

- Typ: `string`
- Pflicht: ja
- Beispiel: `linux-x86_64-generic`

## `resources`

```json
{
  "memory_mb": 2048,
  "vcpu_count": 2,
  "autostart": false
}
```

Felder:

- `memory_mb`
  - Typ: `u32`
  - Mindestwert v1: `256`
- `vcpu_count`
  - Typ: `u16`
  - Mindestwert v1: `1`
- `autostart`
  - Typ: `bool`
  - Default: `false`

## `storage`

```json
{
  "layout": "layered-v1",
  "base_image_path": "/System/var/asl/distros/ubuntu-dev/images/base.img",
  "overlay_image_path": "/System/var/asl/distros/ubuntu-dev/images/overlay.img",
  "state_image_path": "/System/var/asl/distros/ubuntu-dev/images/state.img",
  "state_image_enabled": true
}
```

Felder:

- `layout`
  - Typ: `string`
  - V1-Wert: `layered-v1`
- `base_image_path`
  - Typ: `string`
  - Pflicht: ja
- `overlay_image_path`
  - Typ: `string`
  - Pflicht: ja
- `state_image_path`
  - Typ: `string`
  - Pflicht: nein
- `state_image_enabled`
  - Typ: `bool`
  - Default: `false`

## `network`

```json
{
  "mode": "nat",
  "dns_mode": "host-broker",
  "allow_outbound": true
}
```

Felder:

- `mode`
  - Typ: `string`
  - V1-Default: `nat`
  - Zulaessig in v1: `nat`
- `dns_mode`
  - Typ: `string`
  - V1-Default: `host-broker`
- `allow_outbound`
  - Typ: `bool`
  - Default: `true`

## `mounts`

Liste expliziter Shared-Folder-Exporte.

Beispiel:

```json
[
  {
    "host_path": "/Users/strati/projects",
    "guest_path": "/mnt/projects",
    "mode": "readwrite",
    "metadata_mode": "relaxed",
    "case_mode": "host-native",
    "exec_policy": "inherit",
    "watch_policy": "best-effort",
    "description": "Main project workspace"
  }
]
```

Pflichtfelder pro Eintrag:

- `host_path`
- `guest_path`
- `mode`

Optionale Felder:

- `metadata_mode`
- `case_mode`
- `exec_policy`
- `watch_policy`
- `description`

Defaults:

- `metadata_mode = "relaxed"`
- `case_mode = "host-native"`
- `exec_policy = "inherit"`
- `watch_policy = "best-effort"`

## `port_forwards`

Liste expliziter Portregeln.

Beispiel:

```json
[
  {
    "listen_address": "127.0.0.1",
    "listen_port": 3000,
    "guest_port": 3000,
    "protocol": "tcp",
    "description": "Frontend dev server"
  }
]
```

Pflichtfelder pro Eintrag:

- `listen_address`
- `listen_port`
- `guest_port`
- `protocol`

V1-Regeln:

- `listen_address` sollte standardmaessig `127.0.0.1` sein
- `protocol` in v1: `tcp`

## `agent`

```json
{
  "enabled": true,
  "required_for_rich_integration": true,
  "fallback_console_enabled": true
}
```

Felder:

- `enabled`
  - Typ: `bool`
  - Default: `true`
- `required_for_rich_integration`
  - Typ: `bool`
  - Default: `true`
- `fallback_console_enabled`
  - Typ: `bool`
  - Default: `true`

## `lifecycle`

```json
{
  "restart_on_failure": true,
  "shutdown_timeout_ms": 10000,
  "boot_timeout_ms": 30000
}
```

Felder:

- `restart_on_failure`
  - Typ: `bool`
  - Default: `true`
- `shutdown_timeout_ms`
  - Typ: `u32`
  - Default: `10000`
- `boot_timeout_ms`
  - Typ: `u32`
  - Default: `30000`

## `metadata`

Freies, nicht sicherheitsrelevantes Zusatzfeld fuer UX und Diagnose.

Beispiel:

```json
{
  "distro_family": "ubuntu",
  "distro_version": "24.04",
  "notes": "Primary development distro"
}
```

## Validation Rules

V1-Minimum:

- `schema_version` muss `1` sein
- `name` darf nicht leer sein
- `memory_mb >= 256`
- `vcpu_count >= 1`
- `network.mode == "nat"`
- `mounts[*].host_path` darf nicht leer sein
- `mounts[*].guest_path` muss absolut sein
- `port_forwards[*].listen_port` und `guest_port` muessen gueltige Ports sein
- `base_image_path` und `overlay_image_path` muessen gesetzt sein

## `confd` Ownership Model

Autoritativ in `confd` gehoeren:

- gewollte Ressourcenkonfiguration
- Storage-Layout-Referenzen
- Netzwerkmodus
- Mount-Definitionen
- Port-Forward-Regeln
- Agent-Policy
- Lifecycle-Defaults
- nichtvolatile Metadaten

`asld` bleibt fachlicher Owner dieser Konfiguration, `confd` ist die
persistente Registry darunter.

## State Separation

Dieses Dokument beschreibt **gewollte Konfiguration**, nicht volatile
Laufzeitdaten.

Nicht in die autoritative `confd`-Konfiguration gehoeren:

- aktuelle Uptime
- laufender Agentstatus
- letzte Fehlerursache
- aktuelle Portbelegung
- Runtime-Sockets oder PID-Informationen

Diese Werte gehoeren in Runtime- oder Statusobjekte von `asld`.

Optional kann `asld` fuer Diagnose oder Export ein materialisiertes Snapshot-
Objekt erzeugen, aber dieses ist nicht Source of Truth.

## Example

```json
{
  "schema_version": 1,
  "id": "distro-01HZY3B4Y5R6",
  "name": "ubuntu-dev",
  "owner": "strati",
  "base_image_ref": "ubuntu-24.04-x86_64-v1",
  "kernel_profile": "linux-x86_64-generic",
  "resources": {
    "memory_mb": 2048,
    "vcpu_count": 2,
    "autostart": false
  },
  "storage": {
    "layout": "layered-v1",
    "base_image_path": "/System/var/asl/distros/ubuntu-dev/images/base.img",
    "overlay_image_path": "/System/var/asl/distros/ubuntu-dev/images/overlay.img",
    "state_image_path": "/System/var/asl/distros/ubuntu-dev/images/state.img",
    "state_image_enabled": true
  },
  "network": {
    "mode": "nat",
    "dns_mode": "host-broker",
    "allow_outbound": true
  },
  "mounts": [
    {
      "host_path": "/Users/strati/projects",
      "guest_path": "/mnt/projects",
      "mode": "readwrite",
      "metadata_mode": "relaxed",
      "case_mode": "host-native",
      "exec_policy": "inherit",
      "watch_policy": "best-effort",
      "description": "Main project workspace"
    }
  ],
  "port_forwards": [
    {
      "listen_address": "127.0.0.1",
      "listen_port": 3000,
      "guest_port": 3000,
      "protocol": "tcp",
      "description": "Frontend dev server"
    }
  ],
  "agent": {
    "enabled": true,
    "required_for_rich_integration": true,
    "fallback_console_enabled": true
  },
  "lifecycle": {
    "restart_on_failure": true,
    "shutdown_timeout_ms": 10000,
    "boot_timeout_ms": 30000
  },
  "metadata": {
    "distro_family": "ubuntu",
    "distro_version": "24.04",
    "notes": "Primary development distro"
  }
}
```
