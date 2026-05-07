# anyOS Unified Roadmap

Diese Roadmap fasst die bestehenden Roadmap-, Plan- und Todo-Dokumente zu einer
gemeinsamen Produkt- und Engineering-Sicht zusammen.

Quellen:

- `docs/self-hosting-roadmap.md`
- `docs/anycode-studio-roadmap.md`
- `todos/anyui-roadmap.md`
- `docs/scheduler-power-management-plan.md`
- `todos/architecture-refactoring.md`
- `todos/compositor.md`
- `todos/asl-anyos-subsystem-linux.md`
- `docs/asld-scaffolding-plan.md`
- `docs/asld-control-plane-api.md`

## Zielbild

anyOS soll zu einem nativ selbsttragenden Desktop-System wachsen:

- native Entwicklung direkt in anyOS mit `crust`, `ccargo`, `anycode`, `agit`
  und `anytrace`
- ein ernsthaftes Desktop-Toolkit mit produktiven Controls, Commands,
  Keyboard-/Pointer-UX und Accessibility
- ein stabiler Kernel- und Compositor-Unterbau mit klaren Security- und
  Lifecycle-Contracts
- ein integriertes ASL-Subsystem fuer Linux-Kompatibilitaet auf Basis einer
  gemanagten Utility-VM
- messbare Systemqualitaet durch Smoke-, Regression-, Torture- und Benchmark-
  Tests

Der wichtigste Produktpfad ist Native Self-Hosting: anyOS kann seine eigenen
Libraries, Tools und Apps in anyOS bearbeiten, bauen, testen, debuggen und
versionieren.

## Gesamtprioritaeten

### P0: Native Development MVP

Ziel: Ein Entwickler kann ein kleines anyOS-Projekt direkt in anyOS bearbeiten,
bauen, Fehler korrigieren, starten, debuggen und committen.

Pflichtpunkte:

- `anyrc_tests` reparieren und Parser-Panics in Diagnostics umwandeln.
- `ccargo build libs/stdlib` als ersten nativen Meilenstein stabilisieren.
- Kleine CLI-Tools wie `echo`, `cat`, `ls` und `grep` nativ mit `ccargo`
  bauen.
- `ccargo` liefert maschinenlesbare Build-, Artifact- und Diagnostic-Events.
- `anycode` nutzt Build-, Language-, Debug- und Git-Backends statt lose
  Button-Commands.
- Diagnostics erscheinen in Problems, im Editor und sind klickbar.
- `anytrace` ist als Debug-Backend in anyCode gekapselt.
- `agit`/`libgit` deckt Status, Diff, Stage und Commit IDE-freundlich ab.

Akzeptanz:

1. Projekt oeffnen.
2. Datei bearbeiten.
3. Live-Diagnostic sehen.
4. Problem anklicken und korrigieren.
5. Build starten.
6. App starten.
7. Breakpoint setzen und debuggen.
8. Diff ansehen und committen.

### P1: Desktop Foundation und Security

Ziel: Das System fuehlt sich nicht demoartig an, sondern wie ein belastbares
Desktop-OS.

Pflichtpunkte:

- Compositor-Security: Window-ID-Ownership, Capability-Checks, Clipboard-
  Permission, Control-ID-Kindvalidierung und Resource-Limits.
- anyUI P0/P1: Text-Undo/Redo, Word-Wrap, Menues mit Submenues/Accelerators,
  `ListView`, `CollectionView`, `Popover`/`Sheet`, Command-System.
- Architektur-Refactoring: `libcompress`, `RenderContext`, `GlobalAppState<T>`,
  Desktop-Struct-Split, VFS-/Loader-Split, einheitliches Error-Handling.
- UI-Regressionen ueber `uictl` fuer Explorer, Editor, Problems, Output,
  Settings und wichtige anyUI-Controls.

Akzeptanz:

- Standard-Apps sind per Maus und Tastatur sauber bedienbar.
- Kritische IPC- und Ownership-Pfade lassen fremde Ressourcen nicht manipulieren.
- Neue Controls haben Public API, Theming, Accessibility und mindestens eine
  produktive Referenz-App.

### P2: Scheduler, Power und Kernel-Haertung

Ziel: Kontextwechsel, Speicherfreigabe und CPU-Power-Policy sind robust,
messbar und auf moderne Hardware vorbereitet.

Pflichtpunkte:

- Context-Switch-Invarianten auf allen Architekturen absichern.
- KUnit-/Host-Tests fuer `CpuContext`-Checksum und korrupte Restore-Pfade.
- Torture-Tests fuer Migration, Preemption, Exit, Signal Delivery und
  Address-Space-Switches.
- Per-CPU Power-/Frequency-Telemetrie, APERF/MPERF-Sampling und Topologie-
  Modell.
- Fair-Scheduling-Class, Utilization Tracking und energy-/capacity-aware
  Wakeup/Work-Stealing.
- Benchmarks fuer Fairness, Latenz, Durchsatz, Migration und Energie-Proxies.

Akzeptanz:

- Ein runnable Thread wird nur mit vollstaendig validiertem Kontext restored.
- Scheduler- und Power-Aenderungen haben Benchmark-Gates.
- AnyOS kann realistisch gegen Linux-/Windows-Eigenschaften verglichen werden,
  ohne Paritaet vorzeitig zu behaupten.

### P3: ASL Foundation

Ziel: Linux-Kompatibilitaet entsteht als systemisch gemanagtes Subsystem, nicht
als lose VM-App.

Pflichtpunkte:

- `asld`-Scaffold mit `confd`-Manifest, Runtime-Store, IPC-Grundgeruest und
  strukturierten Stubs fuer VM, Agent, Mounts und Network.
- `aslctl` fuer `list`, `create`, `start`, `stop`, `status`, `shell`, `exec`,
  `logs` und `doctor`.
- Distro-Modell mit Base Image, Overlay, Owner, Profile und Ressourcenlimits.
- NAT als Default-Netzwerkmodus.
- Shared Folders als explizite, brokered Exportpunkte.
- Console-/Session-Broker fuer Terminal-Integration.

Akzeptanz:

- Eine Distro kann registriert, gestartet, gestoppt und abgefragt werden.
- Linux bootet in einer gemanagten Utility-VM.
- Terminal, Netzwerk, Logs und Basisstatus funktionieren.
- Shared Folders und Port-Forwarding sind sichtbar konfiguriert und policy-
  kontrolliert.

## Workstreams

### 1. Native Toolchain und Self-Hosting

Aktueller Stand:

- `anyrc` besitzt Lexer, Parser, HIR, MIR, Borrowcheck, Monomorphisierung,
  x86_64-Codegen sowie ELF-/Objekt-/RLIB-Ausgabe.
- `ccargo` besitzt Manifest-, Workspace-, Dependency-, Feature-, Buildscript-
  und Fingerprint-Grundlagen.
- `agit` deckt viele Porcelain-Operationen und Smart-HTTP bereits ab.
- `anytrace` kann attachen, stoppen, single-steppen, Register refreshen und
  Memory lesen.

Naechste Schritte:

1. `libs/anyrc_tests` API-Drift und Test-Harness reparieren.
2. `anyrc` Parser-Panics in strukturierte Diagnostics umwandeln.
3. `ccargo`-Sweep in kleine Milestone-Tests splitten.
4. `libs/stdlib` nativ bauen.
5. Kleine CLI-Tools nativ bauen und ausfuehren.
6. `ccargo`-Eventstream fuer IDE und CLI stabilisieren.
7. Spaeter `crust`/`ccargo` selbst stufenweise nativ bauen.

### 2. anyCode Studio

Aktueller Stand:

- Explorer, Editor, Tabs, Breadcrumb, Search, Problems, Output, Run Panel,
  Git Panel, Symbols, Settings, Command Palette und AI Panel sind vorhanden.
- Erste Live-Diagnostics, Problems v2, Diagnostic-Marker, Symbolindex,
  Projektmodell, Debug-State und Debug-Backend-Ansaetze sind gestartet.

Zielarchitektur:

- `document.rs` fuer Versionen, Dirty State, Snapshots und Text ranges.
- `diagnostic_model.rs` als zentrale Diagnostic- und CodeAction-Schicht.
- `language_service.rs` fuer Completion, Hover, Symbols, Definition,
  References, Rename, Format und Code Actions.
- `build_backend.rs` fuer `ccargo`, `crust`, C/C++, Make, CMake und generische
  Tasks.
- `debug_backend.rs` ueber `anytrace`/Debug-Syscalls.
- `workspace_index.rs` fuer Dateien, Symbole, Module und Search-Hints.

Naechste Schritte:

1. Diagnostics document-versioniert und cancellation-safe machen.
2. Editor-Dekorationen mit Squiggles, Gutter-Markern, Tooltips und
   Quick-Fixes fertigstellen.
3. `CcargoBackend` als Standard fuer Rust-Projekte etablieren.
4. IntelliSense v1 fuer Rust und C/C++ liefern.
5. Debug-Konfigurationen, Breakpoints und Source-Level-Binding haerten.
6. Source-Control-View mit Staged/Unstaged, Inline-Diff und Commit-UI bauen.
7. Studio-v1-Workflow per Smoke-Test absichern.

### 3. anyUI und Desktop Controls

Aktueller Stand:

- Breite Basis mit Controls, Dialogen, Menubar, TrayIcon, Windowing,
  Accessibility und Popup-/Modal-Grundlagen.
- Bereits erledigt: Text-Control-Basis-APIs, Clipboard-Semantik, ComboBox,
  DataGrid Multi-Selection/Sort/Scroll/Reorder/Resize, ListBox/TreeView-
  Keyboard-Navigation und generisches Drag-&-Drop-Grundmodell.

Naechste Schritte:

1. Text-Controls: Undo/Redo, Word-Wrap fuer `TextArea`, public
   scroll-to-caret und Positionsabfragen.
2. Menues: Submenues, Checkmarks, Shortcut-Anzeige, Accelerator-Verarbeitung
   und bessere Dismiss-/Fokuslogik.
3. Collection-Controls: Inline-Editing, Virtualisierung und generische
   Payload-DnD-Integration.
4. `ListView` fuer Finder, Dateidialoge, Store und Asset-Browser.
5. `CollectionView`/`ItemsControl` als datengetriebenes Framework-Primitive.
6. `Popover`, `Sheet`, `Drawer` und `Inspector`.
7. `PropertyGrid`, `BreadcrumbBar`, `PathBar`, `TokenField` und `RichText`.
8. Command-/Action-System mit zentraler Registry, Shortcuts und Enabled State.
9. Validation-/Forms-Modell mit Inline-Errors und Form Summary.

Definition of Done pro neuem Control:

- Mausinteraktion
- Keyboard-Navigation
- Fokusverhalten
- Disabled State
- Theming
- Accessibility-Tree-Unterstuetzung
- Public API im Client-Wrapper
- mindestens eine produktive Referenz-App

### 4. Compositor und libanyui Security

Statusbild:

- Viele Integer-, Bounds- und Allocation-Probleme sind bereits behoben.
- Kritische offene Punkte betreffen vor allem Ownership, Permissions,
  Capability Checks und unsafe Casts.

Naechste Schritte:

1. Window-ID-Ownership validieren.
2. Capability-Checks fuer IPC-Kommandos einfuehren.
3. Clipboard-Read-Permission und Clipboard-Setter-Tracking.
4. Control-ID-Kindvalidierung vor unsafe Casts.
5. Control-IDs per App namespacen.
6. Menu- und Notification-Pointer vor Kopie validieren.
7. Fullscreen/direct-fb nur mit Capability erlauben.
8. Notification-Rate-Limiting.
9. PendingCallback-/Control-Lifetime waehrend Dispatch absichern.
10. Per-App Resource-Limits fuer Controls, Windows und SHM.

### 5. Architektur-Refactoring

Erledigt oder gut:

- Scheduler ist in Submodule aufgeteilt.
- `anyos_std::fmt` und `anyos_std::path` existieren.
- DLL-Loading-Macro existiert.
- Memory-Modul-API ist reduziert.
- Panic-Handler sind ueber Libraries vereinheitlicht.

Naechste Schritte:

1. `libcompress` fuer DEFLATE/CRC32 extrahieren.
2. `RenderContext` Helper fuer anyUI Controls.
3. `GlobalAppState<T>` Macro fuer Apps.
4. Kernel `logging.rs` und einheitliches Logformat.
5. `Desktop` Struct in `WindowManager`, `InputState`, `UiChrome`,
   `AppProtocol` und `DesktopLifecycle` splitten.
6. VFS-Split mit `mount.rs` und `file_ops.rs` finalisieren.
7. `task/loader` in ELF, Memory und Spawn trennen.
8. Einheitliches Error-Handling fuer Kernel, Compositor und Libraries.
9. anyUI Event-Handler-Duplizierung ueber gemeinsame Traits reduzieren.
10. anyUI Layout Dirty-Tracking einfuehren.

### 6. Scheduler, Memory und Power

Aktueller Stand:

- x86_64 und AArch64 besitzen Context-Switch-Validierung mit Canary,
  Checksum, Stack Pointer, Return Address und `save_complete`.
- CPU Power HAL hat Policy Layer und x86-Backends fuer Intel, AMD und KVM.
- Userspace kann Power-Status und Profile ueber `sys_sysinfo(cmd=5)` und
  `anyos_std::sys` steuern.
- Settings persistiert Power Profile, Placement Policy und Efficiency Bias.

Naechste Schritte:

1. Context-Switch-Torture-Test bauen.
2. Kernel-Stack- und Thread-Reaping-Lifetime auditieren.
3. Per-CPU Epoch Accounting fuer schedulerkritische Reclamation.
4. MMU-Fault-Telemetrie mit TID, CPU, CR3/TTBR, PC und SP.
5. APERF/MPERF pro CPU sampeln.
6. CPPC/ACPI und Virtualization-Backends ergaenzen.
7. CPU-Topologie, Capacity und Energy Cost modellieren.
8. Task-Utilization und Interactive-Wakeup-Signale tracken.
9. Fair Scheduling Class und energy-aware Placement einfuehren.
10. Benchmark-Suite fuer Fairness, Latenz, Durchsatz und Energie-Proxies.

### 7. ASL

Leitentscheidungen:

- ASL ist WSL2-artig: Linux laeuft in einer gemanagten Utility-VM.
- anyOS bleibt Host und Policy Owner.
- Integration erfolgt ueber wenige stabile Kanaele: Control, Console,
  Filesystem, Network, Clipboard und Metrics.
- Rootfs wird aus Base- und Overlay-Layern aufgebaut.
- NAT ist Default, Shared Folders sind explizite Broker-Exports.
- `asl-agent` ist Standard, aber nicht bootkritisch.

Produktstufen:

1. ASL Foundation: Linux bootet, Terminal funktioniert, Netzwerk und Rootfs-
   Verwaltung sind vorhanden.
2. ASL Developer Edition: Shared Folders, Port-Forwarding, Tooling,
   Session-Resume und bessere Developer Experience.
3. ASL Desktop Integration: Linux-GUI-Apps, Clipboard, Notifications und
   Wayland-/X11-Bridge.

Naechste Schritte:

1. `asld`/`aslctl`-Grundgeruest als existierenden Control-Plane-Pfad haerten.
2. Pipe-IPC, Fehlercodes und JSON-/maschinenlesbare CLI-Ausgabe stabilisieren.
3. Direkten Linux-Bootpfad bis zur erreichbaren Fallback-Konsole liefern.
4. Rootfs-Import, Distro-Store und persistente Overlays produktfaehig machen.
5. NAT/DNS, Shared Folders und Port-Forwarding entlang der ADRs realisieren.
6. ASL Developer Edition: Dev-Profil, anyCode-Buildpfad, Session-Resume und
   reales Projekt-E2E-Gate umsetzen.

## Reihenfolge fuer die naechsten Sprints

### Sprint 1: Messbare Native-Baseline

- `anyrc_tests` reparieren.
- `ccargo`-Milestone-Sweep splitten.
- Parser-Panics in Diagnostics umwandeln.
- anyCode Diagnostics document-versionieren.
- `uictl`-Smoke-Test fuer Problems und Editor-Diagnostics anlegen.

### Sprint 2: IDE-Backends und Security

- `CcargoBackend` in anyCode stabilisieren.
- `ccargo` maschinenlesbare Diagnostic-/Artifact-Ausgabe liefern lassen.
- Window-ID-Ownership und IPC-Capability-Checks im Compositor angehen.
- Control-ID-Kindvalidierung in libanyui einfuehren.
- `libcompress` und `RenderContext` als High-Impact-Refactorings starten.

### Sprint 3: Developer Workflow

- `libs/stdlib` nativ bauen.
- Kleine CLI-Tools nativ bauen und Smoke-testen.
- anyCode Build/Run/Test ueber Backends statt Tasks fuehren.
- Editor-Squiggles, Gutter Marker und Problems-Klick finalisieren.
- `agit` Status/Diff/Commit IDE-freundlich integrieren.

### Sprint 4: Debug und Desktop-UX

- `anytrace` Debug-Backend in anyCode haerten.
- Breakpoints, Register, Memory und Disassembly im Studio-Workflow pruefen.
- Text-Control Undo/Redo und Menue-Accelerators liefern.
- `ListView` beginnen und in Finder oder Dateidialog dogfooden.
- Scheduler-/Context-Switch-Torture-Test aufsetzen.

### Sprint 5: ASL Foundation Start

- `asld`-Scaffold mit `confd`-Manifest und IPC-Grundgeruest landen.
- `aslctl list/status/create/start/stop` anbinden.
- Distro-Konfigurationsmodell persistieren.
- VM-Startpfad als ersten realen Adapter vorbereiten.
- Observability-Grundlagen fuer ASL-Status und Logs anlegen.

## Nicht-Ziele fuer die erste gemeinsame Roadmap

- Vollstaendige Visual-Studio-, WPF-, AppKit-, Linux- oder Windows-Paritaet.
- Kernel-Self-Hosting vor stabilen Userspace-Libraries und Tools.
- ASL als WSL1-artiger Syscall-Kompatibilitaetslayer.
- Linux-GUI-App-Integration in ASL v1.
- Perfekte C++-Semantik ohne ausgereiften C++-Frontend-Service.
- Kernel-Debugging mit vollstaendigem Source-Level-Line-Mapping.

Diese Themen bleiben moeglich, aber sie kommen nach dem nativen Entwicklungs-
MVP, der Desktop-Haertung und der ASL Foundation.
