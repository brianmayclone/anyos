# ADR-0001 - ASL uses a VM-first architecture

## Status

Accepted

## Date

2026-04-23

## Context

anyOS soll mit ASL eine produktiv nutzbare Linux-Kompatibilitaetsplattform
erhalten. Die zentrale Architekturfrage ist, ob Linux-Unterstuetzung ueber:

- einen direkten Linux-Syscall-Kompatibilitaetslayer auf anyOS
- oder ueber eine kontrollierte Linux-Utility-VM

bereitgestellt werden soll.

anyOS besitzt bereits relevante Vorleistungen fuer den zweiten Ansatz:

- VM- und vCPU-Syscalls im Kernel
- x86-Virtualisierungsbackends fuer VMX/SVM
- bestehende Service-Orchestrierung via `svc`
- vorhandene Integrationsmuster fuer Host/Guest-Kommunikation

Gleichzeitig waere ein WSL1-artiger ABI-/Syscall-Kompatibilitaetslayer fuer
Linux semantisch breit, fehleranfaellig und langfristig teuer zu pflegen.

## Decision

ASL wird als **VM-first Subsystem** gebaut.

Linux laeuft in einer von anyOS kontrollierten Utility-VM. anyOS bleibt Host,
Policy-Owner und Integrationsplattform.

Die VM wird nicht als isolierte Endanwender-App verstanden, sondern als
systemisches Subsystem mit:

- service-gesteuertem Lifecycle
- klaren Ressourcen- und Sicherheitsgrenzen
- kontrollierten Integrationskanaelen fuer Console, Filesystem, Netzwerk und
  spaeter Desktop-Features

## Decision Drivers

- geringeres technisches und semantisches Risiko
- Nutzung bereits vorhandener Hypervisor-Bausteine
- klarere Sicherheitsgrenzen
- bessere Debugbarkeit und Fehlereingrenzung
- realistischere Time-to-Value fuer ein MVP
- sauberere Erweiterbarkeit fuer spaetere GUI- oder Entwicklerintegration

## Consequences

### Positive

- schnellster realistisch lieferbarer Pfad zu Linux-Unterstuetzung
- gute Isolation zwischen anyOS und Linux-Gast
- Linux kann weitgehend unveraendert betrieben werden
- Host-Policy fuer Mounts, Ports und Ressourcen bleibt klar zentralisiert
- klares Produktmodell: Distributionen statt lose VM-Images

### Negative

- Linux-Binaries laufen nicht direkt auf dem anyOS-Kernel
- Boot- und Laufzeitkosten einer VM bleiben grundsaetzlich vorhanden
- Shared-Folder- und Integrationspfade muessen bewusst gebaut werden
- GUI-Integration fuer Linux-Apps ist ein zusaetzliches Teilprojekt

### Neutral / Follow-up

- ASL benoetigt eigene Host-Daemons fuer Control Plane, Filesystem, Netzwerk,
  Console und Observability
- fuer das MVP wird NAT, Terminal und kontrolliertes Shared-Foldering priorisiert
- Snapshots, GPU und Linux-GUI-Apps werden explizit nachgelagert

## Rejected Alternatives

### 1. Linux-Syscall-Kompatibilitaetslayer auf anyOS

Verworfen wegen:

- hoher Implementierungsbreite
- schwieriger ABI- und POSIX-Semantik
- grosser Kernel-Oberflaeche
- schlechterer Sicherheits- und Testbarkeit

### 2. Externe Voll-VM ohne tiefe Systemintegration

Verworfen wegen:

- schlechter UX
- schwacher Dateisystem- und Portintegration
- fehlendem Subsystem-Charakter
- unklarer Betriebs- und Policy-Verantwortung

## Follow-up Decisions

Nachgelagerte ADRs sollen mindestens festhalten:

- Distro- und Rootfs-Modell
- Standard-Netzwerkmodus
- Shared-Folder-Architektur
- Gastagent-Scope
- Security- und Capability-Modell fuer ASL-Management
