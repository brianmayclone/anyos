# ADR-0011 - ASL image trust: phased verification, GPG/SHA512 deferred

## Status

Accepted

## Date

2026-05-07

## Context

ADR-0010 fordert "Image-Trust und Import-Validierung" als Teil der
Enterprise-Quality-Bar. Aktuell laedt `aslmanager` (ueber
`apps/aslmanager/src/installer.rs`) ein Debian Cloud Image direkt von
`https://cloud.debian.org/images/cloud/trixie/latest/`. Die Verifikation
besteht heute aus:

1. URL-Whitelist (`is_allowed_debian_url`) — nur Debian-Mirror-Hosts.
2. HTTPS mit TLS-Validierung ueber `libhttp_client`.
3. Mindestgroesse (`RAW_DISK_MIN_BYTES`).
4. MBR-Magic-Bytes (`0x55 0xAA` bei Offset 510).

Eine kryptographische Verifikation des heruntergeladenen Images **fehlt**.
Das schuetzt nicht gegen:

- kompromittierten Debian-Mirror.
- Man-in-the-middle vor TLS-Termination (z. B. CA-Compromise).
- Stille Bit-Errors auf Speicher (TLS schuetzt nur den Transport).

Die naheliegende vollstaendige Loesung waere:

- `SHA512SUMS` und `SHA512SUMS.sign` von `cloud.debian.org` herunterladen.
- Debian-Cloud-Signing-Key gegen GPG verifizieren (Trust-Anchor in anyOS
  pinnen).
- Hash des heruntergeladenen Images berechnen und gegen den signierten
  `SHA512SUMS`-Eintrag pruefen.

Das hat zwei Probleme:

- **GPG in anyOS**: Es gibt heute keine GPG-Implementierung im anyOS-Userland.
  Eine zu integrieren ist ein eigenes Projekt — RFC 4880, Subkey-Handling,
  Trust-Database. Mehrere Mannwochen.
- **Streaming-SHA waehrend Download**: `libhttp_client::download_progress`
  schreibt direkt in eine Datei. Eine inline Hash-Berechnung erfordert
  entweder einen Streaming-Callback (existiert nicht) oder einen zweiten
  vollstaendigen Read-Pass nach dem Download (3 GiB I/O pro Install — teuer
  aber machbar).

## Decision

Image-Trust wird **gestaffelt** geliefert. Stufe 1 ist heute aktiv. Stufen 2
und 3 folgen, sind aber nicht Teil dieser ADR-Auslieferung.

### Stufe 1 (heute, Stand 2026-05-07)

- URL-Whitelist auf Debian-Mirror.
- HTTPS mit TLS-Validierung.
- Mindestgroesse-Check.
- MBR-Magic-Validierung.
- **Explizit dokumentierte Limitation**: kein kryptographischer
  Image-Verify. Das wird im Installer-Log als "WARN: image verified by
  size + MBR signature only" sichtbar gemacht, damit Operatoren wissen
  was sie haben.

### Stufe 2 (umgesetzt 2026-05-07)

- SHA512-Hash-Pinning gegen eine **versionsgebundene** Konstante
  `DEBIAN_RAW_SHA512_HEX` in `apps/aslmanager/src/constants.rs`.
- Hash wird **nach** dem Download in einem zweiten Read-Pass berechnet
  (`apps/aslmanager/src/installer.rs::compute_file_sha512`). I/O-Kosten:
  3 GiB Lese-Pass bei Erstinstallation, einmalig.
- Vergleich erfolgt mit konstanter Zeit
  (`aslmanager_core::const_time_eq`).
- SHA-512-Implementierung kommt aus `libtls::crypto::sha512` —
  derselbe Code der TLS verwendet, kein paralleler Hash-Stack.
- Bei Mismatch wird der Download verworfen und eine ausfuehrliche
  Diagnose geloggt (expected vs. actual + Hinweis auf
  `SHA512SUMS`-Aktualisierung).
- 10 NIST-FIPS-180-4-Tests sichern die Hash-Implementierung.
- Image-URL und Pin sind als **Tupel** in `constants.rs` versioniert,
  damit Updates nicht silent eine andere Version laden.

**Update-Prozedur** beim naechsten Debian-Release: `SHA512SUMS` von
`https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS` ziehen,
neue Konstante setzen, Release-Notes aktualisieren.

### Stufe 3 (weitere Ausbaustufe, abhaengig von GPG-Toolchain in anyOS)

- `SHA512SUMS` und `SHA512SUMS.sign` automatisch laden.
- GPG-Verify gegen gepinnten Debian-Cloud-Signing-Key.
- Hash-Pin im Code wird **optional** (Code haelt einen letzten bekannten
  Hash als Defense-in-Depth gegen GPG-Implementierungs-Bugs).
- Vorbedingung: GPG-Implementierung in anyOS ist verfuegbar — heute
  nicht der Fall, eigener Roadmap-Punkt.

### Stage-Indikatoren im Log

- `OK: <label> SHA-512 matches pinned digest (ADR-0011 stage 2).` —
  Stufe 2 erfolgreich.
- `WARN: <label> verified by size + MBR signature only (ADR-0011 stage 1).` —
  Artefakt hat keinen Pin, nur Stufe 1 wurde geprueft (z. B. Drittanbieter-
  Image).
- `ERROR: <label> SHA-512 mismatch (ADR-0011 stage 2).` — Pin ist
  veraltet oder Image kompromittiert/korrupt. Operator muss handeln.

### Geltungsbereich

Dieser ADR gilt fuer den `aslmanager`-Installer. Spaeter koennen weitere
Image-Quellen (z. B. eigene anyOS-distribuierte Images, Drittanbieter)
hinzukommen — fuer die gilt **mindestens** Stufe 1 als Untergrenze, idealerweise
direkt Stufe 2 oder 3.

## Consequences

### Positiv

- Klar dokumentierter Status: Stufe 1 liefert TLS + Whitelist + Size +
  MBR. Niemand glaubt versehentlich es waere mehr.
- Stufe 2 ist **mit minimalen anyOS-Aenderungen** umsetzbar — kein neues
  Subsystem, nur ein zweiter Read-Pass und eine Konstante.
- Stufe 3 ist **nicht blockiert** — wenn jemand GPG fuer anyOS baut,
  ist die Integration straightforward.

### Negativ

- Stufe 1 ist objektiv schwach. Akzeptiert weil Debian-Mirror-Compromise
  ein vergleichsweise hochrangiger Threat-Actor ist und ASL in der
  Erstauslieferung ein Entwickler-Tool ist, kein Deployment-Target fuer
  produktive Workloads in regulierten Umgebungen.
- Doppel-Read fuer Stufe 2 verdoppelt Disk-I/O bei Erstinstallation. Bei
  3 GiB Image und SSD-Geschwindigkeit ~10-20 s zusaetzlich. Akzeptabel
  fuer Einmal-Operation.

### Was bei jedem ASL-Release zu tun ist (sobald Stufe 2 aktiv)

- Pin auf neuesten Debian-Cloud-SHA512 aktualisieren.
- Image-URL pruefen (Debian rotiert Pfade selten, aber moeglich).
- Release-Notes erwaehnen welche Debian-Version eingebunden ist.

## Alternatives Considered

- **Sofort Stufe 3**: Verworfen wegen GPG-Integrationsaufwand. ASL waere
  blockiert auf eine eigene Krypto-Toolchain-Diskussion.
- **Stufe 1 dauerhaft**: Verworfen weil ADR-0010 explizit Image-Trust
  fordert. Stufe 1 ist Ueberbrueckung, nicht Ziel.
- **Streaming-SHA waehrend Download**: Verworfen weil
  `libhttp_client::download_progress` API kein Hook-Mechanismus hat.
  Alternative waere eine API-Erweiterung — ueberholt Stufe 2 nicht
  signifikant.
- **Eigenes ASL-Image hosten** (mit eigener Signatur): Verworfen weil
  Debian's eigene Distribution genuegend Trust mitbringt und ASL keine
  ASL-spezifischen Image-Modifikationen braucht.

## References

- ADR-0010 — Enterprise-Quality-Bar (fordert Image-Trust).
- `apps/aslmanager/src/installer.rs::verified_artifact` — heutige Stufe-1-
  Pruefung.
- `apps/aslmanager/src/config.rs::is_allowed_debian_url` — URL-Whitelist.
- `todos/asl-anyos-subsystem-linux.md` Block B1 — verlinkt diesen ADR
  fuer Stufe-2-/Stufe-3-Implementierung.
