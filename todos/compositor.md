# Sicherheitsaudit: Compositor & libanyui

Status-Update: 2026-04-23. Stand nach Verifikation gegen aktuellen Code.

Zusammenfassung: von den 32 urspruenglichen Sicherheits-Issues sind **14 FIXED**,
**4 PARTIALLY FIXED**, **14 STILL OPEN**. Die kritischsten offenen Punkte sind
Window-ID-Ownership, Capability-Checks und Clipboard-Permission.

## TEIL 1: Compositor (libcompositor / libcompositor_client)

### KRITISCH

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 1 | Integer Overflow | ✅ FIXED | exports.rs:~294 | `checked_mul` bei SHM-Groesse (width * height * 4) |
| 2 | Integer Overflow | ✅ FIXED | exports.rs:~569 | `checked_mul` auch bei `export_resize_shm()` |
| 3 | Integer Overflow | ✅ FIXED | canvas.rs:56 | `put_pixel` hat Bounds-Check |
| 4 | Integer Overflow | ✅ FIXED | canvas.rs:28-29 | VRAM Surface Slice nutzt `saturating_mul` |
| 5 | OOB Read | ❌ OFFEN | exports.rs:~493 | Menu-Daten: `copy_nonoverlapping` ohne Pointer-Validierung (nur 4096-Byte-Limit) |
| 6 | OOB Read | ⚠️ TEILWEISE | exports.rs:~823 | Notification title/msg begrenzt (64/128 B), aber `icon_ptr` weiterhin 1024 B ohne Validierung |
| 7 | OOB Read | ✅ FIXED | exports.rs:~430 | Wallpaper-Pfad: `path_len` validiert (max 255 B) |
| 8 | OOB Read | ✅ FIXED | exports.rs:~622 | Clipboard-Daten: `data_len` validiert (max 65536 B) |
| 9 | Access Control | ❌ OFFEN | exports.rs:~649 | **Clipboard-Lesen weiterhin ohne Permission-Check** -- jede App liest silent Passwoerter/Keys |
| 10 | Access Control | ❌ OFFEN | exports.rs (gesamt) | **Keine Capability-Pruefung** bei IPC-Commands |

### HOCH

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 11 | Window Spoofing | ❌ OFFEN | exports.rs:~356,465,877 | App kann `move/destroy/minimize_window()` mit fremder `window_id` aufrufen; `tid` wird gesendet aber nicht validiert |
| 12 | Bounds Check | ✅ FIXED | exports.rs:~372-376 | `present_rect` Bounds auf u16-Range geclamped |
| 13 | Bounds Check | ⚠️ TEILWEISE | exports.rs:~290 | `MAX_WINDOW_DIM=16384` bei `create_window`, aber VRAM-Windows haben noch kein Limit |
| 14 | DoS | ✅ FIXED | exports.rs | `MAX_WINDOW_DIM` Limit aktiv |
| 15 | DoS | ❌ OFFEN | exports.rs | Kein Rate-Limiting bei Notifications -- unbegrenztes Spamming moeglich |
| 16 | VRAM Bounds | ✅ FIXED | canvas.rs:56 | `put_pixel(x, y)` hat Bounds-Check |
| 17 | Fullscreen | ❌ OFFEN | exports.rs:~904 | `request_fullscreen` akzeptiert `want_direct_fb` ohne Capability-Check |

### MITTEL

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 18 | Race Condition | ⚠️ TEILWEISE | exports.rs:~458,502,535 | `sleep(32)` weiterhin genutzt, aber nur fuer unkritische Wallpaper/Menu/Icon-Uebergaben |
| 19 | Buffer Overflow | ✅ FIXED | exports.rs:~416 | Titel-Packing `title_len.min(12)` |
| 20 | Silent Truncation | ✅ FIXED | exports.rs:~475 | MenuBuilder: `menu_len.min(4096)` explizit |
| 21 | Clipboard | ❌ OFFEN | exports.rs | Weiterhin kein App-Tracking des Clipboard-Setters |

---

## TEIL 2: libanyui -- Sicherheit

### KRITISCH

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 22 | Type Confusion | ❌ OFFEN | lib.rs | `anyui_get_textfield(id)` castet weiterhin `raw as *mut TextField` ohne Kind-Validierung |
| 23 | App-Isolation | ❌ OFFEN | lib.rs | Control-IDs weiterhin global u32-Indizes ohne App-Namespace |
| 24 | Resource Limit | ❌ OFFEN | lib.rs | Kein Limit auf Control-Anzahl pro App |

### HOCH

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 25 | Integer Overflow | ✅ FIXED | canvas.rs:28-30 | `saturating_mul` mit Limit 16384x16384 |
| 26 | Unbounded Alloc | ✅ FIXED | textfield.rs:~497 | `max_length` beim Paste durchgesetzt |
| 27 | Unbounded Alloc | ✅ FIXED | textarea.rs:12 | `max_length` vorhanden |
| 28 | Unsafe Cast | ❌ OFFEN | draw.rs:~165 | ELF-Magic geprueft, aber Array-Zugriffe ohne Groessen-Check |
| 29 | Buffer Overflow | ✅ FIXED | lib.rs:~456-460 | `title_len.min(63)` mit 64-Byte Buffer |

### MITTEL

| # | Typ | Status | Datei | Problem |
|---|-----|--------|-------|---------|
| 30 | UTF-8 | ✅ FIXED | textfield.rs:~491 | Paste-Filter behaelt 0x80+ (UTF-8 Continuation) |
| 31 | UAF-Risiko | ❌ OFFEN | event_loop.rs | PendingCallback-Queue kann Control waehrend Dispatch loeschen |
| 32 | Overflow | ✅ FIXED | textarea.rs:50 | `saturating_mul` fuer line_count * line_height |

---

## TEIL 3: Fehlende Control-Funktionalitaet

### TextField
| Feature | Status |
|---------|--------|
| Cursor-Position get/set (public API) | ✅ Vorhanden |
| Selection Range get/set | ✅ Vorhanden |
| Read-only Modus | ✅ Vorhanden |
| Max Length | ✅ Vorhanden |
| Undo/Redo | ❌ Fehlt |
| Password-Modus, Placeholder, Copy/Paste, Wort-Navigation | ✅ Vorhanden |

### TextArea
| Feature | Status |
|---------|--------|
| Cursor-Position get/set | ✅ Vorhanden |
| Scroll-Position setzen | ✅ Vorhanden |
| Selection | ✅ Vorhanden |
| Read-only Modus | ✅ Vorhanden |
| Max Length | ✅ Vorhanden |
| Zeilennummern | ❌ Fehlt (in TextEditor vorhanden) |
| Word Wrap | ❌ Fehlt |
| Undo/Redo | ❌ Fehlt (in TextEditor vorhanden) |

### TextEditor (Advanced)
| Feature | Status |
|---------|--------|
| Syntax Highlighting, Line Numbers, Undo/Redo, Selection | ✅ Vorhanden |
| Find/Replace | ❌ Fehlt |
| Code Folding | ❌ Fehlt |
| Goto Line | ❌ Fehlt |
| Bracket Matching | ❌ Fehlt |

### ListView / TableView
| Feature | Status |
|---------|--------|
| Scroll-to-Item, Multi-Selection, Drag & Drop, Virtual Scrolling, Sort-Callbacks, Spalten-Resize | ❌ Fehlt |

### DataGrid
| Feature | Status |
|---------|--------|
| Cell Editing, Filtering, Spalten einfrieren, Export | ❌ Fehlt |
| Sort (public API) | ❌ Fehlt (intern vorhanden) |
| Spalten-Resize/Reorder, Per-Cell Colors/Icons | ✅ Vorhanden |

### ComboBox / DropDown
| Feature | Status |
|---------|--------|
| Editable Modus, Auto-Complete/Filtering, Custom Item Rendering, Item-Limit | ❌ Fehlt |

### TabControl
| Feature | Status |
|---------|--------|
| Tab schliessen, Tab Overflow/Scroll | ✅ Vorhanden |
| Tab Reorder (Drag), Dynamisches Tab API | ❌ Fehlt |

### Menu / ContextMenu
| Feature | Status |
|---------|--------|
| Submenues, Keyboard-Navigation, Icons, Checkmarks, Disabled Items, Accelerators | ❌ Fehlt |

### Slider
| Feature | Status |
|---------|--------|
| Konfigurierbarer Range (min/max), Step Size, Vertikal, Tick Marks, Wert-Label | ❌ Fehlt (hardcoded 0-100, 5er-Schritte) |

### Canvas
| Feature | Status |
|---------|--------|
| Hit Testing, Layer-System, Redraw Callback, Clipping Paths | ❌ Fehlt |

### TreeView
| Feature | Status |
|---------|--------|
| Keyboard-Navigation, Drag & Drop, Node-Limit | ❌ Fehlt |
| Expand/Collapse, Icons, Selection | ✅ Vorhanden |

### ProgressBar
| Feature | Status |
|---------|--------|
| Indeterminate Mode, Text-Overlay | ❌ Fehlt |

### Toolbar
| Feature | Status |
|---------|--------|
| Overflow-Menu, Separator, Toggle/Dropdown Buttons | ❌ Fehlt |

### Window
| Feature | Status |
|---------|--------|
| Min/Max Size Constraints, Always-on-Top, Opacity | ❌ Fehlt |

### Label
| Feature | Status |
|---------|--------|
| Auto Word-Wrap, Ellipsis bei Overflow | ❌ Fehlt |

---

## Empfohlene naechste Schritte

**Sicherheitskritisch (noch offen):**
1. **Window-ID Ownership-Validierung (#11)** -- App darf nur eigene Fenster manipulieren; `tid`-Check beim Compositor durchsetzen
2. **Capability-Check bei IPC-Commands (#10)** -- insbesondere Clipboard, Fullscreen, Notifications
3. **Clipboard-Permission (#9, #21)** -- App-Tracking + Permission-Dialog fuer Read-Access
4. **Control-ID Kind-Validierung (#22)** -- `ControlKind` Enum vor jedem unsafe Cast pruefen
5. **App-Isolation Control-IDs (#23)** -- Controls per App namespacen
6. **Menu-Daten Pointer-Validierung (#5)** -- auch bei 4096-Byte-Limit Pointer-Laenge checken
7. **Notification Icon-Pointer (#6)** -- 1024-Byte Icon-Kopie ohne Pointer-Validierung absichern
8. **ELF-Parsing in draw.rs (#28)** -- Bounds-Checks beim Symbol-Table-Zugriff
9. **Fullscreen Capability (#17)** -- `want_direct_fb` nur mit Capability erlauben
10. **Notification Rate-Limiting (#15)** -- Spamming verhindern
11. **event_loop UAF (#31)** -- Control-Lifetime waehrend Callback-Dispatch absichern
12. **Per-App Resource-Limits (#24)** -- max Controls/Windows/SHM pro App

**Feature-Luecken (Prio nach Nutzen):**
- TextField/TextArea: Undo/Redo
- TextEditor: Find/Replace, Goto Line, Bracket Matching
- Menu: Submenues, Keyboard-Nav, Icons, Disabled Items, Accelerators
- Slider: konfigurierbarer Range/Steps, vertikal
- Label: Auto Word-Wrap, Ellipsis
- Window: Min/Max Size, Always-on-Top
- ListView/DataGrid: Multi-Selection, Virtual Scrolling, Cell Editing
- ProgressBar: Indeterminate, Text-Overlay
- Toolbar: Separator, Overflow-Menu
