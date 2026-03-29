# anyOS Applications Reference

anyOS ships with 33 GUI applications built with the anyui widget framework. All apps are `#![no_std]` Rust programs using `libanyui_client` for the GUI layer.

**Installation path**: `/Applications/<AppName>.app/`
**Bundle contents**: `Info.conf`, `Icon.ico`, executable, optional resources

---

## Categories

| Category | Apps |
|---|---|
| **System** | Installer, Keyboard, Runner, Settings, Updater, App Store, FTP Settings, VNC Settings |
| **Utilities** | Calculator, Clipboard Manager, Clock, Font Viewer, Icon Browser, Notifications, Web Manager |
| **Productivity** | Notepad, Diff, Markdown Viewer |
| **Internet** | Surf (Browser), anyMail, anyzilla (FTP), |
| **Graphics** | Paint, Image Viewer |
| **Development** | anyOS Code, anyUI Demo, GL Demo |
| **Games** | Minesweeper, Forger |
| **Media** | Video Player |
| **Benchmark** | anyBench |
| **Diagnostics** | Diagnostics, Screenshot |
| **Virtualization** | VM Manager |

---

## Application Details

### anyBench
**System benchmark suite**

| | |
|---|---|
| ID | `com.anyos.anybench` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, shm, system, process |

Comprehensive benchmark tool testing:
- **CPU**: Integer arithmetic, floating-point, memory bandwidth, matrix multiplication, crypto (SHA-256), sorting algorithms
- **GPU**: Fill rate, pixel throughput, line drawing, alpha blending
- **3D**: Rendering performance

Results are displayed in a DataGrid with scores and comparisons.

---

### anyOS Code
**Integrated Development Environment**

| | |
|---|---|
| ID | `com.anyos.code` |
| Category | Development |
| Capabilities | filesystem, dll, event, thread, pipe, shm, process |
| Special | `working_dir=bundle` |

VS Code-inspired IDE with:
- Syntax-highlighted code editor with line numbers
- File manager sidebar with tree view
- Git integration panel (status, diff, commit)
- Toolbar with build/run actions
- Status bar (line, column, encoding, language)
- Find/replace with regex support
- Multiple file tabs

Reference implementation for complex anyui applications.

---

### anyMail
**Email client**

| | |
|---|---|
| ID | `com.anyos.anymail` |
| Category | Internet |
| Capabilities | filesystem, network, display, dll, event, thread, pipe, shm, process |

Thunderbird-style email client supporting:
- **Protocols**: IMAP4rev1, POP3, SMTP with TLS/STARTTLS
- **Features**: Folder tree, message list grid, HTML message view, attachments
- **Contacts**: Address book with autocomplete in To/CC/BCC fields
- **Accounts**: Multi-account support, import/export

---

### anyzilla
**FTP client**

| | |
|---|---|
| ID | `com.anyos.anyzilla` |
| Category | Internet |
| Capabilities | filesystem, network, display, dll, event, thread, pipe, shm, process |

FileZilla-inspired dual-pane FTP client:
- Site manager with saved server profiles
- Local and remote file browser side by side
- PASV mode transfers
- Directory navigation with breadcrumb path
- Transfer queue and progress display

---

### Calculator
**Desktop calculator**

| | |
|---|---|
| ID | `com.anyos.calculator` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

macOS dark-themed calculator with:
- Numeric keypad, operator buttons (+, -, *, /)
- Scientific functions (sin, cos, sqrt, etc.)
- Expression evaluation engine
- Keyboard input support

---

### Clipboard Manager
**Clipboard history**

| | |
|---|---|
| ID | `com.anyos.clipman` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Tracks clipboard history with:
- Configurable retention (number of entries, time limit)
- Search/filter through clipboard entries
- Click to re-copy any previous entry
- Persistent JSON storage per user (`/Users/<user>/clipman.json`)
- Timer-polled clipboard monitoring

---

### Clock
**Clock & timer widget**

| | |
|---|---|
| ID | `com.anyos.clock` |
| Category | Utilities |
| Capabilities | filesystem, display, dll, event, thread, pipe, shm |

Features:
- Analog clock face with hour/minute/second hands
- Countdown timer with preset buttons (1m, 5m, 15m, 30m, 1h)
- World clocks (multiple timezones)

---

### anyUI Demo
**Widget showcase**

| | |
|---|---|
| ID | `com.anyos.demo_anyui` |
| Category | Development |
| Capabilities | dll, event |

Interactive demonstration of all anyui controls:
- ScrollView, Expander, StackPanel
- ContextMenu, Tooltips
- Buttons, TextFields, CheckBoxes, RadioButtons
- DataGrid, TreeView, TabControl
- Sliders, ProgressBars, ColorPicker

Useful as a reference for widget usage patterns.

---

### Diagnostics
**System diagnostics**

| | |
|---|---|
| ID | `com.anyos.diagnostics` |
| Category | System |
| Capabilities | filesystem, display, system, process, pipe, event, dll, thread, shm, network |

System diagnostics tool for:
- UI performance benchmarks (event loop timing)
- System information display
- Hardware status monitoring

---

### Diff
**File comparison tool**

| | |
|---|---|
| ID | `com.anyos.diff` |
| Category | Productivity |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Meld-inspired diff viewer:
- Side-by-side file comparison
- Color-coded lines: green (added), red (deleted), yellow (changed)
- Hunk navigation (previous/next change)
- Syntax highlighting
- Merge conflict resolution

---

### Font Viewer
**Font browser**

| | |
|---|---|
| ID | `com.anyos.font-viewer` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Previews installed system fonts:
- Character sample display ("The quick brown fox...")
- Multiple size previews (12px, 18px, 24px, 36px, 48px)
- Font list from `/System/fonts/`
- Glyph grid view

---

### Forger
**3D voxel game**

| | |
|---|---|
| ID | `com.anyos.forger` |
| Category | Games |
| Capabilities | dll, event |

Minecraft-inspired 3D voxel world:
- Block placement and removal
- 3D rendering via libgl (OpenGL ES 2.0)
- Physics engine integration (libphysics)
- First-person camera controls

---

### FTP Settings
**FTP server configuration**

| | |
|---|---|
| ID | `com.anyos.ftp-settings` |
| Category | System |
| Capabilities | filesystem, dll, event, thread, pipe, process, system |

GUI for managing the ftpd daemon:
- Server port configuration
- Passive mode settings (PASV port range)
- Share directory management (add/remove/edit)
- User permission control (read/write per share)
- Start/stop ftpd service

Config files: `/System/etc/ftpd/ftpd.conf`, `/System/etc/ftpd/shares.conf`

---

### GL Demo
**OpenGL ES 2.0 demo**

| | |
|---|---|
| ID | `com.anyos.gldemo` |
| Category | Development |
| Capabilities | dll, event |

3D graphics showcase using libgl:
- Gouraud-shaded geometry
- Textured cube and sphere with UV mapping
- Animated point lights with diffuse/specular
- Procedural texture generation
- Realtime rendering loop

---

### Icon Browser
**System icon viewer**

| | |
|---|---|
| ID | `com.anyos.iconview` |
| Category | Utilities |
| Capabilities | filesystem, dll, event |

Browse the system icon pack (`ico.pak`):
- Grid display of all available icons
- Filled and outline icon variants
- Search by icon name
- Color/theme filtering
- Icon name and size info on hover

---

### Image Viewer
**Multi-format image viewer**

| | |
|---|---|
| ID | `com.anyos.image-viewer` |
| Category | Graphics |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Supported formats: BMP, PNG, JPEG, GIF, ICO (via libimage)

Features:
- Pan with mouse drag
- Zoom with scroll wheel
- Fit-to-window / actual-size toggle
- File dialog for opening images
- Status bar with dimensions and zoom level

---

### anyOS Installer
**System installer**

| | |
|---|---|
| ID | `com.anyos.installer` |
| Category | System |
| Capabilities | filesystem, display, dll, event, thread, pipe, shm, process, system |

Installs anyOS to a disk partition:
- Disk/partition selection
- FAT32 and exFAT filesystem support
- File copy with progress bar
- Bootloader installation
- Worker thread for non-blocking UI during install

---

### Keyboard
**On-screen keyboard viewer**

| | |
|---|---|
| ID | `com.anyos.keyboard` |
| Category | System |
| Capabilities | filesystem, display, dll, event, thread, pipe, shm |

Displays the current keyboard layout:
- Visual key representation matching physical layout
- Real-time key press highlighting (pressed keys light up)
- Layout switching support
- Modifier key state display (Shift, Ctrl, Alt, Super)

---

### Markdown Viewer
**Markdown renderer**

| | |
|---|---|
| ID | `com.anyos.mdview` |
| Category | Productivity |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Renders Markdown files with:
- Headers (H1-H6) with font size scaling
- Code blocks with syntax highlighting
- Blockquotes, lists (ordered/unordered)
- Bold, italic, inline code
- Links and images
- Source/preview toggle
- File open dialog

---

### Minesweeper
**Classic puzzle game**

| | |
|---|---|
| ID | `com.anyos.minesweeper` |
| Category | Games |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Standard Minesweeper implementation:
- 9x9 grid with 10 mines
- Left-click to reveal, right-click to flag
- Flood-fill reveal for empty cells
- Mine counter and timer
- Win/lose detection with full board reveal

---

### Notepad
**Text editor**

| | |
|---|---|
| ID | `com.anyos.notepad` |
| Category | Productivity |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Simple, lightweight text editor:
- Toolbar with New, Open, Save, Save As
- Status bar showing filename, cursor position (Ln/Col), encoding
- File open/save dialogs
- Unsaved changes detection
- Keyboard shortcuts (Ctrl+N, Ctrl+O, Ctrl+S)

---

### Notifications
**Notification history viewer**

| | |
|---|---|
| ID | `com.anyos.notifications` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Displays notification history:
- List of past notifications (title, message, timestamp)
- Detail panel for selected notification
- Persistent JSON storage
- Clear all / clear individual entries
- Integrates with notifyd system daemon

---

### Paint
**Drawing application**

| | |
|---|---|
| ID | `com.anyos.paint` |
| Category | Graphics |
| Capabilities | filesystem, dll, event, thread, pipe, shm |

Canvas-based paint application:
- **Tools**: Pencil, brush, eraser, line, rectangle, ellipse, flood fill, text
- **Features**: Color picker with palette, adjustable brush size
- **File**: New canvas, open image, save as BMP/PNG
- **Edit**: Undo/redo stack
- Toolbar with tool icons and color selection

---

### Runner
**Application launcher**

| | |
|---|---|
| ID | `com.anyos.runner` |
| Category | System |
| Capabilities | filesystem, display, dll, event, thread, pipe, shm, process |

Quick-launch dialog for applications:
- Scans `/Applications/` for installed apps
- Autocomplete search field with fuzzy matching
- App icon display next to results
- Enter to launch selected app
- Lightweight overlay-style window

---

### Screenshot
**Screen capture tool**

| | |
|---|---|
| ID | `com.anyos.screenshot` |
| Category | Utilities |
| Capabilities | filesystem, display, dll, event, thread, pipe, shm |

Capture and save screenshots:
- Full screen capture
- Region selection (click and drag)
- PNG encoding via libimage
- Save dialog with default filename (timestamp)
- Clipboard copy option

---

### App Store
**Package manager GUI**

| | |
|---|---|
| ID | `com.anyos.store` |
| Category | System |
| Capabilities | filesystem, dll, event, thread, pipe, process, shm |

Browse and manage software packages:
- **Tabs**: All packages, Installed, Updates available
- Package listing with name, version, description, icon
- Install/uninstall/update actions
- Backend: apkg package repositories
- Category filtering

---

### Surf
**Web browser**

| | |
|---|---|
| ID | `com.anyos.surf` |
| Category | Internet |
| Capabilities | filesystem, network, display, dll, event, thread, pipe, shm |

Tabbed web browser:
- HTML rendering with CSS styling (via libwebview)
- HTTP/1.1 client with TLS (BearSSL)
- WebSocket support
- Multiple tabs
- URL bar with navigation (back, forward, reload)
- Bookmarks
- JavaScript execution (via libjs)

---

### Software Update
**System updater**

| | |
|---|---|
| ID | `com.anyos.updater` |
| Category | System |
| Capabilities | filesystem, network, display, dll, event, thread, pipe, shm, process |

Checks for and installs system updates:
- Compares local system manifest against remote repository
- Lists available updates with changelogs
- Download and install with progress bar
- Worker thread for non-blocking downloads
- System file and package updates

---

### Video Player
**MJV video player**

| | |
|---|---|
| ID | `com.anyos.video-player` |
| Category | Media |
| Capabilities | filesystem, audio, dll, event, thread, pipe, shm |

Plays MJV (Motion JPEG Video) files:
- Frame-by-frame JPEG decoding
- Playback controls (play, pause, stop)
- Progress bar with seek
- Loop toggle
- Audio playback support (via audio capability)

---

### VM Manager
**Virtual machine manager**

| | |
|---|---|
| ID | `com.anyos.vmmanager` |
| Category | System |
| Capabilities | filesystem, dll, event, thread, pipe, shm, process |

VMware Workstation-style VM management:
- Create, configure, start, stop VMs
- VGA framebuffer live display (via shared memory)
- CPU and memory usage monitoring
- VM configuration editor (RAM, disk, boot order, HPET)
- Persistent VM configs in `/Users/<user>/VMs/`
- IPC communication with vmd daemon

---

### VNC Settings
**VNC server configuration**

| | |
|---|---|
| ID | `com.anyos.vnc-settings` |
| Category | System |
| Capabilities | filesystem, dll, event, thread, pipe, process, system |

GUI for managing the vncd daemon:
- Start/stop VNC server
- Port configuration (default: 5900)
- Password management
- Allowed user list
- Connection status display

---

### Web Manager
**HTTP server configuration**

| | |
|---|---|
| ID | `com.anyos.webmanager` |
| Category | Utilities |
| Capabilities | filesystem, dll, event, thread, pipe, process, system |

Manage the httpd web server:
- Virtual host / site management
- Document root configuration
- SSL/TLS certificate settings
- URL rewrite rules
- Server start/stop control
- Access log viewer

---

## Capabilities Reference

Each app declares required capabilities in its `Info.conf`. Missing capabilities cause the app to crash at launch.

| Capability | Description | Used by |
|---|---|---|
| `filesystem` | File system access (open, read, write, readdir) | Most apps |
| `network` | TCP/UDP socket access | Surf, anyMail, anyzilla, Updater, Diagnostics |
| `display` | Direct display/screen access | Clock, Screenshot, Installer, Runner, Keyboard |
| `dll` | Dynamic library loading (libanyui, librender, etc.) | All GUI apps |
| `event` | Event system (compositor events, input) | All GUI apps |
| `thread` | Thread creation (Thread::spawn_with_stack) | Most apps |
| `pipe` | Pipe IPC (compositor communication) | Most apps |
| `shm` | Shared memory (window surface buffers) | Most apps |
| `process` | Process management (spawn, kill) | anyCode, Runner, Installer, VM Manager, Store |
| `system` | System-level operations (service control) | Diagnostics, FTP/VNC Settings, Web Manager, Installer |
| `audio` | Audio playback (AC97/HD Audio) | Video Player |

### Common Capability Sets

**Minimal** (demo apps):
```
capabilities=dll,event
```

**Standard** (most apps):
```
capabilities=filesystem,dll,event,thread,pipe,shm
```

**Network app**:
```
capabilities=filesystem,network,display,dll,event,thread,pipe,shm,process
```

**System tool**:
```
capabilities=filesystem,display,system,process,pipe,event,dll,thread,shm,network
```

---

## Info.conf Format

```ini
id=com.anyos.<appname>          # Reverse-domain identifier
name=<Display Name>             # Shown in dock, window title, app store
exec=<Executable Name>          # Binary name inside .app bundle
version=<version>               # Semantic version
category=<Category>             # One of: System, Utilities, Productivity, Internet,
                                #   Graphics, Development, Games, Media
capabilities=<cap1>,<cap2>,...  # Required permissions (comma-separated)
working_dir=bundle              # Optional: set CWD to app bundle directory
```

---

## Building Apps

### Register in CMakeLists.txt

```cmake
add_rust_app(myapp ${CMAKE_SOURCE_DIR}/apps/myapp "My App" "1.0")
```

### Cargo.toml

```toml
[package]
name = "myapp"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libanyui_client = { path = "../../libs/libanyui_client" }

[profile.dev]
panic = "abort"
opt-level = 2

[profile.release]
panic = "abort"
```

### Info.conf

```ini
id=com.anyos.myapp
name=My App
exec=My App
version=1.0
category=Utilities
capabilities=filesystem,dll,event,thread,pipe,shm
```

### Icon

Place a 32x32 or 48x48 ICO file as `Icon.ico` in the app directory. The build system bundles it into the `.app` package automatically.
