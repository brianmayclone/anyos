# ADR-0002 - ASL uses managed distros with layered rootfs storage

## Status

Proposed

## Date

2026-04-23

## Context

ADR-0001 legt fest, dass ASL ausschliesslich als WSL2-artiges, VM-first
Subsystem gebaut wird. Damit braucht ASL ein klares Modell fuer:

- die Verwaltung einer Linux-Instanz als Produktobjekt
- den Aufbau und die Persistenz des Gast-Rootfs
- Updates, Rollback und Diagnose
- die Trennung zwischen importiertem Basiszustand und benutzerspezifischen
  Aenderungen

Ein loses VM-Image-Modell waere fuer Betrieb, UX und Support zu unpraezise.
ASL braucht stattdessen eine klar definierte Verwaltungseinheit und ein
storage-seitig kontrolliertes Layout.

## Decision

ASL verwaltet Linux-Instanzen als **Distributionen**.

Jede Distribution besitzt ein **layered rootfs model** mit:

- einem read-only Base Layer
- einem persistenten writable Overlay Layer
- optional getrenntem State-/Data-Layer fuer Laufzeit- und Benutzerdaten

Das Basisimage wird als importiertes, versionierbares Artefakt behandelt. Alle
schreibenden Gastaenderungen landen nicht im Base Layer, sondern im Overlay.

## Distro Model

Eine ASL-Distribution ist die zentrale Verwaltungseinheit und enthaelt:

- `id`
- `name`
- `base_image_ref`
- `kernel_profile`
- `resources`
- `network_policy`
- `mounts`
- `port_forwards`
- `storage_layout`
- `state`
- `health`
- `owner`

V1 geht von **einer VM pro Distribution** aus.

## Storage Layout

Empfohlene Host-Struktur:

```text
/System/var/asl/
  distros/
    <name>/
      config.json
      images/
        base.img
        overlay.img
        state.img
      runtime/
      logs/
      sockets/
```

### Layer Roles

#### `base.img`

- read-only
- stammt aus Import oder spaeterem Update
- signierbar oder mit Trust-Metadaten versehen
- kann zwischen Versionen ausgetauscht werden

#### `overlay.img`

- persistent
- enthaelt schreibende Rootfs-Aenderungen
- ist an die jeweilige Distribution gebunden
- kann fuer Diagnose und Recovery separat betrachtet werden

#### `state.img`

- optional in v1, empfohlen im Design
- enthaelt mutable Laufzeit- und Benutzerdaten, die nicht eng an den Rootfs-
  Basiszustand gekoppelt sein sollen
- kann spaeter fuer schnellere Resets, Snapshots oder Profilmigration helfen

## Decision Drivers

- klare Trennung zwischen importierter Linux-Basis und lokalen Aenderungen
- bessere Update- und Rollback-Faehigkeit
- einfacheres Debugging bei Rootfs-Korruption
- geringere Gefahr, dass Distributionen zu unstrukturierten Einzelimages
  degenerieren
- bessere Basis fuer spaetere Export-, Clone- und Repair-Workflows

## Consequences

### Positive

- Distributionen werden als erstklassige Produktobjekte modelliert
- Basisimages koennen versioniert und validiert werden
- lokale Aenderungen sind von der Basis getrennt
- Reset-, Clone- und Diagnosepfade werden einfacher
- CLI und Control Plane bekommen ein stabiles Betriebsmodell

### Negative

- Storage-Management wird komplexer als bei einem simplen Monolith-Image
- ASL benoetigt klare Regeln fuer Layer-Lifecycle und Garbage Collection
- Migration zwischen Basisimage-Versionen braucht definierte Kompatibilitaet

## Non-Goals

- ein einziges grosses Raw-Image ohne Layer-Semantik
- schreibbares Basisimage
- automatische unendliche Snapshot-Ketten
- direktes Durchreichen des Host-Filesystems als Gast-Rootfs

## Operational Rules

- Base Layer wird nie in-place beschrieben
- Overlay Layer wird pro Distribution exklusiv verwendet
- Defekte oder inkompatible Base Images muessen klar als solcher diagnostizierbar sein
- `aslctl export` exportiert Distributionen logisch, nicht nur blind einzelne Dateien
- `aslctl clone` kopiert Distro-Metadaten kontrolliert und erzeugt ein eigenes Overlay

## Follow-up

Diese ADR legt noch nicht fest:

- welches konkrete Image-Format verwendet wird
- wie Copy-on-Write intern technisch umgesetzt wird
- wie Snapshots spaeter repräsentiert werden
- ob `state.img` in v1 zwingend getrennt oder anfangs logisch im Overlay
  enthalten ist
