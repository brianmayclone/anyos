# ADR-0003 - ASL uses NAT as the default network mode

## Status

Proposed

## Date

2026-04-23

## Context

ASL benoetigt fuer Linux-Gaeste einen Netzwerkstandard, der:

- fuer Entwickler sofort nuetzlich ist
- sicher und kontrollierbar bleibt
- das Betriebsmodell fuer `asld`, `aslnetd` und `aslctl` vereinfacht
- ohne komplexe Host-Netzwerkkonfiguration tragfaehig ist

Moegliche Grundmodelle sind:

- NAT
- Bridge
- Host-only aehnliche Sondermodi

Da ASL als eng integriertes Subsystem und nicht als klassische frei
konfigurierbare VM-Plattform positioniert ist, braucht es einen sicheren und
einfachen Default.

## Decision

ASL verwendet **NAT als Default-Netzwerkmodus**.

Jede Distribution erhaelt standardmaessig:

- ausgehenden Netzwerkzugang
- interne private Adressierung
- DNS-Aufloesung ueber den Host-Broker
- keine automatische direkte Exponierung eingehender Dienste

Eingehende Erreichbarkeit vom Host oder darueber hinaus erfolgt nur ueber
explizite Port-Freigaben.

## Decision Drivers

- geringere Sicherheitsoberflaeche
- einfache Erstinbetriebnahme
- gute Eignung fuer typische Entwickler- und Paketmanager-Workloads
- klare Policy fuer Host-zu-Gast Zugriff
- weniger Komplexitaet als Bridge im MVP

## Consequences

### Positive

- Gast kann sofort `git`, `curl`, Paketmanager und Remote-Clients nutzen
- keine direkte L2-/Bridge-Abhaengigkeit vom Host-Netz
- eingehende Dienste bleiben standardmaessig privat
- Port-Forwarding wird zum klaren und dokumentierbaren Integrationsmechanismus

### Negative

- Gast ist nicht automatisch wie ein vollwertiger Peer im Netz sichtbar
- bestimmte Discovery- und Broadcast-Szenarien funktionieren nicht direkt
- einige Server- oder Lab-Workflows brauchen spaeter Bridge- oder Sondermodi

## Non-Goals

- Bridge als Default
- automatische Freigabe aller Gastports
- implizite Exponierung von Diensten ins LAN
- komplexe Multi-NIC-Konfigurationen im MVP

## Operational Rules

- Default-Loopback-Exponierung fuer Port-Forwards ist `127.0.0.1`
- Port-Freigaben muessen explizit pro Distribution konfiguriert werden
- `aslctl port list` muss alle aktiven Regeln sichtbar machen
- `aslnetd` ist fuer DNS-, NAT- und Forwarding-Policy autoritativ
- Host-UI und CLI sollen klar sichtbar machen, welche Dienste oeffentlich,
  lokal oder gar nicht exponiert sind

## Future Extensions

Spaeter optional:

- Bridge-Modus fuer Spezialfaelle
- mDNS/Service Discovery
- Hostname-Aufloesung Host <-> Gast
- feinere Firewall- und Binding-Regeln

## Follow-up

Diese ADR legt noch nicht fest:

- wie NAT intern paketseitig umgesetzt wird
- wie Portkollisionen geloest werden
- ob freie Ports automatisch vorgeschlagen werden
- ob Bridge-Modus spaeter global oder pro Distribution aktiviert wird
