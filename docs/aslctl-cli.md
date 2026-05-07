# `aslctl` CLI Design

## Ziel

`aslctl` ist die primäre Kommandozeilenschnittstelle fuer Verwaltung, Diagnose
und Benutzung von ASL-Distributionen.

Aktueller Implementierungsstand 2026-05-06: die CLI existiert unter
`bin/aslctl/` und spricht lokal mit `asld` ueber Pipe-IPC. Dieses Dokument ist
weiterhin das Zielbild fuer Bedienbarkeit und stabile Semantik; einzelne
Syntaxdetails muessen gegen `bin/aslctl/src/lib.rs` abgeglichen werden.

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

Globale Flags muessen **vor** dem Subcommand stehen. Unbekannte Flags
nach dem ersten erkannten Token werden nicht konsumiert, damit
Subcommand-spezifische Flags wie `--cwd` oder `--env` nicht
versehentlich geschluckt werden.

- `--json` (umgesetzt 2026-05-07)
  Gibt strukturierte JSON-Ausgabe aus. Schema:
  - Erfolg: `{"ok":true,"command":"<name>","lines":[...]}`
  - Fehler: `{"ok":false,"command":"<name>","code":"...","message":"..."}`
  String-Werte sind RFC-8259-konform escaped (Quotes, Backslashes,
  Control-Chars als `\u00XX`). Eine Zeile pro Response, terminiert
  mit `\n` — direkt fuer Tools wie `jq` verwendbar.
- `--quiet` / `-q` (umgesetzt 2026-05-07)
  Unterdrueckt Section-Header-Zeilen wie `distros: 5` oder
  `mounts: 2`. Nuetzlich beim Pipen in `grep` oder fuer
  Skripte im Text-Modus. Wirkt nur im Text-Modus; im JSON-Modus
  ist die Schemashape immer dieselbe.
- `--verbose` / `-v` (geparst 2026-05-07, heute no-op)
  Reserviert fuer per-Command Detail-Output. Wird heute akzeptiert,
  ohne das Verhalten zu aendern, damit Skripte das Flag schon setzen
  koennen, wenn die Detail-Ausgabe nachgereicht wird.
- `--timeout <ms>` *(noch nicht implementiert)*
  Setzt ein Client-Timeout fuer RPCs an `asld`.
- `--user <name>` *(noch nicht implementiert)*
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
aslctl shell <name> --fallback-console
aslctl shell <name> --session <session-name>
aslctl [--json] [--quiet|-q] [--verbose|-v] <subcommand> [args...]

aslctl exec <name> [--fallback-console] [--cwd <path>] [--env KEY=VALUE]... -- <command> [args...]
aslctl run  <name> [--fallback-console] [--cwd <path>] [--env KEY=VALUE]... -- <command> [args...]
```

Verhalten:

- `shell` startet interaktive Sitzung mit PTY
- `shell --fallback-console` versucht einen degradierten Zugriffspfad ohne volle
  Agent-Integration
- `shell --session <session-name>` adressiert oder erzeugt eine benannte
  persistente Sitzung
- `exec` fuehrt einmaligen Befehl aus, optional mit `--cwd` und einer oder
  mehreren `--env KEY=VALUE` Variablen
- `run` ist ein Alias zu `exec` mit identischer Semantik. Empfohlen fuer
  Build-Workflows aus IDEs (anycode), wo `run` lesbarer ist als `exec`.
  Beide Befehle erzeugen denselben Wire-Command (`EXEC ...`).

Hinweis:

- `shell` und `exec` sollen klar anzeigen, ob sie ueber den Agent-Pfad oder
  einen degradierten Fallback laufen

### Filesystem and Mounts

```text
aslctl mount list <name>
aslctl mount add <name> --host <path> --guest <path> --mode <mode> [options]
aslctl mount remove <name> --guest <path>
aslctl mount validate <name>
aslctl mount show <name> --guest <path>
```

Zentrale Optionen fuer `mount add`:

- `--mode readonly|readwrite`
- `--metadata strict|relaxed`
- `--case host-native|case-sensitive|case-folded`
- `--exec inherit|noexec|host-metadata`
- `--watch best-effort|off`
- `--description <text>`

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

Beispiel:

```text
aslctl mount add ubuntu-dev \
  --host /Users/strati/projects \
  --guest /mnt/projects \
  --mode readwrite \
  --metadata relaxed \
  --case host-native \
  --exec inherit \
  --watch best-effort
```

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

Hinweis:

- diese Befehle arbeiten fachlich gegen `asld`
- `asld` persistiert autoritative ASL-Konfiguration ueber `confd`
- `aslctl` soll nicht am Control Plane vorbei direkt im ASL-Namespace von
  `confd` schreiben

### Diagnostics

```text
aslctl logs <name>
aslctl logs <name> --follow
aslctl doctor <name>
aslctl inspect <name>
aslctl events <name>
aslctl agent status <name>
aslctl agent restart <name>
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

### `aslctl mount show ubuntu-dev --guest /mnt/projects`

```text
Host Path:       /Users/strati/projects
Guest Path:      /mnt/projects
Mode:            readwrite
Metadata:        relaxed
Case:            host-native
Exec:            inherit
Watch:           best-effort
Health:          ready
Last Error:      none
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
aslctl mount validate <name>
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

- wie `aslctl config edit` eine `confd`-gestuetzte Bearbeitung ergonomisch
  abbildet
- wie `aslctl config edit` Editor-Integration genau loest
- ob `shell` in v1 standardmaessig ueber Agent, Fallback-Konsole oder adaptiv
  entscheidet
- wie stark Port-Freigaben automatisch vorgeschlagen werden sollen
- ob es einen Alias wie `asl` statt `aslctl` geben soll
