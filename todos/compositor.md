# Sicherheitsaudit: Compositor & libanyui

## TEIL 1: Compositor (libcompositor / libcompositor_client)

### KRITISCH

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 1 | Integer Overflow | exports.rs:287 | `width * height * 4` SHM-Groesse ohne Overflow-Check. `w=0x10000, h=0x10000` -> Overflow zu 0, danach OOB-Zugriffe |
| 2 | Integer Overflow | exports.rs:547 | Gleiches Problem bei `export_resize_shm()` |
| 3 | Integer Overflow | lib.rs:95-97 | `VramWindow::put_pixel`: `y * stride + x` ohne Bounds-Check -> Schreibzugriff in fremde VRAM-Bereiche |
| 4 | Integer Overflow | lib.rs:90 | VRAM Surface Slice: `stride * height` kann ueberlaufen, erzeugt ungueltige Slice-Laenge |
| 5 | OOB Read | exports.rs:457-476 | `copy_nonoverlapping` bei Menu-Daten: Pointer und Laenge werden nicht validiert |
| 6 | OOB Read | exports.rs:791-806 | Notification-Daten: `title_ptr`, `icon_ptr` werden ohne Validierung kopiert. Icon immer 1024 Bytes |
| 7 | OOB Read | exports.rs:412-434 | Wallpaper-Pfad: `copy_nonoverlapping` mit unvalidiertem Pointer |
| 8 | OOB Read | exports.rs:595-613 | Clipboard-Daten: gleiches Problem |
| 9 | Access Control | exports.rs:622-676 | **Clipboard-Lesen ohne jede Permission-Pruefung** -- jede App kann silent Passwoerter/Keys lesen |
| 10 | Access Control | exports.rs (gesamt) | **Keinerlei Capability-Pruefung** bei IPC-Commands -- jede App darf alles |

### HOCH

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 11 | Window Spoofing | exports.rs:342-451 | Window-IDs werden nicht per App validiert -- App B kann `move_window()`, `destroy_window()`, `minimize_window()` auf Fenster von App A ausfuehren |
| 12 | Bounds Check | exports.rs:356 | `present_rect` validiert nicht ob x/y/w/h innerhalb der Fenstergrenzen liegen |
| 13 | Bounds Check | exports.rs:300-315 | Fenster-Dimensionen werden nicht gecappt -- extreme Werte moeglich |
| 14 | DoS | exports.rs:284-340 | Unbegrenzte Fenster-/SHM-Erstellung -> OOM |
| 15 | DoS | exports.rs:746-820 | Unbegrenztes Notification-Spamming |
| 16 | VRAM Bounds | lib.rs:71-99 | `put_pixel(x, y)` ohne Bounds-Check -- bei `x >= width` wird in den VRAM anderer Fenster geschrieben |
| 17 | Fullscreen | exports.rs:880-903 | Jede App kann Fullscreen anfordern -> Phishing-Lockscreen moeglich, plus `want_direct_fb` fuer GPU-Framebuffer-Zugriff |

### MITTEL

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 18 | Race Condition | exports.rs (mehrfach) | SHM wird nach `sleep(32)` zerstoert -- Compositor koennte noch lesen (Use-After-Free bei hoher Last) |
| 19 | Buffer Overflow | exports.rs:395-404 | Titel-Packing liest bis 12 Bytes von unvalidiertem Pointer |
| 20 | Silent Truncation | lib.rs:700-712 | MenuBuilder truncated Daten ohne Fehler -> malformierte Menu-Daten an Compositor |
| 21 | Clipboard | exports.rs:595-620 | Kein Tracking welche App Clipboard gesetzt hat -> Malicious-Content-Injection |

---

## TEIL 2: libanyui -- Sicherheit

### KRITISCH

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 22 | Type Confusion | lib.rs (mehrfach) | `anyui_get_textfield(id)` castet `raw as *mut TextField` **ohne Kind-Validierung**. Falsche ID -> Memory Corruption |
| 23 | App-Isolation | lib.rs | Alle Controls in globalem `AnyuiState.controls` -- Control-IDs sind einfache u32-Indizes, keine App-Namespaces. App B kann Controls von App A manipulieren |
| 24 | Resource Limit | lib.rs | Kein Limit auf Control-Anzahl pro App -> OOM-DoS |

### HOCH

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 25 | Integer Overflow | canvas.rs:28 | `base.w * base.h` ohne Overflow-Check -> falsche Allokations-Groesse |
| 26 | Unbounded Alloc | textfield.rs:379 | Kein `max_length` -- Clipboard-Paste kann unbegrenzt Heap allokieren |
| 27 | Unbounded Alloc | textarea.rs:176 | Gleiches Problem bei TextArea |
| 28 | Unsafe Cast | draw.rs:143-201 | Manuelles ELF-Parsing fuer LibRender-Symbole ohne Bounds-Checks |
| 29 | Buffer Overflow | lib.rs:399 | `copy_nonoverlapping(title, buf, len)` -- `len` kommt vom User, Buffer ist 128 Bytes, kein Check |

### MITTEL

| # | Typ | Datei | Problem |
|---|-----|-------|---------|
| 30 | UTF-8 | textfield.rs:374 | Paste-Filter ist ASCII-only (`b >= 0x20 && b < 0x7F`) -- internationale Zeichen werden silent gedroppt |
| 31 | UAF-Risiko | event_loop.rs | Callback koennte Control loeschen waehrend Event-Dispatch -> nachfolgende Renders auf gefreitem Speicher |
| 32 | Overflow | textarea.rs:32 | `line_count * line_height` kann bei grossem Text ueberlaufen |

---

## TEIL 3: Fehlende Control-Funktionalitaet

### TextField
| Feature | Status |
|---------|--------|
| Cursor-Position get/set (public API) | **Fehlt** |
| Selection Range get/set | **Fehlt** (nur `select_all()`) |
| Read-only Modus | **Fehlt** |
| Max Length | **Fehlt** (Sicherheitsproblem!) |
| Undo/Redo | **Fehlt** |
| Password-Modus | Vorhanden |
| Placeholder | Vorhanden |
| Copy/Paste | Vorhanden |
| Wort-Navigation | Vorhanden |

### TextArea
| Feature | Status |
|---------|--------|
| Cursor-Position get/set | **Fehlt** (intern vorhanden, kein public API) |
| Scroll-Position setzen | Nur `scroll_to_bottom()` |
| Selection | **Fehlt komplett** |
| Read-only Modus | **Fehlt** |
| Zeilennummern | **Fehlt** |
| Word Wrap | **Fehlt** |
| Undo/Redo | **Fehlt** |
| Max Length/Lines | **Fehlt** |

### ListView / TableView
| Feature | Status |
|---------|--------|
| Scroll-to-Item | **Fehlt** |
| Multi-Selection | **Fehlt** |
| Drag & Drop | **Fehlt** |
| Virtual Scrolling | **Fehlt** (rendert alle Rows) |
| Sort-Callbacks | **Fehlt** |
| Spalten-Resize | **Fehlt** |

### DataGrid
| Feature | Status |
|---------|--------|
| Cell Editing | **Fehlt** |
| Filtering | **Fehlt** |
| Spalten einfrieren | **Fehlt** |
| Export | **Fehlt** |
| Sort (intern) | Vorhanden (sort_column/sort_direction) aber kein public API |
| Spalten-Resize/Reorder | Vorhanden |
| Per-Cell Colors/Icons | Vorhanden |

### ComboBox / DropDown
| Feature | Status |
|---------|--------|
| Editable Modus (Eingabe) | **Fehlt** |
| Auto-Complete/Filtering | **Fehlt** (separates AutoCompleteTextField existiert) |
| Custom Item Rendering | **Fehlt** |
| Item-Limit | **Fehlt** |

### TabControl
| Feature | Status |
|---------|--------|
| Tab schliessen | Vorhanden |
| Tab Reorder (Drag) | **Fehlt** |
| Tab Overflow/Scroll | Vorhanden |
| Dynamisches Tab erstellen (API) | **Fehlt** |

### Menu / ContextMenu
| Feature | Status |
|---------|--------|
| Submenues | **Fehlt** |
| Keyboard-Navigation | **Fehlt** |
| Icons | **Fehlt** |
| Checkmarks | **Fehlt** |
| Disabled Items | **Fehlt** |
| Keyboard Shortcuts / Accelerators | **Fehlt** |

### Slider
| Feature | Status |
|---------|--------|
| Konfigurierbarer Range (min/max) | **Fehlt** (hardcoded 0-100) |
| Step Size | **Fehlt** (feste 5er-Schritte) |
| Vertikale Orientierung | **Fehlt** |
| Tick Marks | **Fehlt** |
| Wert-Anzeige/Label | **Fehlt** |

### Canvas
| Feature | Status |
|---------|--------|
| Hit Testing | **Fehlt** |
| Layer-System | **Fehlt** |
| Redraw Callback | **Fehlt** |
| Clipping Paths | **Fehlt** (nur Rechteck) |

### TreeView
| Feature | Status |
|---------|--------|
| Keyboard-Navigation | **Fehlt** |
| Drag & Drop | **Fehlt** |
| Node-Limit | **Fehlt** (unbounded) |
| Expand/Collapse, Icons, Selection | Vorhanden |

### ProgressBar
| Feature | Status |
|---------|--------|
| Indeterminate Mode | **Fehlt** |
| Text-Overlay | **Fehlt** |

### Toolbar
| Feature | Status |
|---------|--------|
| Overflow-Menu | **Fehlt** |
| Separator | **Fehlt** |
| Toggle/Dropdown Buttons | **Fehlt** |

### Window
| Feature | Status |
|---------|--------|
| Min/Max Size Constraints | **Fehlt** |
| Always-on-Top | **Fehlt** |
| Opacity | **Fehlt** |

### Label
| Feature | Status |
|---------|--------|
| Auto Word-Wrap | **Fehlt** (nur manuelles `\n`) |
| Ellipsis bei Overflow | **Fehlt** |

### TextEditor (Advanced)
| Feature | Status |
|---------|--------|
| Syntax Highlighting, Line Numbers, Undo/Redo, Selection | Vorhanden |
| Find/Replace | **Fehlt** |
| Code Folding | **Fehlt** |
| Goto Line | **Fehlt** |
| Bracket Matching | **Fehlt** |

---

## Empfohlene Fix-Reihenfolge

**Sofort (Sicherheitskritisch):**
1. Checked Multiplication bei allen Buffer-Groessen-Berechnungen (#1, #2, #3, #4, #25)
2. Window-ID Ownership-Validierung (#11) -- App darf nur eigene Fenster manipulieren
3. Control-ID Kind-Validierung vor unsafe Cast (#22) -- `ControlKind` Enum pruefen
4. Bounds-Check bei `put_pixel()` (#16) und `present_rect()` (#12)
5. Pointer/Laenge-Validierung bei allen `copy_nonoverlapping` (#5-8, #29)
6. Clipboard: Permission-Dialog oder Capability-Check (#9)

**Hoch (Stabilitaet):**
7. Per-App Resource-Limits: max Controls, max Windows, max SHM (#14, #24)
8. `max_length` fuer TextField und TextArea (#26, #27)
9. App-Isolation: Control-IDs per App namespaced (#23)
10. SHM-Lifetime mit Synchronisation statt `sleep(32)` (#18)

**Mittel (Funktionalitaet):**
11. TextField/TextArea: public Cursor-Position, Selection-API, Read-only
12. Menu: Submenues, Keyboard-Navigation, Icons, Disabled Items
13. Slider: konfigurierbarer Range/Steps
14. Label: Auto Word-Wrap, Ellipsis
