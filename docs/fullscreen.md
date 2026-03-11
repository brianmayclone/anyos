# Fullscreen-Modus — Design & Implementierung

## Übersicht

Fullscreen-Support für anyOS-Anwendungen. Apps können sich beim Compositor als fullscreen-fähig registrieren und zwischen Fenster- und Fullscreen-Modus wechseln. Im Fullscreen-Modus wird der gesamte Bildschirm der App überlassen — keine Menubar, keine Titelleiste, keine anderen Fenster sichtbar.

LibGL-Apps (z.B. gldemo) erhalten im Fullscreen direkten Framebuffer-Zugriff ohne Compositor-Compositing für maximale Performance.

## Systemweite Tastenkombinationen

| Shortcut | Aktion |
|----------|--------|
| **Alt+Enter** | Fullscreen-Toggle (nur wenn App fullscreen-fähig) |
| **Ctrl+Alt+Delete** | System-Dialog (Force-Quit, Task-Manager, Logout, Shutdown) — immer erreichbar, auch im Fullscreen |
| **Alt+F4** | Aktuelles Fenster schließen |

Diese Shortcuts werden vom Compositor **vor** der Weiterleitung an Apps abgefangen und sind daher unblockierbar.

## Architektur

### Fullscreen-Lebenszyklus

```
App                         Compositor
 │                              │
 ├─CMD_SET_FULLSCREEN_CAP──────>│  "Ich unterstütze Fullscreen"
 │                              │  (setzt fullscreen_capable Flag)
 │                              │
 ├─CMD_REQUEST_FULLSCREEN──────>│  App fordert Fullscreen an
 │                              │  ODER User drückt Alt+Enter
 │                              │
 │   ┌──────────────────────────┤
 │   │ Compositor:              │
 │   │ - Speichert Window-Bounds│
 │   │ - Setzt Fenster auf      │
 │   │   (0,0) + Bildschirmgröße│
 │   │ - Versteckt Menubar      │
 │   │ - Versteckt andere Fenster│
 │   │ - Optional: Mapped FB    │
 │   └──────────────────────────┤
 │                              │
 │<─RESP_FULLSCREEN_ENTERED─────┤  Enthält fb_ptr, stride, w, h
 │  (oder EVT_FULLSCREEN_ENTER) │  (fb_ptr=0 wenn kein Direkt-FB)
 │                              │
 │  ... App rendert fullscreen ...
 │                              │
 │<─EVT_FULLSCREEN_EXIT─────────┤  User drückt Alt+Enter oder Ctrl+Alt+Del
 │  ODER                        │
 ├─CMD_EXIT_FULLSCREEN─────────>│  App beendet Fullscreen selbst
 │                              │
 │<─RESP_FULLSCREEN_EXITED──────┤  Alte Bounds wiederhergestellt
```

### Rendering-Modi im Fullscreen

#### Modus A: SHM-Compositing (Standard)
- Fenster wird auf Bildschirmgröße maximiert, borderless
- Compositor compositet nur dieses eine Fenster (kein Wallpaper, keine Menubar)
- Für normale GUI-Apps (Notepad, Browser, etc.)

#### Modus B: Direkter Framebuffer-Zugriff (für LibGL)
- App erhält direkten Pointer auf den Framebuffer
- Compositor pausiert sein Compositing komplett
- `present()` löst nur GPU-Flush aus (kein Compositing)
- Maximale Performance: LibGL → Framebuffer ohne Umwege

## IPC-Protokoll

### Neue Commands (App → Compositor)

```
CMD_SET_FULLSCREEN_CAP  = 0x1030
  [CMD, window_id, auto_enter (0/1), 0, 0]
  Markiert Fenster als fullscreen-fähig.
  auto_enter=1: Sofort in Fullscreen gehen.

CMD_REQUEST_FULLSCREEN  = 0x1031
  [CMD, window_id, want_direct_fb (0/1), 0, 0]
  Fordert Fullscreen an.
  want_direct_fb=1: App will direkten Framebuffer-Zugriff.

CMD_EXIT_FULLSCREEN     = 0x1032
  [CMD, window_id, 0, 0, 0]
  Verlässt Fullscreen-Modus.
```

### Neue Responses (Compositor → App)

```
RESP_FULLSCREEN_ENTERED = 0x2020
  [RESP, window_id, (width << 16) | height, stride, fb_ptr_or_0]
  Fullscreen aktiv. fb_ptr=0 wenn SHM-Modus, sonst Framebuffer-VA.

RESP_FULLSCREEN_EXITED  = 0x2021
  [RESP, window_id, 0, 0, 0]
  Zurück im Fenster-Modus.
```

### Neue Events (Compositor → App, via Hotkey)

```
EVT_FULLSCREEN_ENTER    = 0x300D
  [EVT, window_id, (width << 16) | height, stride, fb_ptr_or_0]
  Compositor hat Fullscreen aktiviert (z.B. via Alt+Enter).

EVT_FULLSCREEN_EXIT     = 0x300E
  [EVT, window_id, 0, 0, 0]
  Compositor hat Fullscreen beendet (z.B. via Alt+Enter, Ctrl+Alt+Del).
```

## WindowInfo-Erweiterung

```rust
pub struct WindowInfo {
    // ... bestehende Felder ...
    pub fullscreen: bool,                              // Aktuell im Fullscreen?
    pub fullscreen_capable: bool,                      // App hat sich registriert?
    pub saved_bounds_fs: Option<(i32, i32, u32, u32)>, // Bounds vor Fullscreen
    pub fullscreen_direct_fb: bool,                    // Hat direkten FB-Zugriff?
}
```

Neuer Window-Flag:
```rust
pub const WIN_FLAG_FULLSCREEN_CAPABLE: u32 = 0x400;
```

## Desktop-Erweiterung

```rust
pub struct Desktop {
    // ... bestehende Felder ...
    pub(crate) fullscreen_window: Option<u32>,  // Window-ID im Fullscreen
}
```

## Compositor Compositing-Optimierung

In `compose()`:
```rust
if let Some(fs_win_id) = self.fullscreen_window {
    // Nur Fullscreen-Fenster compositen
    // Kein Wallpaper, keine Menubar, keine Shadow-Berechnung
    // → Massive Performance-Verbesserung
}
```

## Globale Hotkeys

Implementiert in `Desktop::handle_key()` (desktop/input.rs), **vor** Weiterleitung an Apps:

### Alt+Enter — Fullscreen Toggle
- Prüft ob fokussiertes Fenster `fullscreen_capable` ist
- Toggle zwischen Fullscreen und Fenster-Modus
- Sendet EVT_FULLSCREEN_ENTER / EVT_FULLSCREEN_EXIT an App

### Ctrl+Alt+Delete — System-Escape
- Beendet Fullscreen sofort (falls aktiv)
- Zeigt System-Dialog (direkt im Compositor gerendert)
- Optionen: Force-Quit, Task-Manager, Abmelden, Neustart, Herunterfahren

### Alt+F4 — Fenster schließen
- Sendet EVT_WINDOW_CLOSE an fokussiertes Fenster
- Funktioniert auch im Fullscreen (beendet zuerst Fullscreen)

## Kernel-Syscall für Framebuffer-Zugriff

Neuer Syscall: `sys_grant_framebuffer(target_tid) -> (fb_addr, width, height, pitch)`
- Nur vom Compositor aufrufbar
- Mapped Framebuffer-Seiten in den Adressraum der Ziel-App
- Setzt `EXCLUSIVE_FB_TID` Flag im Kernel

Aufräum-Syscall: `sys_revoke_framebuffer(target_tid)`
- Entfernt Mapping aus dem Adressraum der App
- Löscht `EXCLUSIVE_FB_TID` Flag

## LibGL Fullscreen-Integration

Neue Funktionen in libgl_client:
```rust
/// Initialisiert LibGL im Fullscreen-Modus mit direktem Framebuffer.
pub fn gl_init_fullscreen(fb_ptr: *mut u32, width: u32, height: u32, stride: u32);

/// Swap-Buffers im Fullscreen: kein Memcopy, nur GPU-Flush.
pub fn swap_buffers_fullscreen();
```

Performance-Gewinn:
- Aktuell: LibGL → SwFramebuffer → canvas.copy_pixels_from → SHM → Compositing → FB
- Fullscreen: LibGL → Framebuffer (direkt)
- Spart 3-4 Memcopy-Operationen pro Frame

## GLDemo Fullscreen

apps/gldemo wird die erste Fullscreen-App:
1. Registriert sich mit `CMD_SET_FULLSCREEN_CAP` + `auto_enter=1`
2. Bei `EVT_FULLSCREEN_ENTER`: `gl_init_fullscreen()` mit FB-Pointer
3. Bei `EVT_FULLSCREEN_EXIT`: Zurück auf Canvas-Rendering
4. Alt+Enter toggled zwischen Fullscreen und Fenster-Modus

## Dateien die geändert werden

### Compositor
- `system/compositor/compositor/src/ipc_protocol.rs` — Neue IPC-Konstanten
- `system/compositor/compositor/src/desktop/window.rs` — WindowInfo erweitern
- `system/compositor/compositor/src/desktop/mod.rs` — Desktop.fullscreen_window
- `system/compositor/compositor/src/desktop/ipc.rs` — Fullscreen IPC-Handler
- `system/compositor/compositor/src/desktop/input.rs` — Globale Hotkeys
- `system/compositor/compositor/src/compositor/compositing.rs` — Fullscreen-Optimierung
- `system/compositor/compositor/src/main.rs` — IPC-Command Routing
- `system/compositor/compositor/src/keys.rs` — KEY_ENTER Konstante exportieren

### libcompositor (DLL)
- `libs/libcompositor/src/exports.rs` — Neue Export-Funktionen
- `libs/libcompositor/exports.def` — Symbol-Export

### libcompositor_client
- `libs/libcompositor_client/src/lib.rs` — Client-Wrapper Funktionen

### Kernel
- `kernel/src/syscall/handlers/display.rs` — sys_grant/revoke_framebuffer
- `kernel/src/syscall/mod.rs` — Syscall-Nummern

### LibGL
- `libs/libgl/src/lib.rs` — gl_init_fullscreen, swap_buffers_fullscreen
- `libs/libgl_client/src/lib.rs` — Client-Wrapper

### GLDemo
- `apps/gldemo/src/main.rs` — Fullscreen-Modus
