# anyCode Studio Roadmap

## Ziel

anyCode soll von einem einfachen Editor mit Build-Buttons zu einer nativen
anyOS-Entwicklungsumgebung wachsen. Der Zielstandard ist nicht "ein paar
VS-Code-Features", sondern ein echtes Entwicklungsstudio:

- Projekt-/Workspace-Verwaltung mit Targets, Konfigurationen und Artefakten
- Echtzeitdiagnostik fuer Rust, C, C++, Shell, JSON/TOML und Builddateien
- Fehlerlisten, Editor-Markierungen, Quick Fixes und Navigation
- IntelliSense-artige Sprachefeatures: Symbole, Outline, Completion, Hover,
  Go-to-definition, References und Rename
- nativer Build-, Run-, Test- und Debug-Workflow
- Git, Search, Tasks, Terminal, Output und Problems als zusammenhaengende IDE

Der erste vollstaendige Zielpunkt heisst **Studio v1**: Ein Entwickler kann ein
anyOS-Rust- oder C/C++-Projekt in anyCode oeffnen, bearbeiten, live pruefen,
bauen, starten, debuggen und committen, ohne externe Host-Tools bedienen zu
muessen.

## Aktueller Stand

Vorhanden:

- `apps/anycode` mit Explorer, Editor, Tabs, Breadcrumb, Search, Problems,
  Output, Run Panel, Git Panel, Symbols, Settings, Command Palette und AI Panel.
- `libs/libanyui` `TextEditor` mit Syntaxhighlighting, Zeilennummern,
  Auswahl, Undo/Redo, Copy/Cut/Paste, Read-only-Modus und Line-Highlights.
- Buildprozesse mit Pipe-Ausgabe ueber `BuildProcess`.
- Task-Erkennung fuer Cargo/ccargo, CMake, Make, Python, Node und generische
  Projekte.
- Einfache Diagnostics-Parser fuer Rust-, GCC/Clang- und Python-Ausgaben.
- Sprachregistry mit Keywords/Snippets und Dateityp-Erkennung.

Kernprobleme:

- Live-Diagnostics existieren jetzt als erste Pipeline, sind aber noch nicht
  document-versioniert und noch nicht semantisch tief genug.
- Build/Run ist noch kommandoorientiert; es fehlt ein strukturierter Backend-
  Contract fuer `ccargo`, `crust`, C/C++ und Tasks.
- Der Editor kann erste Diagnostic-Ranges/Gutter-Marker zeichnen, aber noch
  keine Tooltips, Quick-Fixes, Breakpoints oder Inline-Hints.
- Sprachefeatures sind keyword-/regex-nah, nicht projektsemantisch.
- Debugging hat jetzt ein erstes anyCode-Backend fuer Launch/Attach/Pause/
  Continue/Step und Register-Snapshots; Source-Level-Breakpoint-Binding,
  Memory/Disassembly und Watch-Auswertung fehlen noch.
- UI ist funktionsreich, aber noch nicht dicht, konsistent und ergonomisch genug
  fuer taegliche Projektarbeit.

## Leitentscheidungen

1. **Backends statt Button-Commands.**
   anyCode bekommt stabile Backend-Traits fuer Build, Lint, Language, Debug und
   VCS. Commands werden nur noch UI-Aktionen, keine fachliche Logik.

2. **Strukturierte Diagnostics als zentrale Waehrung.**
   Alle Compiler, Linter und Parser liefern `Diagnostic` mit Datei, Range,
   Severity, Code, Message, Source, Related Locations und optionalem Fix.

3. **Editor-Dekorationen zuerst.**
   Ohne Squiggles, Gutter Marker, Inline Messages und klickbare Problems fuehlt
   sich keine Echtzeitpruefung ernsthaft an.

4. **Rust/anyOS zuerst, C/C++ direkt danach.**
   Der native Kernpfad ist `ccargo`/`crust`/`anyrc`. C/C++ folgt ueber `cc`,
   `clang`-kompatible Ausgabe und spaeter eigene anyOS-Toolchain-Backends.

5. **LSP-Form innen, anyOS-native Implementierung aussen.**
   Wir kopieren nicht zwingend LSP als Prozessprotokoll, aber die IDE-internen
   Datenmodelle sollen LSP-aehnlich sein: Document, Position, Range,
   TextEdit, CompletionItem, Symbol, Hover, CodeAction.

## Zielarchitektur

### Core-Module

Neue oder umzubauende Module in `apps/anycode/src/logic/`:

- `document.rs`
  - Open documents, versions, dirty state, snapshots, text ranges.
  - Debounced change events fuer Linter und Symbolindex.

- `diagnostic_model.rs`
  - Canonical `Diagnostic`, `Range`, `RelatedInformation`, `CodeAction`.
  - Merge/Dedupe nach `source + code + file + range + message`.

- `language_service.rs`
  - Trait fuer `check`, `complete`, `hover`, `symbols`, `definition`,
    `references`, `rename`, `format`, `code_actions`.
  - Service registry pro `LanguageId`.

- `build_backend.rs`
  - Trait fuer `discover`, `build`, `check`, `test`, `run`, `clean`.
  - Backends: `CcargoBackend`, `CrustBackend`, `CBackend`, `CppBackend`,
    `MakeBackend`, `CMakeBackend`, `GenericTaskBackend`.

- `diagnostic_pipeline.rs`
  - Debounce, cancellation, queueing, stale-result filtering by document version.
  - Background checks fuer aktive Datei und Workspace.

- `debug_backend.rs`
  - Launch/attach, breakpoints, pause/resume, stepping, stack/register/memory.
  - Erste Implementierung ueber `anytrace`/Debug-Syscalls.

- `workspace_index.rs`
  - Dateiliste, Symbolindex, Include/import/module graph, search index hints.

### Editor- und UI-Erweiterungen

Erweiterungen in `libs/libanyui/src/controls/text_editor.rs` und
`libs/libanyui_client/src/controls/texteditor.rs`:

- Range-Dekorationen: underline/squiggle, background, foreground, gutter icon.
- Diagnostic Tooltips bei Hover.
- Breakpoint-Gutter mit Click-Handling.
- Inline Hints und Ghost Text fuer Completion.
- Multi-cursor-Grundlage oder mindestens saubere Selection-API.
- Visible range API fuer performantes Rendering grosser Dateien.
- Per-range hit testing fuer Problems, Quick Fixes und References.

Erweiterungen in `apps/anycode/src/ui/`:

- Problems Panel gruppiert nach Datei, Severity, Source und Code.
- Klick auf Problem springt exakt zu Zeile/Spalte und markiert Range.
- Quick-Fix-Popup an Diagnostic-Position.
- Output/Problems/Test Results/Debug Console als klar getrennte Tabs.
- Statusbar mit Language Service State: `checking`, `clean`, `errors`.
- Run/Debug Toolbar mit Config-Auswahl, nicht nur globalen Buttons.

## Phasenplan

### Phase 0: Baseline und harte Akzeptanztests

Ziel: Der Ist-Zustand wird messbar.

Aufgaben:

- Smoke-Test fuer anyCode-Start, Projekt oeffnen, Datei oeffnen, Build starten.
- UI-Regression ueber `uictl` fuer Explorer, Editor, Problems und Output.
- Kleine Fixture-Projekte:
  - Rust: valides Projekt, Syntaxfehler, Typefehler, Warning.
  - C: valides Projekt, Syntaxfehler, Include-Fehler.
  - C++: valides Projekt, Template-/Header-Fehler.
  - JSON/TOML: Syntaxfehler.
- Diagnostics-Golden-Files fuer Parserausgaben.

Akzeptanz:

- `anycode` kann Fixtures oeffnen.
- Fehler erscheinen reproduzierbar im Problems Panel.
- Klick auf Fehler springt zur richtigen Datei und Zeile.

### Phase 1: Diagnostic Core und Problems v2

Ziel: Eine robuste, zentrale Diagnostics-Pipeline.

Aufgaben:

- `Diagnostic` um `range`, `source`, `related`, `suggested_fixes` erweitern.
- Rust-Parser reparieren: `error[...]` plus nachfolgende `--> file:line:col`
  zu einem Diagnostic zusammenfuehren.
- GCC/Clang Parser fuer Pfade mit Laufwerks-/Colon-Faellen und Spaltenbereiche
  robuster machen.
- JSON/TOML/Shell-Minilinter fuer schnelle lokale Syntaxfehler.
- Problems Panel nach Severity und Datei sortieren.
- Statusbar mit exakter Problemzahl synchronisieren.

Akzeptanz:

- Build- und Check-Ausgaben erzeugen keine file-losen Rust-Fehler mehr.
- Problems zeigt nur verwertbare Eintraege oder bewusst globale Meldungen.
- Doppelte Diagnostics werden dedupliziert.

Status:

- Gestartet: robuste Rust/GCC/Python-Ausgabe, Problems v2, Editor-
  Diagnostic-Marker und erste Live-Analyse sind implementiert.
- Gestartet: zentrale Diagnostic-Deduplizierung verhindert doppelte Eintraege
  aus Live-Analyse, Build- und Check-Ausgaben.

### Phase 2: Editor Decorations

Ziel: Fehler sind direkt im Code sichtbar.

Aufgaben:

- `TextEditorDecoration` im Server-Control einbauen.
- Client-API:
  - `set_diagnostics(file_version, diagnostics)`
  - `clear_diagnostics(source)`
  - `set_breakpoints(lines)`
  - `set_inline_hints(hints)`
- Rendering:
  - rote/gelbe Squiggles unter Ranges
  - Gutter Marker
  - aktive Diagnostic-Line dezent hervorheben
  - Tooltip/Peek-Panel bei Hover oder Cursor auf Fehler
- Problems-Klick setzt Cursor auf Diagnostic-Range und scrollt hin.

Akzeptanz:

- Fehler erscheinen im Editor ohne manuellen Build-Neustart.
- Gutter und Problems bleiben nach Editieren konsistent.
- Rendering grosser Dateien bleibt fluessig.

### Phase 3: Echtzeit-Linting

Ziel: Beim Tippen prueft anyCode schnell und ohne UI-Blockade.

Aufgaben:

- Document-Versioning und Debounce nach Textaenderung.
- `RustLanguageService`:
  - erste Stufe: `crust --emit hir/mir/check` oder `ccargo check` mit
    maschinenlesbarer Diagnostic-Ausgabe.
  - spaeter: direkte `anyrc` Library-API fuer Parser/Typechecker-Diagnostics.
- `CLanguageService` und `CppLanguageService`:
  - erste Stufe: Compiler-Frontend im `-fsyntax-only`/Check-Modus, Ausgabe
    parsen.
  - spaeter: nativer C/C++ Frontend-Service.
- JSON/TOML/Shell direkt im Prozess pruefen.
- Cancellation: alte Lint-Jobs duerfen neue Editorversionen nicht ueberschreiben.

Akzeptanz:

- Nach einer Aenderung erscheinen Syntaxfehler innerhalb von ca. 300-800 ms.
- Speichern ist nicht noetig.
- UI bleibt waehrend Checks bedienbar.

Status:

- Gestartet: Debounced Live-Check-Timer, In-Memory-Lints fuer Konfliktmarker,
  Delimiter, Textqualitaet und Python-Indentation sowie externe Checks fuer
  Check-Tasks, C/C++, Python, Shell, JavaScript und TypeScript.
- Gestartet: offene Dateien besitzen Dokumentversionen; alte externe
  Check-Ergebnisse werden verworfen, wenn der Editor inzwischen neueren Inhalt
  hat.
- Gestartet: Live-Check-Zustand liegt in `diagnostic_pipeline.rs`; Analyse und
  externe Check-Auswahl laufen ueber ein erstes `language_service.rs`.
- Gestartet: Command Palette besitzt Analyse-Kommandos fuer aktive Datei,
  Live-Analyse-Neustart und das gezielte Leeren der Problemansicht.
- Gestartet: Problemnavigation via Command Palette (`Next Problem`/`Previous Problem`)
  springt dateiuebergreifend zu Diagnosepositionen.
- Gestartet: Problemansicht hat eine IDE-artige Filterleiste fuer alle Probleme,
  Fehler, Warnungen und die aktive Datei; Navigation respektiert den Filter.
- Gestartet: Error List sortiert sichtbare Diagnostics stabil nach Schweregrad,
  Datei, Position, Quelle und Meldung.

### Phase 4: Project Model und Build Backends

Ziel: anyCode versteht Projekte, Targets und Artefakte, statt nur Commands zu
starten.

Aufgaben:

- `BuildBackend`-Trait einfuehren.
- `ccargo` maschinenlesbare Ausgabe stabilisieren:
  - task-start, compile, diagnostic, artifact, task-finish.
- TaskManager auf Backend/Target/Configuration umstellen.
- UI fuer Build-Konfigurationen:
  - Debug/Release
  - Target triple
  - Run target
  - Test target
- Artefakte im Run Panel sichtbar machen.

Akzeptanz:

- Rust-Projekt: `check`, `build`, `test`, `run` laufen ueber `CcargoBackend`.
- Buildfehler erscheinen strukturiert in Problems.
- Letzter erfolgreicher Build liefert klickbare Artefakte.

Status:

- Gestartet: Solution Explorer zeigt eine Solution-/Projektstruktur statt nur
  roher Dateien, inklusive Konfigurationen, Targets, Build-/Run-/Test-Tasks und
  Dateiunterbaum.
- Gestartet: Debug/Release-Konfigurationen sind im Projektmodell verankert,
  koennen per Command Palette umgeschaltet werden und beeinflussen Cargo-Build-
  Tasks.

### Phase 5: IntelliSense v1

Ziel: Sprachefeatures, die beim echten Arbeiten tragen.

Aufgaben:

- Completion Popup fuer Keywords, Snippets, Workspace-Symbole und Imports.
- Outline und Breadcrumb aus Language Service statt nur regex-nah.
- Hover:
  - Rust: Symbolart, Typ, Signatur, Doc-Kommentar wenn verfuegbar.
  - C/C++: Symbol, Include-Hinweis, Signatur.
- Go-to-definition und find references ueber Workspace-Index.
- Rename fuer lokale Symbole als erste sichere Stufe.
- Format Document fuer einfache Sprachen und Rust/C ueber Backend wenn vorhanden.

Akzeptanz:

- Rust-Funktionen/Structs im Projekt sind completion- und navigierbar.
- Outline aktualisiert live.
- Rename erzeugt Vorschau und laesst sich abbrechen.

Status:

- Gestartet: `symbol_index.rs` baut beim Workspace-Oeffnen einen begrenzten
  Workspace-Symbolindex fuer Rust, C/C++, Python, JS/TS, Shell und Makefiles.
- Gestartet: Symbolindex kann per Command Palette neu aufgebaut werden und
  meldet die Symbolzahl in Statusbar/Output.

### Phase 6: Debug Studio

Ziel: Starten und Debuggen aus anyCode.

Aufgaben:

- `debug_backend.rs` ueber `anytrace`/Debug-Syscalls kapseln.
- Debug-Konfigurationen im Run Panel.
- Breakpoints im Editor-Gutter.
- Debug Toolbar: Start, Attach, Pause, Continue, Step Into/Over/Out, Stop.
- Panels:
  - Call Stack
  - Variables/Registers
  - Watch
  - Memory
  - Debug Console
  - Disassembly
- Symbol-/Line-Mapping vorbereiten.

Akzeptanz:

- Prozess aus anyCode starten.
- Breakpoint setzen.
- Prozess haelt am Breakpoint.
- Register und Memory werden angezeigt.

Status:

- Gestartet: `debug_session.rs` haelt Debug-Status, Launch-Ziel und
  Breakpoints, Call-Stack-Frames, Variables und Registerwerte.
- Gestartet: Run-and-Debug-Panel besitzt einen Debug-Start, Session-Status und
  Breakpoint-Zaehler; F9 toggelt Breakpoints an der aktuellen Editorzeile.
- Gestartet: Run-and-Debug-Panel zeigt einen eigenen Debug-State-Tree fuer
  Session, Breakpoints, Call Stack, Variables und Registers.
- Gestartet: Bottom Panel besitzt eine eigene Debug Console; Debug-Kommandos
  schreiben dort Launch-, Breakpoint-, Pause-, Continue-, Step- und Exit-Events.
- Gestartet: Continue, Pause und Step Over sind als Buttons und Command-Palette-
  Kommandos vorhanden.
- Gestartet: `debug_backend.rs` kapselt die Kernel-Debug-Syscalls aus
  `anyos_std::debug`; gestartete Prozesse werden per TID attached, gepollt und
  mit echten RIP/RSP/Registerwerten im Debug-State-Tree angezeigt.
- Gestartet: anyCode deklariert die `debug`-Capability explizit, damit der
  Debugger nicht nur in System-Tools, sondern auch im Studio selbst laufen kann.

### Phase 7: Git und Review Workflow

Ziel: Versionierung ist Teil des Studios.

Aufgaben:

- `libgit`-direkt oder strukturierter `agit`-Contract fuer Status/Diff/Commit.
- Source Control View mit Staged/Unstaged/Untracked.
- Inline-Diff und Side-by-side Diff im Editor.
- Commit-UI mit Validierung.
- Branch wechseln/erstellen, Pull/Push mit Fehlerdialog.

Akzeptanz:

- Aenderungen erscheinen live.
- Datei diffen, stagen, committen funktioniert.
- Merge-Konflikte werden im Editor angezeigt.

### Phase 8: UI/UX-Haertung auf Studio-Niveau

Ziel: Das Ding fuehlt sich wie ein ernsthaftes Werkzeug an.

Aufgaben:

- Einheitliche Panel-Dichte, Tastaturfokus, Kontextmenues und Tooltips.
- Command Palette mit fuzzy search fuer alle Commands, Dateien und Symbole.
- Persistente Layouts: Panelbreiten, Tabs, Breakpoints, Run configs.
- Kein sichtbares Debug-Logging im normalen UI-Pfad.
- Accessibility-Labels fuer wichtige Controls.
- Performance bei grossen Projekten und Dateien messen.

Akzeptanz:

- Komplett per Tastatur nutzbar fuer Standardworkflow.
- Keine sichtbaren Layoutspruenge beim Oeffnen/Schliessen von Panels.
- 1000-Dateien-Projekt bleibt bedienbar.

### Phase 9: Studio v1 Freeze

Ziel: Die erste Version ist nicht feature-vollstaendig wie Visual Studio, aber
vollstaendig als anyOS Studio.

Pflicht-Workflow:

1. Projekt oeffnen.
2. Datei bearbeiten.
3. Live-Diagnostic sehen.
4. Problem anklicken und korrigieren.
5. Completion/Hover/Symbolnavigation nutzen.
6. Build starten.
7. Tests starten.
8. App starten.
9. Breakpoint setzen und debuggen.
10. Diff ansehen und committen.

Akzeptanz:

- Alle zehn Schritte funktionieren fuer ein Rust-anyOS-Projekt.
- Schritte 1-8 funktionieren fuer ein C/C++-Projekt.
- Fehler werden nicht nur im Output, sondern im Editor und Problems Panel
  angezeigt.
- Keine bekannten Panic-/Crash-Pfade in normalen Workflows.

## Erste konkrete Tickets

1. `diagnostics.rs`: Rust-Location-Zeilen korrekt an vorherige Diagnostic
   binden.
2. `problems_panel.rs`: globale Meldungen und file-bound Diagnostics getrennt
   anzeigen.
3. `text_editor.rs`: Range-Dekoration-Datenmodell einfuehren.
4. `texteditor.rs`: Client-API fuer Diagnostics/Decorations exportieren.
5. `commands.rs`: `on_text_changed` an Document-Versioning und Debounce haengen.
6. `logic/language_service.rs`: Trait und Registry anlegen.
7. `logic/build_backend.rs`: Trait und `CcargoBackend`-Skeleton anlegen.
8. `ccargo`: maschinenlesbaren Diagnostic-Output als stabilen Contract
   definieren und ausgeben.
9. `anycode`: `ccargo check` als Live-Rust-Linter fuer aktive Datei/Projekt
   integrieren.
10. `uictl`-Smoke-Test fuer Fehlerliste und Editor-Squiggle-Rendering anlegen.

## Nicht-Ziele fuer Studio v1

- Vollstaendige Visual-Studio-Paritaet inklusive Designer, Profiler, NuGet-
  Aequivalent und Enterprise-Projektformaten.
- Perfekte C++-Semantik ohne ausgereiften C++-Frontend-Service.
- Kernel-Debugging mit Source-Level-Line-Mapping.
- Multi-user Collaboration.

Diese Dinge sind spaetere Studio-v2-Themen. Studio v1 muss zuerst den nativen
anyOS-Entwicklungsfluss kompromisslos gut machen.
