# ADR-0010 - ASL ships at an enterprise quality bar

## Status

Accepted

## Date

2026-05-07

## Context

ADR-0001 bis ADR-0009 legen Architektur, Distro-Modell, Netzwerk, Shared
Folders, Agent, Boot, Firmware, Toolchain-Scope und Editor-Modell fest. Damit
ist beschrieben **was** ASL liefert. Nicht beschrieben ist **mit welcher
Qualitaetsstufe**.

In der Praxis macht der Unterschied zwischen einer "tut-meistens"-Linux-VM und
einer Plattform, der Entwickler ihren Arbeitstag anvertrauen, **nicht** die
Featureliste, sondern Performance, Stabilitaet, Diagnostizierbarkeit und
Testabdeckung. ASL ist als Entwicklerplattform positioniert (ADR-0008) — wenn
ein `cargo build` regelmaessig haengt, ein `aslctl shell` mal antwortet und mal
nicht, oder ein Crash der asld den Workflow killt, ist das gesamte Subsystem
unbenutzbar, egal wie viele Features es hat.

Diese Erwartungshaltung steht teilweise schon in den vorhandenen Doks
(`asl-anyos-subsystem-linux.md` "Serviceability vor Feature-Fuelle",
"Enterprise-taugliche Verwaltungsfunktionen"), aber sie ist nicht als
verbindliche Querschnittsanforderung verankert. Folge: Beim Implementieren
einzelner Features ist nicht klar, ob es reicht "es laeuft" zu zeigen — oder
ob Performance-Budget, Stabilitaetsbudget und Testabdeckung Pflicht sind.

## Decision

Jede ASL-Komponente und jedes neu hinzukommende Feature wird gegen einen
**Enterprise-Quality-Bar** geliefert. Dieser ADR macht das verbindlich.

### 1. Performance-Budget

- Fuer jeden User-spuerbaren Pfad gibt es ein dokumentiertes Latenz- oder
  Durchsatz-Budget.
- Referenzpfade fuer ASL:
  - `aslctl run` einer trivialen Distro-Operation (z. B. `true`):
    Setup-Latenz **<= 250 ms** in einer warmen Distro.
  - `cargo build` eines mittelgrossen Projekts auf Shared-Mount:
    **<= 2x** der nativen Distro-FS-Geschwindigkeit. Bei groesserem Faktor
    ist FS-Performance ein Block-Issue.
  - Distro-Boot bis Shell-Ready: **<= 5 s** in warmem Page-Cache,
    **<= 10 s** kalt.
  - asld IPC roundtrip (LIST, STATUS): **<= 50 ms** P95.
- Performance wird **gemessen**, nicht behauptet. Benchmarks gehoeren zur
  Definition of Done eines Features.

### 2. Stabilitaetsanforderungen

- Kein Feature wird als "fertig" markiert ohne:
  - definiertes Verhalten bei Distro-Crash.
  - definiertes Verhalten bei asld-Restart (Runtime-State-Recovery).
  - definiertes Verhalten bei Broker-Ausfall (`aslfsd`, `aslnetd`,
    `aslconsoled`, `aslobsd`).
  - definiertes Verhalten bei Concurrency (≥2 parallele Operationen).
- Fehlerpfade haben **definierte Fehlercodes** (`AsldError`-Varianten) und
  **menschenlesbare Meldungen**. Kein generisches `panic!` im Hot-Path.
- Restart-Strategien: exponentielles Backoff, kein Endlos-Loop, expliziter
  `failed` State mit Diagnosehinweis (siehe `asl-anyos-subsystem-linux.md`,
  Abschnitt "Restart-Strategie").

### 3. Testabdeckung

Jede ASL-Komponente liefert Tests in drei Schichten:

1. **Unit-Tests** fuer Parser, Validatoren, Wire-Encoding, Config-Roundtrip.
   Mindestabdeckung der Edge-Cases:
   - leere Inputs
   - Maximalgrenzen (lange Pfade, viele Mounts, viele Port-Forwards)
   - ungueltige Inputs (must-reject statt must-accept-and-fail-later)
   - Concurrency-Sicherheit von Datenstrukturen
2. **Integrationstests** fuer asld mit `MockStore` / `MockBackend`. Mindest-
   szenarien: Lifecycle (create → start → stop → delete), Mount-Roundtrip,
   Port-Forward-Roundtrip, Exec-Roundtrip.
3. **Wire-Compatibility-Tests** zwischen aslctl ↔ asld: jeder neue Subcommand
   hat einen Test der den Wire-Command festschreibt. Verhindert Spec-Drift.

End-to-End-Tests gegen eine echte Distro-VM kommen sobald Block B
(Distro-Bringup) lieferbar ist — als CI-Smoketest mit definiertem
Pflichtszenario-Set (siehe `asl-anyos-subsystem-linux.md`, Abschnitt
"Pflichtszenarien").

### 4. Observability als Pflicht

- **Strukturiertes Logging** mit Komponenten-Tag (`[asld]`, `[asld:vm]`,
  `[asld:agent]`, `[aslnetd]`, `[aslnetd:dns]`, `[aslnetd:nat]`, `[aslfsd]`,
  `[aslconsoled]`, `[aslobsd]`).
- Log-Levels: ERROR, WARN, INFO, DEBUG, TRACE. Default INFO. Kein `print` im
  Hot-Path.
- **Status-Endpoints** schreiben strukturiert: jede ASL-Komponente fuehrt
  ein Status-File `/System/var/asl/<daemon>.status` mit definierten Feldern.
- **Event-Trail**: aslobsd ist die einzige Quelle der Wahrheit fuer
  Lifecycle-Events. Andere Daemons routen ihre Events dorthin, kein
  Eigen-Event-Speicher.

### 5. Definition of Done fuer Features

Ein Feature gilt als ausgeliefert, wenn alle Punkte erfuellt sind:

1. Code in main, kompiliert ohne Warnings unter `cargo +stable build`.
2. Unit-Tests fuer den happy path und mindestens 3 Edge-Cases.
3. Wire-Compatibility-Test wenn IPC betroffen.
4. Performance-Budget gemessen und im Budget — sonst dokumentiertes
   Tradeoff.
5. Fehlerpfade dokumentiert: was passiert bei Crash, Restart,
   Concurrency-Konflikt?
6. Doku aktualisiert: Spec-Doc + Plan-Doc + ADR falls
   Architekturentscheidung.

## Consequences

### Positiv

- Klare Latte fuer jedes Feature. Diskussionen ob "fertig oder nicht" sind
  faktenbasiert pruefbar.
- Spec-Drift wird durch Wire-Tests strukturell verhindert.
- Performance-Regressionen werden frueh sichtbar statt im Feldeinsatz.
- Diagnostizierbarkeit ist nicht optional — Operatoren koennen ein Problem
  ohne Source-Code-Zugriff einkreisen.

### Negativ

- Hoeherer Implementierungsaufwand pro Feature. Insbesondere
  Performance-Budgets und Concurrency-Tests sind nicht trivial.
- Bestehender Code (Audit-Stand) erfuellt diese Latte teilweise nicht. Es
  entsteht Refactoring-Druck (z. B. strukturiertes Logging, Status-Endpoints,
  Event-Trail-Konsolidierung).
- Geschwindigkeit der Erstauslieferung sinkt. Das ist akzeptiert: ASL
  positioniert sich als Plattform, nicht als Demo.

### Migration des Bestandscodes

Bestehender Code wird **nicht** retroaktiv blockiert, aber jede Aenderung
an einer Datei zieht die betroffenen Dateien auf den neuen Bar (Boy-Scout-
Regel). Konkrete erste Refactor-Ziele:

- aslnetd: strukturiertes Logging mit `[aslnetd:nat]`, `[aslnetd:dns]`,
  `[aslnetd:dhcp]` Tags statt eines pauschalen `[aslnetd]`.
- asld: Performance-Mess-Hooks an LIST/STATUS/EXEC mit p95-Reporting in
  Status-File.
- aslobsd als alleinige Event-Quelle etablieren — andere Daemons schreiben
  hin, lesen nicht.

## Alternatives Considered

- **Keine explizite Quality-Bar**: Verworfen, weil sich ohne klare Latte
  in der Praxis "es kompiliert" als Definition durchsetzt.
- **Test-Pyramide nach Standard 80/20-Regel**: Verworfen, weil ASL-Subsysteme
  unterschiedliche Risiko-Charakteristik haben (Wire-Protokoll ist hoechstes
  Risiko, GUI-Flow geringeres). Die drei Schichten Unit/Integration/Wire-Compat
  sind risiko-orientiert, nicht zahlenorientiert.
- **Performance-Budgets nach Inbetriebnahme**: Verworfen, weil
  Performance-Probleme nach Auslieferung deutlich teurer zu fixen sind.

## References

- ADR-0008 — Developer Toolchains (definiert die Last-Pfade fuer
  Performance-Budgets).
- `asl-anyos-subsystem-linux.md` — Abschnitte "Architekturprinzipien" (Punkt
  5: Serviceability vor Feature-Fuelle), "Observability und Betrieb",
  "Teststrategie".
- `asl-implementation-audit.md` — aktueller Stand vs. Spec.
