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
- **Interaktiver Modus**: minifb-Fenster mit Mausrad-Scrolling

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

Wichtig: Muss mit `cargo +stable` gebaut werden (nicht nightly), da die anyOS-Root `.cargo/config.toml` das nightly-only `build-std` Feature aktiviert, das fuer den Host-Build nicht benoetigt wird.

## Benutzung

### Interaktiver Modus (Fenster)

```bash
# Standard-Viewport (1024x768)
./build.sh run https://www.wikipedia.de

# Benutzerdefinierter Viewport
./build.sh run https://www.wikipedia.de 1280x960

# Lokale HTML-Datei
./build.sh run file:///tmp/test.html
```

**Tastenbelegung im Fenster:**

| Taste     | Funktion                          |
|-----------|-----------------------------------|
| Mausrad   | Scrollen                          |
| F5        | Screenshot (aktueller Viewport)   |
| F6        | Full-Page Screenshot (ganze Seite)|
| Esc       | Beenden                           |

Screenshots werden als `screenshot_1.png`, `screenshot_2.png`, ... im aktuellen Verzeichnis gespeichert.

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
surf-host <url> [options]

  --screenshot <pfad.png>   Screenshot speichern und beenden
  --fullpage                Gesamte Seitenhoehe erfassen
  -y <start-end>            Y-Bereich, z.B. -y 400-900
  --delay <ms>              Wartezeit vor Screenshot
  --width <px>              Viewport-Breite (Default: 1024)
  --height <px>             Viewport-Hoehe (Default: 768)
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
