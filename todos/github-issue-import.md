# GitHub Issue Import Candidates

Stand: 2026-05-06

Diese Liste fasst offene Todos aus Roadmaps, Todo-Dokumenten und Source-
Kommentaren zusammen. Punkte, die beim Code-Abgleich bereits erledigt oder
ueberholt wirkten, stehen unten unter "Nicht importieren".

Prioritaeten:

- `P0`: Security, Datenverlust, Native-Development-MVP-Blocker
- `P1`: Roadmap-Kernpfad / Developer-Workflow / Desktop-Foundation
- `P2`: Produktivitaetsfeatures und wichtige Plattformluecken
- `P3`: Long-tail, Komfort, Spezialisierung

## P0

### [P0] Clipboard-Read-Permission und Setter-Tracking einfuehren

Labels: `priority:P0`, `area:compositor`, `area:security`, `roadmap:desktop-foundation`

Quellen:

- `todos/compositor.md`
- `system/compositor/compositor/src/desktop/ipc.rs`
- `docs/anyos-roadmap.md`

Status: Offen. `CMD_GET_CLIPBOARD` liefert Clipboard-Daten weiterhin ohne
sichtbaren Permission-/Consent-Check aus; `clipboard_format` wird gespeichert,
aber kein App-/Owner-Tracking fuer den Setzer.

Beschreibung:

Apps duerfen Clipboard-Inhalte nicht still und global lesen. Der Compositor
braucht ein Permission-Modell fuer Clipboard-Read-Zugriff, sichtbare Policy und
mindestens Setter-/Reader-Tracking fuer Audit und spaetere UI.

Akzeptanz:

- Clipboard-Read wird pro App/TID policy-geprueft.
- Clipboard-Setter wird mit TID/App-Identitaet gespeichert.
- Bestehende `clipboard_set/get`-APIs verhalten sich fuer erlaubte Apps
  kompatibel.
- Tests oder ein Smoke-Szenario zeigen: nicht erlaubte App erhaelt keine
  geheimen Clipboard-Daten.

### [P0] Compositor-Capability-Checks fuer privilegierte IPC-Kommandos

Labels: `priority:P0`, `area:compositor`, `area:security`, `roadmap:desktop-foundation`

Quellen:

- `todos/compositor.md`
- `system/compositor/compositor/src/desktop/ipc.rs`
- `system/compositor/compositor/src/ipc_protocol.rs`

Status: Teilweise offen. Window-Ownership ist fuer Move/Destroy/Minimize/
Fullscreen inzwischen validiert. Ein generelles Capability-Modell fuer
privilegierte IPC-Pfade ist aber nicht erkennbar.

Beschreibung:

Privilegierte Kommandos wie direct-framebuffer Fullscreen, Notification-
Emission, Keyboard-Injection, System-Overlays und spaetere Power-/Session-
Kommandos sollen nicht nur syntaktisch, sondern anhand deklarierter App-
Capabilities erlaubt werden.

Akzeptanz:

- Zentrale Capability-Pruefung fuer privilegierte Compositor-IPC-Pfade.
- `want_direct_fb` bei Fullscreen wird nur mit expliziter Capability erlaubt.
- Keyboard-/Input-Injection ist auf vertrauenswuerdige Systemdienste begrenzt.
- Ablehnungen werden rate-limited geloggt und brechen kompatible normale Apps
  nicht.

### [P0] notifyd Rate-Limiting und Quotas pro Sender

Labels: `priority:P0`, `area:notifications`, `area:security`, `roadmap:desktop-foundation`

Quellen:

- `todos/compositor.md`
- `system/daemons/notifyd/src/main.rs`

Status: Offen. `notifyd` nimmt `CMD_SHOW_NOTIFICATION`-Events an und pusht sie
in `notifications`, ohne erkennbare Rate-/Burst-/Queue-Limits pro Sender.

Beschreibung:

Eine App kann Notifications spammen und damit UI, History und Speicher belasten.
`notifyd` braucht pro Sender Limits, globale Queue-Grenzen und saubere
Drop-/Coalesce-Semantik.

Akzeptanz:

- Pro-TID Burst- und Dauerlimits.
- Globale maximale aktive Notifications und History-Grenzen.
- Dropped/coalesced Notifications werden optional diagnostisch gezaehlt.
- Normale Systemnotifications bleiben sichtbar und responsiv.

### [P0] anyUI Control-IDs namespacen und Resource-Limits setzen

Labels: `priority:P0`, `area:anyui`, `area:security`, `roadmap:desktop-foundation`

Quellen:

- `todos/compositor.md`
- `libs/libanyui/src/lib.rs`

Status: Teilweise offen. Unsichere Kind-Casts sind durch `ControlKind`-
gepruefte `cast_mut`/`cast_ref`-Helper entschaerft. Die Controls bleiben aber
prozessglobal fortlaufende IDs mit unlimitiertem `controls.push()`.

Beschreibung:

anyUI braucht harte Limits und eine klarere Ownership-/Namespace-Semantik, damit
eine App nicht unbegrenzt Controls/Windows/SHM-Strukturen erzeugt oder fremde
IDs missbrauchen kann.

Akzeptanz:

- Max Controls/Windows/Popups pro App/Prozess.
- Fehlerpfad fuer Limit-Ueberschreitung ohne Panic.
- Control-IDs sind nicht app-uebergreifend manipulierbar.
- Hosttests oder Smoke-App pruefen Limit und Ablehnung.

### [P0] TLS 1.3 Finished verify_data pruefen

Labels: `priority:P0`, `area:tls`, `area:security`

Quellen:

- `libs/libtls/src/handshake/tls13.rs`

Status: Offen. Source-TODO akzeptiert Server-Finished im Trust-all-Pfad ohne
`verify_data`-Pruefung.

Beschreibung:

Auch wenn Zertifikatsvalidierung noch trust-all ist, muss TLS 1.3 die
Handshake-Transcript-Integritaet ueber Finished verify_data pruefen, sonst ist
der Kanal kryptographisch nicht belastbar.

Akzeptanz:

- Server Finished verify_data wird aus Transcript und Handshake Secret
  berechnet und verglichen.
- Fehler fuehrt zu `TlsError` und Verbindungsabbruch.
- Positiv- und Negativtest fuer TLS-1.3-Finished.

### [P0] `ccargo` maschinenlesbaren Build-/Diagnostic-/Artifact-Stream geben

Labels: `priority:P0`, `area:toolchain`, `area:ccargo`, `roadmap:native-development`

Quellen:

- `docs/self-hosting-roadmap.md`
- `docs/anycode-studio-roadmap.md`
- `docs/anyos-roadmap.md`
- `bin/acargo/src/`

Status: Offen. `ccargo` hat Build-, Registry-, Lockfile- und Sweep-Basis, aber
der stabile Eventstream `task-start/compile/diagnostic/artifact/task-finish`
ist im Code nicht als Contract sichtbar.

Beschreibung:

anyCode und spaetere CI-Sweeps sollen nicht Terminaltext parsen muessen. `ccargo`
braucht eine maschinenlesbare Ausgabe mit stabiler Versionierung und
menschenlesbarer Ausgabe als separatem Modus.

Akzeptanz:

- Optionaler `--message-format`/`--json`/aehnlicher Modus.
- Events fuer Task-Start, Compile, Diagnostic, Artifact und Task-Finish.
- anyCode kann Diagnostics und Artefakte daraus ohne Regex ableiten.
- Golden-Test fuer Beispielprojekt mit Fehler und erfolgreichem Build.

### [P0] anyCode BuildBackend-Contract statt Task-Parsing vollenden

Labels: `priority:P0`, `area:anycode`, `area:ide`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `docs/anyos-roadmap.md`
- `apps/anycode/src/logic/tasks.rs`
- `apps/anycode/src/logic/rust_backend.rs`

Status: Teilweise offen. Es gibt `RustBuildBackend` und Task-Discovery, aber
kein allgemeiner `BuildBackend`-Trait/Contract fuer Build-, Check-, Test-,
Run- und Clean-Operationen ueber `ccargo`, `crust`, C/C++, Make und CMake.

Beschreibung:

anyCode soll Build-/Run-/Test-Workflows ueber Backends modellieren, nicht als
lose Commands. Das ist Blocker fuer Studio-v1 und den nativen Self-Hosting-Pfad.

Akzeptanz:

- Allgemeiner Backend-Contract existiert.
- `CcargoBackend` ist Standard fuer Rust-Projekte.
- Build/Check/Test/Run liefern strukturierte Results, Diagnostics und
  Artifacts.
- UI-Tasks bleiben Aktionen, enthalten aber keine fachliche Buildlogik mehr.

### [P0] anyCode Breakpoints an Symbole/Line-Mapping binden

Labels: `priority:P0`, `area:anycode`, `area:debugger`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/commands.rs`
- `apps/anycode/src/logic/debug_backend.rs`

Status: Offen. Source-Breakpoints werden gespeichert; Code meldet, dass
Address-Binding noch auf Symbol-Mapping wartet.

Beschreibung:

Debugging ist erst Studio-tauglich, wenn Breakpoints von Datei/Zeile auf echte
Adressen gebunden und beim Launch/Attach ins Backend uebertragen werden.

Akzeptanz:

- Breakpoint in Editor-Gutter wird beim Debug-Start auf Adresse gebunden.
- Ungebundene Breakpoints zeigen nachvollziehbaren Status.
- Prozess haelt an einem gebundenen Breakpoint.
- Register/Memory/Disassembly aktualisieren sich nach Stop.

## P1

### [P1] Self-Hosting-Testgate fuer `anyrc`/`ccargo` dauerhaft gruen halten

Labels: `priority:P1`, `area:toolchain`, `area:anyrc`, `area:ccargo`, `roadmap:native-development`

Quellen:

- `docs/self-hosting-roadmap.md`
- `tools/acargo-hosttests/tests/sweep.rs`
- `libs/anyrc_tests/tests/30_bin_apps.rs`
- `libs/anyrc_tests/tests/31_kernel_workspace.rs`
- `libs/anyrc_tests/tests/32_binary_compat.rs`

Status: Teilweise erledigt. Die Milestone-/Sweep-Tests existieren inzwischen.
Offen ist, sie als explizites Roadmap-Gate zu dokumentieren und stabil in den
normalen Testpfad einzubinden.

Beschreibung:

Der native Development MVP braucht ein klares Gate: `anyrc_tests`, ccargo-
Binary-Compatibility, Userspace-App-Sweeps und Kernel-Workspace-Builds sollen
regelmaessig laufen und Regressionen blockieren.

Akzeptanz:

- Ein dokumentierter Befehl prueft die Self-Hosting-Baseline.
- Failures sind nach Milestone/Crate getrennt lesbar.
- Parser-Panics werden als Diagnostics gemeldet oder als Testfailure isoliert.
- CI-/lokaler Testpfad ist praktikabel und nicht ein monolithischer Blindflug.

### [P1] anyCode Studio-v1 Workflow-Gate automatisieren

Labels: `priority:P1`, `area:anycode`, `area:testing`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `docs/anyos-roadmap.md`
- `apps/anycode/src/`

Status: Offen. Die Roadmap definiert den Pflicht-Workflow, aber es fehlt ein
automatisierbares Gate fuer "mit anyCode entwickeln koennen".

Beschreibung:

anyCode soll erst als Studio-v1 gelten, wenn ein Entwickler ein Rust-anyOS-
Projekt oeffnen, bearbeiten, live pruefen, bauen, testen, starten, debuggen und
committen kann.

Akzeptanz:

- Fixture-Projekte fuer Rust-anyOS und C/C++ liegen dokumentiert bereit.
- Smoke-Test oeffnet Projekt und Datei, loest Diagnostics aus und startet Build.
- Problems-Klick springt zur korrekten Datei/Range.
- Build, Test, Run und Debug liefern nachvollziehbare Status-/Output-Daten.
- Git-Diff und Commit sind Teil des manuellen oder automatisierten Gate.

### [P1] anyCode Diagnostics versioniert, cancellation-safe und editornah machen

Labels: `priority:P1`, `area:anycode`, `area:diagnostics`, `area:ide`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/diagnostic_pipeline.rs`
- `apps/anycode/src/logic/language_service.rs`
- `apps/anycode/src/logic/diagnostics.rs`
- `apps/anycode/src/ui/problems_panel.rs`

Status: Teilweise offen. Dokumentversionen und erste stale-result-Filterung
existieren; die Roadmap nennt aber weiterhin fehlende robuste Pipeline-,
Range-, Tooltip- und Quick-Fix-Integration.

Beschreibung:

Echte Entwicklung braucht Diagnostics beim Tippen, die nicht veralten, nicht
doppelt erscheinen und direkt im Editor verstaendlich sind.

Akzeptanz:

- Offene Dokumente haben stabile Versionen, Dirty-State und Snapshots.
- Alte externe Check-Ergebnisse ueberschreiben nie neuere Editorversionen.
- Diagnostics enthalten Range, Source, Code, Severity, Related Information und
  optionale Fixes.
- Editor zeigt Squiggles/Gutter/Tooltip oder Peek fuer Diagnostics.
- Problems, Statusbar und Editor bleiben nach Editieren synchron.

### [P1] anyCode IntelliSense v1 fuer Rust/anyOS und C/C++ liefern

Labels: `priority:P1`, `area:anycode`, `area:language-service`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/intellisense.rs`
- `apps/anycode/src/logic/symbol_index.rs`
- `apps/anycode/src/logic/language_service.rs`
- `apps/anycode/src/ui/symbols_panel.rs`

Status: Teilweise offen. Keyword-/Snippet-Completion, Hover-Basis und
Workspace-Symbolindex existieren; semantische Navigation, Rename und Format
sind noch nicht als Studio-v1-Workflow geschlossen.

Beschreibung:

Ein Entwickler soll im Projekt navigieren und editieren koennen, ohne sich nur
auf Textsuche und Regex-nahe Symbolerkennung zu verlassen.

Akzeptanz:

- Completion kombiniert Keywords, Snippets, Workspace-Symbole und Imports.
- Hover zeigt fuer Rust/anyOS mindestens Symbolart, Signatur/Typ und Quelle.
- Go-to-definition und References funktionieren ueber Workspace-Index fuer
  Kernfaelle.
- Rename fuer lokale Symbole erzeugt Vorschau und ist abbrechbar.
- Format Document ist fuer einfache Sprachen und Rust/C ueber Backend oder
  Toolchain angebunden.

### [P1] anyCode Test Explorer und Test Results zum echten Workflow machen

Labels: `priority:P1`, `area:anycode`, `area:testing`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/test_explorer.rs`
- `apps/anycode/src/ui/run_panel.rs`
- `apps/anycode/src/logic/rust_backend.rs`

Status: Teilweise offen. Test-Discovery fuer Rust existiert und Run Panel zeigt
einen Test Explorer; Ausfuehrung, Ergebniszuordnung und Navigation muessen als
Entwicklerpfad geschlossen werden.

Beschreibung:

Tests sollen nicht nur als Build-Task laufen, sondern als IDE-Workflow mit
Testliste, gezieltem Start, Ergebnissen und Sprung zum Test.

Akzeptanz:

- Test Explorer entdeckt Cargo-/Rust-Tests in Workspace und Membern.
- Einzelne Tests, Datei-/Projekt-Tests und gesamter Testlauf sind startbar.
- Ergebnisse werden Testcases zugeordnet, inklusive Passed/Failed/Output.
- Klick auf Test springt zur Testfunktion.
- Fehlgeschlagene Tests erzeugen Problems/Test-Results-Eintraege mit Output.

### [P1] anyCode integriertes Terminal auf Entwicklerniveau haerten

Labels: `priority:P1`, `area:anycode`, `area:terminal`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/ui/output_panel.rs`
- `apps/anycode/src/main.rs`

Status: Teilweise offen. Terminal-Tab und Shell-Prozess mit Pipe-I/O existieren;
fuer taegliche Arbeit fehlen robuste Terminal-Semantik und Projektintegration.

Beschreibung:

Entwickler brauchen ein Terminal im Projektkontext fuer Git, Tooling,
Einmalbefehle und Diagnose, ohne anyCode verlassen zu muessen.

Akzeptanz:

- Terminal startet im Workspace-/Projekt-Root und zeigt klare Shell-Statusdaten.
- Ein-/Ausgabe, Exitstatus, Stop/Kill und Neustart sind stabil.
- Scrollback ist begrenzt, aber ausreichend und verliert keine frischen Zeilen.
- Environment und Working Directory werden pro Projekt nachvollziehbar gesetzt.
- Spaetere ASL-Terminals koennen denselben UI-Pfad nutzen.

### [P1] anyCode Run-Konfigurationen, Artefakte und Startup-Target persistieren

Labels: `priority:P1`, `area:anycode`, `area:run-debug`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/project.rs`
- `apps/anycode/src/logic/rust_backend.rs`
- `apps/anycode/src/ui/run_panel.rs`
- `apps/anycode/src/ui/project_properties_dialog.rs`

Status: Teilweise offen. Projektmodell, Debug/Release-Konfigurationen und
Run-Config-UI sind gestartet; Artefakte und Startup-Auswahl sind noch nicht als
stabiler Arbeitsvertrag geschlossen.

Beschreibung:

Build, Run und Debug sollen auf expliziten Konfigurationen und Artefakten
arbeiten, nicht auf geratenen Tasks.

Akzeptanz:

- Projekt speichert aktive Build-Konfiguration, Startup-Target und Run-Configs.
- Letzter erfolgreicher Build liefert klickbares Artefakt im Run Panel.
- Run/Debug nutzt das ausgewaehlte Artefakt oder Target deterministisch.
- Fehlende Toolchain/Target-Auswahl ist ein Auswahlzustand mit Fix-Hinweis,
  kein generischer Fehler.
- Konfigurationen bleiben nach Neustart von anyCode erhalten.

### [P1] `aslctl` CLI fuer asld-Control-Plane bauen

Labels: `priority:P1`, `area:asl`, `area:cli`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/asld-control-plane-api.md`
- `docs/asld-scaffolding-plan.md`
- `system/daemons/asld/src/ipc.rs`
- `bin/aslctl/src/lib.rs`

Status: Teilweise offen. `bin/aslctl` existiert inzwischen und spricht mit
`asld`; offen bleibt, die CLI als stabile Bedien- und Testoberflaeche mit
vollstaendigem JSON-/Fehler-/Smoke-Contract zu haerten.

Beschreibung:

ASL braucht eine CLI als stabile Bedien- und Testoberflaeche fuer Distro-
Lifecycle, Status, Shell/Exec, Logs und Doctor.

Akzeptanz:

- `aslctl list/status/create/start/stop` spricht mit `asld`.
- Fehlercodes sind maschinenlesbar und menschenlesbar.
- CLI laesst sich fuer Smoke-Tests skripten.
- Spaetere Subcommands fuer shell/exec/logs/doctor sind vorbereitet.

### [P1] ASL Shell/Exec/Console-Integration bis zum nutzbaren Terminalpfad

Labels: `priority:P1`, `area:asl`, `area:terminal`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `system/daemons/asld/src/runtime.rs`
- `system/daemons/asld/src/agent.rs`

Status: Teilweise offen. Modelle fuer Shell/Exec existieren in `asld`, aber
der Produktpfad "Terminal oeffnet Linux-Shell" ist als GitHub-Aufgabe noch
offen zu schneiden.

Beschreibung:

Nach `aslctl` braucht ASL einen echten Sessionpfad: Console-Broker, Fallback-
Konsole ohne Agent und shell/exec ueber Agent, sobald der Gast ready ist.

Akzeptanz:

- `aslctl shell <name>` verbindet zu einer Distro.
- Fallback-Konsole funktioniert auch bei fehlendem Agent.
- `aslctl exec <name> -- <cmd>` liefert Exitstatus und Output.
- Status unterscheidet VM-ready und Agent-ready.

### [P1] ASL Bootable Linux Utility VM als Produktpfad liefern

Labels: `priority:P1`, `area:asl`, `area:virtualization`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/adr/adr-0006-asl-direct-linux-boot.md`
- `system/daemons/asld/src/boot.rs`
- `system/daemons/asld/src/vm.rs`
- `system/daemons/asld/src/vm/serial.rs`

Status: Teilweise offen. `asld` hat Boot-/VM-/Device-Module, aber der
produktfaehige Pfad "`aslctl start demo` bootet Linux bis zur erreichbaren
Shell" ist noch nicht als eigenstaendiges GitHub-Ziel geschnitten.

Beschreibung:

ASL ist erst brauchbar, wenn eine verwaltete Linux-Utility-VM reproduzierbar
startet, stoppbar ist und bei fehlendem Agent nicht als Blackbox endet.

Akzeptanz:

- `aslctl start demo` startet Kernel + initrd ueber den direkten Linux-Bootpfad.
- Paravirt- oder serielle Konsole zeigt Bootlog und Login-/Fallback-Shell.
- `aslctl stop demo` beendet die VM sauber und idempotent.
- State Machine unterscheidet `starting`, `booting`, `ready`, `degraded`,
  `failed` und `stopped`.
- Bootfehler enthalten Kernel-/Initrd-/VM-Exit-Ursache in Logs und Status.

### [P1] ASL Rootfs-Import, Distro-Store und persistente Overlays

Labels: `priority:P1`, `area:asl`, `area:storage`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/adr/adr-0002-asl-distro-and-rootfs-model.md`
- `docs/asl-config-schema.md`
- `system/daemons/asld/src/distro.rs`
- `system/daemons/asld/src/storage.rs`
- `system/daemons/asld/src/store.rs`

Status: Teilweise offen. Distro-/Storage-/Store-Module existieren; Import,
Persistenz und Upgrade-/Rollback-Semantik muessen als nutzbarer Workflow
abgesichert werden.

Beschreibung:

Nutzer sollen eine unterstuetzte Linux-Distribution importieren oder erzeugen
koennen, Daten ueber Neustarts behalten und defekte Images nachvollziehbar
ablehnen koennen.

Akzeptanz:

- `aslctl import <rootfs>` oder `aslctl create <name> --profile dev` legt eine
  verwaltete Distribution an.
- Base-Image und beschreibbarer Overlay-Layer sind getrennt.
- Neustart der Distribution behaelt installierte Pakete und Nutzerdaten.
- Image-/Tarball-Validierung verhindert offensichtliche Pfad- und Formatfehler.
- Status zeigt Image-Version, Overlay-Pfad und Speicherverbrauch.

### [P1] ASL NAT-Netzwerk, DNS und Paketmanager-Kompatibilitaet

Labels: `priority:P1`, `area:asl`, `area:network`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/adr/adr-0003-asl-default-network-mode.md`
- `system/daemons/asld/src/network.rs`
- `system/daemons/asld/src/vm/aslnet.rs`
- `system/daemons/asld/src/vm/e1000.rs`

Status: Teilweise offen. Netzwerk-Module sind vorhanden; das Produktziel
"Paketmanager und Entwickler-Netzwerk funktionieren im Gast" braucht ein
eigenes Akzeptanzgate.

Beschreibung:

ASL soll fuer CLI-Workloads brauchbar sein: Der Gast braucht ausgehende
Konnektivitaet, DNS und stabile Fehlerbilder fuer Netzwerkprobleme.

Akzeptanz:

- Gast erhaelt private IP und Default-Route ueber NAT.
- DNS-Aufloesung laeuft ueber Host-Policy/Broker.
- `apt`, `git`, `curl` und `ssh` funktionieren in der Referenz-Distro.
- Netzwerkstatus ist ueber `aslctl status`/Logs sichtbar.
- Keine eingehenden Ports werden ohne explizite Freigabe exponiert.

### [P1] ASL Shared Folders mit klarer Mount-Policy

Labels: `priority:P1`, `area:asl`, `area:filesystem`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/adr/adr-0004-asl-shared-folder-architecture.md`
- `system/daemons/asld/src/mounts.rs`
- `docs/asl-config-schema.md`

Status: Offen/teilweise offen. Mount-Modelle sind beschrieben, aber der
benutzbare Broker-/Policy-Pfad fuer Projektordner ist noch als Aufgabe zu
schneiden.

Beschreibung:

Entwickler brauchen kontrollierte Host-Projektordner im Linux-Gast, ohne CoreFS
zu einem vollstaendigen Linux-POSIX-Dateisystem umzubauen.

Akzeptanz:

- `aslctl mount add <name> <host_path> <guest_path>` legt explizite Freigabe an.
- Mounts haben `readonly`/`readwrite`, Metadata-, Case-, Exec- und Watch-Policy.
- Symlinks, Executable-Bits und Fehler bei nicht unterstuetzter POSIX-Semantik
  sind dokumentiert und getestet.
- Projektordner ist im Gast les-/schreibbar und ueber Neustart stabil.
- Host-Zugriff bleibt opt-in und auditierbar.

### [P1] ASL Port-Forwarding fuer Dev-Server

Labels: `priority:P1`, `area:asl`, `area:network`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `system/daemons/asld/src/network.rs`
- `docs/aslctl-cli.md`

Status: Offen/teilweise offen. Port-Forwarding ist im Zielbild enthalten, aber
noch nicht als importierbares Feature mit CLI und Status definiert.

Beschreibung:

Dev-Server im Linux-Gast muessen vom anyOS-Host aus erreichbar sein, ohne den
Gast pauschal ins Netzwerk zu exponieren.

Akzeptanz:

- `aslctl port add/list/remove <name>` verwaltet Forwarding-Regeln.
- Host-Port -> Guest-Port wird lokal ueber `127.0.0.1` erreichbar.
- Konflikte, belegte Ports und Gast-Neustarts haben klare Fehler-/Recovery-Pfade.
- `aslctl status` zeigt aktive Forwardings.
- Beispiel: Gast-Service auf `:3000` ist vom Host-Browser erreichbar.

### [P1] ASL Dev-Profil mit Toolchain-Bootstrap bereitstellen

Labels: `priority:P1`, `area:asl`, `area:developer-workflow`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/anyos-roadmap.md`
- `docs/aslctl-cli.md`
- `system/daemons/asld/src/distro.rs`
- `system/daemons/asld/src/config.rs`

Status: Offen. ASL beschreibt eine Developer Edition, aber ein explizites
Dev-Profil mit vorbereiteter Distribution, Basispaketen und Pruefpfad ist noch
nicht als Aufgabe geschnitten.

Beschreibung:

Ein Nutzer soll ASL als Entwicklungsumgebung verwenden koennen, ohne erst alle
Grundlagen manuell zusammenzusuchen. Das Dev-Profil soll eine Referenz-Distro
mit Shell, Paketmanager, Git, SSH und gaengigen Build-Werkzeugen bereitstellen
oder reproduzierbar bootstrapen.

Akzeptanz:

- `aslctl create <name> --profile dev` erzeugt eine entwicklungsfaehige Distro.
- Git, SSH, Paketmanager, Compiler-/Build-Basics und CA-Zertifikate sind
  vorhanden oder werden reproduzierbar installiert.
- Profil dokumentiert Mindestressourcen fuer RAM, CPU und Disk.
- `aslctl doctor <name>` erkennt fehlende Dev-Basiswerkzeuge.
- Referenzbefehle wie `git clone`, `make`/`cargo`/`npm`-Smoke laufen in der
  Distro oder sind als bewusstes Nichtziel markiert.

### [P1] ASL Build/Run/Test-Backend fuer anyCode anbinden

Labels: `priority:P1`, `area:asl`, `area:anycode`, `area:developer-workflow`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/anyos-roadmap.md`
- `apps/anycode/src/`
- `docs/aslctl-cli.md`

Status: Offen. Die ASL-Doku sagt, dass anyOS Code spaeter Build/Run Tasks im
Gast starten kann; in der Issue-Liste fehlt dafuer noch der konkrete
Developer-Workflow.

Beschreibung:

anyCode soll Projekte wahlweise im ASL-Gast bauen, testen und starten koennen,
wobei Diagnostics, Exitstatus und Artefakte strukturiert zurueck in die IDE
laufen.

Akzeptanz:

- Projekt kann in anyCode einem ASL-Kontext/Distro zugeordnet werden.
- Build/Run/Test-Kommandos laufen via `aslctl exec` oder ASL-Backend im Gast.
- Working Directory wird ueber Shared Folder oder Linux-native Workspace-Pfade
  korrekt gemappt.
- Exitstatus, stdout/stderr und Diagnostics erscheinen im Problems-/Output-
  Bereich.
- Port-Forwarding fuer gestartete Dev-Server kann aus dem Run-Profil aktiviert
  oder angezeigt werden.

### [P1] ASL Session-Resume fuer langlebige Entwicklungs-Shells

Labels: `priority:P1`, `area:asl`, `area:terminal`, `area:developer-workflow`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `system/daemons/asld/src/agent.rs`
- `system/daemons/asld/src/runtime.rs`

Status: Offen/teilweise offen. Session-Verwaltung ist im Zielbild enthalten;
das konkrete Verhalten fuer persistente Entwickler-Shells ist noch nicht als
Issue abgebildet.

Beschreibung:

Entwicklungs-Sessions sollen nicht verloren gehen, nur weil ein Terminalfenster
oder die UI neu startet. ASL braucht benannte Shell-Sessions mit Reconnect.

Akzeptanz:

- `aslctl shell <name> --session dev` erstellt oder verbindet eine benannte
  Session.
- Terminal-Resize und UTF-8/Escape-Sequenzen bleiben ueber Reconnect korrekt.
- UI-/Terminal-Neustart beendet die Gast-Shell nicht automatisch.
- `aslctl session list/attach/kill <name>` oder gleichwertige Subcommands sind
  definiert.
- Degraded/Fallback-Console-Pfade sind klar von Agent-Sessions getrennt.

### [P1] ASL "Open in Linux" fuer Projektordner integrieren

Labels: `priority:P1`, `area:asl`, `area:desktop`, `area:developer-workflow`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/anyos-roadmap.md`
- `docs/aslctl-cli.md`

Status: Offen. Finder/File-Dialogs und anyOS Code werden als spaetere
Integrationspunkte genannt; der schnelle Entwicklerpfad "hier Linux-Shell
oeffnen" fehlt noch als Aufgabe.

Beschreibung:

Ein Nutzer soll in einem Host-Projektordner direkt eine ASL-Shell oeffnen
koennen, inklusive sicherer Mount-Erstellung und korrektem Working Directory.

Akzeptanz:

- Kontextaktion "Open in Linux" fuer Projektordner ist als CLI/UI-Contract
  definiert.
- Fehlende Mount-Freigabe wird interaktiv oder per CLI nachvollziehbar
  angelegt.
- Shell startet im passenden Gastpfad.
- Rechte, Mount-Modus und Distro-Auswahl sind sichtbar.
- Fehler bei nicht mountbaren Pfaden enthalten konkrete Recovery-Hinweise.

### [P1] ASL Developer-MVP mit realem Projekt durchtesten

Labels: `priority:P1`, `area:asl`, `area:testing`, `area:developer-workflow`, `roadmap:asl-developer-edition`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/anyos-roadmap.md`
- `docs/aslctl-cli.md`

Status: Offen. Die bestehende Usable-MVP-Matrix prueft Foundation-Faelle; ein
Developer-MVP-Gate fuer echte Projektarbeit fehlt noch.

Beschreibung:

ASL gilt fuer Entwickler erst als brauchbar, wenn ein realer Projektzyklus
funktioniert: Projekt bereitstellen, Dependencies holen, bauen/testen,
Dev-Server starten und vom Host erreichen.

Akzeptanz:

- Referenzprojekt wird in eine ASL-Distro gebracht oder als Shared Folder
  gemountet.
- Dependency-Install, Build und Test laufen im Gast.
- Dev-Server im Gast ist per Port-Forwarding vom Host erreichbar.
- Neustart der Distro behaelt Projektzustand und installierte Dependencies.
- Testbericht unterscheidet Netzwerk-, Mount-, Toolchain-, Port- und
  Sessionfehler.

### [P1] ASL Observability, Logs und Doctor fuer Supportfaelle

Labels: `priority:P1`, `area:asl`, `area:observability`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/asld-control-plane-api.md`
- `system/daemons/asld/src/diagnostics.rs`
- `system/daemons/asld/src/log.rs`
- `system/daemons/asld/src/status.rs`

Status: Teilweise offen. Diagnose-/Log-/Status-Module existieren; fuer
Brauchbarkeit fehlt ein durchgehender Nutzerpfad fuer Fehleranalyse.

Beschreibung:

Wenn Boot, Netzwerk, Rootfs oder Agent scheitern, muss ASL konkrete Ursachen
liefern statt nur "VM failed" zu melden.

Akzeptanz:

- `aslctl logs <name>` zeigt Bootlog, Agent-Status und relevante Host-Events.
- `aslctl doctor <name>` prueft Kernel/initrd, Rootfs, Netzwerk, Mounts und
  Agent-Erreichbarkeit.
- Fehler werden mit stabilen Codes und menschenlesbarer Erklaerung ausgegeben.
- Wiederholte Start/Stop-Zyklen erzeugen keine unbounded Logs.
- Degraded-State hat konkrete Recovery-Hinweise.

### [P1] ASL Usable-MVP E2E-Testmatrix definieren und automatisieren

Labels: `priority:P1`, `area:asl`, `area:testing`, `roadmap:asl-foundation`

Quellen:

- `todos/asl-anyos-subsystem-linux.md`
- `docs/anyos-roadmap.md`
- `docs/aslctl-cli.md`

Status: Offen. Die ASL-Phasen nennen Akzeptanzkriterien, aber es fehlt ein
automatisierbares Gate, das "ASL ist am Ende brauchbar" objektiv prueft.

Beschreibung:

ASL soll erst als Foundation erledigt gelten, wenn der komplette Nutzerpfad
gegen eine Referenz-Distro reproduzierbar laeuft.

Akzeptanz:

- Testmatrix deckt Create/Import, Start, Shell, Exec, Netzwerk, Persistenz,
  Logs, Doctor und Stop ab.
- Referenz-Distro und minimale Testbefehle sind dokumentiert.
- Mindestens ein E2E-Smoke laeuft lokal skriptbar ueber `aslctl`.
- Failures nennen den gebrochenen Subsystembereich statt nur den Gesamttest.
- Matrix wird als Release-/Milestone-Gate in der Roadmap referenziert.

### [P1] displayd Runtime-"Save Layout" und Config-Schema finalisieren

Labels: `priority:P1`, `area:display`, `area:desktop`, `roadmap:desktop-foundation`

Quellen:

- `system/daemons/displayd/src/main.rs`
- `docs/multimonitor-architecture.md`

Status: Offen. Kommentar sagt, runtime save-layout lebt bewusst hinter einem
TODO, bis die IPC-Oberflaeche stabil ist.

Beschreibung:

Multi-Monitor-Layouts sollen nicht nur beim Boot aus Seed/Config entstehen,
sondern zur Laufzeit aus Settings/Tools gespeichert und spaeter exakt wieder
aktiviert werden koennen.

Akzeptanz:

- IPC/API zum Speichern des aktuellen Layouts.
- Persistenz in confd/displayd-Schema.
- Hotplug-Reapply nutzt gespeichertes Layout.
- Settings kann Layout speichern, ohne direkt Kernel-Layout-Owner zu sein.

### [P1] desktopd App-Menues speichern und bei Fokuswechsel an Compositor senden

Labels: `priority:P1`, `area:desktop`, `area:menus`, `roadmap:desktop-foundation`

Quellen:

- `system/daemons/desktopd/src/main.rs`
- `todos/anyui-roadmap.md`

Status: Offen. `desktopd` empfaengt Menu-Registrierungen, speichert/forwarded
aber noch nichts.

Beschreibung:

App-Menues sollen systemweit zur fokussierten App passen. `desktopd` braucht
per-App Menu-State und muss bei Fokuswechsel die richtige Menubar an den
Compositor liefern.

Akzeptanz:

- `CMD_SET_MENU` speichert MenuBarDef pro Sender.
- Fokuswechsel triggert Weitergabe an den Compositor.
- Menu-Update fuer aktive App wird sofort sichtbar.
- Menues werden beim App-Exit bereinigt.

### [P1] CoreFS/FUSE E2E-Harness in QEMU implementieren

Labels: `priority:P1`, `area:corefs`, `area:test`, `roadmap:quality`

Quellen:

- `tests/e2e/corefs_fuse/README.md`
- `kernel/src/fs/corefs/integration_tests.rs`

Status: Offen. README ist Platzhalter; Reporter, QEMU-Harness und Scenario-
Driver fehlen.

Beschreibung:

CoreFS/FUSE braucht einen echten End-to-End-Test, der Kernel `/dev/fuse` und
`corefsd` innerhalb von AnyOS ueber QEMU prueft.

Akzeptanz:

- Serial-Test-Reporter-Konvention `__E2E_PASS__/__E2E_FAIL__`.
- Minimaler AnyOS `e2e-driver`.
- `tests/e2e/corefs_fuse/run.sh` bootet QEMU headless und wertet Serial aus.
- Erstes Szenario `write_read` laeuft automatisiert.

### [P1] anyUI TextField/TextArea Undo/Redo, Word-Wrap und public scroll-to-caret

Labels: `priority:P1`, `area:anyui`, `area:controls`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`
- `libs/libanyui/src/controls/textfield.rs`
- `libs/libanyui/src/controls/textarea.rs`

Status: Teilweise offen. `TextEditor` hat Undo/Redo; `TextField`/`TextArea`
haben Cursor/Selection/max_length/read_only, aber keine vergleichbare Undo/Redo-
API. `ensure_cursor_visible()` ist intern.

Beschreibung:

Basistexteingaben sind fuer Settings, Dialoge, Mail, Surf und anyCode zentral.
Sie brauchen Undo/Redo, Word-Wrap fuer `TextArea` und eine Client-API zum
Cursor/Scroll-Handling.

Akzeptanz:

- Ctrl+Z/Ctrl+Y in TextField und TextArea.
- TextArea kann optional word-wrappen.
- Public API fuer scroll-to-caret/line-column query.
- Host- oder Demo-Test fuer Undo/Redo und Wrap.

### [P1] anyUI Menues mit Submenues, Checkmarks und Accelerators

Labels: `priority:P1`, `area:anyui`, `area:menus`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`
- `libs/libanyui/src/controls/context_menu.rs`
- `libs/libanyui_client/src/controls/menubar.rs`

Status: Teilweise offen. Disabled Items, Icons und Keyboard-Basics existieren;
Submenues, Checkmarks und Accelerator-Anzeige/-Verarbeitung fehlen noch als
vollstaendiges Desktop-Modell.

Beschreibung:

Menues sollen klassische Desktop-Interaktion tragen und aus denselben Commands
wie Toolbars/ContextMenus gespeist werden.

Akzeptanz:

- Submenues mit Pointer- und Keyboard-Navigation.
- Checkmark-/Radio-Menuitems.
- Accelerator-Text rechts und Ausfuehrung ueber Shortcut.
- Fokus-/Dismiss-Logik ist konsistent und regression-getestet.

### [P1] anyUI `ListView` als First-Class-Control bauen

Labels: `priority:P1`, `area:anyui`, `area:controls`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`
- `libs/libanyui/src/controls/`

Status: Offen. Es gibt `ListBox`, aber kein `ListView`-Control fuer Icon/List/
Details-Ansichten.

Beschreibung:

Finder, Dateidialoge, Store und Asset-Browser brauchen ein Desktop-ListView mit
Mehrfachauswahl, Rename, DnD und optionalen Details-Spalten.

Akzeptanz:

- Icon-, List- und Details-Modus oder klarer MVP-Schnitt.
- Multi-Selection, Keyboard-TypeSelect, In-place Rename.
- DnD ueber vorhandenes Payload-System.
- Dogfooding in Finder oder Dateidialog.

### [P1] anyUI `CollectionView`/`ItemsControl` als datengetriebenes Primitive

Labels: `priority:P1`, `area:anyui`, `area:controls`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`

Status: Offen. Kein entsprechendes Control im `controls/`-Verzeichnis.

Beschreibung:

Wiederholte UI-Elemente sollen nicht mehr ad hoc per FlowPanel/Subcontrols
gebaut werden. Ein ItemsControl schafft Basis fuer Store, Mail, Settings,
Notifications und Launcher.

Akzeptanz:

- DataSource/ItemAdapter-Modell.
- Selection, Focus und Command-Hooks.
- Item-Recycling oder Virtualisierungs-Vorbereitung.
- Referenz-App nutzt das Control produktiv.

### [P1] `libcompress` fuer DEFLATE/CRC32 extrahieren

Labels: `priority:P1`, `area:architecture`, `area:libraries`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `libs/libzip/src/`
- `libs/libhttp/src/`
- `libs/libimage/src/`
- `libs/libfont/src/`

Status: Offen. Kein `libs/libcompress` gefunden; mehrere Inflate/Deflate-
Implementierungen bestehen weiter.

Beschreibung:

DEFLATE/CRC32 soll in eine gemeinsame statische Library wandern, damit
Sicherheitsfixes und Performance-Verbesserungen nicht viermal gepflegt werden.

Akzeptanz:

- Neue `libcompress` mit Inflate, Deflate und CRC32.
- `libzip`, `libhttp`, `libimage`, `libfont` nutzen die gemeinsame Library.
- Bestehende Tests bleiben gruen.
- Entfernte Duplikation ist im Diff klar sichtbar.

### [P1] Desktop-God-Struct in Substates splitten

Labels: `priority:P1`, `area:compositor`, `area:architecture`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `system/compositor/compositor/src/desktop/mod.rs`

Status: Offen/teilweise. Impl-Split existiert, `Desktop` selbst bleibt ein sehr
grosser State-Container.

Beschreibung:

`Desktop` soll in klarere Substates wie `WindowManager`, `InputState`,
`UiChrome`, `AppProtocol` und `DesktopLifecycle` zerlegt werden.

Akzeptanz:

- Substructs kapseln thematisch zusammenhaengende Felder.
- Bestehende Module greifen ueber klare APIs statt direkt auf alle Felder zu.
- Kein funktionaler Regress im Compositor-Smoke-Test.

### [P1] VFS- und Loader-Monolithen weiter splitten

Labels: `priority:P1`, `area:kernel`, `area:architecture`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `kernel/src/fs/vfs/mod.rs`
- `kernel/src/task/loader.rs`

Status: Offen/teilweise. VFS hat `path.rs`, `types.rs`, `cache.rs`; `mount.rs`
und `file_ops.rs` fehlen. `task/loader.rs` ist weiterhin nicht in Module
geteilt.

Beschreibung:

Die grossen Kernel-Monolithen sollen in wartbare Module zerlegt werden, ohne
Semantik zu aendern.

Akzeptanz:

- VFS Mount- und File-Operationen in eigene Module verschoben.
- Loader in ELF-, Memory- und Spawn-Teile getrennt.
- Kernel baut und vorhandene VFS/Loader-Tests bleiben gruen.

### [P1] Einheitliches Error-Handling fuer Kernel, Compositor und Libraries

Labels: `priority:P1`, `area:architecture`, `area:kernel`, `area:compositor`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `kernel/src/fs/vfs/types.rs`
- `kernel/src/drivers/hal.rs`

Status: Offen/teilweise. Einzelne Error-Enums existieren, aber kein
einheitlicher Kernel-/Compositor-/Library-Fehlercontract fuer gemeinsame
Systempfade.

Beschreibung:

Viele Pfade nutzen weiterhin gemischte `Option`, Strings, Panics oder lokale
Enums. Ein konsistentes Error-Modell soll Diagnose, IPC-Antworten und Tests
verbessern.

Akzeptanz:

- Kernel-Kernpfade nutzen einen gemeinsamen oder klar konvertierbaren Error-
  Typ.
- Compositor-IPC-Fehler werden strukturiert und rate-limited gemeldet.
- Libraries haben einen gemeinsamen Dll-/Library-Error-Ansatz.
- Bestehende callers koennen ohne grosse Semantik-Aenderung migriert werden.

## P2

### [P2] Kernel-Logging vereinheitlichen

Labels: `priority:P2`, `area:kernel`, `area:observability`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `kernel/src/drivers/serial.rs`

Status: Offen/teilweise. `serial_println!` und `serial_verbose_println!`
existieren, aber kein zentrales `kernel/src/logging.rs` mit Leveln,
Komponentenformat und Filterung.

Beschreibung:

Kernel-Logging soll einheitlicher, filterbarer und besser auswertbar werden,
ohne kritische Pfade durch Locks oder Allokationen zu gefaehrden.

Akzeptanz:

- Zentrale Kernel-Logging-API mit Level und Komponente.
- Kritische Interrupt-/Scheduler-Pfade bleiben sicher.
- Bestehende `serial_println!`-Callsites koennen schrittweise migriert werden.
- Boot-/Panic-Logs bleiben lesbar.

### [P2] TextEditor Syntax-Cache einfuehren

Labels: `priority:P2`, `area:anyui`, `area:editor`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `libs/libanyui/src/controls/text_editor.rs`

Status: Offen. `SyntaxDef` und zeilenweises Tokenizing existieren, aber kein
`token_cache` oder inkrementeller Syntax-Cache.

Beschreibung:

Grosse Dateien sollen nicht in jedem Renderpfad komplett neu tokenisiert werden.
Ein Cache pro Zeile/Version senkt Editor-Kosten und hilft anyCode.

Akzeptanz:

- Tokenisierung wird pro Zeile/Version gecached.
- Edits invalidieren nur betroffene Bereiche.
- Rendering grosser Dateien bleibt fluessig.
- Regressionstest oder Benchmark fuer grosse Datei.

### [P2] libanyui ELF-Symbolresolver bounds-sicher machen

Labels: `priority:P2`, `area:anyui`, `area:security`

Quellen:

- `todos/compositor.md`
- `libs/libanyui/src/draw.rs`

Status: Offen/teilweise. ELF-Magic wird geprueft, aber `resolve_sym()` liest
Header-, Program-Header- und Dynamic-Table-Felder per rohen Pointerzugriffen
ohne sichtbare Laengen-/Mappinggrenzen.

Beschreibung:

Der Mini-ELF-Resolver fuer geladene `.so`-Symbole soll Bounds-Checks bekommen
oder auf einen zentralen, validierenden Loader-Helfer umgestellt werden.

Akzeptanz:

- Header- und Table-Zugriffe pruefen bekannte Mapping-/Dateigroessen.
- Defekte/kurze ELF-Daten liefern `None`, keine OOB-Lesezugriffe.
- Test mit abgeschnittenem/korruptem ELF.

### [P2] anyCode Source-Control-Diff und Commit-Workflow haerten

Labels: `priority:P2`, `area:anycode`, `area:git`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/git.rs`
- `apps/anycode/src/ui/git_panel.rs`

Status: Teilweise offen. Staged/Unstaged-State und Source-Control-Panel
existieren; vollstaendiger Inline-/Side-by-side-Diff, Konflikt-Workflow und
robuste Commit-Validierung sind noch Roadmap.

Beschreibung:

Git soll Teil des Studios sein, nicht nur Prozessausgabe. Ziel ist Stage,
Unstage, Diff, Commit und spaeter Konfliktbearbeitung direkt in anyCode.

Akzeptanz:

- Datei-Diff aus Source-Control-Panel oeffnen.
- Stage/Unstage/Commit mit Validierung.
- Fehler von `agit`/`git` strukturiert anzeigen.
- Merge-Konfliktdateien werden erkannt und im Editor markiert.

### [P2] anyCode Quick-Fixes und CodeActions an Diagnostics anbinden

Labels: `priority:P2`, `area:anycode`, `area:ide`, `roadmap:native-development`

Quellen:

- `docs/anycode-studio-roadmap.md`
- `apps/anycode/src/logic/ide_model.rs`
- `apps/anycode/src/logic/plugin.rs`

Status: Teilweise offen. `CodeAction`-Modelle/Plugin-Capability existieren;
Diagnostic-nahe Quick-Fix-Popups und Apply-Flow sind noch nicht sichtbar
vollstaendig.

Beschreibung:

Diagnostics sollen optionale Fixes/Actions liefern, die am Editor-Ort angezeigt
und sicher angewendet werden koennen.

Akzeptanz:

- Diagnostic kann CodeActions tragen.
- UI zeigt Quick-Fix an der Diagnostic-Position.
- TextEdits werden previewbar und abbrechbar angewendet.
- Mindestens ein echter Fix fuer JSON/TOML/Rust-Minicase.

### [P2] anyUI Popover/Sheet/Drawer-Familie bauen

Labels: `priority:P2`, `area:anyui`, `area:controls`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`

Status: Offen. Kein entsprechendes First-Class-Control gefunden.

Beschreibung:

Moderne Desktop-Interaktionen brauchen ankergestutzte Popovers, Sheets,
Drawers und Inspectors mit standardisierten Dismiss- und Fokusregeln.

Akzeptanz:

- Ankerbasierte Positionierung.
- Escape, Outside-click und Focus-loss Semantik.
- Fokus kehrt zum Ausloeser zurueck.
- Referenznutzung in Settings, Finder oder anyCode.

### [P2] anyUI PropertyGrid, BreadcrumbBar/PathBar und RichText planen/umsetzen

Labels: `priority:P2`, `area:anyui`, `area:controls`, `roadmap:desktop-foundation`

Quellen:

- `todos/anyui-roadmap.md`

Status: Offen. Keine First-Class-Controls fuer diese Familien gefunden.

Beschreibung:

Diese Controls heben Developer-/Admin-Tools, Finder, Dateidialoge, Mail und
Dokumentenansichten auf ein reiferes Desktop-Niveau.

Akzeptanz:

- Separater MVP-Schnitt pro Control-Familie.
- Public Client API.
- Accessibility-Rollen.
- Mindestens eine produktive Referenz-App pro neuem Control.

### [P2] libwebview CSS-Masking-Rest implementieren

Labels: `priority:P2`, `area:webview`, `area:css`

Quellen:

- `docs/css-gaps.md`
- `libs/libwebview/src/`

Status: Offen/teilweise. Single-layer Masking ist dokumentiert; Multi-layer,
`mask-composite`, `mask-mode`, `mask-type`, `mask-border`, volle Clip-Path-
Rasterung und SVG/reference masks fehlen.

Beschreibung:

CSS Masking soll ueber den aktuellen Partial-Scope hinaus spezifikationsnaeher
werden.

Akzeptanz:

- Multi-layer `mask-image` mit per-layer Geometrie.
- `mask-composite`, `mask-mode`/`mask-type` mindestens fuer Kernfaelle.
- Clip-path wird beim Paint/Raster angewendet.
- Webview-Regressionen fuer Masking-Faelle.

### [P2] libwebview Subgrid und Table-Column-Hints vervollstaendigen

Labels: `priority:P2`, `area:webview`, `area:layout`, `area:css`

Quellen:

- `docs/css-gaps.md`
- `libs/libwebview/src/layout/grid.rs`
- `libs/libwebview/src/layout/table.rs`

Status: Teilweise offen. Subgrid ist partial; `table.rs` hat Source-TODO fuer
Column-Width-Hints.

Beschreibung:

Grid/Subgrid und Tabellenlayout sollen bei modernen Seiten weniger Layout-
Abweichungen produzieren.

Akzeptanz:

- Subgrid deckt die wichtigsten CSS Grid Level 2 Sizing-/Placement-Faelle ab.
- Table column width hints werden geparst/beruecksichtigt.
- Layout-Regressionen fuer verschachtelte Grids und Tabellen.

### [P2] Surf/Surf-host Navigation: Anchors, History, Downloads und Zoom

Labels: `priority:P2`, `area:surf`, `area:webview`

Quellen:

- `tools/surf-host/src/main.rs`
- `apps/surf/src/main.rs`

Status: Offen. Source-TODOs fuer `#anchor`-Scroll und Menuepunkte History,
Downloads, Zoom In/Out/Reset sowie About.

Beschreibung:

Surf soll Browser-Basisfunktionen nicht als leere Menueeintraege fuehren.

Akzeptanz:

- `#anchor`-Links scrollen zur Zielposition.
- History und Downloads oeffnen funktionale Views oder Panels.
- Zoom In/Out/Reset wirkt auf den aktiven Tab.
- About zeigt Version/Engine-Info.

### [P2] Surf/anysurf persistente Cookies und Cookie-Attribute

Labels: `priority:P2`, `area:surf`, `area:webview`, `area:http`

Quellen:

- `apps/surf/src/http.rs`
- `libs/libwebview/src/js/document.rs`

Status: Teilweise offen. Cookies parsen Domain/Path/Secure/HttpOnly, aber
`Max-Age` und `Expires` sind laut Source-Kommentar noch nicht behandelt; damit
bleiben Cookies effektiv Session-only.

Beschreibung:

Reale Webseiten brauchen Cookie-Ablauf, Loeschung und Attribut-Semantik, damit
Login-/Consent-/Session-Flows stabil funktionieren.

Akzeptanz:

- `Max-Age` und `Expires` setzen Ablaufzeiten korrekt.
- Abgelaufene Cookies werden nicht gesendet und aus dem Store entfernt.
- Cookie-Loeschung via `Max-Age=0`/vergangenem `Expires` funktioniert.
- Persistenter Cookie-Store ist optional aktivierbar und testbar.
- Domain/Path/Secure/HttpOnly-Semantik bleibt durch Regressionen abgesichert.

### [P2] libwebview DOM Range, TreeWalker und Selection ausbauen

Labels: `priority:P2`, `area:webview`, `area:javascript`, `area:dom`

Quellen:

- `libs/libwebview/src/js/document.rs`
- `docs/libwebview-api.md`

Status: Offen/Stub. `createTreeWalker` und `createRange` sind explizit als
Stubs markiert; Range-Methoden sind groesstenteils No-ops.

Beschreibung:

Framework-Hydration, Rich-Text-Editoren und viele DOM-Diffing-Strategien
erwarten funktionale Range-/Selection-/TreeWalker-APIs.

Akzeptanz:

- `Range` verwaltet Start/Ende, Collapse, Clone und Node-Selection korrekt.
- `TreeWalker` respektiert `whatToShow` und Filter fuer Kernfaelle.
- `window.getSelection()`/Selection-Basis kann Range aufnehmen und auslesen.
- Tests mit typischen React-/Editor-Kompatibilitaetsfaellen.

### [P2] libwebview Dialog-APIs an Surf-UI anbinden

Labels: `priority:P2`, `area:webview`, `area:surf`, `area:javascript`

Quellen:

- `docs/libwebview-api.md`
- `libs/libwebview/src/js/window.rs`

Status: Offen/Stub. `alert()`, `confirm()` und `prompt()` sind als Dialog-Stubs
dokumentiert; `alert` schreibt nur in die Console, `confirm` liefert statisch
`false`.

Beschreibung:

JavaScript-Dialoge sollen in Surf/anysurf als echte modale UI erscheinen und
dabei den JS-Rueckgabewert korrekt liefern.

Akzeptanz:

- `alert`, `confirm` und `prompt` oeffnen Surf-Dialoge.
- Rueckgabewerte werden korrekt in die JS-Ausfuehrung zurueckgefuehrt.
- Dialoge blockieren/reihen sich so ein, dass kein UI-Deadlock entsteht.
- Tests fuer OK/Cancel/Text-Eingabe.

### [P2] Fetch/XHR asynchron ueber Surf-Netzwerkworker fuehren

Labels: `priority:P2`, `area:surf`, `area:webview`, `area:javascript`, `area:http`

Quellen:

- `docs/libwebview-api.md`
- `libs/libwebview/src/js/fetch.rs`
- `libs/libwebview/src/js/xhr.rs`
- `apps/surf/src/net_worker.rs`

Status: Teilweise offen. Doku und Source beschreiben `fetch()`/XHR als
synchronen Host-HTTP-Aufruf mit synchroner Promise-aehnlicher Abwicklung.

Beschreibung:

Fetch und XHR sollen Browser-artig nicht die JS-/UI-Ausfuehrung blockieren und
sauber mit Microtasks, readyState-Events und Netzwerkfehlern zusammenspielen.

Akzeptanz:

- `fetch()` bleibt pending, bis der Netzwerkworker antwortet.
- Promise-Resolution laeuft ueber Microtasks/Event-Loop.
- XHR-readyState-/load-/error-/timeout-Events werden asynchron ausgeliefert.
- Surf zeigt Netzwerkfehler und Abbrueche nachvollziehbar an.
- Regressionen fuer parallele Requests und Redirect-/Cookie-Interaktion.

### [P2] ES-Module-Ladepfad in Surf/libjs vervollstaendigen

Labels: `priority:P2`, `area:surf`, `area:javascript`, `area:libjs`

Quellen:

- `apps/surf/src/net_worker.rs`
- `libs/libjs/src/compiler.rs`
- `docs/libjs-api.md`

Status: Teilweise offen. Surf hat `ModuleScript`-Ladepfade; im libjs-Compiler
ist `export * from "module"` noch ein No-op, weil Namespace-Enumeration fehlt.

Beschreibung:

Moderne Webseiten und Bundler verlassen sich auf statische Module, Re-Exports,
Modulcache und nachvollziehbare Lade-/Fehlersemantik.

Akzeptanz:

- `export * from` erzeugt korrekte Namespace-/Re-Export-Semantik.
- Modulcache verhindert doppelte Ausfuehrung.
- Zyklen und fehlgeschlagene Imports haben definierte Fehlerpfade.
- Surf laedt ModuleScript-Abhaengigkeiten relativ zur Dokument-URL.
- Tests fuer statische Imports, Re-Exports und Fehlermeldungen.

### [P2] CSS Animations/Transitions Timing-Semantik vervollstaendigen

Labels: `priority:P2`, `area:webview`, `area:css`

Quellen:

- `libs/libwebview/src/style/engine.rs`

Status: Teilweise offen. Der Animation-Parser ignoriert aktuell
`direction`, `fill-mode` und `play-state`-Keywords statt sie im Style zu
tracken.

Beschreibung:

Animationen sollen bei realen Seiten nicht nur starten, sondern Richtung,
Fuellmodus und Pausenzustand korrekt beruecksichtigen.

Akzeptanz:

- `animation-direction`, `animation-fill-mode` und `animation-play-state`
werden im Stylemodell abgebildet.
- Renderer/Timeline respektiert Pause, Reverse/Alternate und Fill-Modes.
- Tests fuer ShortHand-Parsing und sichtbares End-/Zwischenstadium.

### [P2] CSS Filter und Backdrop-Filter bis zum Paint anwenden

Labels: `priority:P2`, `area:webview`, `area:css`, `area:renderer`

Quellen:

- `libs/libwebview/src/style/engine.rs`
- `docs/css-gaps.md`

Status: Zu pruefen/teilweise offen. `filter` wird geparst und im Style
abgelegt; die Import-Prioritaet ist die vollstaendige Paint-/Compositing-
Anwendung inklusive Backdrop-Faellen.

Beschreibung:

Viele moderne UIs nutzen Blur, Shadow, Grayscale und Backdrop-Effekte fuer
Overlays, Navigation und Medienkarten.

Akzeptanz:

- Kernfilter wie `blur`, `brightness`, `contrast`, `grayscale`, `opacity` und
  `drop-shadow` rendern sichtbar.
- `backdrop-filter` wirkt auf den darunterliegenden Inhalt.
- Filter interagieren korrekt mit Border-Radius, Clip und Transforms.
- Screenshot-Regressionen fuer Filter- und Backdrop-Faelle.

### [P2] Webfont-Coverage fuer WOFF 1.0 und WOFF2 erweitern

Labels: `priority:P2`, `area:surf`, `area:webview`, `area:fonts`, `area:css`

Quellen:

- `tools/surf-host/README.md`
- `apps/surf/src/resources.rs`

Status: Teilweise offen. Raw TrueType/sfnt und WOFF2 mit TrueType-Outlines
werden geladen; WOFF 1.0 und bestimmte WOFF2-Outline-Formate sind noch
ausgeschlossen.

Beschreibung:

CSS `@font-face` soll auf realen Webseiten weniger haeufig in Fallback-Fonts
fallen, ohne unsichere Fontdaten blind zu akzeptieren.

Akzeptanz:

- WOFF 1.0 wird sicher dekodiert oder klar begruendet abgelehnt.
- WOFF2-Varianten mit bisher fehlenden Outline-Formaten haben Support oder
  explizite Fallback-Tests.
- Font-Fallback bleibt deterministisch und crasht nicht bei kaputten Fonts.
- Regressionen fuer `@font-face`-Laden, Cache und Fallback.

### [P2] surf-host Remote-Control fuer JS/CSS-Regressionen erweitern

Labels: `priority:P2`, `area:surf-host`, `area:testing`, `area:webview`

Quellen:

- `tools/surf-host/README.md`
- `tools/surf-host/src/main.rs`
- `docs/libwebview-api.md`

Status: Offen/Erweiterung. Remote-Control und Console-Ausgabe existieren; fuer
JS/CSS-Kompatibilitaet fehlen noch gezielte Test-Kommandos.

Beschreibung:

surf-host soll als reproduzierbarer Harness fuer Layout-, CSS- und JavaScript-
Regressionen dienen koennen.

Akzeptanz:

- Remote-Kommandos fuer `eval`, Console-Auslesen, DOM-/Layout-Snapshot und
  `wait-idle`.
- Screenshot- und Pixelvergleich lassen sich skriptbar ausloesen.
- Netzwerk-/Resource-Fehler koennen im Testlauf ausgelesen werden.
- Dokumentierte Beispieltests fuer eine JS- und eine CSS-Regression.

### [P2] anyOS stdlib Window-IPC fuer List/Focus implementieren

Labels: `priority:P2`, `area:stdlib`, `area:desktop`

Quellen:

- `libs/stdlib/src/ui/window.rs`

Status: Offen. `list_windows()` und `focus()` sind Stub-TODOs.

Beschreibung:

System- und Utility-Apps brauchen eine stabile stdlib-API, um offene Fenster zu
listen und ein Fenster zu fokussieren/anzuheben.

Akzeptanz:

- Compositor-IPC fuer Window-List und Focus/Raise existiert.
- `anyos_std::ui::window::list_windows` liefert nutzbare Daten.
- `focus(window_id)` validiert und aktiviert das Ziel.
- Taskmanager/Dock oder Testtool nutzt die API.

### [P2] WiFi-Passwortdialog in wifimon ueber anyUI

Labels: `priority:P2`, `area:network`, `area:anyui`, `app:wifimon`

Quellen:

- `system/compositor/wifimon/src/main.rs`

Status: Offen. WPA2-Netze verweisen aktuell auf Terminalkommando.

Beschreibung:

wifimon soll fuer geschuetzte Netze einen nativen Passwortdialog anzeigen und
die Verbindung anstossen.

Akzeptanz:

- Dialog mit SSID, Passwortfeld und Cancel/Connect.
- Passwort wird nicht geloggt.
- Erfolg/Fehler wird im UI angezeigt.
- Bestehender CLI-Pfad bleibt als Fallback moeglich.

### [P2] Lock Screen und Ctrl+Alt+Delete-Systemdialog

Labels: `priority:P2`, `area:compositor`, `area:session`, `roadmap:desktop-foundation`

Quellen:

- `system/compositor/compositor/src/desktop/input.rs`

Status: Offen. Source-TODOs fuer LockScreen-Shortcut und Ctrl+Alt+Delete-
Systemdialog.

Beschreibung:

Session-Kontrollen sollen ueber definierte System-UI laufen: Lock, Force-Quit,
Taskmanager, Logout und Shutdown.

Akzeptanz:

- LockScreen-Shortcut oeffnet echten Lock-Screen oder gesicherten Stub.
- Ctrl+Alt+Delete zeigt Systemdialog.
- Fullscreen-Apps koennen den Pfad nicht blockieren.
- Aktionen sind capability-/policy-geprueft.

### [P2] FAT32-Formatierung in diskutil

Labels: `priority:P2`, `area:storage`, `app:diskutil`

Quellen:

- `system/utilities/diskutil/src/main.rs`

Status: Offen. Kontextmenue zeigt "FAT32 formatting is not yet implemented".

Beschreibung:

diskutil soll FAT32-Volumes formatieren koennen oder den Menuepunkt bis zur
Implementierung sauber ausblenden.

Akzeptanz:

- FAT32-Formatpfad implementiert und sicher bestaetigt.
- Blockdevice-/Partition-Auswahl wird validiert.
- Fehler sind nutzerverstaendlich.
- Smoke-Test oder kleines Image-Testcase.

### [P2] FUSE Wait/Wake statt Polling-API

Labels: `priority:P2`, `area:kernel`, `area:fs`

Quellen:

- `kernel/src/fs/fuse/mod.rs`

Status: Offen. Kommentar beschreibt Phase 5.7: blockierende VFS-Calls sollen
schlafen, bis passende Reply eintrifft.

Beschreibung:

FUSE-Session soll in den Kernel-Wait-/Wake-Mechanismus integriert werden, damit
FUSE nicht polling-basiert bleibt.

Akzeptanz:

- Pending Replies koennen wartende Threads schlafen legen.
- Reply weckt exakt passende Waiter.
- Timeouts/Unmount/Daemon-Exit wecken mit Fehler.
- Tests fuer erfolgreiche Reply und Daemon-Abbruch.

### [P2] ARM64 Debug-Memory Page-Table-Walk und Present-Checks

Labels: `priority:P2`, `area:kernel`, `area:arm64`, `area:debugger`

Quellen:

- `kernel/src/task/scheduler/debug_trace.rs`

Status: Offen. ARM64-Pfade kopieren direkt und geben bei Memory-Map leere
Regionen zurueck.

Beschreibung:

Debugger-Memory-Reads/Writes und Memory-Map brauchen auf ARM64 dieselben
Safety-Eigenschaften wie x86_64: Page-Present-Checks und Page-Table-Walk.

Akzeptanz:

- ARM64 Read/Write prueft Present/Permission pro Page.
- Memory-Map liefert echte Regionen.
- Ungueltige Adressen erzeugen Fehler statt unsicheren Zugriff.
- Host-/KUnit-Test oder Architektur-Smoke.

### [P2] ARM64 CPU-Frequenztelemetrie anbinden

Labels: `priority:P2`, `area:kernel`, `area:arm64`, `roadmap:scheduler-power`

Quellen:

- `kernel/src/task/cpu_monitor.rs`
- `docs/scheduler-power-management-plan.md`

Status: Offen. ARM64 meldet Frequenz aktuell als `0`.

Beschreibung:

Power-/Scheduler-Telemetrie braucht auch auf ARM64 sinnvolle Frequenzdaten oder
einen klaren "unknown"-Status statt hartem Nullwert.

Akzeptanz:

- ARM64-Frequenz wird aus verfuegbarem Counter/Firmwarepfad gelesen oder als
  explizit unknown modelliert.
- Userspace zeigt unknown nicht als 0 MHz.
- Pfad ist in Power-Telemetrie dokumentiert.

### [P2] Intel-WiFi Firmware-Loading und TX-Queue

Labels: `priority:P2`, `area:kernel`, `area:network`, `driver:wifi`

Quellen:

- `kernel/src/drivers/network/iwl_wifi.rs`

Status: Offen. Device-Erkennung existiert; Firmware-Loading und TX via
Firmware-Command-Queue sind TODO/Stubs.

Beschreibung:

Intel-WiFi braucht UCode-Loading, Firmware-Section-Parsing, DMA-Transfer und
TX-Queue-Anbindung, bevor echte WiFi-Nutzung moeglich ist.

Akzeptanz:

- Firmware aus `/System/Drivers/wifi/` laden und validieren.
- Firmware-Sections korrekt an Device uebertragen.
- TX-Pfad sendet Frames ueber Firmware-Command-Queue.
- Fehlerstatus ist fuer wifimon/CLI sichtbar.

### [P2] Bluetooth USB Bulk OUT fuer ACL-Daten

Labels: `priority:P2`, `area:kernel`, `driver:bluetooth`

Quellen:

- `kernel/src/drivers/bluetooth/usb_transport.rs`

Status: Offen. `send_acl()` ist Stub.

Beschreibung:

Bluetooth-L2CAP/ACL-Senden braucht einen USB-Bulk-OUT-Transferpfad.

Akzeptanz:

- Bulk-OUT Endpoint wird erkannt und gespeichert.
- `send_acl()` schreibt Daten ueber USB-Subsystem.
- Fehler und Backpressure werden propagiert.
- Minimaler Smoke mit kontrolliertem HCI/ACL-Paket.

### [P2] libGL Conditional Execution und Sub-Image-Update

Labels: `priority:P2`, `area:graphics`, `area:libgl`

Quellen:

- `libs/libgl/src/compiler/lower.rs`
- `libs/libgl/src/lib.rs`

Status: Offen. Shader-Lowering evaluiert Branches flach; Sub-Image-Update ist
Stub.

Beschreibung:

libGL braucht korrekte bedingte Ausfuehrung im Lowering und Teilupdates fuer
Texturen/Bilder.

Akzeptanz:

- If/else fuehrt nur den passenden Branch aus oder erzeugt korrektes IR.
- Sub-image update aktualisiert Rechteckbereiche ohne komplette Reallocation.
- Tests fuer Branching und Teilupdate.

### [P2] libjs computed methods nicht als enumerable Property setzen

Labels: `priority:P2`, `area:javascript`, `area:libjs`

Quellen:

- `libs/libjs/src/compiler.rs`

Status: Offen. TODO: `SetProp` fuer computed methods erzeugt falsche
Enumerable-Semantik.

Beschreibung:

Computed methods in Klassen/Objekten sollen mit korrekten Property-
Descriptors erzeugt werden.

Akzeptanz:

- Computed method ist nicht enumerable, wenn ECMAScript das verlangt.
- Descriptor-Attribute stimmen fuer relevante Method-Arten.
- Testcase prueft `Object.keys`/Descriptor.

### [P2] libjs Destructuring-/Assignment-Ziele vollstaendig unterstuetzen

Labels: `priority:P2`, `area:javascript`, `area:libjs`

Quellen:

- `libs/libjs/src/compiler.rs`

Status: Offen/teilweise offen. Der Compiler verwirft Werte fuer nicht
unterstuetzte Assignment-Ziele explizit, statt einen korrekten Compile-Fehler
oder Semantik zu liefern.

Beschreibung:

Destructuring, Rest/Spread und komplexe Assignment-Targets muessen fuer
moderne JS-Bundles deterministisch funktionieren.

Akzeptanz:

- Nicht unterstuetzte Targets erzeugen klare Fehler statt stiller No-ops.
- Objekt-/Array-Destructuring mit Defaults, Rest und verschachtelten Patterns
  hat Regressionstests.
- Compound Assignments auf Member-/Pattern-Ziele verhalten sich spec-nahe.

### [P2] libjs RegExp-Kompatibilitaet mit Test262 absichern

Labels: `priority:P2`, `area:javascript`, `area:libjs`, `area:testing`

Quellen:

- `libs/libjs/src/regexp.rs`
- `libs/libjs_tests/test262/README.md`

Status: Teilweise offen. Die RegExp-Engine dokumentiert eine ECMAScript-
Teilmenge; Lookaround, named groups und Flags sind im Code angelegt, brauchen
breite Kompatibilitaetstests.

Beschreibung:

RegExp ist ein Hotspot fuer Frameworks, Router, Parser und Polyfills. Fehler
fallen oft erst auf realen Webseiten auf.

Akzeptanz:

- Test262-RegExp-Suite wird in sinnvolle Gruppen aufgeteilt und regelmaessig
  ausgefuehrt.
- Named groups/backrefs, lookbehind, `u`/`y`/`s`-Flags und Unicode-Klassen sind
  als pass/fail dokumentiert.
- Bekannte Abweichungen bekommen explizite Skip-/Tracking-Eintraege.

### [P2] libjs Test262-Abdeckung und Browser-Kompatibilitaetsdashboard ausbauen

Labels: `priority:P2`, `area:javascript`, `area:libjs`, `area:testing`

Quellen:

- `libs/libjs_tests/test262/README.md`
- `libs/libjs/tests/test262_core.rs`

Status: Offen/Erweiterung. Es gibt eine kuratierte Test262-Integration; fuer
Surf-Readiness fehlt eine priorisierte Sicht auf echte Web-Kompatibilitaet.

Beschreibung:

libjs braucht eine nachvollziehbare Matrix, welche ECMAScript-Features fuer
Surf stabil sind und welche noch bewusst ausstehen.

Akzeptanz:

- Test262-Ergebnisse werden nach Feature-Buckets zusammengefasst.
- P0/P1-Webfeatures wie Promise, async/await, modules, classes, typed arrays,
  RegExp und destructuring haben sichtbare Passraten.
- Neue libjs-Fixes muessen mindestens einen passenden Test-Bucket beruehren.
- Dashboard/Markdown-Report ist lokal generierbar und importierbar.

### [P2] libjs Intl-Basis fuer haeufige Webseiten evaluieren

Labels: `priority:P2`, `area:javascript`, `area:libjs`

Quellen:

- `docs/libjs-api.md`
- `libs/libjs/src`

Status: Offen/Pruefpunkt. In der dokumentierten API-Matrix taucht `Intl` nicht
als vorhandene Runtime-API auf; viele Webseiten setzen mindestens
`Intl.DateTimeFormat` oder `Intl.NumberFormat` voraus.

Beschreibung:

Intl muss nicht sofort vollstaendig sein, aber Surf braucht eine klare
Kompatibilitaetsstrategie fuer Lokalisierung und Formatierung.

Akzeptanz:

- Audit, welche `Intl`-APIs reale Zielseiten tatsaechlich nutzen.
- Minimal-Implementierung oder bewusstes Stub-Verhalten fuer DateTimeFormat und
  NumberFormat.
- Feature-Detection verhaelt sich konsistent.
- Tests fuer Deutsch/Englisch-Basisformatierung oder dokumentierte Nichtziele.

## P3

### [P3] SPICE virtio-input Tablet, LED/StatusQ und Multimedia-Keys

Labels: `priority:P3`, `area:drivers`, `area:spice`

Quellen:

- `docs/spice-vdagent.md`
- `kernel/src/drivers/virtio/input.rs`

Status: Offen. Dokumentierte Limitierungen: Tablet-ABS-Positionierung, Force-
Feedback/LED-Status ueber `statusq`, Multimedia-/Browser-Keys.

Beschreibung:

SPICE-Gastintegration soll ueber Maus/Tastatur-Basis hinaus vollstaendiger
werden.

Akzeptanz:

- Tablet-ABS skaliert korrekt auf Displaykoordinaten.
- LED-Status/StatusQ zumindest fuer Tastatur-LEDs.
- Multimedia-/Browser-Keys werden gemappt oder bewusst ignoriert dokumentiert.

### [P3] `ac` Hidden-Files Toggle

Labels: `priority:P3`, `app:ac`, `area:file-manager`

Quellen:

- `bin/ac/src/main.rs`

Status: Offen. `Ctrl+H` ist als TODO vermerkt.

Beschreibung:

Der Dateimanager/Commander `ac` soll versteckte Dateien per Ctrl+H ein- und
ausblenden koennen.

Akzeptanz:

- Ctrl+H toggelt Hidden-State.
- Anzeige aktualisiert sich ohne Pfadwechsel.
- Einstellung ist optional persistierbar.

### [P3] Crate-/Package-Registry-Lookups in anyCode verbinden

Labels: `priority:P3`, `area:anycode`, `area:packages`

Quellen:

- `apps/anycode/src/ui/crate_manager_dialog.rs`
- `apps/anycode/src/logic/crates.rs`

Status: Offen. UI meldet, dass crates.io-Version-Lookup vorbereitet, aber noch
nicht an Registry-Backend angebunden ist.

Beschreibung:

Dependency-Dialogs sollen Versionen und Paketmetadaten aus lokalen/remote
Registry-Backends abfragen koennen.

Akzeptanz:

- Lokale Registry-Abfrage fuer ccargo/acargo.
- Optionaler remote crates.io Lookup, wenn Netzwerk/Policy erlaubt.
- UI zeigt Versionen/Fehler ohne Blockade.

### [P3] anyUI Event-Handler-Duplizierung reduzieren

Labels: `priority:P3`, `area:anyui`, `area:architecture`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `libs/libanyui/src/controls/button.rs`
- `libs/libanyui/src/controls/icon_button.rs`
- `libs/libanyui/src/controls/checkbox.rs`
- `libs/libanyui/src/controls/toggle.rs`

Status: Offen. Mehrere Button-/Toggle-artige Controls implementieren aehnliche
MouseDown/Click/Pressed-State-Logik separat; kein gemeinsames
`ToggleableControl`-/Clickable-Pattern gefunden.

Beschreibung:

Wiederkehrende Eventlogik soll in kleine gemeinsame Helper oder Traits
wandern, ohne die Controls unnoetig zu abstrahieren.

Akzeptanz:

- Gemeinsamer Helper fuer pressed/click/keyboard activation, wo passend.
- Button, IconButton, Checkbox, Toggle und RadioButton bleiben visuell gleich.
- Weniger duplizierte State-Transitions.

### [P3] Compositor Rounded-Corner-/SDF-Rechenwege konsolidieren

Labels: `priority:P3`, `area:compositor`, `area:rendering`, `roadmap:refactoring`

Quellen:

- `todos/architecture-refactoring.md`
- `system/compositor/compositor/src/compositor/blend.rs`
- `system/compositor/compositor/src/compositor/mod.rs`

Status: Teilweise offen. `rounded_rect_sdf()` existiert, aber im Compositing-
Pfad gibt es weiterhin inline Corner-Math fuer Layer-Corners.

Beschreibung:

Rounded-Corner-Berechnungen sollen ueber gemeinsame Helfer laufen, damit
Antialiasing, Schatten und Layer-Clipping konsistent bleiben.

Akzeptanz:

- Wiederverwendbarer Helper fuer Corner-/SDF-Masken.
- Compositor-Pfade nutzen denselben Helper.
- Screenshot-Smoke fuer Fenster-Corners zeigt keine Regression.

### [P3] ftpd RFC-Compliance-Restpunkte buendeln

Labels: `priority:P3`, `area:network`, `daemon:ftpd`

Quellen:

- `system/daemons/ftpd/src/session.rs`

Status: Offen. Dokumentierte Low-Priority-TODOs: FEAT-Erweiterung, EPSV IPv6,
chunked MLSD, SITE CHMOD und 64-bit file sizes.

Beschreibung:

Die FTP-Server-Restpunkte sollten als ein niedriger priorisiertes Issue
gebuendelt werden, bis konkrete Nutzeranforderungen eine Aufteilung sinnvoll
machen.

Akzeptanz:

- FEAT listet unterstuetzte TYPE/STRU/MODE sauber.
- EPSV fuer IPv6 ist implementiert oder klar abgelehnt.
- MLSD kann grosse Verzeichnisse chunked liefern.
- 64-bit Groessen warten auf `stat64/lseek64`-Basis oder nutzen sie, wenn
  vorhanden.

## Nicht importieren / bereits erledigt oder ueberholt

- Window-ID-Ownership fuer Move/Destroy/Minimize/Fullscreen: im aktuellen
  `desktop/ipc.rs` ueber `owns_window()` erkennbar umgesetzt.
- Unsichere TextField-Type-Casts: durch `ControlKind`-gepruefte
  `cast_mut`/`cast_ref`-Helper entschaerft.
- `RenderContext` Helper fuer anyUI Controls: existiert in
  `libs/libanyui/src/control.rs`.
- anyUI Layout Dirty-Tracking: `needs_layout`, `needs_repaint` und
  `mark_needs_layout()` existieren.
- `GlobalAppState<T>` Macro: als `anyos_std::global_app_state!` vorhanden;
  einzelne Apps nutzen noch altes `static mut APP`, das ist eher eine
  Migrationsaufgabe und kein Architektur-Blocker.
- `asld` Scaffolding: `system/daemons/asld/` existiert mit vielen Modulen ueber
  das alte Scaffolding hinaus.
- DnD-Framework "von Null aufbauen": Basis ist vorhanden; nur Restfeatures wie
  Drag-Image-Ghost/Cross-Window-Haertung sollten spaeter separat geschnitten
  werden.
- ComboBox "Control existiert": vorhanden; Feinschliff ist Teil der allgemeinen
  anyUI-Control-Haertung, kein eigener P0-Import.
- Generierte Template-TODOs in anyCode Storyboard/Designer und Hosttests
  (`TODO: handle event`, `connect this to navigation host`) sind bewusst nicht
  importiert; sie gehoeren in neu generierte App-Projekte, nicht in den
  anyOS-Backlog.
- False Positives aus Unicode-Daten, `mktemp`-Templates und Beispielstrings sind
  nicht importiert.
