# ADR-0008 - ASL targets Java, C/C++, Rust, and Node.js as primary developer toolchains

## Status

Accepted

## Date

2026-05-07

## Context

ADR-0001 bis ADR-0007 legen die technische Architektur von ASL fest: WSL2-artige
Utility-VM, Distro-Modell, NAT, brokered Shared Folders, minimaler Agent,
direkter Linux-Boot mit SeaBIOS-Fallback. Damit steht der Unterbau, aber nicht
das Produktziel: **fuer wen** wird ASL gebaut?

Ohne klar benannte Zielgruppe besteht das Risiko, dass die ASL-Roadmap ueber die
Zeit zu einer beliebigen Linux-VM-Erfahrung verflacht. Sowohl die
Performance-Anforderungen (FS-Throughput, Boot-Latenz) als auch die
Integrations-Anforderungen (Shared Folders, Port-Forwards, IDE-Hooks) haengen
direkt davon ab, welche konkreten Workflows erstklassig laufen muessen.

Moegliche Zielgruppen waeren:

- "Hobby-Linux-Nutzer" — Shell und apt reichen.
- "Server-Workloads" — Container, Datenbanken, Background-Daemons.
- "Entwickler" — Build-Toolchains, Editoren, Dev-Server, Debugger.

Der Anwendungsfall, der die meisten ASL-Subsysteme gleichzeitig stresst und der
am ehesten fuer anyOS-Nutzer Mehrwert generiert, ist die
**Entwickler-Workstation**.

## Decision

ASL wird als **Entwicklerplattform** positioniert. Die offiziell unterstuetzten
und priorisierten Sprach-/Toolchain-Profile sind:

1. **Java** (OpenJDK + Maven/Gradle)
2. **C / C++** (gcc, clang, gdb, make, cmake, ninja)
3. **Rust** (rustup-Toolchain, cargo)
4. **Node.js** (nvm + LTS-Node, npm/pnpm)

Diese vier Profile sind die Referenz fuer:

- Performance-Benchmarks (Build-Zeiten auf Shared-Mounts).
- FS-Semantik-Anforderungen (Executable-Bits, Symlinks, Watch-Events).
- Netz-Anforderungen (Outbound HTTPS fuer Paketmanager, Inbound Port-Forward
  fuer Dev-Server).
- Toolchain-Profile in `aslmanager` und `aslctl` (`dev-c`, `dev-rust`,
  `dev-node`, `dev-java`).

Andere Toolchains (Python, Go, .NET, Ruby, PHP, …) duerfen funktionieren, sind
aber **nicht** Ziel der ersten Auslieferung. Sie werden auch nicht aktiv
getestet.

Datenbanken lokal (Postgres, Redis), Container-in-Container (Docker, Podman),
GPU-Workloads und SSH-Server-Betrieb in der Distro sind explizit **out of
scope** fuer die Erstauslieferung.

## Consequences

### Positiv

- Klare Priorisierung fuer Block A-D des Implementierungsplans.
- FS-Performance-Ziel ist konkret messbar (`cargo build`, `npm install`,
  `gradle build`, C++ Build).
- Toolchain-Profile in `aslmanager` haben definierte Inhalte und werden nicht
  zur Wunschliste.
- Spec-Drift in `aslctl-cli.md` wird vermeidbar: Doc-Beispiele koennen sich
  konkret auf diese vier Sprachen beziehen.

### Negativ

- Erwartungsmanagement: Nutzer mit Python- oder Go-Workflows muessen explizit
  geklaert bekommen, dass das nicht der primaere Zielkorridor ist.
- Spaeter-Erweiterung um weitere Toolchains erfordert separate Entscheidung
  (neuer ADR oder Erweiterung dieses ADRs).

### Out of Scope (explizit)

Diese Punkte sind **nicht** Teil des Erstauslieferungsumfangs. Bei spaeterem
Bedarf neuer ADR:

- Container-Runtimes (Docker, Podman, containerd) **innerhalb** der Distro.
- GPU-Beschleunigung im Linux-Gast (CUDA, Mesa-Hardware-Pfad).
- Datenbanken als gemanagte ASL-Services.
- SSH-Server-Betrieb (`sshd` in der Distro mit Inbound vom Aussennetz).
- Zusaetzliche Sprachen: Python, Go, .NET, Ruby, PHP, Swift, Kotlin (Native),
  Haskell, Erlang/Elixir.

## Alternatives Considered

- **Generischer Linux-VM-Anbieter**: Ohne Zielsprachen positionieren. Verworfen,
  weil dann keine messbaren Performance-Ziele und kein Schwerpunkt fuer
  Integration moeglich.
- **Single-Language-Fokus** (nur Rust, weil anyOS in Rust gebaut ist):
  Verworfen, weil das die ASL-Nutzer von anyOS-Nutzern entkoppelt — viele
  Entwickler bringen heterogene Stacks mit (z. B. Java-Backend +
  Node.js-Frontend).
- **Auch Python und Go aufnehmen**: Verworfen fuer Erstauslieferung wegen
  Test-/Wartungsaufwand. Spaeterer Aufnahme-Pfad bleibt offen.

## References

- `todos/asl-anyos-subsystem-linux.md` — Dev-Plattform Implementierungsplan,
  insbesondere Block B2 (Toolchain-Profile).
- ADR-0001 — WSL2-style only.
- ADR-0009 — Editor-Modell.
