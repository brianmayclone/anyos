# `aslctl` CLI Design

## Ziel

`aslctl` ist die primäre Kommandozeilenschnittstelle fuer Verwaltung, Diagnose
und Benutzung von ASL-Distributionen.

Die CLI soll:

- fuer Endnutzer einfach genug sein
- fuer Entwickler skriptbar bleiben
- fuer Betrieb und Debugging eindeutige Status- und Fehlerausgaben liefern

## Designprinzipien

1. Eine Distribution ist die zentrale Verwaltungseinheit.
2. Jede Aktion soll sowohl menschenlesbar als auch maschinenlesbar ausgebbar sein.
3. Kurze Standardkommandos fuer haeufige Flows, explizite Subcommands fuer Verwaltung.
4. Fehlercodes und Statusbegriffe muessen stabil bleiben.
5. Shell- und Exec-Flows sind erstklassig, nicht nachtraeglich angehaengt.

## Grundsyntax

```text
aslctl [global-options] <command> [subcommand] [arguments]
```

## Globale Optionen

- `--json`
  Gibt strukturierte JSON-Ausgabe aus.
- `--quiet`
  Reduziert Ausgabe auf Fehler oder angeforderte Daten.
- `--verbose`
  Zeigt erweiterte Diagnoseinformationen.
- `--timeout <ms>`
  Setzt ein Client-Timeout fuer RPCs an `asld`.
- `--user <name>`
  Fuehrt den Befehl im Kontext eines bestimmten Benutzers aus.

## Kommandogruppen

### Discovery

```text
aslctl list
aslctl show <distro>
aslctl status <distro>
```

Verhalten:

- `list` zeigt alle registrierten Distributionen
- `show` zeigt Konfiguration, Mounts, Ports und Ressourcenprofil
- `status` zeigt Laufzustand, Uptime, Health und letzte Fehlerursache

### Lifecycle

```text
aslctl create <name> [--from <image>] [--profile <profile>]
aslctl start <name>
aslctl stop <name>
aslctl restart <name>
aslctl suspend <name>
aslctl resume <name>
aslctl delete <name>
```

Hinweise:

- `suspend` und `resume` koennen in v1 als `not implemented` reserviert bleiben
- `delete` muss standardmaessig bestaetigt werden, ausser bei `--force`

### Import and Export

```text
aslctl import <archive-or-image> [--name <name>]
aslctl export <name> --output <path>
aslctl clone <source> <target>
```

Ziel:

- Linux-Distributionen als verwaltbare Artefakte behandeln
- spaeter Rollout, Backup und Sharing vorbereiten

### Shell and Exec

```text
aslctl shell <name>
aslctl exec <name> -- <command> [args...]
aslctl run <name> --cwd <path> --env KEY=VALUE -- <command> [args...]
```

Verhalten:

- `shell` startet interaktive Sitzung mit PTY
- `exec` fuehrt einmaligen Befehl aus
- `run` ist die erweiterte Variante mit Arbeitsverzeichnis und Umgebungsvariablen

### Filesystem and Mounts

```text
aslctl mount list <name>
aslctl mount add <name> --host <path> --guest <path> --mode <mode>
aslctl mount remove <name> --guest <path>
aslctl mount validate <name>
```

Modi:

- `readonly`
- `readwrite`
- `metadata-relaxed`

### Network and Ports

```text
aslctl port list <name>
aslctl port add <name> --listen 127.0.0.1:3000 --guest 3000/tcp
aslctl port remove <name> --listen 127.0.0.1:3000
aslctl network show <name>
aslctl network set <name> --mode nat
```

### Configuration

```text
aslctl config get <name>
aslctl config edit <name>
aslctl config set <name> --memory-mb 4096 --vcpu 4
aslctl profile list
aslctl profile show <profile>
```

### Diagnostics

```text
aslctl logs <name>
aslctl logs <name> --follow
aslctl doctor <name>
aslctl inspect <name>
aslctl events <name>
```

Ziel:

- Diagnose ohne direkte Daemon-Interna
- konsistente Betriebs- und Support-Flows

## Standardausgaben

### `aslctl list`

```text
NAME         STATE     HEALTH    VCPU  RAM   DISTRO
ubuntu-dev   running   ready     2     2G    ubuntu-24.04
debian-ci    stopped   n/a       1     1G    debian-13
```

### `aslctl status ubuntu-dev`

```text
Name:           ubuntu-dev
State:          running
Health:         ready
Uptime:         00:12:41
Kernel Profile: linux-x86_64-generic
Resources:      2 vCPU, 2048 MiB RAM
Network:        nat
Agent:          connected
Last Error:     none
```

### `aslctl status ubuntu-dev --json`

```json
{
  "name": "ubuntu-dev",
  "state": "running",
  "health": "ready",
  "uptime_ms": 761000,
  "vcpu": 2,
  "memory_mb": 2048,
  "network_mode": "nat",
  "agent": {
    "connected": true
  },
  "last_error": null
}
```

## Exit Codes

- `0`
  Erfolg
- `1`
  Allgemeiner Fehler
- `2`
  Ungueltige Argumente
- `3`
  Distribution nicht gefunden
- `4`
  Daemon nicht erreichbar
- `5`
  Timeout
- `6`
  Distribution in ungueltigem Zustand
- `7`
  Sicherheits- oder Policy-Verletzung

## Zustandsmodell

Stabile States fuer CLI und API:

- `created`
- `starting`
- `booting`
- `ready`
- `degraded`
- `stopping`
- `stopped`
- `failed`

## MVP-Schnitt

Fuer ein erstes lieferbares `aslctl` sollten nur diese Befehle Pflicht sein:

```text
aslctl list
aslctl create <name> --from <image>
aslctl start <name>
aslctl stop <name>
aslctl status <name>
aslctl shell <name>
aslctl exec <name> -- <command>
aslctl mount add <name> --host <path> --guest <path> --mode readonly
aslctl port list <name>
aslctl logs <name>
aslctl doctor <name>
```

## Nicht-Ziele fuer v1

- voll ausmodellierte TUI
- verschachtelte Profilvererbung
- Live-Migration
- Multi-host-Remote-Management
- Kompatibilitaet zu Docker- oder Kubernetes-CLIs

## Offene Punkte

- wie `aslctl config edit` Editor-Integration genau loest
- ob `shell` den Gastagent zwingend voraussetzt
- wie stark Port-Freigaben automatisch vorgeschlagen werden sollen
- ob es einen Alias wie `asl` statt `aslctl` geben soll
