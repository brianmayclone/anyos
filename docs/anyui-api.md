# anyui Controls Framework API Reference

The **anyui** framework is a Windows Forms-inspired UI toolkit providing 41 control types for anyOS GUI applications. It consists of a server-side library (**libanyui**, `.so` at `0x04400000`) compiled into the compositor, and a client-side wrapper (**libanyui_client**) that user programs link against.

**Exports:** 112 (C ABI, `#[no_mangle]`)
**Client crate:** `libanyui_client`
**Controls:** 41 types (ControlKind 0–40)
**Symbol resolution:** `dl_open`/`dl_sym` (ELF `.dynsym`/`.hash`)

---

## Table of Contents

- [Getting Started](#getting-started)
- [Architecture](#architecture)
- [Controls Overview](#controls-overview)
- [TextEditor](#texteditor)
- [TreeView](#treeview)
- [Toolbar](#toolbar)
- [DataGrid](#datagrid)
- [Layout Containers](#layout-containers)
- [Events](#events)
- [Syntax Highlighting](#syntax-highlighting)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libanyui_client = { path = "../../libs/libanyui_client" }
```

### Minimal Program

```rust
#![no_std]
#![no_main]

use libanyui_client::*;
anyos_std::entry!(main);

fn main() {
    init();
    let win = Window::new("My App", 400, 300);
    let label = Label::new("Hello, anyui!");
    label.set_position(20, 20);
    win.add(&label);
    run();
}
```

### Widget Trait

All controls implement the `Widget` trait, which provides:

- `id() -> u32` — unique control identifier
- `set_position(x, y)` — set position within parent
- `set_size(w, h)` — set dimensions
- `set_visible(visible)` — show/hide
- `set_color(color)` — set foreground color
- `set_text(text)` — set text content (where applicable)
- `get_text(buf) -> u32` — get text content
- `set_state(state)` / `get_state() -> u32` — get/set numeric state
- `set_padding(left, top, right, bottom)` — inner spacing
- `set_margin(left, top, right, bottom)` — outer spacing
- `set_dock(dock_style)` — docking within parent layout
- `remove()` — remove from parent

### Container Trait

Containers extend Widget and additionally provide:

- `add(child)` — add a child widget

---

## Architecture

### Server-Side (libanyui)

Each control implements the `Control` trait:

```rust
trait Control {
    fn base(&self) -> &ControlBase;
    fn base_mut(&mut self) -> &mut ControlBase;
    fn kind(&self) -> ControlKind;
    fn render(&self, surface: &Surface, ax: i32, ay: i32);
    fn handle_click(&mut self, lx: i32, ly: i32, button: u32) -> EventResponse;
    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse;
    fn handle_scroll(&mut self, delta: i32) -> EventResponse;
    fn handle_focus(&mut self);
    fn handle_blur(&mut self);
    fn is_interactive(&self) -> bool;
    fn accepts_focus(&self) -> bool;
    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>>;
}
```

### Client-Side (libanyui_client)

Uses two macros for boilerplate:

- `leaf_control!(Name, KIND_CONSTANT)` — for leaf controls (no children)
- `container_control!(Name, KIND_CONSTANT)` — for containers with `add()` support

### EventResponse

| Variant | Description |
|---------|-------------|
| `IGNORED` | Event not handled, propagate to parent |
| `CONSUMED` | Event handled, no state change |
| `CLICK` | Event triggers a click callback |
| `CHANGED` | Event causes a value/state change callback |
| `CLICK_AND_CHANGED` | Both click and change callbacks |

### Dock Styles

| Constant | Value | Description |
|----------|-------|-------------|
| `DOCK_NONE` | 0 | Manual positioning (default) |
| `DOCK_TOP` | 1 | Dock to top edge |
| `DOCK_BOTTOM` | 2 | Dock to bottom edge |
| `DOCK_LEFT` | 3 | Dock to left edge |
| `DOCK_RIGHT` | 4 | Dock to right edge |
| `DOCK_FILL` | 5 | Fill remaining space |

---

## Controls Overview

| Kind | Control | Type | Description |
|------|---------|------|-------------|
| 0 | Window | Container | Top-level window |
| 1 | View | Container | Generic container |
| 2 | Label | Leaf | Text label |
| 3 | Button | Leaf | Clickable button |
| 4 | TextField | Leaf | Single-line text input |
| 5 | Toggle | Leaf | On/off switch |
| 6 | Checkbox | Leaf | Checkbox with label |
| 7 | Slider | Leaf | Value slider |
| 8 | RadioButton | Leaf | Radio button |
| 9 | ProgressBar | Leaf | Progress indicator |
| 10 | Stepper | Leaf | Increment/decrement |
| 11 | SegmentedControl | Leaf | Segment selection |
| 12 | TableView | Leaf | Simple table |
| 13 | ScrollView | Container | Scrollable container |
| 14 | Sidebar | Leaf | Navigation sidebar |
| 15 | NavigationBar | Leaf | Top navigation bar |
| 16 | TabBar | Leaf | Tab selection |
| 17 | Toolbar | Container | Horizontal toolbar |
| 18 | Card | Container | Rounded card |
| 19 | GroupBox | Container | Titled group |
| 20 | SplitView | Container | Split pane |
| 21 | Divider | Leaf | Separator line |
| 22 | Alert | Leaf | Alert dialog |
| 23 | ContextMenu | Leaf | Popup menu |
| 24 | Tooltip | Leaf | Tooltip popup |
| 25 | ImageView | Leaf | Image display |
| 26 | StatusIndicator | Leaf | Status dot |
| 27 | ColorWell | Leaf | Color swatch |
| 28 | SearchField | Leaf | Search text input |
| 29 | TextArea | Leaf | Multi-line text |
| 30 | IconButton | Leaf | Icon button |
| 31 | Badge | Leaf | Notification badge |
| 32 | Tag | Leaf | Tag/chip |
| 33 | StackPanel | Container | Stack layout |
| 34 | FlowPanel | Container | Flow layout |
| 35 | TableLayout | Container | Grid layout |
| 36 | Canvas | Leaf | Pixel drawing surface |
| 37 | Expander | Container | Collapsible section |
| 38 | DataGrid | Leaf | Data grid/spreadsheet |
| 39 | TextEditor | Leaf | Code editor |
| 40 | TreeView | Leaf | Hierarchical tree |

---

## TextEditor

Multi-line code editor with syntax highlighting, line numbers, auto-indent, and smooth scrolling. Designed for editing source code with configurable syntax definition files.

**ControlKind:** 39
**C API exports:** 11 (`anyui_texteditor_*`)

### Features

- Token-based syntax highlighting via `.syn` definition files
- Configurable colors for keywords, types, builtins, strings, comments, numbers, operators
- Line numbers with auto-sizing gutter
- Current line highlighting
- Auto-indent on Enter (copies leading whitespace from previous line)
- Tab key inserts configurable number of spaces (default 4)
- Vertical scrollbar for long files
- Monospace font (Andale Mono, font_id=4) by default
- Click-to-position cursor
- Keyboard navigation: arrows, Home/End, Page Up/Down

### Client API

```rust
use libanyui_client::TextEditor;

// Create
let editor = TextEditor::new(600, 400);             // empty editor
let editor = TextEditor::from_file("/path/to/file.rs", 600, 400);  // load from file

// Text content
editor.set_text("fn main() {\n    println!(\"Hello\");\n}");
editor.set_text_bytes(raw_bytes);
let len = editor.get_text(&mut buf);                 // returns bytes written

// Syntax highlighting
editor.load_syntax("/System/syntax/rust.syn");       // load from .syn file
editor.load_syntax_from_bytes(syn_data);             // load from bytes

// Cursor
editor.set_cursor(0, 5);                             // row=0, col=5
let (row, col) = editor.cursor();

// Configuration
editor.set_line_height(20);                          // default 20
editor.set_tab_width(4);                             // spaces per Tab
editor.set_show_line_numbers(true);                  // default true
editor.set_editor_font(4, 13);                       // font_id, size

// Editing
editor.insert_text("inserted text");                 // at cursor position
let lines = editor.line_count();

// Events
editor.on_text_changed(|e| {
    let text = e.text();                             // get current content
});
```

### C API Exports

| Export | Signature | Description |
|--------|-----------|-------------|
| `anyui_texteditor_set_text` | `(id: u32, ptr: *const u8, len: u32)` | Set editor text content |
| `anyui_texteditor_get_text` | `(id: u32, buf: *mut u8, cap: u32) -> u32` | Get text into buffer, returns length |
| `anyui_texteditor_set_syntax` | `(id: u32, ptr: *const u8, len: u32)` | Load syntax definition from bytes |
| `anyui_texteditor_set_cursor` | `(id: u32, row: u32, col: u32)` | Set cursor position |
| `anyui_texteditor_get_cursor` | `(id: u32, row: *mut u32, col: *mut u32)` | Get cursor position |
| `anyui_texteditor_set_line_height` | `(id: u32, height: u32)` | Set line height (min 12) |
| `anyui_texteditor_set_tab_width` | `(id: u32, width: u32)` | Set tab width in spaces (min 1) |
| `anyui_texteditor_set_show_line_numbers` | `(id: u32, show: u32)` | Show/hide line numbers (0/1) |
| `anyui_texteditor_set_font` | `(id: u32, font_id: u32, size: u32)` | Set font and size |
| `anyui_texteditor_insert_text` | `(id: u32, ptr: *const u8, len: u32)` | Insert text at cursor |
| `anyui_texteditor_get_line_count` | `(id: u32) -> u32` | Get number of lines |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Printable chars | Insert at cursor |
| Enter | New line with auto-indent |
| Backspace | Delete before cursor (merges lines) |
| Delete | Delete after cursor (merges lines) |
| Tab | Insert tab_width spaces |
| Left/Right | Move cursor (wraps between lines) |
| Up/Down | Move cursor vertically |
| Home | Move to line start |
| End | Move to line end |
| Page Up/Down | Scroll by viewport height |

### Example: Code Editor Application

```rust
#![no_std]
#![no_main]

use libanyui_client::*;
anyos_std::entry!(main);

fn main() {
    init();
    let win = Window::new("Code Editor", 800, 600);

    // Toolbar
    let toolbar = Toolbar::new();
    toolbar.set_dock(DOCK_TOP);
    toolbar.set_size(0, 36);
    toolbar.set_padding(4, 4, 4, 4);
    let save_btn = toolbar.add_button("Save");
    toolbar.add_separator();
    let line_lbl = toolbar.add_label("Line 1, Col 1");
    win.add(&toolbar);

    // Editor fills remaining space
    let editor = TextEditor::new(800, 564);
    editor.set_dock(DOCK_FILL);
    editor.load_syntax("/System/syntax/rust.syn");
    editor.on_text_changed(move |_| {
        // update status
    });
    win.add(&editor);

    run();
}
```

---

## TreeView

Hierarchical tree control with expandable/collapsible nodes, per-node icons, text styles, and keyboard navigation.

**ControlKind:** 40
**C API exports:** 14 (`anyui_treeview_*`)

### Features

- Flat node storage with parent index references and cached depth
- Disclosure triangles (▶/▼) for expand/collapse
- Per-node ARGB icon pixels
- Per-node text style (normal, bold) and custom text color
- Selection highlight with accent color, hover highlight
- Keyboard navigation: Up/Down through visible nodes, Left=collapse/parent, Right=expand/child
- Auto-scroll to keep selection visible
- Node removal with automatic descendant cleanup and index fixup

### Node Style Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `STYLE_NORMAL` | 0 | Default text style |
| `STYLE_BOLD` | 1 | Bold text |

### Client API

```rust
use libanyui_client::{TreeView, treeview};

// Create
let tree = TreeView::new(250, 400);

// Add nodes
let root = tree.add_root("Project");                      // root-level node
let src = tree.add_child(root, "src");                    // child of root
let main_rs = tree.add_child(src, "main.rs");

// Node properties
tree.set_node_text(root, "My Project");
tree.set_node_style(root, treeview::STYLE_BOLD);
tree.set_node_text_color(main_rs, 0xFF4ECDC4);           // teal

// Icons (ARGB pixel data)
tree.set_node_icon(root, &pixels, 16, 16);
tree.set_node_icon_from_file(src, "/System/icons/folder.ico", 16);

// Expand/collapse
tree.set_expanded(root, true);
let expanded = tree.is_expanded(root);

// Selection
tree.set_selected(main_rs);
let sel = tree.selected();                                // u32::MAX if none

// Management
tree.remove_node(src);                                    // removes src + main_rs
let count = tree.node_count();
tree.clear();                                             // remove all nodes

// Configuration
tree.set_indent_width(20);                                // pixels per depth level
tree.set_row_height(24);                                  // row height in pixels

// Events
tree.on_selection_changed(|e| {
    let node_index = e.index;                             // selected node index
});
tree.on_node_clicked(|e| {
    // node was clicked or Enter pressed
});
```

### C API Exports

| Export | Signature | Description |
|--------|-----------|-------------|
| `anyui_treeview_add_node` | `(id: u32, parent: u32, ptr: *const u8, len: u32) -> u32` | Add node (parent=MAX for root), returns index |
| `anyui_treeview_remove_node` | `(id: u32, index: u32)` | Remove node and descendants |
| `anyui_treeview_set_node_text` | `(id: u32, index: u32, ptr: *const u8, len: u32)` | Set node text |
| `anyui_treeview_set_node_icon` | `(id: u32, index: u32, pixels: *const u32, w: u32, h: u32)` | Set node ARGB icon |
| `anyui_treeview_set_node_style` | `(id: u32, index: u32, style: u32)` | Set node style bits |
| `anyui_treeview_set_node_text_color` | `(id: u32, index: u32, color: u32)` | Set node text color (0=default) |
| `anyui_treeview_set_expanded` | `(id: u32, index: u32, expanded: u32)` | Set expand/collapse (0/1) |
| `anyui_treeview_get_expanded` | `(id: u32, index: u32) -> u32` | Get expand state |
| `anyui_treeview_get_selected` | `(id: u32) -> u32` | Get selected index (MAX=none) |
| `anyui_treeview_set_selected` | `(id: u32, index: u32)` | Set selected node |
| `anyui_treeview_clear` | `(id: u32)` | Remove all nodes |
| `anyui_treeview_get_node_count` | `(id: u32) -> u32` | Get total node count |
| `anyui_treeview_set_indent_width` | `(id: u32, width: u32)` | Set indent per depth level |
| `anyui_treeview_set_row_height` | `(id: u32, height: u32)` | Set row height |

### Keyboard Navigation

| Key | Action |
|-----|--------|
| Up | Select previous visible node |
| Down | Select next visible node |
| Left | Collapse node, or move to parent |
| Right | Expand node, or move to first child |
| Enter | Fire click event on selected node |

### Example: File Browser Tree

```rust
#![no_std]
#![no_main]

use libanyui_client::*;
anyos_std::entry!(main);

fn main() {
    init();
    let win = Window::new("File Browser", 300, 500);

    let tree = TreeView::new(280, 480);
    tree.set_position(10, 10);

    // Build tree structure
    let root = tree.add_root("Documents");
    tree.set_node_style(root, treeview::STYLE_BOLD);
    tree.set_node_icon_from_file(root, "/System/icons/folder.ico", 16);

    let photos = tree.add_child(root, "Photos");
    tree.set_node_icon_from_file(photos, "/System/icons/folder.ico", 16);
    tree.add_child(photos, "vacation.jpg");
    tree.add_child(photos, "profile.png");

    let code = tree.add_child(root, "Code");
    tree.set_node_icon_from_file(code, "/System/icons/folder.ico", 16);
    tree.add_child(code, "main.rs");
    tree.add_child(code, "lib.rs");

    tree.on_selection_changed(|e| {
        anyos_std::println!("Selected node: {}", e.index);
    });

    win.add(&tree);
    run();
}
```

---

## Toolbar

Enhanced horizontal toolbar container that lays out children left-to-right with configurable spacing. Provides convenience methods for adding common toolbar items.

**ControlKind:** 17
**Layout:** Horizontal left-to-right with spacing

### Features

- Automatic horizontal layout of children with configurable spacing (default 4px)
- Respects padding for inner spacing
- Dark background (0xFF2C2C2E) with 1px bottom separator
- Convenience methods for adding buttons, labels, separators, and icon buttons
- Children auto-sized to toolbar height minus padding

### Client API

```rust
use libanyui_client::Toolbar;

// Create
let toolbar = Toolbar::new();
toolbar.set_dock(DOCK_TOP);
toolbar.set_size(0, 36);
toolbar.set_padding(4, 4, 4, 4);

// Add items via convenience methods
let new_btn = toolbar.add_button("New");
let open_btn = toolbar.add_button("Open");
toolbar.add_separator();                    // 1px vertical divider, 16px high
let status = toolbar.add_label("Ready");
let icon = toolbar.add_icon_button("X");    // icon text

// Event handling
new_btn.on_click(|_| {
    // handle new
});

// Add to window
win.add(&toolbar);
```

### Convenience Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `add_button(text)` | `Button` | Create and add a labeled button |
| `add_label(text)` | `Label` | Create and add a text label |
| `add_separator()` | `Divider` | Create and add a 1x16 vertical divider |
| `add_icon_button(icon_text)` | `IconButton` | Create and add an icon button |

---

## DataGrid

Spreadsheet-style data grid with column headers, sortable columns, cell-level text and colors, and multi-row selection.

**ControlKind:** 38
**C API exports:** 16 (`anyui_datagrid_*`)

### Client API

```rust
use libanyui_client::DataGrid;

let grid = DataGrid::new(600, 300);

// Columns (pipe-separated header string)
grid.set_columns("Name|Size|Modified");
grid.set_column_width(0, 200);
grid.set_column_width(1, 100);

// Data
grid.set_row_count(100);
grid.set_cell(0, 0, "file.txt");
grid.set_cell(0, 1, "4.2 KB");
grid.set_cell_colors(0, 0, 0xFFE6E6E6, 0xFF1E1E1E);  // fg, bg

// Selection
grid.set_selection_mode(1);                  // 0=none, 1=single, 2=multi
let sel = grid.get_selected_row();

// Sort
grid.sort(1, true);                          // column 1, ascending

// Events
grid.on_selection_changed(|e| { /* ... */ });
```

---

## Layout Containers

### StackPanel (Kind 33)

Arranges children in a single row or column.

```rust
let stack = StackPanel::new();
stack.set_orientation(ORIENTATION_VERTICAL);  // or ORIENTATION_HORIZONTAL
stack.add(&label);
stack.add(&button);
```

### FlowPanel (Kind 34)

Arranges children left-to-right with wrapping.

### TableLayout (Kind 35)

Grid layout with rows and columns.

```rust
let table = TableLayout::new();
table.set_columns_count(3);  // 3-column grid
```

### Expander (Kind 37)

Collapsible section with header.

```rust
let exp = Expander::new("Advanced Settings");
exp.add(&checkbox);
exp.add(&slider);
```

---

## Events

### Event Types

Events are registered via callbacks on the client side:

```rust
// Click event (buttons, icon buttons, tags, canvas)
button.on_click(|e: &ClickEvent| {
    let control_id = e.id;
});

// Value/state change event (sliders, text fields, toggles, data grid, tree view)
slider.on_value_changed(|e: &ValueChangedEvent| {
    let new_value = e.value;
});

// Text change event (text fields, text area, text editor)
editor.on_text_changed(|e: &TextChangedEvent| {
    let text = e.text();  // queries current content
});

// Selection change event (data grid, tree view)
tree.on_selection_changed(|e: &SelectionChangedEvent| {
    let index = e.index;
});
```

### Event Structs

| Struct | Fields | Fired by |
|--------|--------|----------|
| `ClickEvent` | `id: u32` | Button, IconButton, Tag, Canvas, TreeView |
| `ValueChangedEvent` | `id: u32, value: u32` | Slider, Stepper, Toggle, Checkbox |
| `TextChangedEvent` | `id: u32` + `text()` method | TextField, SearchField, TextArea, TextEditor |
| `SelectionChangedEvent` | `id: u32, index: u32` | DataGrid, TreeView |

---

## Frame Pacing & VSync

The anyui event loop implements automatic VSync-driven frame pacing with back-pressure, modeled after Windows DWM and macOS CVDisplayLink. Applications using anyui benefit from this without any code changes.

### How It Works

1. **Present + track**: When anyui renders a dirty window, it calls `present()` and sets a `frame_presented` flag on the window
2. **Back-pressure**: On the next loop iteration, windows with `frame_presented = true` are skipped (their previous frame hasn't been composited yet)
3. **ACK receipt**: When the compositor's render thread finishes compositing, it emits `EVT_FRAME_ACK` (0x300B). anyui clears `frame_presented`, allowing the next frame to render
4. **Safety timeout**: If an ACK is not received within 64ms (4 frames), the flag is forcibly cleared to prevent stalls from lost events

### Adaptive Event Loop

The `run()` function uses adaptive sleep timing instead of a fixed 16ms interval:

| State | Sleep interval | Reason |
|-------|---------------|--------|
| Frame pending (waiting for ACK) | 2ms | Fast polling to receive ACK quickly |
| Idle (no pending frames) | 16ms | Normal frame pacing, saves CPU |

This means:
- **Active rendering**: ~2ms ACK turnaround → smooth 60fps with minimal latency
- **Idle state**: ~16ms sleep → near-zero CPU usage when nothing is happening
- **No wasted frames**: Apps never render faster than the compositor can composite

### Latency

The end-to-end latency from `present()` to receiving `EVT_FRAME_ACK` is typically **4-9ms**, compared to **18-53ms** with the previous fixed-timer approach. Window dragging, scrolling, and animations are noticeably smoother.

### For Custom Event Loops

If you build a custom event loop instead of using `run()`, you can handle `EVT_FRAME_ACK` manually:

```rust
// In your event polling loop:
match event_type {
    0x300B => {
        // Frame ACK received — safe to present next frame
        frame_pending = false;
    }
    // ... other events ...
}
```

See the [libcompositor API](libcompositor-api.md#evt_frame_ack--vsync-callback) for the low-level protocol details.

---

## Syntax Highlighting

TextEditor uses a simple key=value `.syn` file format for syntax definitions.

### File Format

```
keywords=if,else,while,for,fn,let,mut,return,match,struct,enum,impl,pub,use,mod,const,static,trait,type,where,async,await,loop,break,continue,in,as,ref,move,self,super,crate,unsafe,extern,dyn,true,false
types=u8,u16,u32,u64,i8,i16,i32,i64,f32,f64,bool,usize,isize,str,String,Vec,Option,Result,Box,Self
builtins=println,print,format,panic,assert,vec,todo,unimplemented,unreachable,dbg,include,cfg
line_comment=//
block_comment_start=/*
block_comment_end=*/
string_delimiters="
char_delimiter='
keyword_color=0xFFFF6B6B
type_color=0xFF4ECDC4
builtin_color=0xFFDCDCAA
string_color=0xFFE2B93D
comment_color=0xFF6A737D
number_color=0xFF9B59B6
operator_color=0xFF56B6C2
```

### Token Categories

| Category | Default Color | Description |
|----------|--------------|-------------|
| Keywords | `0xFFFF6B6B` (red) | Language keywords |
| Types | `0xFF4ECDC4` (teal) | Type names |
| Builtins | `0xFFDCDCAA` (yellow) | Built-in functions/macros |
| Strings | `0xFFE2B93D` (gold) | String and char literals |
| Comments | `0xFF6A737D` (gray) | Line and block comments |
| Numbers | `0xFF9B59B6` (purple) | Numeric literals (decimal, hex, float) |
| Operators | `0xFF56B6C2` (cyan) | Operators and punctuation |
| Default | `0xFFE6E6E6` (white) | Identifiers and other text |

### Tokenizer Behavior

- Scans left-to-right through each line
- Handles escape sequences in strings (`\"`, `\\`)
- Tracks block comment state across lines
- Recognizes hex literals (`0xFF`), float literals (`3.14`), type suffixes (`42u32`)
- Identifiers matched against keyword/type/builtin lists (exact match)

### Creating a Syntax File

To add syntax highlighting for a new language, create a `.syn` file:

1. List keywords, types, and builtins as comma-separated values
2. Set comment delimiters (line and/or block)
3. Set string/char delimiters
4. Optionally customize colors (ARGB hex format: `0xAARRGGBB`)

Place syntax files in `/System/syntax/` on the disk for system-wide access.
