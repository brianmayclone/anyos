# anyOS Architecture Refactoring

Ergebnisse der Tiefenanalyse von Kernel, Compositor, anyui, Libraries und Apps.

---

## Gesamtbewertung

| Bereich | Score | Kritischstes Problem |
|---------|-------|---------------------|
| **Kernel** | 7/10 | `schedule_inner()` — 630-Zeilen-Monolith |
| **Compositor** | 8/10 | `Desktop` struct — God Object mit 37+ Feldern |
| **anyui Framework** | 7/10 | 20+ identische Render-Boilerplate-Blöcke |
| **Shared Libraries** | 6/10 | 4x duplizierte DEFLATE-Implementierung (~1800 LOC) |
| **Apps** | 6/10 | Massives Boilerplate-Duplication über 22 GUI-Apps |

---

## Prio 1 — Hoher Impact, Architektur-Verbesserung

### 1.1 `schedule_inner()` in 5-6 Funktionen aufteilen
- **Datei:** `kernel/src/task/scheduler.rs:887-1517`
- **Problem:** 630 Zeilen, 10 Verantwortlichkeiten vermischt (CPU-Flags, PD Destruction, Timer Accounting, Thread Reaping, Deferred Wakes, Canary-Checks, Sleep-Wakeup, Affinity-Rebalancing, Thread-Selection, Context-Switch)
- **Lösung:**
  ```rust
  fn schedule_inner(from_timer: bool) {
      manage_scheduler_flags();
      drain_deferred_pd_destroy();
      let mut guard = acquire_scheduler_lock(from_timer);
      reap_terminated_threads(&mut guard);
      wake_expired_sleepers(&mut guard);
      if from_timer { rebalance_affinity(&mut guard); }
      let switch = pick_next_thread(&mut guard);
      drop(guard);
      perform_context_switch(switch);
  }
  ```
- **Aufwand:** 2-3 Tage

### 1.2 `Desktop` God Object in Sub-Structs aufbrechen
- **Datei:** `bin/compositor/src/desktop/mod.rs`
- **Problem:** 37+ Felder, 7 Domänen vermischt (Window-Management, Input-State, UI-Chrome, App-Protokoll, Lifecycle, Wallpaper, Rendering)
- **Lösung:**
  ```rust
  pub struct Desktop {
      pub compositor: Compositor,
      window_manager: WindowManager,
      input_state: InputState,
      ui_chrome: UiChrome,
      app_protocol: AppProtocol,
      lifecycle: DesktopLifecycle,
  }
  ```
- **Aufwand:** 2 Tage

### 1.3 `libcompress` extrahieren (DEFLATE/CRC32)
- **Problem:** 4 unabhängige DEFLATE-Implementierungen mit ~1800 LOC total:
  - `libs/libzip/src/deflate.rs` + `inflate.rs` (612 LOC)
  - `libs/libhttp/src/deflate.rs` (487 LOC)
  - `libs/libimage/src/deflate.rs` (442 LOC)
  - `libs/libfont/src/inflate.rs` (251 LOC)
- **Lösung:** Neue `libcompress` staticlib mit `inflate()`, `deflate()`, `crc32()` — alle Server-Libs linken statisch
- **Aufwand:** 2 Tage

### 1.4 `RenderContext` Helper für anyui Controls
- **Problem:** 29+ Controls wiederholen identischen Render-Setup (20+ Zeilen Boilerplate pro Control)
  ```rust
  let b = self.text_base.base;
  let p = scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
  let (x, y, w, h) = (p.x, p.y, p.w, p.h);
  let tc = theme::colors();
  let disabled = b.disabled;
  let hovered = b.hovered;
  let focused = b.focused;
  ```
- **Lösung:**
  ```rust
  pub struct RenderContext {
      pub x: i32, pub y: i32, pub w: u32, pub h: u32,
      pub disabled: bool, pub hovered: bool, pub focused: bool,
  }
  pub fn prepare_render(base: &ControlBase, ax: i32, ay: i32) -> RenderContext
  ```
- **Betroffene Dateien:** `button.rs`, `checkbox.rs`, `toggle.rs`, `radio_button.rs`, `slider.rs`, `icon_button.rs`, `progress_bar.rs`, + 22 weitere
- **Aufwand:** 1 Tag

---

## Prio 2 — Code-Hygiene & Konsistenz

### 2.1 Syscall-Wrapper in libheap konsolidieren
- **Problem:** Identische `syscall0/1/2/3` inline Assembly in 6 Server-Libraries dupliziert (~150 LOC)
- **Betroffene Dateien:** `libfont/src/syscall.rs`, `libhttp/src/syscall.rs`, `libimage/src/syscall.rs`, `libzip/src/syscall.rs`, `libdb/src/syscall.rs`, `libsvg/src/syscall.rs`
- **Aufwand:** 0.5 Tage

### 2.2 VFS in separate Module aufteilen
- **Datei:** `kernel/src/fs/vfs.rs` (1796 Zeilen)
- **Problem:** Mount-Management, Pfad-Auflösung, File-Lifecycle, Directory-Ops, Permissions, Symlinks alles in einer Datei
- **Lösung:** Aufteilen in `mount.rs`, `path.rs`, `file_ops.rs`, VFS bleibt als Coordinator
- **Aufwand:** 2 Tage

### 2.3 `anyos_std::fmt` + `anyos_std::path` erstellen
- **Problem:** Number-Formatting (`fmt_u32`, `fmt_i32`, `fmt_f64`) in 8+ Apps dupliziert; `basename()`, `dirname()` in 4+ Apps dupliziert
- **Betroffene Apps:** paint, calculator, clock, minesweeper, notepad, fontviewer, screenshot
- **Aufwand:** 1 Tag

### 2.4 Einheitliches Error-Handling
- **Kernel:** Einheitliches `KernelError` Enum statt Mix aus `FsError`, `Option<>`, Panics
- **Compositor:** Logging für IPC-Fehler, SHM-Map-Fehler, File-I/O statt stille Fehler
- **Libraries:** Einheitliches `DllError` Enum für alle Server-Exports
- **Aufwand:** 2 Tage

### 2.5 Loader in Sub-Module aufteilen
- **Datei:** `kernel/src/task/loader.rs` (1250 Zeilen)
- **Problem:** ELF-Parsing, Memory-Mapping, Address-Space-Setup, Spawn-Logik, ASLR vermischt
- **Lösung:** `task/loader/elf.rs`, `task/loader/memory.rs`, `task/loader/spawn.rs`
- **Aufwand:** 1.5 Tage

---

## Prio 3 — Nice-to-Have

### 3.1 TextEditor Syntax-Cache
- **Datei:** `libs/libanyui/src/controls/text_editor.rs` (1351 LOC)
- **Problem:** Syntax-Tokenization passiert jeden Frame, auch ohne Textänderung
- **Lösung:** Lazy Re-Tokenization nur bei Textänderung, Token-Spans cachen
- **Aufwand:** 1 Tag

### 3.2 DLL-Loading Macro (`dll_exports!`)
- **Problem:** 8 Client-Libraries reimplementieren identisches `ExportTable` struct + `resolve()` Pattern
- **Lösung:** Deklarativer Macro:
  ```rust
  dll_exports! {
      "libfont.so" => {
          font_init() -> (),
          font_load(path_ptr: *const u8, len: u32) -> u32,
      }
  }
  ```
- **Aufwand:** 1 Tag

### 3.3 `GlobalAppState<T>` Macro für Apps
- **Problem:** 17 Apps nutzen identisches `static mut APP: Option<T>` Pattern
- **Lösung:** Type-safe Wrapper in libanyui_client
- **Aufwand:** 0.5 Tage

### 3.4 Memory-Modul API reduzieren
- **Datei:** `kernel/src/memory/virtual_mem.rs`
- **Problem:** 51 pub Funktionen exportiert, ~12 sind tatsächlich nötig
- **Lösung:** Debug-Funktionen hinter Feature-Flag, interne Funktionen `pub(crate)`
- **Aufwand:** 1 Tag

### 3.5 anyui Event-Handler Redundanz eliminieren
- **Problem:** Button, IconButton, RadioButton, Checkbox, Toggle implementieren identische `handle_click`/`handle_mouse_down`/`handle_mouse_up`
- **Lösung:** `ToggleableControl` Trait extrahieren
- **Aufwand:** 0.5 Tage

### 3.6 anyui Layout Dirty-Tracking
- **Problem:** Layout wird jeden Frame für gesamten Baum berechnet
- **Lösung:** `needs_layout` Dirty-Flag pro Control, Skip wenn unchanged
- **Aufwand:** 1 Tag

### 3.7 Compositor Rounded-Corner Deduplizierung
- **Problem:** Identical Corner-Pixel-Berechnung in 4+ Funktionen (~80 LOC)
- **Lösung:** Shared `compute_corner_pixels()` Helper
- **Aufwand:** 0.5 Tage

### 3.8 Kernel Logging vereinheitlichen
- **Problem:** 791 `serial_println!` ohne einheitliches Format (`[OK]`, `[WARN]`, `!`, kein Prefix)
- **Lösung:** `kernel/src/logging.rs` mit `log_info/warn/error/critical`
- **Aufwand:** 0.5 Tage

### 3.9 Panic-Handler vereinheitlichen
- **Problem:** Inkonsistent über Libraries (loop{}, exit(1), silent abort)
- **Lösung:** Alle: Panik-Nachricht loggen + `exit(1)`
- **Aufwand:** 0.5 Tage

### 3.10 Legacy GUI-Apps nach libanyui migrieren
- **Problem:** Calculator, Clock, FontViewer, Screenshot nutzen altes `anyos_std::ui::window` Framework
- **Lösung:** Schrittweise Migration zu libanyui
- **Aufwand:** 2-3 Tage

---

## Positiv-Befunde (Was bereits gut ist)

- **Compositor Performance:** Damage-based Compositing, GPU-Acceleration, Occlusion-Culling, Adaptive Idle-Sleep — exzellent optimiert
- **DLL-Isolation:** Jede Library hat eigenen Heap, kein Inter-DLL Memory-Sharing
- **Compositor Module Boundaries:** impl-Splitting (input.rs, window.rs, ipc.rs) verhindert God Objects auf Code-Ebene
- **stdlib Modul-Organisation:** 30 Module mit klarer Einzelverantwortung, konsistentes Error-Handling
- **HAL-Abstraktion im Kernel:** Saubere Trait-Definitionen für x86/ARM64 Port
- **anycode als Vorzeige-App:** Saubere Trennung in `mod logic`, `mod ui`, `mod util`
- **Parser-Spezialisierung:** HTML, CSS, SQL, SVG, TTF Parser korrekt domänenspezifisch isoliert

---

## Geschätzter Gesamtaufwand

| Priorität | Tasks | Aufwand |
|-----------|-------|---------|
| **Prio 1** | 4 Tasks | ~7 Tage |
| **Prio 2** | 5 Tasks | ~7 Tage |
| **Prio 3** | 10 Tasks | ~8 Tage |
| **Gesamt** | 19 Tasks | ~22 Tage |
