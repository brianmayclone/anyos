# ADR-0004 - ASL uses brokered explicit shared folders

## Status

Proposed

## Date

2026-04-23

## Context

ASL soll Linux-Gaeste produktiv nutzbar machen, ohne anyOS-Hostdaten,
CoreFS-Semantik und Linux-POSIX-Erwartungen unsauber zu vermischen.

Shared Folders sind fuer ASL aus drei Gruenden zentral:

- Entwickler wollen Projekte, Quelltexte und Artefakte zwischen Host und Gast
  austauschen
- anyOS-Apps wie Terminal, Finder und spaeter Editor-Integration brauchen einen
  kontrollierten Dateipfad in den Gast
- der Gast darf dennoch nicht pauschal oder implizit Zugriff auf Hostdaten
  erhalten

Eine naive Strategie wie "Host-Home automatisch mounten" oder "CoreFS direkt als
Gast-Dateisystem exponieren" waere sicherheitlich und semantisch zu riskant.

## Decision

ASL verwendet **brokered explicit shared folders**.

Hostverzeichnisse werden als explizite Exportpunkte durch den Hostdienst
`aslfsd` freigegeben und ueber einen paravirtualen Dateisystemkanal im Linux-
Gast eingehangen.

Wesentliche Festlegungen:

- keine automatischen pauschalen Host-Mounts
- keine direkte CoreFS-Durchreichung als Linux-kompatibles Root- oder Universal-
  Dateisystem
- Freigaben sind immer pro Distribution und explizit konfiguriert
- die Freigabe ist ein kontrollierter Export mit Policy, Mapping und Audit
- Shared Folders sind fuer DX wichtig, aber nicht als 100 Prozent POSIX-
  identische Linux-Dateisysteme positioniert

## Architecture

### Components

#### `aslfsd`

Host-seitiger Broker fuer:

- Exportdefinitionen
- Pfadvalidierung
- Rechtepruefung
- Datei- und Metadatenoperationen
- Caching- und Konsistenzpolitik
- Event-Weitergabe
- Audit und Fehlerdiagnose

#### Guest FS Client

Gast-seitiger Client fuer:

- Mount der freigegebenen Exportpunkte
- Request/Reply-Verkehr zum Host-Broker
- Uebersetzung auf Linux-VFS-Operationen
- optionale Cache-Hinweise

#### Control Plane

`asld` bleibt Policy-Owner fuer:

- welche Exporte fuer welche Distribution erlaubt sind
- mit welchem Modus ein Export eingebunden wird
- Lebensdauer und Aktivierung der Mounts

## Mount Model

Jeder Shared Folder ist ein eigenes Mount-Objekt mit mindestens:

- `host_path`
- `guest_path`
- `mode`
- `metadata_mode`
- `case_mode`
- `exec_policy`
- `watch_policy`

### Access Modes

- `readonly`
- `readwrite`

### Metadata Modes

- `strict`
  Ziel: moeglichst genaue Metadatenabbildung, konservativer und langsamer
- `relaxed`
  Ziel: bessere Praktikabilitaet fuer typische Dev-Workloads, mit tolerierter
  semantischer Unschärfe

### Case Modes

- `host-native`
- `case-sensitive`
- `case-folded`

V1 sollte konservativ mit `host-native` starten und Abweichungen sichtbar
dokumentieren.

### Exec Policy

- `inherit`
- `noexec`
- `host-metadata`

V1-Empfehlung:

- fuer allgemeine Projektordner `inherit`
- fuer sensible Freigaben optional `noexec`

## Security Model

### Default Policy

- keine Shared Folders ohne explizite Konfiguration
- kein automatischer Mount des Host-Homes
- keine impliziten schreibbaren Standardfreigaben
- schreibende Freigaben muessen bewusst aktiviert werden

### Trust Boundary

Der Linux-Gast ist fuer Shared Folders ein untrusted oder semi-trusted Client.
`aslfsd` validiert deshalb jede Anfrage als Host-Grenze.

### Required Controls

- Normalisierung und Validierung aller Pfade
- Schutz gegen Path Traversal
- keine Flucht ausserhalb von `host_path`
- Rate Limits fuer fehlerhafte oder missbraeuchliche Clients
- klares Logging bei Mount- und Zugriffsfehlern

## Semantics

### Positionierung

ASL-Shared-Folders sind **POSIX-nahe Entwicklungsfreigaben**, aber keine
Garantie fuer vollstaendige Linux-Dateisystemsemantik.

Das muss im Produkt klar kommuniziert werden.

### Consequences

Folgende Bereiche koennen abweichen oder bewusst eingeschraenkt sein:

- UID/GID-Mapping
- Executable-Bits
- Symlink-Verhalten
- Advisory Locking
- Hardlinks
- inotify-/watcher-Verhalten
- Timestamp-Aufloesung
- atomare Rename-/Replace-Faelle ueber Host- und Gastgrenzen

### Operational Guidance

Fuer folgende Workloads sollen Linux-native Verzeichnisse im Gast empfohlen
werden statt Host-Shares:

- Paketmanager-Caches
- Datenbanken
- Container- oder VM-nahe Engines
- grosse Build-Output-Verzeichnisse mit hoher Metadatenlast
- Workloads mit starker Dateiwatcher-Abhaengigkeit

## Integration with CoreFS

ASL behandelt CoreFS als Host-Dateisystem, nicht als Linux-kompatible
Universalabstraktion.

Darum gilt:

- CoreFS bleibt auf Host-Seite lokal autoritativ
- `aslfsd` uebersetzt Host-Semantik in ein kontrolliertes Gastprotokoll
- Linux sieht einen ASL-Share, nicht "CoreFS direkt"

## Caching and Consistency

V1-Empfehlung:

- konservatives Metadata-Caching
- Write-Through oder sehr kurze Dirty-Window-Strategie
- klare Mount-Optionen statt versteckter Heuristiken

Designziele:

- vorhersehbares Verhalten vor Maximalperformance
- einfache Diagnose bei Inkonsistenzen
- spaeter schrittweise Performance-Optimierung pro Workloadklasse

## Event and Watch Model

Watcher-/Event-Semantik ist wichtig fuer Editor-, Build- und Dev-Server-UX.

V1-Festlegung:

- best-effort Event-Weitergabe
- keine vollstaendige Garantie fuer Linux-inotify-Aequivalenz
- klare Dokumentation fuer Tools mit hoher Watch-Abhaengigkeit

## Failure Model

Shared Folders muessen robuste Fehlerzustaende haben fuer:

- Hostdienst-Neustart
- Gast-Neustart
- kurzfristig ungueltige Hostpfade
- Rechteaenderungen am Host
- harten Abbruch einzelner Clients

V1-Empfehlung:

- Fehler als I/O-Fehler oder klarer Degraded-Status im Gast sichtbar machen
- `aslctl doctor` soll Shared-Folder-Gesundheit pruefen koennen
- `aslctl mount validate` soll Konfiguration und Erreichbarkeit vorab testen

## Non-Goals

- automatischer Vollzugriff auf Benutzerdaten des Hosts
- direkte Exponierung des Host-Root-Dateisystems
- vollstaendige POSIX-Kompatibilitaet fuer alle Linux-Workloads
- CoreFS als direktes Gast-Dateisystem
- unsichtbare Hintergrund-Mounts ohne Benutzerwissen

## Operational Rules

- jeder Export ist pro Distribution explizit
- schreibbare Mounts muessen klar sichtbar sein
- Host-UI und CLI sollen Mount-Modus und Risikostufe anzeigen
- Pfad-Aliasing und Case-Semantik muessen fuer Benutzer nachvollziehbar sein
- Shared Folders sind von Rootfs und Distro-State logisch getrennt

## Future Extensions

Spaeter optional:

- workload-spezifische Cache-Profile
- intelligentere Watch-Weitergabe
- differenzierte Symlink-Policies
- Team-/Projektprofile fuer standardisierte Mountsets
- virtio-fs-aehnlicher nativer Transport, falls V1-Protokoll an Grenzen stoesst

## Follow-up

Diese ADR legt noch nicht fest:

- welches konkrete Protokoll `aslfsd` spricht
- ob der Gastclient kernel-nah oder user-space-basiert umgesetzt wird
- wie UID/GID-Mapping exakt modelliert wird
- welche Mount-Optionen in `aslctl` ab v1 schon freigegeben werden
