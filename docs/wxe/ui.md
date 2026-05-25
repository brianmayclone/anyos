# WXE Win32 UI Surface

WXE is still console-first. This document pins down the UI work so
`user32.dll`, `gdi32.dll` and `win32u.dll` can be generated and imported
without accidentally claiming full desktop compatibility.

## Layering

```text
Windows PE application
  -> user32.dll / gdi32.dll
     -> win32u.dll route thunks where Windows expects that split
        -> WXE UI backend
           -> libanyui.so C exports
              -> anyOS compositor, font and render services
```

The WXE UI backend is a compatibility layer over `libanyui`, not a direct
exposure of native anyOS GUI objects to Windows programs. HWND, HDC, HMENU,
HICON, HBRUSH and HFONT stay Windows-shaped handles even when they wrap
libanyui windows, controls or canvas resources internally.

## Current Implementation Slice

`wxe init` installs `/System/var/wxe/db/ui-routes` and
`/System/var/wxe/db/anyui-bindings`. `ui-routes` lists the Win32 export routes;
`anyui-bindings` pins those routes to concrete `libanyui.so!anyui_*` exports.
The first route names cover:

- `user32.dll`: message boxes, desktop/window handles, class registration,
  window creation/destruction, show/update, message queue calls and timers.
- `gdi32.dll`: DC creation, stock/select/delete object calls, text metrics,
  text output, simple brushes/pens, rectangles and blits.
- `win32u.dll`: the lower `NtUser*` and `NtGdi*` route names used by the later
  backend.

For the console milestone, these routes let the import resolver satisfy benign
GUI imports. Callable Tier-0 behavior is intentionally small:

| Function family | Tier-0 behavior |
| --- | --- |
| `MessageBoxA/W` | console diagnostic fallback, return `IDOK` |
| `GetDesktopWindow` | stable pseudo-HWND |
| `GetDC` / `ReleaseDC` | pseudo-HDC for desktop/console-safe metrics |
| `GetSystemMetrics` | fixed profile metrics |
| window creation/message loops | fail with unsupported until GUI tier |
| GDI drawing | fail or no-op unless writing to a pseudo-HDC is harmless |

GUI subsystem PE images remain rejected before entry. The route table is there
so console tools with incidental UI imports can still load predictably.

## Handle Model

The UI backend needs a WXE handle table separate from POSIX fds and LXE state.
That table owns the conversion between Win32 handles and libanyui control IDs:

| Handle kind | Backing object |
| --- | --- |
| `HWND` | WXE window/control object wrapping a libanyui window or control ID |
| `HDC` | paint/session context wrapping a libanyui Canvas or memory surface |
| `HBRUSH`, `HPEN`, `HFONT` | small WXE-owned GDI objects |
| `HMENU`, `HICON`, `HCURSOR` | explicit stubs first, real resources later |

Pseudo handles such as the desktop window must be stable inside a process and
must never collide with fd-backed NT handles.

## libanyui Mapping

The mapping must stay centralized in the WXE UI backend. `user32.dll` and
`gdi32.dll` call Win32-shaped routines; those routines translate arguments,
own last-error behavior and then invoke libanyui primitives.

| Win32 family | WXE backend route | libanyui primitive |
| --- | --- | --- |
| `MessageBoxA/W` | `anyui:dialog:message-box-*` | `anyui_message_box` |
| top-level `CreateWindowExA/W` | `anyui:window:create-*` | `anyui_create_window` |
| child controls | `anyui:control:create` | `anyui_create_control`, `anyui_add_child` |
| show/update/destroy | `anyui:window:*` | `anyui_set_visible`, `anyui_flush_display`, `anyui_destroy_window` |
| event pumping | `anyui:message:*` | `anyui_run_once`, `anyui_on_event` |
| window DC drawing | `anyui:gdi:*` | Canvas-backed HDC over `anyui_canvas_*` |
| text output | `anyui:gdi:text:*` | `anyui_canvas_draw_text` |
| text metrics | `anyui:gdi:text:extent-*` | `anyui_measure_text` |
| fills/rectangles | `anyui:gdi:shape:*`, `anyui:gdi:blit:pat` | `anyui_canvas_fill_rect`, `anyui_canvas_draw_rect` |
| buffer blits | `anyui:gdi:blit:bit` | WXE memory DC copy plus `anyui_canvas_copy_from` |

Window-class registration, stock GDI objects, default window procedures,
timers, `PostMessage` and `SendMessage` remain WXE-owned state machines. They
must not be pushed into libanyui, because their observable behavior is Win32
compatibility policy.

## Message Pump

The first GUI-capable milestone needs a per-thread message queue:

- `PostMessageA/W`, `SendMessageA/W`
- `GetMessageA/W`, `PeekMessageA/W`
- `TranslateMessage`
- `DispatchMessageA/W`
- `PostQuitMessage`
- timer messages from `SetTimer` / `KillTimer`

The queue should live in the WXE backend, because it combines Windows message
semantics with libanyui event delivery. It must not be bolted onto LXE signals
or native anyOS process state. `GetMessage` and `PeekMessage` drain the WXE
queue; when it is empty, the backend calls `anyui_run_once` to collect pending
libanyui/compositor events and translate them into WM_* messages.

## GDI Bridge

GDI starts with a software-backed subset:

- selected pen/brush/font state per HDC
- window HDCs backed by libanyui Canvas controls
- memory HDCs backed by WXE-owned software surfaces until selected into a window
- `TextOutA/W` through `anyui_canvas_draw_text`
- `GetTextExtentPoint32A/W` through `anyui_measure_text`
- `Rectangle`, `PatBlt` and initial `BitBlt` through libanyui Canvas operations
- invalidation from GDI writes via `anyui_flush_display`

Advanced GDI, DWM composition, OpenGL/WGL, printer DCs and device-dependent
bitmap formats are later milestones.

## Acceptance Order

1. Generate importable PE DLLs for `user32.dll`, `gdi32.dll` and `win32u.dll`
   from `/System/var/wxe/db/ui-routes`. This is wired through `wxe init`;
   unsupported UI calls still return fallback values until the backend lands.
2. Let console binaries load if they import UI DLLs but do not exercise real
   GUI behavior.
3. Implement console-safe `MessageBoxA/W`, `GetDesktopWindow`,
   `GetSystemMetrics`, `GetDC` and `ReleaseDC`.
4. Add the WXE UI backend, HWND/HDC tables, libanyui binding loader and message
   queues.
5. Enable GUI subsystem PE images only after `CreateWindowExW` plus a minimal
   message pump works against the anyOS compositor.
