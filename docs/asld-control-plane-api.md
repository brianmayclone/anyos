# `asld` Control Plane API v1

## Ziel

Dieses Dokument beschreibt eine erste Host-seitige Control-Plane-API fuer
`asld`.

Ziele:

- stabiles Interface zwischen `aslctl`, spaeteren UIs und `asld`
- Trennung zwischen gewollter Konfiguration und volatilem Runtime-Status
- klare Fehler- und Zustandssemantik

Dieses Dokument ist absichtlich transportneutral. Ob `asld` spaeter ueber IPC,
Socket-RPC oder einen anderen lokalen Mechanismus angesprochen wird, ist hier
noch nicht festgelegt.

## Designprinzipien

1. Distribution ist die zentrale Verwaltungseinheit.
2. Mutationen sind explizite Befehle, keine impliziten Seiteneffekte.
3. Statusobjekte sind read-only Sichten auf Runtime-Zustand.
4. Fehler muessen maschinenlesbar und menschenlesbar sein.
5. Agent-Zustand und Distro-Zustand werden getrennt modelliert.

## Objektmodell

### `DistroConfig`

Entspricht dem autoritativen Konfigurationsmodell aus
[asl-config-schema.md](/daten1/development/brian/anyos/docs/asl-config-schema.md),
das durch `asld` in `confd` registriert und aus `confd` gelesen wird.

### `DistroStatus`

Runtime-Sicht fuer eine Distribution.

Beispiel:

```json
{
  "name": "ubuntu-dev",
  "state": "running",
  "health": "ready",
  "uptime_ms": 761000,
  "resources": {
    "memory_mb": 2048,
    "vcpu_count": 2
  },
  "network": {
    "mode": "nat"
  },
  "agent": {
    "state": "ready",
    "connected": true
  },
  "last_error": null
}
```

### `OperationResult`

Standardantwort fuer mutierende Befehle.

```json
{
  "ok": true,
  "operation_id": "op-01HZZ7M8N9P0",
  "message": "distro started"
}
```

### `ApiError`

```json
{
  "code": "DISTRO_NOT_FOUND",
  "message": "distribution 'ubuntu-dev' was not found",
  "retryable": false
}
```

## Zustandsmodell

### Distro States

- `created`
- `starting`
- `booting`
- `ready`
- `degraded`
- `stopping`
- `stopped`
- `failed`

### Agent States

- `not_present`
- `starting`
- `connected`
- `ready`
- `degraded`
- `disconnected`

## Configuration Backend

`asld` ist der fachliche Owner der ASL-Konfiguration, `confd` ist der
autoritative Persistenz- und Registry-Backend-Dienst.

Darum gilt:

- `aslctl` spricht fuer ASL-Verwaltung mit `asld`, nicht direkt mit `confd`
- `asld` registriert seinen Namespace-Vertrag bei `confd`
- `asld` liest und schreibt effektive ASL-Konfiguration ueber `confd`
- Runtime-Status bleibt bei `asld` und wird nicht als normale Konfiguration in
  `confd` behandelt

Empfohlene Namespace-Wurzel:

```text
system/platform/asl/...
```

## API Surface

## Discovery

### `ListDistros`

Anfrage:

```json
{}
```

Antwort:

```json
{
  "items": [
    {
      "name": "ubuntu-dev",
      "state": "running",
      "health": "ready"
    }
  ]
}
```

### `GetDistroConfig`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

- `DistroConfig`

### `GetDistroStatus`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

- `DistroStatus`

## Lifecycle

### `CreateDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "base_image_ref": "ubuntu-24.04-x86_64-v1",
  "owner": "strati",
  "profile": "dev"
}
```

Antwort:

- `OperationResult`

### `StartDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

- `OperationResult`

### `StopDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "force": false
}
```

Antwort:

- `OperationResult`

### `RestartDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

- `OperationResult`

### `DeleteDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "force": false
}
```

Antwort:

- `OperationResult`

## Import / Export

### `ImportBaseImage`

Anfrage:

```json
{
  "source_path": "/Users/strati/downloads/ubuntu-rootfs.tar",
  "name": "ubuntu-24.04-x86_64-v1"
}
```

Antwort:

- `OperationResult`

### `ExportDistro`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "output_path": "/tmp/ubuntu-dev-export.asl"
}
```

Antwort:

- `OperationResult`

## Config Mutation

### `UpdateResources`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "memory_mb": 4096,
  "vcpu_count": 4
}
```

Antwort:

- `OperationResult`

### `SetNetworkMode`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "mode": "nat"
}
```

Antwort:

- `OperationResult`

## Mount Management

### `ListMounts`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

```json
{
  "items": [
    {
      "guest_path": "/mnt/projects",
      "host_path": "/Users/strati/projects",
      "mode": "readwrite",
      "health": "ready"
    }
  ]
}
```

### `AddMount`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "mount": {
    "host_path": "/Users/strati/projects",
    "guest_path": "/mnt/projects",
    "mode": "readwrite",
    "metadata_mode": "relaxed",
    "case_mode": "host-native",
    "exec_policy": "inherit",
    "watch_policy": "best-effort"
  }
}
```

Antwort:

- `OperationResult`

### `RemoveMount`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "guest_path": "/mnt/projects"
}
```

Antwort:

- `OperationResult`

### `ValidateMounts`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

```json
{
  "ok": true,
  "items": [
    {
      "guest_path": "/mnt/projects",
      "valid": true,
      "message": "mount export reachable"
    }
  ]
}
```

## Port Management

### `ListPortForwards`

### `AddPortForward`

### `RemovePortForward`

Diese API folgt strukturell dem Mount-Management mit:

- `listen_address`
- `listen_port`
- `guest_port`
- `protocol`

## Console and Exec

Diese Befehle sind agent-sensitiv und duerfen einen degradierten Pfad explizit
abbilden.

### `OpenShellSession`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "session_name": "dev",
  "fallback_console": false
}
```

Antwort:

```json
{
  "session_id": "sh-01HZZ8R2",
  "mode": "agent"
}
```

`mode` ist in v1:

- `agent`
- `fallback-console`

### `ExecCommand`

Anfrage:

```json
{
  "name": "ubuntu-dev",
  "argv": ["cargo", "test"],
  "cwd": "/workspace/app",
  "env": {
    "RUST_BACKTRACE": "1"
  }
}
```

Antwort:

```json
{
  "exec_id": "exec-01HZZ8T4",
  "mode": "agent"
}
```

## Agent

### `GetAgentStatus`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

```json
{
  "state": "ready",
  "connected": true,
  "last_seen_ms": 42
}
```

### `RestartAgent`

Anfrage:

```json
{
  "name": "ubuntu-dev"
}
```

Antwort:

- `OperationResult`

## Diagnostics

### `GetLogs`

### `RunDoctor`

### `ListEvents`

### `InspectDistro`

Diese Endpunkte liefern Diagnoseobjekte und muessen mindestens:

- Runtime-Status
- Agent-Status
- Mount-Status
- Netzwerk-Status
- letzte Fehlerursache

sichtbar machen.

## Error Codes

V1-Mindestmenge:

- `INVALID_ARGUMENT`
- `DISTRO_NOT_FOUND`
- `DISTRO_ALREADY_EXISTS`
- `INVALID_STATE`
- `TIMEOUT`
- `POLICY_DENIED`
- `BACKEND_UNAVAILABLE`
- `AGENT_UNAVAILABLE`
- `MOUNT_INVALID`
- `PORT_CONFLICT`

## Versioning

Jede Anfrage und Antwort soll logisch an `schema_version = 1` gekoppelt sein.

Erweiterungen muessen additive Kompatibilitaet bevorzugen:

- neue optionale Felder sind erlaubt
- bestehende Zustandsnamen und Fehlercodes sollen stabil bleiben
- breaking changes brauchen eine neue API-Version

## Open Points

- welches konkrete lokale IPC-Transportformat `asld` nutzt
- ob langlaufende Operationen synchron oder ueber Job-Handles beobachtet werden
- wie Streaming fuer Shell, Exec und `logs --follow` exakt modelliert wird
- ob Profiles als eigene API-Ressource modelliert werden
