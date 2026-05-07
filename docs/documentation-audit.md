# Documentation Audit

Stand: 2026-05-06

Scope:

- `docs/*.md`
- `docs/adr/*.md`
- `todos/*.md`
- Root-Markdown-Dateien wurden nur auf offensichtliche Querverweise geprueft.

## Ergebnis

Die Dokumentation ist groesstenteils nutzbar, aber nicht homogen aktuell. Die
wichtigsten Abweichungen lagen bei ASL-Pfaden/Implementierungsstand,
Multimonitor-Status und alten Verweisen auf `CLAUDE.md`. Diese Punkte wurden
direkt markiert oder korrigiert.

## Direkt Korrigiert

### ASL

- `todos/asl-anyos-subsystem-linux.md`
  - `aslctl`-Sourcepfad von `system/utilities/aslctl/` auf `bin/aslctl/`
    korrigiert.
- `docs/aslctl-cli.md`
  - Implementierungsnotiz ergaenzt: `bin/aslctl/` existiert und spricht ueber
    Pipe-IPC mit `asld`.
- `docs/asld-control-plane-api.md`
  - "transportneutral, noch nicht festgelegt" ersetzt durch aktuellen
    Implementierungsstand: Pipe-IPC in `system/daemons/asld/src/ipc.rs`.
- `docs/asld-scaffolding-plan.md`
  - Als historisches Scaffolding-Dokument markiert.
- `docs/anyos-roadmap.md`
  - ASL-Naechste-Schritte von "scaffolden" auf Haertung, Bootpfad,
    Rootfs/Network/Developer-Edition aktualisiert.
- `todos/github-issue-import.md`
  - `aslctl`-Issue von "nicht vorhanden" auf "existiert, Haertung offen"
    aktualisiert.

### Desktop / SPICE / Architektur

- `docs/multimonitor-architecture.md`
  - Status von altem Branch-Hinweis auf "groesstenteils in main gelandet"
    aktualisiert; DPI-Pipeline bleibt offen.
- `docs/spice-vdagent.md`
  - Verweis auf `CLAUDE.md` durch Verweis auf `todos/github-issue-import.md`
    ersetzt.
- `docs/architecture.md`
  - ARM64/Raspberry-Pi-Verweis auf `CLAUDE.md` entfernt und auf aktuelle
    Folgeaufgaben verwiesen.

## Aktuell Oder Plausibel

Diese Dokumente passen beim Stichprobenabgleich zu vorhandenen Codepfaden und
wirken als Ziel-/API-Doku weiter verwendbar:

- `docs/ami.md`
- `docs/ami-v1.md`
- `docs/anycode-studio-roadmap.md`
- `docs/anyrc-compiler-architecture.md`
- `docs/anyui-api.md`
- `docs/applications.md`
- `docs/asl-confd-manifest.md`
- `docs/asl-config-schema.md`
- `docs/async-foundation.md`
- `docs/bootloader.md`
- `docs/confd.md`
- `docs/corefs.md`
- `docs/corevm-api.md`
- `docs/crust-ccargo-api.md`
- `docs/css-gaps.md`
- `docs/fullscreen.md`
- `docs/libc-api.md`
- `docs/libcompositor-api.md`
- `docs/libcxx-api.md`
- `docs/libdb-api.md`
- `docs/libfont-api.md`
- `docs/libgl-api.md`
- `docs/libimage-api.md`
- `docs/libini-api.md`
- `docs/libjs-api.md`
- `docs/libm-api.md`
- `docs/librender-api.md`
- `docs/libsvg-api.md`
- `docs/libwebview-api.md`
- `docs/libzip-api.md`
- `docs/nogui.md`
- `docs/scheduler-power-management-plan.md`
- `docs/searchd-api.md`
- `docs/self-hosting-roadmap.md`
- `docs/services.md`
- `docs/stdlib-api.md`
- `docs/syscalls.md`
- `docs/uictl.md`
- `docs/uisys-api.md`
- `docs/vmctl.md`
- `todos/architecture-refactoring.md`
- `todos/compositor.md`
- `todos/github-issue-import.md`

## Braucht Folgepruefung

Diese Dokumente sind nicht zwingend falsch, sollten aber als eigene kleine
Doku-Aufgaben abgeglichen werden:

- `docs/aslctl-cli.md`
  - Zielsyntax gegen die tatsaechliche Syntax in `bin/aslctl/src/lib.rs`
    abgleichen. Beispiel: `create` erwartet im Code aktuell
    `<name> <image-ref> <owner>`, waehrend die Doku teilweise noch
    `[--from <image>] [--profile <profile>]` beschreibt.
- `docs/asld-control-plane-api.md`
  - Objektmodell mit den realen `asld`-Wire-Kommandos und Antwortformaten
    synchronisieren.
- `docs/asld-scaffolding-plan.md`
  - Entweder im Archiv/Historie-Bereich belassen oder durch eine aktuelle
    `asld-runtime-architecture.md` ersetzen.
- `docs/adr/adr-0006-asl-direct-linux-boot.md` und
  `docs/adr/adr-0007-asl-seabios-userspace-firmware.md`
  - Zusammen klarstellen: normaler ASL-Pfad bleibt Direct Linux Boot,
    SeaBIOS ist nur ein separates PC-Boot-Profil.
- `todos/asl-anyos-subsystem-linux.md`
  - Nach den neuen ASL-Developer-Issues die Phasen mit
    `todos/github-issue-import.md` synchronisieren.
- `todos/anyui-roadmap.md`
  - Hat ein altes Status-Update vom 2026-04-23; erneut gegen `libs/libanyui`
    und `libs/libanyui_client` pruefen.
- `docs/multimonitor-architecture.md`
  - Historische Phasen sind lang und groesstenteils erledigt. Eine kurze
    "Current State" Sektion wuerde den Einstieg verbessern.
- `README.md`
  - Sehr umfangreich; Build-/Feature-Liste sollte separat gegen die aktuelle
    App-/Daemon-/Lib-Struktur verifiziert werden.
- `RELEASE_NOTES.md`
  - Letzter sichtbarer Eintrag ist v0.4.38 vom 2026-03-13. Gegen `VERSION`
    und neuere Arbeiten pruefen.
- `JPEG_PORT_NOTES.md` und `JPEG_PORT_REPORT.md`
  - Wirken als historische Port-Notizen; bei Bedarf nach `docs/` oder
    `third_party/` einsortieren oder explizit als historisch markieren.

## Nicht Geprueft

- Vollstaendige API-Signatur-Genauigkeit aller `*-api.md` gegen Rust/C-Header.
- Vollstaendige README-Buildanleitung auf frischem System.
- Externe Standards oder URLs.

## Empfehlung

1. ASL-Doku als naechstes konsolidieren: `aslctl-cli`, `asld-control-plane-api`,
   `asl-anyos-subsystem-linux` und ADR-0006/0007.
2. Danach `anyui-roadmap.md` erneut gegen Code verifizieren.
3. API-Dokumente spaeter maschinell pruefbarer machen, z. B. mit einem kleinen
   Doc-Audit, der Codepfade und wichtige Symbolnamen kontrolliert.
