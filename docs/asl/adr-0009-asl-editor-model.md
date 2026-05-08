# ADR-0009 - ASL editor model: anyOS-side first, in-distro GUI later

## Status

Accepted

## Date

2026-05-07

## Context

ADR-0008 stellt fest, dass ASL als Entwicklerplattform positioniert ist. Damit
stellt sich unmittelbar die Frage, **wo der Editor laeuft**. Drei Optionen
stehen architektonisch zur Verfuegung:

1. **Editor auf anyOS** — z. B. `anycode`. Quelltext liegt in der Distro,
   Zugriff ueber Shared-Mount (siehe ADR-0004). Builds laufen via
   `aslctl run` (oder `exec`) in der Distro.
2. **Editor in der Distro** — Linux-GUI-App (VS Code, JetBrains, Vim+tmux ueber
   Terminal, Emacs etc.) laeuft im Gast. Erfordert eine GUI-Bridge zum
   anyOS-Compositor: Wayland-Proxy, X11-Server in anyOS oder VNC/RDP.
3. **Hybrid 1c** — beide Optionen parallel, Nutzer waehlt nach Workflow.

Optionen 2 und 3 erfordern eine eigene grosse Architekturentscheidung
(Wayland vs. X11 vs. VNC, Compositor-Side-Implementierung, IME, DPI, Clipboard,
Audio). Diese Entscheidung ist nicht trivial und bindet Implementierungs- und
Wartungsaufwand fuer Monate.

Gleichzeitig liefert Option 1 schon den Kern-Workflow (Editieren, Bauen,
Debuggen, Run-Konfiguration mit Port-Forward) ohne die GUI-Bridge. Sie ist
auch identisch mit dem WSL2-Default-Modell von Microsoft, das in der Praxis
die ueberwaeltigende Mehrheit der WSL-Nutzer abdeckt.

## Decision

ASL liefert das Editor-Erlebnis in **zwei Stufen**:

### Stufe 1 (Erstauslieferung): Editor auf anyOS

- Primaerer Editor ist `anycode` auf anyOS.
- Quelltext liegt im Shared-Mount (ADR-0004), Distro sieht ihn unter
  `/mnt/<name>` o. ae.
- Build/Run/Test laeuft ueber `aslctl run --distro <name> --cwd <path> --env … --
  <cmd>` als Sub-Prozess des Editors.
- Stdout/Stderr und Exit-Code werden vom Editor ausgewertet (Build-Fehler-Liste,
  klickbare Zeilen).
- Language-Server (rust-analyzer, clangd, jdtls, typescript-language-server)
  laufen in der Distro, sprechen ueber Stdio mit dem Editor auf anyOS.

Diese Stufe ist Voraussetzung fuer "ASL ist nutzbar fuer Entwicklung" und
**nicht verhandelbar** fuer die Erstauslieferung.

### Stufe 2 (spaeter): Linux-GUI-Editor in der Distro

GUI-Forwarding wird als **separater Architekturblock** behandelt und
**nicht** Teil der Erstauslieferung. Erst nach stabilem Stufe-1-Workflow wird
darueber entschieden, welche Bridge-Technologie verwendet wird (eigener ADR).

Mogliche Pfade fuer Stufe 2 (zur Diskussion in einem zukuenftigen ADR):

- **Wayland-Proxy** — `WAYLAND_DISPLAY` in der Distro,
  Socket-Forwarding zum anyOS-Compositor. Mittlere Komplexitaet, modern, gut
  fuer GTK4/Qt6.
- **X11-Server in anyOS** — minimaler Xwayland-artiger Server, Distro-Apps
  nutzen `DISPLAY=:0`. Hoher anyOS-seitiger Aufwand, kompatibel mit aelterer
  Software.
- **VNC/RDP-artig** — Distro startet VNC-Server, anyOS hat einen Client.
  Pragmatisch, schwaechere UX (Latenz, Clipboard-Bruch, Copy-Paste).

## Consequences

### Positiv

- Klare Priorisierung: Block A-D des Implementierungsplans muss nur Stufe 1
  liefern.
- Stufe-1-Anforderungen sind in der bestehenden ASL-Architektur bereits
  weitgehend abgedeckt: Shared-Mount (ADR-0004), `aslctl run` (Block A4),
  TTY-Qualitaet (Block C2).
- Keine Bindung an eine GUI-Bridge-Technologie bevor der Kernworkflow steht.
- Kompatibel mit ADR-0008-Toolchains: alle vier Sprachen (Java, C/C++, Rust,
  Node.js) haben gut funktionierende Stdio-Language-Server.

### Negativ

- Native Linux-IDEs (JetBrains, VS Code) laufen in Stufe 1 nicht. Nutzer
  muessen mit `anycode` arbeiten oder mit Terminal-Editoren (Vim, Emacs in
  `aslctl shell`).
- Wenn `anycode` Lueken hat (kein Java-Support, kein Debugger fuer C++), fallen
  diese auf den Stufe-1-Workflow zurueck und mindern den Nutzen. Folge:
  `anycode` bekommt im Rahmen dieser Strategie indirekten Funktionsdruck.
- Stufe-2-Entscheidung wird hinausgezoegert. Das ist gewollt, kann aber
  Erwartungen wecken die spaeter enttaeuscht werden — daher dieser ADR statt
  einer impliziten Annahme.

## Alternatives Considered

- **Stufe 2 sofort mitliefern**: Verworfen wegen Aufwand
  (Compositor-seitiger Wayland-/X11-Stack, IME, Clipboard, DPI, Audio). Wuerde
  ASL um Monate verzoegern ohne dass Stufe 1 davon profitiert.
- **Nur Stufe 2** (kein anyOS-Editor-Pfad): Verworfen, weil dann der Kern-Loop
  Editieren/Bauen vom GUI-Forwarding abhaengt — ein Crash der Bridge legt den
  ganzen Workflow lahm. Stufe 1 bleibt auch dann sinnvoll wenn Stufe 2 spaeter
  existiert (z. B. fuer schnelle Edits, Build-Skripte, Headless-Betrieb).
- **VS Code Remote-artiges Modell** (Editor lokal, "Remote Server" in Distro):
  Architektonisch eine Variante von Stufe 1 und damit kompatibel. Nicht eigener
  Pfad, sondern eine moegliche spaetere Auspraegung des Stufe-1-Modells wenn
  `anycode` das anbietet.

## References

- ADR-0004 — Shared-Folder-Architektur (Voraussetzung fuer Stufe 1).
- ADR-0008 — Developer-Toolchains.
- `todos/asl-anyos-subsystem-linux.md` — Block C (Stufe 1) und Block F
  (Stufe 2, separater Brocken).
