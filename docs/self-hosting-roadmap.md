# anyOS Native Self-Hosting Roadmap

## Zielbild

Self-hosting fuer anyOS bedeutet: anyOS kann seine eigene Software direkt in
anyOS entwickeln, bauen, testen, versionieren und debuggen. Der gesamte
Primaerpfad ist nativ:

- `crust` kompiliert Rust-Code direkt unter anyOS
- `ccargo` baut Workspaces, Libraries, Apps, Tools und spaeter den Kernel
- `agit`/`libgit` deckt Versionierung nativ ab
- `anycode` ist die vollstaendige IDE-Oberflaeche
- `anytrace` ist der native Debugger fuer Prozesse, Libraries und spaeter
  Kernel-nahe Szenarien

Linux-, ASL- oder Host-Toolchains sind kein Bestandteil dieses Zielpfads.
Sie duerfen hoechstens zur Entwicklung ausserhalb des Zielsystems benutzt
werden, aber nicht als Self-Hosting-Loesung gelten.

## Leitentscheidung

Der tragfaehige Pfad ist ein nativer Stufenplan:

1. **Compiler zuerst stabilisieren:** `anyrc`/`crust` muss `anyos_std` und eine
   kleine, definierte Rust-Untermenge reproduzierbar bauen.
2. **Buildsystem als Produkt behandeln:** `ccargo` ist nicht nur ein Wrapper,
   sondern die zentrale native Build-API fuer IDE, CLI und CI-Sweeps.
3. **IDE integriert native Dienste:** `anycode` spricht mit `ccargo`, `agit`
   und `anytrace` ueber strukturierte Ausgabe oder direkte Libraries, nicht ueber
   externe Toolchains.
4. **Kernel zuletzt:** Kernel-Self-Hosting kommt erst, wenn Libraries und
   Userspace stabil nativ gebaut werden.

Das vermeidet einen Alles-oder-nichts-Sprung. Jeder Schritt erzeugt ein
benutzbares Stueck des nativen Systems.

## Aktueller Stand

### `crust` / `anyrc`

Vorhanden:

- Lexer, Parser, HIR, MIR, Borrowcheck, Monomorphisierung, x86_64-Codegen
- ELF-, Objekt- und RLIB-Ausgabe
- Module, `cfg`, Makros, Generics, Traits, Enums und `no_std`-Features in
  Teilen
- eigener Testbereich in `libs/anyrc_tests`

Blocker:

- Hosttests waren nicht korrekt im Host-Modus verdrahtet
- Tests haben API-Drift gegen `CompileOptions` und `TokenKind::IntLit`
- `anyos_std` scheitert aktuell noch an Parser-/Typechecker-Luecken
- Parser-Panics duerfen im Buildpfad nicht mehr auftreten, sondern muessen
  Diagnostics werden

### `ccargo` / `acargo`

Vorhanden:

- Manifest-, Workspace-, Dependency- und Feature-Handling
- Buildscript-Auswertung
- Fingerprints fuer inkrementelle Builds
- Registry-Code und Lockfile-Ansaetze
- direkte Nutzung von `anyrc` als Library

Blocker:

- der grosse Sweep ist zu grob und erzeugt zu viele gleichzeitige Fehler
- `libs/stdlib` ist noch nicht baubar
- externe crates duerfen fuer den nativen Kernpfad nicht vorausgesetzt werden
- Buildfehler muessen strukturierter aus `ccargo` herauskommen

### `agit` / `libgit`

Vorhanden:

- `init`, `clone`, `add`, `status`, `commit`, `log`, `diff`, `branch`,
  `checkout`, `remote`, `fetch`, `pull`, `push`, `config`, `tag`, `rm`,
  `reset`, `show`, `rev-parse`, `hash-object`, `cat-file`
- Smart-HTTP-Transport und Pack-Handling
- `anycode` kann Git bereits ueber Prozessaufrufe integrieren

Naechster Schritt:

- IDE-freundliche `libgit`-API statt nur Porcelain-Parsing
- strukturierter Status/Diff fuer grosse Repos
- robuste Credentials-, Remote- und Konflikt-Workflows

### `anycode`

Vorhanden:

- Editor, Tabs, File Tree, Search, Symbols, Problems, Output, Run Panel,
  Git Panel, Settings und Task-Logik
- Buildprozesse mit Pipe-Ausgabe
- Rust/C/Makefile-Erkennung und Tool-Prerequisite-Check

Blocker:

- Buildpfad ist noch kommandoorientiert statt backend-/diagnostics-orientiert
- keine harte `ccargo`-Projekt-API fuer Targets, Artifacts und Problems
- Debugger ist noch nicht als IDE-Backend integriert

### `anytrace`

Vorhanden:

- Attach/Detach
- Suspend/Resume
- Single-Step
- Register-Refresh
- Memory-Read
- Debug-Event-Polling
- UI fuer Register, Stack, Memory, Disassembly, Timeline und Output

Blocker:

- Breakpoint-Set/Clear muss stabiler Contract werden
- Symbol- und Line-Mapping fehlen fuer IDE-Debugging
- Launch-Konfigurationen fehlen
- `anycode` braucht eine Debug-Backend-Schicht ueber `anytrace`

## Native End-of-Day MVP

Ein tragfaehiger erster nativer Self-Hosting-Zustand ist erreicht, wenn:

1. `anyrc_tests` wieder kompilieren und die Compiler-Basics pruefen.
2. `ccargo build libs/stdlib` nativ durchlaeuft.
3. `ccargo build bin/cat`, `bin/echo`, `bin/ls`, `bin/grep` nativ durchlaeuft.
4. `anycode` diese Builds startet und Diagnostics in Problems anzeigt.
5. `agit status/diff/add/commit` aus `anycode` heraus sauber nutzbar ist.
6. `anytrace` einen aus `anycode` gestarteten Prozess attachen, stoppen,
   single-steppen und Register/Memory anzeigen kann.

Das ist noch nicht "Kernel baut sich selbst", aber es ist echte native
Entwicklung im System.

## Messleiter fuer `crust` und `ccargo`

### Stufe 0: Test-Harness reparieren

- `libs/anyrc_tests` muss ohne Panic-/Alloc-Handler-Kollision laufen.
- Test-API an aktuelle `CompileOptions` anpassen.
- Lexer-Tests an aktuelle Tokenformen anpassen.
- Parser-Panics in Tests identifizieren und in Diagnostics umwandeln.

### Stufe 1: Core Compiler Slice

Ziel:

- einfache `no_std` Binaries
- Funktionen, Structs, Enums, Patterns, Arrays, Slices, References, Raw Pointers
- `extern "C"` und `#[no_mangle]`
- stabile ELF-Ausgabe

Akzeptanz:

- `crust hello.rs -o hello`
- `crust --emit obj`
- `crust --emit rlib`
- Tests fuer jede Sprachfunktion als kleine Fixtures

### Stufe 2: `anyos_std`

Ziel:

- `ccargo build libs/stdlib`
- `alloc`, `String`, `Vec`, `HashMap`, Slices, Iteration, Makros
- `#[panic_handler]`, `#[alloc_error_handler]`, `entry!`

Akzeptanz:

- `anyos_std` baut als RLIB
- ein simples Programm linkt gegen `anyos_std`
- Buildfehler sind Diagnostics, keine Panics

### Stufe 3: kleine CLI-Tools

Startset:

- `bin/true`
- `bin/false`
- `bin/echo`
- `bin/cat`
- `bin/pwd`
- `bin/ls`
- `bin/grep`

Akzeptanz:

- jedes Tool baut mit `ccargo`
- jedes Tool laeuft in anyOS
- Smoke-Tests pruefen Exitcode und Basis-Ausgabe

### Stufe 4: Toolchain selbst

Ziel:

- `ccargo build bin/anyrc`
- `ccargo build bin/acargo`
- `crust` kann einen kleinen Teil von `crust` selbst bauen

Akzeptanz:

- Stage-A: Host-rustc baut `crust`, `crust` baut Fixtures
- Stage-B: `crust` baut eine reduzierte Compiler-Library
- Stage-C: `crust` baut sich komplett

### Stufe 5: IDE und Git

Ziel:

- `anycode` baut eigene Beispielprojekte mit `ccargo`
- `agit`-Operationen laufen ohne externe Abhaengigkeit
- Problems/Output/Artifacts sind strukturiert

Akzeptanz:

- Projekt oeffnen
- Build starten
- Fehler anklicken
- Datei korrigieren
- erneut bauen
- committen

### Stufe 6: Kernel

Ziel:

- `ccargo build kernel --target x86_64-anyos`
- Linker-Script, Assembly-Objekte und Kernel-CFGs funktionieren nativ

Akzeptanz:

- Kernel-ELF wird unter anyOS erzeugt
- Image-Pipeline kann daraus ein bootbares System bauen
- Smoke-Boot in VM oder echter Maschine

## Build- und Diagnostics-Contract

`ccargo` soll eine maschinenlesbare Ausgabe bekommen, die `anycode` direkt
verarbeiten kann:

```text
kind=task-start id=... label=...
kind=compile crate=... target=...
kind=diagnostic severity=error file=... line=... col=... message=...
kind=artifact path=... kind=rlib
kind=artifact path=... kind=exe
kind=task-finish id=... status=success
```

Die menschenlesbare Ausgabe bleibt fuer Terminalnutzer erhalten, aber die IDE
haengt an diesem Contract.

## `anycode` Integration

`anycode` bekommt native Backends:

- `BuildBackend::Ccargo`
- `BuildBackend::CrustSingleFile`
- `BuildBackend::Make`
- `BuildBackend::Tcc`

Fuer Rust-Projekte ist `BuildBackend::Ccargo` der Standard. Tasks enthalten:

- `backend`
- `working_dir`
- `target`
- `args`
- `problem_matcher`
- `artifact_paths`

## `agit` Integration

Kurzfristig:

- `anycode` ruft `git` als Prozess auf
- `status --porcelain`, `diff`, `add`, `commit`, `pull`, `push` werden robust
  verarbeitet

Mittelfristig:

- `libgit` bietet direkte IDE-Funktionen:
  - `repo_status(root)`
  - `repo_diff(root, path)`
  - `stage(root, path)`
  - `unstage(root, path)`
  - `commit(root, message)`
  - `current_branch(root)`

## `anytrace` Integration

`anycode` nutzt `anytrace` als natives Debug-Backend:

- Launch oder Attach
- Pause/Continue
- Step Into/Over/Out
- Breakpoints
- Registers
- Stack
- Memory
- Disassembly
- Output

Der erste sinnvolle Scope ist Userspace-Debugging. Kernel-Debugging kommt
spaeter ueber dedizierte Kernel-Symbol- und VM-/Hardware-Hooks.

## Naechste konkrete Aufgaben

1. `anyrc_tests` API-Drift reparieren.
2. `ccargo`-Sweep in einzelne Milestone-Tests splitten.
3. `anyrc` Parser-Panics in Diagnostics umwandeln.
4. `ccargo build libs/stdlib` als ersten echten nativen Milestone gruenden.
5. `anycode` auf ein natives Build-Backend-Modell umstellen.
6. `agit`-Status/Diff/Commit in `anycode` stabilisieren.
7. `anytrace` als Debug-Backend fuer `anycode` kapseln.
