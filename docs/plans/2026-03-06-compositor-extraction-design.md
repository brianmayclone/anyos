# Compositor Extraction Design

## Goal

Extract Desktop and Menüleiste from the compositor into a separate "Shell" process, extract CrashDialog into a standalone .app, and add a Sessionhost to manage system process lifecycle. The compositor becomes a pure window management and compositing engine.

## Architecture

### Shell (`system/shell/`)

A new process that owns the Desktop (wallpaper, icons) and the Menüleiste (app menus, active app title).

- Renders the desktop background, desktop icons, and the menu bar
- Apps register their menus **directly with the Shell** via IPC (not via compositor)
- Compositor notifies the Shell when the active/focused app changes, so the menu bar shows the correct app name and menus
- Installed to `/System/Shell`

### Compositor (existing `system/compositor/`)

Becomes a pure compositing and window management engine:

- Window creation, destruction, focus, move, resize
- Compositing and rendering window buffers to the framebuffer
- Input dispatch to windows
- Notifies the Shell about focus changes
- All desktop/menu bar rendering code removed
- `desktop/` module stripped down to window management only (no wallpaper, no icons, no crash dialog, no menu rendering)

### CrashDialog (`system/crashdialog/`)

A standalone .app that displays a crash notification to the user.

- Built as a `.app` bundle with `Info.conf` + `Icon.ico`
- Launched by the Sessionhost when a monitored process crashes
- Receives crash info (process name, signal) via command-line args or IPC
- Source: `system/crashdialog/`
- Installed to `/System/CrashDialog.app`

### Sessionhost (`system/sessionhost/`)

Manages system process lifecycle:

- Starts the Shell process
- Monitors critical processes for crashes
- Launches CrashDialog when a crash is detected
- Launches PermissionDialog when needed
- Source: `system/sessionhost/`
- Installed to `/System/Sessionhost`

## IPC Flow

```
Apps ---[menu registration]--> Shell
Compositor --[focus changed]--> Shell
Sessionhost --[launches]------> Shell, CrashDialog, PermissionDialog
```

## Source Layout

```
system/
  shell/            -- new: Desktop + Menüleiste
  crashdialog/      -- new: standalone .app
  sessionhost/      -- new: process lifecycle manager
  compositor/       -- existing: stripped to pure compositing
```

## Sysroot Layout

```
/System/
  Shell
  CrashDialog.app/
    CrashDialog
    Info.conf
    Icon.ico
  Sessionhost
```

## What Moves Where

| Current location (compositor) | Destination |
|-------------------------------|-------------|
| Desktop wallpaper rendering | Shell |
| Desktop icon rendering | Shell |
| Menu bar rendering | Shell |
| Menu registration IPC | Shell (new IPC channel) |
| CrashDialog struct + rendering | CrashDialog .app |
| Window management | Stays in compositor |
| Compositing / render loop | Stays in compositor |
| Input dispatch to windows | Stays in compositor |
