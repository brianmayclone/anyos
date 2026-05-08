# ADR-0005 - ASL uses a minimal but first-class guest agent

## Status

Accepted

## Date

2026-04-23, status promoted 2026-05-07

## Promotion note

`asl-agent` Scope ist in der asld-Implementierung als minimal-but-first-class
verankert: Schema-Felder `agent.enabled`, `required_for_rich_integration`,
`fallback_console_enabled` (siehe `system/daemons/asld/src/schema.rs:14-15`,
`config.rs:243-253`). Boot-Pfad ueber `aslconsoled`-Fallback funktioniert ohne
Agent. `GetAgentStatus` und `RestartAgent` API implementiert.

## Context

ASL braucht fuer gute Integration zwischen anyOS-Host und Linux-Gast eine
kontrollierte Gastkomponente. Gleichzeitig darf der Linux-Bootpfad nicht an
einen komplexen Agenten gekoppelt werden, weil sonst Debugging, Recovery und
Grundfunktion zu fragil werden.

Die zentrale Frage lautet daher nicht, ob es einen Gastagenten gibt, sondern:

- welchen Minimalumfang der Agent haben soll
- welche Verantwortungen im Host bleiben muessen
- welche Features den Agenten zwingend voraussetzen duerfen

## Decision

ASL verwendet einen **minimalen, aber erstklassigen Gastagenten** namens
`asl-agent`.

Der Agent ist fuer "rich integration" zustaendig, aber **nicht** fuer den
grundsaetzlichen Linux-Boot oder die letzte Rettungskonsole.

ASL folgt damit drei Regeln:

- Linux muss ohne funktionsfaehigen Agent noch booten koennen
- der Agent ist Standardbestandteil offizieller ASL-Distributionen
- Host-Policy und Lifecycle-Entscheidungen bleiben bei `asld` und den
  Host-Brokern

## Responsibilities

Der `asl-agent` ist in v1 verantwortlich fuer:

- Readiness-/Heartbeat-Signale an den Host
- Session- und Console-Koordination fuer komfortable `shell`-/`exec`-Flows
- geordnete Shutdown- und Stop-Koordination
- Mount- und Distro-Metadatenruemeldungen an den Host
- Inventar einfacher Gastinformationen fuer Status und Diagnose

Spaeter optional:

- Clipboard-Integration
- Notifications
- Port- und Prozessinventar mit hoeherem Detailgrad
- GUI-bezogene Hooks

## Non-Responsibilities

Der `asl-agent` ist nicht verantwortlich fuer:

- das Booten des Linux-Kernels
- das Mounten des Rootfs-Grundpfads
- Host-Policy fuer Ressourcen, Ports oder Shared Folders
- NAT-, DNS- oder Port-Forwarding-Entscheidungen
- Sicherheitskritische Hostvalidierung

## Architecture

### Host Side

Host-Komponenten:

- `asld` fuer Control Plane und Lifecycle
- `aslconsoled` fuer PTY-/Session-Brokerage
- `aslfsd` fuer Shared Folders
- `aslnetd` fuer Netzwerk und Port-Policy
- `aslobsd` fuer Metriken und Diagnose

### Guest Side

`asl-agent` laeuft als normaler Systemdienst im Gast und kommuniziert ueber
einen kontrollierten paravirtualen oder virtio-serial-artigen Kanal mit dem
Host.

Der Agent soll klein, klar versioniert und restartbar sein.

## Boot and Recovery Model

### Required Boot Invariant

Ein offizielles ASL-Gastimage muss auch ohne Agent in einen Zustand booten
koennen, in dem mindestens eine Fallback-Konsole erreichbar ist.

### Consequences

- Agent-Ausfall darf nicht mit Gast-Totalausfall gleichgesetzt werden
- `asld` und `aslconsoled` muessen degraded states unterscheiden koennen
- `aslctl shell` kann in einem degradierten Modus auf eine Fallback-Konsole
  zurueckfallen

## Readiness Model

Zustaende auf Agent-Ebene:

- `not_present`
- `starting`
- `connected`
- `ready`
- `degraded`
- `disconnected`

V1-Empfehlung:

- Distribution kann `running` sein, auch wenn Agent nur `connected` oder
  `degraded` ist
- "fully integrated" entspricht erst `agent=ready`

## Security Model

Der Gastagent ist ein Integrationshelfer, kein Vertrauensanker fuer
Host-Sicherheit.

Darum gilt:

- Host validiert weiterhin alle sicherheitsrelevanten Eingaben
- Agentdaten werden als hilfreich, aber nicht blind autoritativ behandelt
- versionierte Protokolle und defensive Parsing-Regeln sind Pflicht

## CLI Consequences

`aslctl` soll agentabhaengige und agentunabhaengige Pfade sauber trennen.

### Agent-sensitive commands

- `aslctl shell`
- `aslctl exec`
- spaeter Clipboard-/Notification-Kommandos

### Agent-independent or mostly host-driven commands

- `aslctl list`
- `aslctl status`
- `aslctl start`
- `aslctl stop`
- `aslctl mount add`
- `aslctl port list`

Wenn der Agent fehlt oder degradiert ist, muss die CLI einen klaren
Funktionsverlust anzeigen statt nur generisch zu scheitern.

## Failure Model

Wichtige Fehlerfaelle:

- Agent startet nicht
- Agent verliert Hostverbindung
- Agent haengt
- Agent-Version ist inkompatibel
- Agent liefert unvollstaendige Metadaten

V1-Empfehlung:

- Agentfehler fuehren zu `degraded`, nicht automatisch zu `failed`
- `aslctl doctor` unterscheidet zwischen Gastlaufzeit und Agentgesundheit
- Restart des Agenten soll ohne Distro-Neustart moeglich sein, wenn der Gast
  selbst gesund ist

## Non-Goals

- monolithischer Agent als Zentralpunkt aller ASL-Funktion
- zwingende Agentabhaengigkeit fuer Basiskonnektivitaet
- tiefe Host-Sicherheitsentscheidungen im Gast
- permanenter Hintergrundagent fuer beliebige, unklare Erweiterungslogik

## Future Extensions

Spaeter optional:

- Remote command dispatch mit engeren Sicherheitsregeln
- Prozess- und Serviceinventar
- Desktop- und GUI-bezogene Integrationsfunktionen
- richer telemetry und proactive diagnostics

## Follow-up

Diese ADR legt noch nicht fest:

- welches konkrete Agent-Protokoll verwendet wird
- ob `shell` in v1 den Agent zwingend nutzt oder einen Host-Fallback hat
- wie Agent-Upgrades mit Distro-Updates gekoppelt werden
