# ADR-0001 - ASL is WSL2-style only

## Status

Accepted

## Date

2026-04-23

## Context

anyOS soll mit ASL eine produktiv nutzbare Linux-Kompatibilitaetsplattform
erhalten. Dabei soll es genau einen Architekturpfad geben: eine kontrollierte
Linux-Utility-VM nach WSL2-artigem Modell. Ein WSL1-artiger
Syscall-/ABI-Kompatibilitaetslayer ist nicht Teil der Produktstrategie.

anyOS besitzt bereits relevante Vorleistungen fuer den zweiten Ansatz:

- VM- und vCPU-Syscalls im Kernel
- x86-Virtualisierungsbackends fuer VMX/SVM
- bestehende Service-Orchestrierung via `svc`
- vorhandene Integrationsmuster fuer Host/Guest-Kommunikation

Diese Festlegung verhindert, dass ASL in zwei konkurrierende
Kompatibilitaetsmodelle zerfaellt. Der WSL1-artige Pfad wird fuer anyOS nicht
verfolgt.

## Decision

ASL wird als **ausschliesslich WSL2-artiges, VM-first Subsystem** gebaut.

Linux laeuft in einer von anyOS kontrollierten Utility-VM. anyOS bleibt Host,
Policy-Owner und Integrationsplattform.

Die VM wird nicht als isolierte Endanwender-App verstanden, sondern als
systemisches Subsystem mit:

- service-gesteuertem Lifecycle
- klaren Ressourcen- und Sicherheitsgrenzen
- kontrollierten Integrationskanaelen fuer Console, Filesystem, Netzwerk und
  spaeter Desktop-Features

Ein WSL1-artiger Linux-Syscall-Kompatibilitaetslayer auf dem anyOS-Kernel wird
weder parallel entwickelt noch spaeter als zweite Betriebsart eingefuehrt.

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

## Non-Goals

- Linux-Syscall-Kompatibilitaetslayer auf anyOS
- duales Modell aus WSL1-artigem und WSL2-artigem Betrieb
- externe Voll-VM ohne tiefe Systemintegration
- unkontrollierte Linux-Laufzeit ausserhalb des ASL-Control-Plane-Modells

## Follow-up Decisions

Nachgelagerte ADRs sollen mindestens festhalten:

- Distro- und Rootfs-Modell
- Standard-Netzwerkmodus
- Shared-Folder-Architektur
- Gastagent-Scope
- Security- und Capability-Modell fuer ASL-Management
