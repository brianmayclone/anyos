# surf-host

Standalone Linux-Build der anyOS Surf Rendering-Engine. Rendert Webseiten mit der gleichen HTML/CSS/JS-Pipeline wie der Surf-Browser in anyOS — ohne dass das Betriebssystem laufen muss.

## Features

- **Identische Rendering-Pipeline** wie anyOS Surf (libwebview, libfont, libjs)
- **HTTP/HTTPS** mit TLS (via ureq + rustls)
- **Bild-Dekodierung**: PNG, JPEG, GIF, BMP, ICO (via image Crate), SVG (via resvg)
- **Font-Rendering**: Eingebettete SF Pro Fonts mit Greyscale-Antialiasing
- **JavaScript-Ausfuehrung**: Vollstaendige JS-Engine (libjs)
- **CSS**: Cascade, Selectors, Flexbox, Grid, Table-Layout
- **DOM-basierte Ressourcenerkennung**: CSS, Bilder, @font-face, @import — identisch zu anyOS Surf
- **Screenshot-Modus**: Headless Screenshots (Viewport, Full-Page, Y-Range)
- **Interaktiver Modus**: egui-Fenster mit URL-Leiste, Mausrad-Scrolling und Screenshot-Button
- **Fernsteuerung fuer Hosttests**: localhost-Control-Port mit `open`, `reload`, `scroll`, `screenshot`, `fullpage`, `status`

## Voraussetzungen

- Linux x86_64
- Rust stable Toolchain (`rustup toolchain install stable`)
- X11 oder Wayland (fuer den interaktiven Fenster-Modus)

## Build

```bash
cd tools/surf-host
./build.sh
```

Das Binary wird unter `target/x86_64-unknown-linux-gnu/release/surf-host` erstellt.

Wichtig: Wird mit `cargo +stable` gebaut. Die anyOS-spezifischen `build-std`-Flags werden im Repository nur noch an den echten OS-Buildstellen gesetzt, nicht mehr global fuer alle Cargo-Aufrufe.

## Benutzung

### Interaktiver Modus (Fenster)

```bash
# Standard-Viewport (1024x768), egui ist der Standard
./build.sh run https://www.wikipedia.de

# Benutzerdefinierter Viewport
./build.sh run https://www.wikipedia.de 1280x960

# Lokale HTML-Datei
./build.sh run file:///tmp/test.html

# Ohne Start-URL oeffnet Surf Host eine lokale about:blank-Startseite
cargo +stable run --release

# Legacy-Fenster ohne egui
./build.sh run https://www.wikipedia.de --minifb
```

**Fensterbedienung:**

- URL in die Leiste eingeben und Enter oder `Go` druecken
- `Reload` laedt die aktuelle Seite neu
- Mausrad scrollt den gerenderten WebView
- `Shot` speichert den aktuellen Viewport als `screenshot_1.png`, `screenshot_2.png`, ...
- Links und einfache Formulare werden an die libwebview-Hit-Tests weitergereicht

Der alte minifb-Modus bleibt mit `--minifb` verfuegbar. Dort gelten weiterhin F5/F6/Esc.

### Fernsteuerung

Im egui-Modus lauscht surf-host standardmaessig auf `127.0.0.1:8787`. Hosttests koennen pro TCP-Verbindung genau einen Textbefehl senden:

```bash
printf 'open file:///tmp/test.html\n' | nc 127.0.0.1 8787
printf 'scroll 800\n' | nc 127.0.0.1 8787
printf 'screenshot /tmp/surf.png\n' | nc 127.0.0.1 8787
printf 'status\n' | nc 127.0.0.1 8787
```

Befehle:

| Befehl             | Wirkung                                  |
|--------------------|-------------------------------------------|
| `open <url>`       | Navigiert zur URL                         |
| `reload`           | Laedt die aktuelle Seite neu              |
| `scroll <y>`       | Setzt den Dokument-Scrolloffset           |
| `screenshot <png>` | Speichert den aktuellen Viewport          |
| `fullpage <png>`   | Speichert die gesamte Dokumenthoehe       |
| `status`           | Gibt URL, Viewport und Dokumenthoehe aus  |

Mit `--remote-listen <addr>` kann der Port geaendert werden, mit `--no-remote` wird er abgeschaltet.

### Screenshot-Modus (Headless)

```bash
# Viewport-Screenshot (Standard: 1024x768)
./build.sh screenshot https://example.com

# Mit Dateiname
./build.sh screenshot https://example.com wiki.png

# Full-Page (gesamte Seitenhoehe)
./build.sh screenshot https://example.com wiki.png full

# Benutzerdefinierter Viewport
./build.sh screenshot https://example.com 1280x960

# Y-Range (vertikaler Ausschnitt)
./build.sh screenshot https://example.com 400-900 crop.png

# Kombiniert: Full-Page, 1280px breit, 3 Sekunden Wartezeit
./build.sh screenshot https://example.com out.png full 1280x960 3000
```

**Screenshot-Optionen (beliebige Reihenfolge):**

| Option       | Beschreibung                                  |
|--------------|-----------------------------------------------|
| `out.png`    | Ausgabedatei (Default: `screenshot.png`)       |
| `1280x960`   | Viewport-Groesse                              |
| `full`       | Gesamte Seitenhoehe statt nur Viewport         |
| `400-900`    | Y-Bereich in Pixeln (vertikaler Ausschnitt)    |
| `3000`       | Wartezeit in ms vor dem Screenshot             |

### Direkter Aufruf (ohne build.sh)

```bash
cargo +stable build --release
./target/x86_64-unknown-linux-gnu/release/surf-host <url> [optionen]
```

**CLI-Optionen:**

```
surf-host [url] [options]

  --screenshot <pfad.png>   Screenshot speichern und beenden
  --fullpage                Gesamte Seitenhoehe erfassen
  -y <start-end>            Y-Bereich, z.B. -y 400-900
  --delay <ms>              Wartezeit vor Screenshot
  --width <px>              Viewport-Breite (Default: 1024)
  --height <px>             Viewport-Hoehe (Default: 768)
  --no-js                   JavaScript-Ausfuehrung deaktivieren
  --minifb                  Legacy-minifb-Fenster statt egui
  --remote-listen <addr>    Fernsteuer-Port (Default: 127.0.0.1:8787)
  --no-remote               Fernsteuer-Port deaktivieren
```

## Architektur

surf-host nutzt die gleichen Rendering-Libraries wie anyOS Surf, kompiliert fuer den Linux-Host mittels `host` Feature-Flags:

```
surf-host (Linux Binary)
  |
  +-- libwebview     HTML/CSS/JS Rendering-Engine (identischer Code)
  |     +-- libfont  TTF Font-Engine (eingebettete System-Fonts)
  |     +-- libjs    JavaScript-Engine
  |     +-- libanyui_client [host-stubs]
  |           Canvas -> Vec<u32> Pixel-Buffer
  |           ScrollView, TextField, etc. -> No-ops
  |
  +-- ureq + rustls  HTTP/HTTPS mit TLS (ersetzt anyOS TCP-Stack)
  +-- image + resvg  Bild-Dekodierung (ersetzt anyOS libimage/libsvg)
  +-- minifb         Fenster-Anzeige (ersetzt anyOS Compositor)
```

### Host Feature-Flags

Folgende Crates haben ein `host` Feature das die anyOS-spezifischen Teile durch Linux-Aequivalente ersetzt:

| Crate              | anyOS-Modus                    | Host-Modus                         |
|--------------------|--------------------------------|-------------------------------------|
| `anyos_std`        | Eigene Syscalls, Heap, I/O     | std::fs, std::net, System-Allocator |
| `libheap`          | Bump-Allocator (sbrk/mmap)     | No-op (System-Allocator)            |
| `dynlink`          | ELF64 dl_open/dl_sym           | Stubs (kein DLL-Loading)            |
| `libfont`          | anyOS Syscalls fuer File-I/O   | std::fs, kein GPU-Accel             |
| `libfont_client`   | DLL-Binding via dl_sym         | extern "C" direkter Link            |
| `libanyui_client`  | Compositor IPC, echte Controls | Canvas=Vec<u32>, Rest=Stubs         |
| `libjs`            | (kein Unterschied)             | (kein Unterschied)                  |
| `libwebview`       | (propagiert Features)          | (propagiert Features)               |

### Unterschiede zu anyOS Surf

| Aspekt             | anyOS Surf                     | surf-host                          |
|--------------------|--------------------------------|-------------------------------------|
| Font-Smoothing     | Subpixel LCD (Compositor)      | Greyscale AA                        |
| Formular-Controls  | Echte TextField/Checkbox/Radio | Stubs (unsichtbar)                  |
| Bild-Dekodierung   | libimage (eigener Decoder)     | image + resvg Crates                |
| HTTP/TLS           | anyOS TCP-Stack + BearSSL      | ureq + rustls                       |
| Fenster-System     | anyOS Compositor               | minifb (X11/Wayland)                |

Die Rendering-Pipeline (HTML-Parsing, CSS-Cascade, Layout, Font-Rasterisierung, Display-List, Tile-Compositing) ist **identisch**.

### Webfonts und Fallbacks

surf-host laedt `@font-face` Ressourcen aus Stylesheets, registriert aber nur
Fontdaten, die libfont sicher parsen kann. WOFF und WOFF2 werden derzeit bewusst
nicht registriert, weil der vorhandene WOFF2-Konverter bei realen Webfonts noch
unvollstaendige oder kaputte Glyphdaten liefern kann. Ein fehlgeschlagener
`font_load_data`-Aufruf ist deshalb kein harter Renderfehler: die CSS-Family
bleibt unregistriert und libwebview faellt auf die naechste Family aus
`font-family` bzw. auf System-Aliase wie `sans-serif`, `serif`, `monospace`,
Arial, Helvetica, Inter oder Source Sans Pro zurueck.

Das ist die aktuelle Hosttest-Strategie fuer reale Webseiten ohne JavaScript:
lesbarer Text ueber robuste Fallbacks zuerst, vollstaendiger WOFF2-Support
spaeter mit eigener Validierung.
