# anyOS Architecture Refactoring

Ergebnisse der Tiefenanalyse von Kernel, Compositor, anyui, Libraries und Apps.

Status-Update 2026-04-23 nach Code-Verifikation: **7 DONE, 4 PARTIAL, 8 OPEN**
von 19 Tasks.

---

## Gesamtbewertung

| Bereich | Score | Kritischstes Problem |
|---------|-------|---------------------|
| **Kernel** | 7/10 | ~~`schedule_inner()` Monolith~~ ✅ -- scheduler in 11+ Submodule aufgeteilt |
| **Compositor** | 8/10 | `Desktop` struct immer noch ~60 Felder (impl-Split vorhanden, Struct-Split nicht) |
| **anyui Framework** | 7/10 | 20+ identische Render-Boilerplate-Bloecke |
| **Shared Libraries** | 6/10 | 4x duplizierte DEFLATE-Implementierung (~1800 LOC) |
| **Apps** | 6/10 | Boilerplate-Duplication (`static mut APP` Pattern in 17 Apps) |

---

## Prio 1 — Hoher Impact, Architektur-Verbesserung

### 1.1 ✅ DONE -- `schedule_inner()` aufgeteilt
- **Datei:** `kernel/src/task/scheduler/` (ehemals `scheduler.rs`)
- **Status:** Scheduler ist in 11+ Submodule aufgeteilt: `run_queue.rs`,
  `lifecycle.rs`, `priority.rs`, `spawn.rs`, `fork.rs`, `signals.rs`,
  `wait.rs`, `deferred.rs`, `diagnostics.rs`, `debug_trace.rs`, `fd_table.rs`,
  `perm.rs`, `fpu.rs`. Funktionen wie `reap_terminated()`, `pick_next()`
  existieren separat.

### 1.2 ⚠️ PARTIAL -- `Desktop` God Object
- **Datei:** `bin/compositor/src/desktop/mod.rs` (44 KB, ~60 Felder)
- **Status:** impl-Split via Submodule vorhanden
  (`desktop/{window,input,ipc,drawing,theme,cursors,volume_hud}.rs`), aber
  der `Desktop` struct selbst wurde nicht in Sub-Structs aufgebrochen. Alle
  Felder weiterhin direkt im monolithischen struct.
- **Noch zu tun:** Struct-Split in `WindowManager`, `InputState`, `UiChrome`,
  `AppProtocol`, `DesktopLifecycle`.
- **Aufwand:** 2 Tage

### 1.3 ❌ OPEN -- `libcompress` extrahieren (DEFLATE/CRC32)
- **Problem:** 4 unabhaengige DEFLATE-Implementierungen mit ~1800 LOC total:
  - `libs/libzip/src/deflate.rs` + `inflate.rs` (612 LOC)
  - `libs/libhttp/src/deflate.rs` (487 LOC)
  - `libs/libimage/src/deflate.rs` (442 LOC)
  - `libs/libfont/src/inflate.rs` (251 LOC)
- **Loesung:** Neue `libcompress` staticlib mit `inflate()`, `deflate()`,
  `crc32()` -- alle Server-Libs linken statisch
- **Aufwand:** 2 Tage

### 1.4 ❌ OPEN -- `RenderContext` Helper fuer anyui Controls
- **Problem:** 29+ Controls wiederholen identischen Render-Setup (20+ Zeilen
  Boilerplate pro Control)
- **Loesung:**
  ```rust
  pub struct RenderContext {
      pub x: i32, pub y: i32, pub w: u32, pub h: u32,
      pub disabled: bool, pub hovered: bool, pub focused: bool,
  }
  pub fn prepare_render(base: &ControlBase, ax: i32, ay: i32) -> RenderContext
  ```
- **Aufwand:** 1 Tag

---

## Prio 2 — Code-Hygiene & Konsistenz

### 2.1 ⚠️ PARTIAL -- Syscall-Wrapper in libheap konsolidieren
- **Status:** `libfont` nutzt `#[path = "host_syscall.rs"]` conditional
  Compilation, aber keine zentrale libheap-basierte Syscall-Wrapper-Library.
  Andere Server-Libs duplizieren weiterhin inline asm.
- **Aufwand:** 0.5 Tage

### 2.2 ⚠️ PARTIAL -- VFS aufteilen
- **Datei:** `kernel/src/fs/vfs/mod.rs` (3521 Zeilen)
- **Status:** Teilweise aufgeteilt -- `vfs/{path.rs, types.rs, cache.rs}`
  existieren. Fehlt: `mount.rs` und `file_ops.rs` (beide weiterhin inline in
  `vfs/mod.rs`).
- **Aufwand:** 1 Tag (restlicher Split)

### 2.3 ✅ DONE -- `anyos_std::fmt` + `anyos_std::path`
- Beide Module existieren: `libs/stdlib/src/fmt.rs` und
  `libs/stdlib/src/path.rs`.

### 2.4 ❌ OPEN -- Einheitliches Error-Handling
- **Kernel:** Kein einheitliches `KernelError` Enum -- nur `FsError` in
  `kernel/src/fs/vfs/types.rs`, Rest ist Mix aus `Option<>` und Panics.
- **Compositor:** Logging fuer IPC/SHM/File-Fehler weiterhin ad-hoc.
- **Libraries:** Kein einheitliches `DllError`.
- **Aufwand:** 2 Tage

### 2.5 ❌ OPEN -- Loader aufteilen
- **Datei:** `kernel/src/task/loader.rs` (1892 Zeilen -- groesser als vorher!)
- **Status:** Weiterhin monolithisch. Kein `task/loader/` Subverzeichnis.
- **Loesung:** `task/loader/elf.rs`, `task/loader/memory.rs`,
  `task/loader/spawn.rs`
- **Aufwand:** 1.5 Tage

---

## Prio 3 — Nice-to-Have

### 3.1 ❌ OPEN -- TextEditor Syntax-Cache
- **Datei:** `libs/libanyui/src/controls/text_editor.rs`
- **Status:** Kein `token_cache` oder aehnliches. `SyntaxDef` vorhanden, aber
  Re-Tokenization jeden Frame.
- **Aufwand:** 1 Tag

### 3.2 ✅ DONE -- DLL-Loading Macro
- `dll_exports!` Macro in `libs/dynlink/src/lib.rs` vorhanden.

### 3.3 ❌ OPEN -- `GlobalAppState<T>` Macro
- Nicht in `libs/libanyui_client/` vorhanden. `static mut APP` Pattern in 17
  Apps weiterhin dupliziert.
- **Aufwand:** 0.5 Tage

### 3.4 ✅ DONE -- Memory-Modul API
- `kernel/src/memory/virtual_mem.rs`: 1452 Zeilen, **27** `pub fn` (statt
  ehemaliger 51). API wurde reduziert.

### 3.5 ❌ OPEN -- anyui Event-Handler Redundanz
- Kein `ToggleableControl` Trait. Button, IconButton, RadioButton, Checkbox,
  Toggle duplizieren weiterhin `handle_click/_mouse_down/_mouse_up`.
- **Aufwand:** 0.5 Tage

### 3.6 ❌ OPEN -- anyui Layout Dirty-Tracking
- Kein `needs_layout` Flag in `ControlBase`. Layout weiterhin jeden Frame
  fuer gesamten Baum.
- **Aufwand:** 1 Tag

### 3.7 ❌ OPEN -- Compositor Rounded-Corner Deduplizierung
- Kein `compute_corner_pixels()` Helper. Corner-Pixel-Berechnung weiterhin in
  4+ Funktionen dupliziert.
- **Aufwand:** 0.5 Tage

### 3.8 ❌ OPEN -- Kernel Logging vereinheitlichen
- `kernel/src/logging.rs` existiert nicht. 791 `serial_println!` ohne
  einheitliches Format weiterhin.
- **Aufwand:** 0.5 Tage

### 3.9 ✅ DONE -- Panic-Handler vereinheitlichen
- Alle Libs haben `#[panic_handler]`: stdlib, libanyui, libhttp, libfont,
  libzip, libini, libdb, libsvg, libgl, libm, libphysics.

### 3.10 ⚠️ PARTIAL -- Legacy GUI-Apps migrieren
- **Migriert:** `calc`, `clock`, `fontviewer` nutzen `libanyui_client`.
- **Noch nicht migriert:** `screenshot` verwendet weiterhin nur `anyos_std`.
- **Aufwand:** 0.5 Tage (fuer screenshot)

---

## Positiv-Befunde (Was bereits gut ist)

- **Scheduler-Modularisierung (neu):** `schedule_inner()` Monolith aufgeloest
  durch Submodule-Split. ✅
- **Compositor Performance:** Damage-based Compositing, GPU-Acceleration,
  Occlusion-Culling, Adaptive Idle-Sleep -- exzellent optimiert
- **DLL-Isolation:** Jede Library hat eigenen Heap, kein Inter-DLL
  Memory-Sharing
- **Compositor Module Boundaries:** impl-Splitting (input.rs, window.rs,
  ipc.rs) verhindert God Objects auf Code-Ebene
- **stdlib Modul-Organisation:** 30 Module mit klarer Einzelverantwortung,
  konsistentes Error-Handling; `fmt` und `path` ergaenzt ✅
- **HAL-Abstraktion im Kernel:** Saubere Trait-Definitionen fuer x86/ARM64
  Port
- **anycode als Vorzeige-App:** Saubere Trennung in `mod logic`, `mod ui`,
  `mod util`
- **Parser-Spezialisierung:** HTML, CSS, SQL, SVG, TTF Parser korrekt
  domaenenspezifisch isoliert
- **virtual_mem.rs API reduziert:** von 51 auf 27 `pub fn` ✅
- **Panic-Handler einheitlich** ueber alle Libraries ✅

---

## Verbleibender Aufwand

| Prio | Verbleibend | Status |
|------|-------------|--------|
| **Prio 1** | 2.5 offen von 4 | 1.1 DONE, 1.2 PARTIAL, 1.3/1.4 OPEN -- ~5 Tage |
| **Prio 2** | 3 offen von 5 | 2.3 DONE, 2.1/2.2 PARTIAL, 2.4/2.5 OPEN -- ~4.5 Tage |
| **Prio 3** | 7 offen von 10 | 3.2/3.4/3.9 DONE, 3.10 PARTIAL, Rest OPEN -- ~4.5 Tage |
| **Gesamt** | ~12 Tasks | ~14 Tage |

---

## Empfohlene naechste Schritte

**Hoher Impact, wenig Aufwand:**
1. `libcompress` extrahieren (1.3) -- ~1800 LOC Code-Reduktion
2. `RenderContext` Helper (1.4) -- 29+ Controls profitieren, nur 1 Tag
3. `GlobalAppState<T>` Macro (3.3) -- 17 Apps profitieren, 0.5 Tage
4. Kernel `logging.rs` (3.8) -- 791 Call-Sites werden konsistent, 0.5 Tage

**Groessere Refactorings:**
5. `Desktop` Struct-Split (1.2) -- 2 Tage
6. VFS `mount.rs` + `file_ops.rs` finalisieren (2.2) -- 1 Tag
7. `loader.rs` Split (2.5) -- 1.5 Tage
8. `KernelError` Enum (2.4) -- 2 Tage
