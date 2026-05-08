# anyui Controls Framework API Reference

The **anyui** framework is a Windows Forms-inspired UI toolkit providing 44 control types for anyOS GUI applications. It consists of a server-side library (**libanyui**, `.so` at `0x04400000`) compiled into the compositor, and a client-side wrapper (**libanyui_client**) that user programs link against.

**Exports:** 191 (C ABI, `#[no_mangle]`)
**Client crate:** `libanyui_client`
**Controls:** 44 types (ControlKind 0-43)
**Symbol resolution:** `dl_open`/`dl_sym` (ELF `.dynsym`/`.hash`)

---

## Table of Contents

- [Getting Started](#getting-started)
- [Architecture](#architecture)
- [Constants Reference](#constants-reference)
- [Control Base Class](#control-base-class)
- [Container Base Class](#container-base-class)
- [Controls Reference](#controls-reference)
  - [Window](#window)
  - [View](#view)
  - [Label](#label)
  - [Button](#button)
  - [TextField](#textfield)
  - [TextArea](#textarea)
  - [Toggle](#toggle)
  - [Checkbox](#checkbox)
  - [RadioButton](#radiobutton)
  - [Slider](#slider)
  - [ProgressBar](#progressbar)
  - [Stepper](#stepper)
  - [SegmentedControl](#segmentedcontrol)
  - [Divider](#divider)
  - [ImageView](#imageview)
  - [StatusIndicator](#statusindicator)
  - [ColorWell](#colorwell)
  - [SearchField](#searchfield)
  - [IconButton](#iconbutton)
  - [Badge](#badge)
  - [Tag](#tag)
  - [Canvas](#canvas)
  - [DataGrid](#datagrid)
  - [TextEditor](#texteditor)
  - [TreeView](#treeview)
  - [ImageButton](#imagebutton)
  - [DropDown](#dropdown)
  - [AutoCompleteTextField](#autocompletetextfield)
- [Container Controls](#container-controls)
  - [Card](#card)
  - [GroupBox](#groupbox)
  - [SplitView](#splitview)
  - [ScrollView](#scrollview)
  - [Sidebar](#sidebar)
  - [NavigationBar](#navigationbar)
  - [TabBar](#tabbar)
  - [Toolbar](#toolbar)
  - [Alert](#alert)
  - [ContextMenu](#contextmenu)
  - [TableView](#tableview)
  - [Expander](#expander)
  - [Tooltip](#tooltip)
  - [StackPanel](#stackpanel)
  - [FlowPanel](#flowpanel)
  - [TableLayout](#tablelayout)
  - [RadioGroup](#radiogroup)
- [System Components](#system-components)
  - [TrayIcon](#trayicon)
  - [MenuBar](#menubar)
- [Dialogs](#dialogs)
- [Icon System](#icon-system)
- [Events Reference](#events-reference)
- [Layout System](#layout-system)
- [Timer API](#timer-api)
- [Marshal API (Cross-Thread)](#marshal-api)
- [Clipboard API](#clipboard-api)
- [Theme API](#theme-api)
- [Key Constants](#key-constants)
- [Window Management](#window-management)
- [Fullscreen API](#fullscreen-api)
- [Modal Dialogs](#modal-dialogs)
- [Display & Scaling](#display--scaling)
- [Text Measurement](#text-measurement)
- [Window Lifecycle](#window-lifecycle)
- [Utilities](#utilities)
- [Syntax Highlighting](#syntax-highlighting)
- [Frame Pacing & VSync](#frame-pacing--vsync)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libanyui_client = { path = "../../libs/libanyui_client" }
```

### Minimal Program

```rust
#![no_std]
#![no_main]

use libanyui_client as anyui;
anyos_std::entry!(main);

fn main() {
    if !anyui::init() { return; }
    let win = anyui::Window::new("My App", -1, -1, 400, 300);
    let label = anyui::Label::new("Hello, anyui!");
    label.set_position(20, 20);
    win.add(&label);
    win.on_close(|_| { anyui::quit(); });
    anyui::run();
}
```

### Lifecycle Functions

```rust
fn init() -> bool           // Load libanyui.so, resolve symbols. Returns false on failure.
fn run()                    // Blocking event loop. Returns when quit() is called.
fn run_once() -> bool       // Process one event cycle. Returns false when quitting.
fn quit()                   // Signal event loop to stop.
fn shutdown()               // Clean up resources.
```

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
    fn is_interactive(&self) -> bool;
    fn accepts_focus(&self) -> bool;
    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>>;
}
```

### Client-Side (libanyui_client)

Uses two macros for boilerplate:

- `leaf_control!(Name, KIND_CONSTANT)` -- for leaf controls (no children)
- `container_control!(Name, KIND_CONSTANT)` -- for containers with `add()` support

### EventResponse

| Variant | Description |
|---------|-------------|
| `IGNORED` | Event not handled, propagate to parent |
| `CONSUMED` | Event handled, no state change |
| `CLICK` | Event triggers a click callback |
| `CHANGED` | Event causes a value/state change callback |
| `CLICK_AND_CHANGED` | Both click and change callbacks |

---

## Constants Reference

### Dock Layout (DOCK_*)

```rust
DOCK_NONE               = 0    // Absolute positioning via set_position()
DOCK_TOP                = 1    // Top edge, stretches full width
DOCK_BOTTOM             = 2    // Bottom edge, stretches full width
DOCK_LEFT               = 3    // Left edge, stretches full height
DOCK_RIGHT              = 4    // Right edge, stretches full height
DOCK_FILL               = 5    // Fills remaining space (add last)
```

### Orientation (ORIENTATION_*)

```rust
ORIENTATION_VERTICAL    = 0
ORIENTATION_HORIZONTAL  = 1
```

### Text Alignment (TEXT_ALIGN_*)

```rust
TEXT_ALIGN_LEFT          = 0
TEXT_ALIGN_CENTER        = 1
TEXT_ALIGN_RIGHT         = 2
```

### Column Alignment (ALIGN_*)

Used with DataGrid ColumnDef:

```rust
ALIGN_LEFT              = 0 (u8)
ALIGN_CENTER            = 1 (u8)
ALIGN_RIGHT             = 2 (u8)
```

### Window Flags (WIN_FLAG_*)

OR-able flags for `Window::new_with_flags()`:

```rust
WIN_FLAG_BORDERLESS     = 0x01    // No title bar/border
WIN_FLAG_NOT_RESIZABLE  = 0x02    // Fixed size
WIN_FLAG_ALWAYS_ON_TOP  = 0x04    // Stays above other windows
WIN_FLAG_NO_CLOSE       = 0x08    // Hide close button
WIN_FLAG_NO_MINIMIZE    = 0x10    // Hide minimize button
WIN_FLAG_NO_MAXIMIZE    = 0x20    // Hide maximize button
WIN_FLAG_SHADOW         = 0x40    // Draw drop shadow
```

### DataGrid Constants

```rust
SELECTION_SINGLE        = 0
SELECTION_MULTI         = 1
SORT_NONE               = 0
SORT_ASCENDING          = 1
SORT_DESCENDING         = 2
SORT_STRING             = 0 (u8)    // Lexicographic
SORT_NUMERIC            = 1 (u8)    // Numeric
```

### Icon Constants (ICON_*)

```rust
ICON_NEW_FILE           = 1
ICON_FOLDER_OPEN        = 2
ICON_SAVE               = 3
ICON_SAVE_ALL           = 4
ICON_BUILD              = 5
ICON_PLAY               = 6
ICON_STOP               = 7
ICON_SETTINGS           = 8
ICON_FILES              = 9
ICON_GIT_BRANCH         = 10
ICON_SEARCH             = 11
ICON_REFRESH            = 12
```

### ImageView Scale Mode (SCALE_*)

```rust
SCALE_NONE              = 0    // No scaling
SCALE_FIT               = 1    // Fit within bounds (maintain aspect)
SCALE_FILL              = 2    // Fill bounds (may crop)
SCALE_STRETCH           = 3    // Stretch to fill (distorts)
```

### TreeView Node Style (STYLE_*)

```rust
STYLE_NORMAL            = 0
STYLE_BOLD              = 1
```

### Control Kind (KIND_*)

```rust
KIND_WINDOW = 0, KIND_VIEW = 1, KIND_LABEL = 2, KIND_BUTTON = 3,
KIND_TEXTFIELD = 4, KIND_TOGGLE = 5, KIND_CHECKBOX = 6, KIND_SLIDER = 7,
KIND_RADIO_BUTTON = 8, KIND_PROGRESS_BAR = 9, KIND_STEPPER = 10,
KIND_SEGMENTED = 11, KIND_TABLE_VIEW = 12, KIND_SCROLL_VIEW = 13,
KIND_SIDEBAR = 14, KIND_NAVIGATION_BAR = 15, KIND_TAB_BAR = 16,
KIND_TOOLBAR = 17, KIND_CARD = 18, KIND_GROUP_BOX = 19, KIND_SPLIT_VIEW = 20,
KIND_DIVIDER = 21, KIND_ALERT = 22, KIND_CONTEXT_MENU = 23, KIND_TOOLTIP = 24,
KIND_IMAGE_VIEW = 25, KIND_STATUS_INDICATOR = 26, KIND_COLOR_WELL = 27,
KIND_SEARCH_FIELD = 28, KIND_TEXT_AREA = 29, KIND_ICON_BUTTON = 30,
KIND_BADGE = 31, KIND_TAG = 32, KIND_STACK_PANEL = 33, KIND_FLOW_PANEL = 34,
KIND_TABLE_LAYOUT = 35, KIND_CANVAS = 36, KIND_EXPANDER = 37,
KIND_DATA_GRID = 38, KIND_TEXT_EDITOR = 39, KIND_TREE_VIEW = 40,
KIND_RADIO_GROUP = 41, KIND_DROP_DOWN = 42, KIND_AUTO_COMPLETE_TEXT_FIELD = 43
```

---

## Control Base Class

All controls inherit from `Control`. Methods available on every control via `Deref`:

### Position & Size

```rust
fn set_position(&self, x: i32, y: i32)
fn set_size(&self, w: u32, h: u32)
fn set_auto_size(&self, enabled: bool)
fn set_min_size(&self, min_w: u32, min_h: u32)
fn set_max_size(&self, max_w: u32, max_h: u32)
```

### Visibility & State

```rust
fn set_visible(&self, visible: bool)     // Hidden controls receive no events
fn set_enabled(&self, enabled: bool)     // Disabled = non-interactive + dimmed
fn set_state(&self, value: u32)          // Numeric state (slider pos, toggle, icon ID)
fn get_state(&self) -> u32
```

### Color & Text

```rust
fn set_color(&self, color: u32)          // ARGB background color
fn set_text_color(&self, color: u32)     // ARGB text color
fn set_text(&self, text: &str)
fn get_text(&self, buf: &mut [u8]) -> u32  // Returns bytes written
```

### Font

```rust
fn set_font_size(&self, size: u32)
fn get_font_size(&self) -> u32
fn set_font(&self, font_id: u32)
```

### Layout

```rust
fn set_dock(&self, dock_style: u32)      // DOCK_NONE..DOCK_FILL
fn set_padding(&self, left: i32, top: i32, right: i32, bottom: i32)
fn set_margin(&self, left: i32, top: i32, right: i32, bottom: i32)
```

### Queries

```rust
fn get_size(&self) -> (u32, u32)         // Returns (width, height)
fn get_position(&self) -> (i32, i32)     // Returns (x, y)
```

### Tooltip

```rust
fn set_tooltip(&self, text: &str)        // Shown on hover
```

### Focus & Misc

```rust
fn focus(&self)                          // Set keyboard focus
fn set_tab_index(&self, index: u32)
fn set_context_menu(&self, menu: &impl Widget)
fn bring_to_front(&self)                 // Move to end of parent's child list (render on top)
fn remove(&self)                         // Remove from parent
fn from_id(id: u32) -> Self             // Wrap existing control ID
fn id(&self) -> u32                      // Get control ID
```

---

## Container Base Class

Extends Control with:

```rust
fn add(&self, child: &impl Widget)               // Add child control
fn remove_child(&self, child: &impl Widget)      // Remove and destroy a specific child (including descendants)
fn clear(&self)                                  // Remove and destroy ALL children (container preserved)
```

---

## Controls Reference

### Window

Top-level window container.

```rust
Window::new(title: &str, x: i32, y: i32, w: u32, h: u32) -> Self
Window::new_with_flags(title: &str, x: i32, y: i32, w: u32, h: u32, flags: u32) -> Self
// x, y = -1 for auto-placement

fn set_title(&self, title: &str)         // Change window title after creation
fn destroy(&self)
fn resize(&self, w: u32, h: u32)        // Programmatically resize (SHM buffer + control)
fn move_to(&self, x: i32, y: i32)       // Move window to new screen position
fn minimize(&self)                       // Minimize (hide off-screen, restore via dock)
fn set_modal(&self, owner: &Window)      // Make this a modal child of owner (blocks owner input)
fn set_cursor_visible(&self, visible: bool)  // Hide/show mouse cursor for this window
fn set_fullscreen_capable(&self, auto_enter: bool)  // Mark window as fullscreen-capable
fn on_close(&self, f: impl FnMut(&EventArgs) + 'static)
fn on_resize(&self, f: impl FnMut(&EventArgs) + 'static)
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)   // Window background click
fn on_key_down(&self, f: impl FnMut(&KeyEvent) + 'static)  // Unhandled key events that bubble up
fn on_key_up(&self, f: impl FnMut(&KeyEvent) + 'static)
fn on_fullscreen_enter(&self, f: impl FnMut(&EventArgs) + 'static)
fn on_fullscreen_exit(&self, f: impl FnMut(&EventArgs) + 'static)
```

### View

Generic container for layout purposes.

```rust
View::new() -> Self
```

No specific methods. Use `set_color()`, `set_size()`, `set_dock()`, `set_visible()`, and `add()`.

### Label

Text display.

```rust
Label::new(text: &str) -> Self
fn set_text_align(&self, align: u32)  // TEXT_ALIGN_LEFT/CENTER/RIGHT
```

### Button

Clickable button.

```rust
Button::new(text: &str) -> Self
fn auto_size(&self)    // Automatically size button to fit text
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
```

**Auto-sizing:** Call `auto_size()` to automatically resize the button's width to fit its text content with 12px padding on each side.

```rust
let btn = Button::new("Click Me");
btn.auto_size();  // Button width = text_width + 24px
win.add(&btn);
```

Without `auto_size()`, you must manually set the button size with `set_size(w, h)`.

### TextField

Single-line text input.

```rust
TextField::new() -> Self
fn set_placeholder(&self, text: &str)
fn set_prefix_icon(&self, icon_code: u32)
fn set_postfix_icon(&self, icon_code: u32)
fn set_password_mode(&self, enabled: bool)
fn select_all(&self)                              // Select all text in the field
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
fn on_submit(&self, f: impl FnMut(&SubmitEvent) + 'static)   // Enter key
```

Use `set_text()` / `get_text()` from Control base to read/write content.

### TextArea

Multi-line text input.

```rust
TextArea::new() -> Self
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
```

### Toggle

On/off switch. State: 0=off, non-zero=on.

```rust
Toggle::new(on: bool) -> Self
fn on_checked_changed(&self, f: impl FnMut(&CheckedChangedEvent) + 'static)
```

### Checkbox

Checkbox with label. State: 0=unchecked, non-zero=checked.

```rust
Checkbox::new(label: &str) -> Self
fn on_checked_changed(&self, f: impl FnMut(&CheckedChangedEvent) + 'static)
```

### RadioButton

Radio button with label. Mutual exclusion not enforced by widget.

```rust
RadioButton::new(label: &str) -> Self
fn on_checked_changed(&self, f: impl FnMut(&CheckedChangedEvent) + 'static)
```

### Slider

Value slider (0-100).

```rust
Slider::new(value: u32) -> Self
fn on_value_changed(&self, f: impl FnMut(&ValueChangedEvent) + 'static)
```

Use `get_state()` / `set_state()` for current value.

### ProgressBar

Progress display (0-100). Non-interactive.

```rust
ProgressBar::new(value: u32) -> Self
```

Use `set_state(value)` to update.

### Stepper

Increment/decrement spin box.

```rust
Stepper::new() -> Self
fn on_value_changed(&self, f: impl FnMut(&ValueChangedEvent) + 'static)
```

### SegmentedControl

Multi-segment selector.

```rust
SegmentedControl::new(labels: &str) -> Self   // Pipe-separated: "Tab 1|Tab 2|Tab 3"
fn connect_panels(&self, panels: &[&impl Widget])  // Auto-switch panel visibility
fn on_active_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

### Divider

Visual separator line.

```rust
Divider::new() -> Self
```

### ImageView

Image display. Supports BMP, PNG, JPEG, GIF, ICO.

```rust
ImageView::new(w: u32, h: u32) -> Self
ImageView::from_file(path: &str, w: u32, h: u32) -> Self
ImageView::from_bytes(data: &[u8], w: u32, h: u32) -> Self

fn load_from_bytes(&self, data: &[u8])
fn load_from_file(&self, path: &str)
fn load_ico(&self, path: &str, preferred_size: u32)
fn set_pixels(&self, pixels: &[u32], w: u32, h: u32)  // Raw ARGB
fn set_scale_mode(&self, mode: u32)   // SCALE_NONE/FIT/FILL/STRETCH
fn image_size(&self) -> (u32, u32)
fn clear(&self)
```

### StatusIndicator

Status dot with label.

```rust
StatusIndicator::new(label: &str) -> Self
```

Use `set_color()` for indicator color.

### ColorWell

Color picker control.

```rust
ColorWell::new() -> Self
fn set_selected_color(&self, color: u32)    // ARGB
fn get_selected_color(&self) -> u32
fn on_color_selected(&self, f: impl FnMut(&ColorSelectedEvent) + 'static)
```

### SearchField

Search input with icon.

```rust
SearchField::new() -> Self
fn set_placeholder(&self, text: &str)
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
fn on_submit(&self, f: impl FnMut(&SubmitEvent) + 'static)
```

### IconButton

Button with built-in icon. Supports legacy pixel-art icons (ICON_* constants) and system SVG icons from ico.pak.

```rust
IconButton::new(icon_text: &str) -> Self
fn auto_size(&self)                         // Automatically size button to fit icon + text
fn set_icon(&self, icon_id: u32)            // Legacy ICON_* constants
fn set_system_icon(&self, name: &str, icon_type: IconType, color: u32, size: u32)
                                            // Render SVG from ico.pak (6000+ Tabler Icons)
fn set_pixels(&self, pixels: &[u32], w: u32, h: u32)  // Raw ARGB pixel data
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
```

Example with system icons:

```rust
let btn = toolbar.add_icon_button("");
btn.set_size(34, 34);
btn.set_system_icon("device-floppy", IconType::Outline, 0xFFCCCCCC, 24);
btn.set_tooltip("Save");
```

### Badge

Notification badge (non-interactive).

```rust
Badge::new(text: &str) -> Self
```

### Tag

Clickable tag/chip.

```rust
Tag::new(text: &str) -> Self
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
```

### Canvas

Pixel drawing surface with full drawing primitives.

```rust
Canvas::new(w: u32, h: u32) -> Self

// Drawing primitives
fn set_pixel(&self, x: i32, y: i32, color: u32)
fn get_pixel(&self, x: i32, y: i32) -> u32
fn clear(&self, color: u32)
fn fill_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32)
fn draw_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32)
fn draw_line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32)
fn draw_thick_line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32, thickness: u32)
fn draw_circle(&self, cx: i32, cy: i32, radius: i32, color: u32)
fn fill_circle(&self, cx: i32, cy: i32, radius: i32, color: u32)
fn draw_ellipse(&self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32)
fn fill_ellipse(&self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32)
fn flood_fill(&self, x: i32, y: i32, color: u32)
fn draw_text(&self, x: i32, y: i32, color: u32, font_id: u32, size: u16, text: &str)
             // font_id: 0=system, 1=bold, 2=thin, 3=italic, 4=mono (Andale Mono)

// Buffer access
fn get_buffer(&self) -> *mut u32       // Raw ARGB pixel buffer
fn get_stride(&self) -> u32            // Pixels per row
fn get_height(&self) -> u32
fn copy_pixels_from(&self, src: &[u32])
fn copy_pixels_to(&self, dst: &mut [u32]) -> usize

// Interactive mode (for drag-drawing)
fn set_interactive(&self, enabled: bool)
fn get_mouse(&self) -> (i32, i32, u32)   // (x, y, button_state)

// Events
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
fn on_mouse_down(&self, f: impl FnMut(i32, i32, u32) + 'static)
fn on_mouse_up(&self, f: impl FnMut(i32, i32, u32) + 'static)
fn on_mouse_move(&self, f: impl FnMut(i32, i32) + 'static)  // Cursor movement
fn on_draw(&self, f: impl FnMut(i32, i32, u32) + 'static)   // Drag events (requires set_interactive(true))
```

### DataGrid

Spreadsheet-style data grid with sortable columns and per-cell styling.

```rust
DataGrid::new(w: u32, h: u32) -> Self

// Column definition
fn set_columns(&self, cols: &[ColumnDef])
fn column_count(&self) -> u32
fn set_column_width(&self, col_index: u32, width: u32)
fn set_column_sort_type(&self, col_index: u32, sort_type: u32)  // SORT_STRING or SORT_NUMERIC

// Data
fn set_data(&self, rows: &[Vec<&str>])
fn set_data_raw(&self, data: &[u8])      // 0x1E=row sep, 0x1F=col sep
fn set_cell(&self, row: u32, col: u32, text: &str)
fn get_cell(&self, row: u32, col: u32, buf: &mut [u8]) -> u32
fn set_row_count(&self, count: u32)
fn row_count(&self) -> u32

// Cell styling (flat arrays: index = row * col_count + col)
fn set_cell_colors(&self, colors: &[u32])      // ARGB text colors (0=default)
fn set_cell_bg_colors(&self, colors: &[u32])   // ARGB background colors (0=transparent)
fn set_char_colors(&self, char_colors: &[u32], offsets: &[u32])  // Per-character text colors
fn set_cell_icon(&self, row: u32, col: u32, pixels: &[u32], w: u32, h: u32)

// Display
fn set_row_height(&self, height: u32)           // Min 16
fn set_header_height(&self, height: u32)        // Min 16

// Scroll
fn scroll_offset(&self) -> u32                  // First visible row
fn set_scroll_offset(&self, offset: u32)        // Set first visible row

// Selection
fn set_selection_mode(&self, mode: u32)          // SELECTION_SINGLE or SELECTION_MULTI
fn selected_row(&self) -> u32                    // u32::MAX if none
fn set_selected_row(&self, row: u32)             // Also scrolls to row
fn is_row_selected(&self, row: u32) -> bool

// Sorting
fn sort(&self, column: u32, direction: u32)      // SORT_NONE/ASCENDING/DESCENDING

// Minimap & Connectors
fn set_minimap_colors(&self, colors: &[u32])     // Per-row colors in scrollbar track (0=no marker)
fn click_column(&self) -> i32                    // Display column of last click (-1 if none)
fn set_connector_lines(&self, lines: &[(u32, u32, u32, u8)])  // (start_row, end_row, color, filled)
fn set_connector_column(&self, col: u32)         // Which column to draw connectors in (default: 2)

// Events
fn on_selection_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
fn on_submit(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)  // Enter or double-click
```

#### ColumnDef (Builder)

```rust
ColumnDef::new(header: &str) -> Self
fn width(self, w: u32) -> Self           // Default: 100
fn align(self, a: u8) -> Self            // ALIGN_LEFT/CENTER/RIGHT
fn numeric(self) -> Self                 // Enable numeric sort

// Example
grid.set_columns(&[
    ColumnDef::new("Name").width(200),
    ColumnDef::new("Size").width(80).align(ALIGN_RIGHT).numeric(),
]);
```

### TextEditor

Full-featured code editor with syntax highlighting.

```rust
TextEditor::new(w: u32, h: u32) -> Self
TextEditor::from_file(path: &str, w: u32, h: u32) -> Self

// Text
fn set_text(&self, text: &str)
fn set_text_bytes(&self, data: &[u8])
fn get_text(&self, buf: &mut [u8]) -> u32
fn insert_text(&self, text: &str)         // At cursor
fn line_count(&self) -> u32

// Syntax
fn load_syntax(&self, path: &str)
fn load_syntax_from_bytes(&self, data: &[u8])

// Cursor
fn set_cursor(&self, row: u32, col: u32)
fn cursor(&self) -> (u32, u32)

// Display
fn set_line_height(&self, h: u32)         // Min 12
fn set_tab_width(&self, w: u32)           // Spaces per Tab
fn set_show_line_numbers(&self, show: bool)
fn set_editor_font(&self, font_id: u32, size: u32)

// Clipboard
fn copy(&self) -> bool                   // Copy selection to system clipboard
fn cut(&self) -> bool                    // Cut selection to system clipboard
fn paste(&self) -> u32                   // Paste from clipboard; returns bytes pasted
fn select_all(&self)                     // Select all text

// Line Highlights
fn highlight_line(&self, line: u32, color: u32)  // Highlight a line with ARGB background color
fn clear_highlights(&self)               // Remove all line highlights

// Read-Only & Navigation
fn set_read_only(&self, read_only: bool) // Disable editing (navigation/selection/copy still work)
fn ensure_line_visible(&self, line: u32) // Scroll so the given line is visible (centered if possible)

// Events
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
fn on_key_down(&self, f: impl FnMut(&KeyEvent) + 'static)   // Key events on this editor
```

**Keyboard:** Arrow keys, Home/End, Page Up/Down, Backspace, Delete, Tab (inserts spaces), Enter (auto-indent), Ctrl+C/X/V (copy/cut/paste), Ctrl+A (select all).

### TreeView

Hierarchical tree with expandable nodes.

```rust
TreeView::new(w: u32, h: u32) -> Self

// Node management
fn add_root(&self, text: &str) -> u32              // Returns node index
fn add_child(&self, parent: u32, text: &str) -> u32
fn remove_node(&self, index: u32)                   // Removes descendants too
fn set_node_text(&self, index: u32, text: &str)
fn set_node_icon(&self, index: u32, pixels: &[u32], w: u32, h: u32)
fn set_node_icon_from_file(&self, index: u32, path: &str, size: u32)
fn set_node_style(&self, index: u32, style: u32)    // STYLE_NORMAL or STYLE_BOLD
fn set_node_text_color(&self, index: u32, color: u32)  // 0=default
fn clear(&self)
fn node_count(&self) -> u32

// Expand/collapse
fn set_expanded(&self, index: u32, expanded: bool)
fn is_expanded(&self, index: u32) -> bool

// Selection
fn selected(&self) -> u32                           // u32::MAX if none
fn set_selected(&self, index: u32)

// Display
fn set_indent_width(&self, width: u32)
fn set_row_height(&self, height: u32)

// Events
fn on_selection_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
fn on_node_clicked(&self, f: impl FnMut(&ClickEvent) + 'static)
fn on_enter(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

**Keyboard:** Up/Down = navigate, Left = collapse/parent, Right = expand/child, Enter = fire click.

### ImageButton

Clickable button that displays an image (PNG, ICO, BMP, JPEG, GIF) instead of text.

```rust
ImageButton::new(w: u32, h: u32) -> Self

fn load_file(&self, path: &str)
fn load_bytes(&self, data: &[u8])
fn load_ico(&self, path: &str, preferred_size: u32)
fn set_pixels(&self, pixels: &[u32], w: u32, h: u32)  // Raw ARGB
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
```

### DropDown

Drop-down selection list.

```rust
DropDown::new(items: &str) -> Self               // Pipe-separated: "640x480|800x600|1024x768"
fn set_items(&self, items: &str)                 // Replace item list (pipe-separated)
fn selected_index(&self) -> u32                  // Get selected index (0-based)
fn set_selected_index(&self, idx: u32)           // Set selected index
fn on_selection_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

Use `set_text()` / `get_text()` from Control base to set/get the item list.

### AutoCompleteTextField

Text input with autocomplete suggestions popup.

```rust
AutoCompleteTextField::new() -> Self
fn set_placeholder(&self, text: &str)
fn set_suggestions(&self, items: &str)           // Pipe-separated suggestions
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
fn on_submit(&self, f: impl FnMut(&SubmitEvent) + 'static)   // Enter key
```

The server filters the suggestions as the user types, displaying matching entries in a popup.

```rust
let field = AutoCompleteTextField::new();
field.set_placeholder("Search contacts...");
field.set_suggestions("Alice <alice@example.com>|Bob <bob@example.com>|Carol <carol@example.com>");
field.on_submit(|e| { /* user pressed Enter */ });
```

---

## Container Controls

### Card

Styled container with card/panel appearance.

```rust
Card::new() -> Self
```

### GroupBox

Container with labeled border.

```rust
GroupBox::new(title: &str) -> Self
```

### SplitView

Resizable split pane.

```rust
SplitView::new() -> Self
fn set_orientation(&self, orientation: u32)     // VERTICAL or HORIZONTAL
fn set_split_ratio(&self, ratio: u32)           // 0-100
fn set_min_split(&self, min_ratio: u32)
fn set_max_split(&self, max_ratio: u32)
fn set_resizable(&self, resizable: bool)        // Enable/disable drag-to-resize splitter
fn on_split_changed(&self, f: impl FnMut(&ValueChangedEvent) + 'static)
```

### ScrollView

Scrollable container.

```rust
ScrollView::new() -> Self
fn on_scroll(&self, f: impl FnMut(&ScrollChangedEvent) + 'static)
```

### Sidebar

Navigation sidebar with selectable items.

```rust
Sidebar::new() -> Self
fn on_selection_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

### NavigationBar

Top navigation bar container.

```rust
NavigationBar::new(title: &str) -> Self
```

### TabBar

Multi-tab interface with closable tabs.

```rust
TabBar::new(labels: &str) -> Self           // Pipe-separated: "File 1|File 2"
fn connect_panels(&self, panels: &[&impl Widget])  // Auto-switch visibility
fn show_plus(&self, show: bool)                // Show/hide "+" (new-tab) button
fn on_active_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
fn on_tab_close(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
fn on_double_click(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)  // Double-click on a tab
```

### Toolbar

Horizontal toolbar with convenience methods.

```rust
Toolbar::new() -> Self
fn add_button(&self, text: &str) -> Button
fn add_label(&self, text: &str) -> Label
fn add_separator(&self) -> Divider              // 1x16 vertical divider
fn add_icon_button(&self, icon_text: &str) -> IconButton
```

**Important:** Toolbar defaults to size (0,0). Always call `set_size()`, `set_padding()`, and `set_color()` explicitly:

```rust
toolbar.set_dock(DOCK_TOP);
toolbar.set_size(800, 36);
toolbar.set_color(0xFF252526);
toolbar.set_padding(4, 4, 4, 4);
```

### Alert

Inline alert/banner.

```rust
Alert::new(message: &str) -> Self
```

### ContextMenu

Right-click popup menu.

```rust
ContextMenu::new(items: &str) -> Self       // Pipe-separated: "Cut|Copy|Paste"
fn on_item_click(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

Attach to a control with `control.set_context_menu(&menu)`.

### TableView

Simple table container.

```rust
TableView::new() -> Self
fn on_selection_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
```

### Expander

Collapsible section.

```rust
Expander::new(title: &str) -> Self
fn is_expanded(&self) -> bool
fn set_expanded(&self, expanded: bool)
fn on_toggled(&self, f: impl FnMut(&CheckedChangedEvent) + 'static)
```

### Tooltip

Tooltip container (shows tooltip on hover of children).

```rust
Tooltip::new(text: &str) -> Self
```

### StackPanel

Stacks children vertically or horizontally.

```rust
StackPanel::new(orientation: u32) -> Self
StackPanel::vertical() -> Self
StackPanel::horizontal() -> Self
fn set_orientation(&self, orientation: u32)
```

### FlowPanel

Arranges children left-to-right with wrapping.

```rust
FlowPanel::new() -> Self
```

### TableLayout

Grid layout with configurable columns.

```rust
TableLayout::new(columns: u32) -> Self
fn set_columns(&self, columns: u32)
fn set_row_height(&self, row_height: u32)
fn set_column_widths(&self, widths: &[u32])  // Per-column pixel widths (last column gets remaining space)
```

### RadioGroup

Container for grouping RadioButton controls. Provides mutual exclusion so only one RadioButton within the group can be selected at a time.

```rust
RadioGroup::new() -> Self
```

Add RadioButton children with `add()`. When one is checked, the others are automatically unchecked.

---

## System Components

### TrayIcon

System tray icon (16x16 ARGB, displayed in the menu bar). Unlike normal controls, TrayIcon does not have a control ID or parent window -- it communicates directly with the compositor via IPC.

```rust
TrayIcon::new(icon_id: u32, pixels: &[u32; 256]) -> Self   // Create and register a 16x16 ARGB tray icon
fn set_pixels(&self, pixels: &[u32; 256])        // Update the icon pixel data
fn on_click(&self, f: impl FnMut() + 'static)   // Register click callback
fn remove(&self)                                 // Remove from menu bar
```

TrayIcon implements `Drop`, so it is automatically removed when the struct is dropped.

```rust
let mut pixels = [0xFF007AFFu32; 256]; // solid blue 16x16
let tray = TrayIcon::new(1, &pixels);
tray.on_click(|| {
    // user clicked the tray icon
});
```

### MenuBar

Window menu bar with nested menus and item flags. Built using `MenuBarBuilder`, then applied to a window via `MenuBar::set()`.

**Constants:**

```rust
MENU_FLAG_DISABLED  = 0x01    // Greyed out, not clickable
MENU_FLAG_SEPARATOR = 0x02    // Horizontal separator line
MENU_FLAG_CHECKED   = 0x04    // Checkmark next to item
```

**Types:**

```rust
MenuBar::set(win_id: u32, data: &[u8]) -> Self          // Attach menu bar to window
fn on_item(&self, f: impl FnMut(&MenuItemEvent) + 'static)  // Menu item click callback
fn update_item(&self, item_id: u32, new_flags: u32)      // Update item flags at runtime
```

**Builder:**

```rust
let mut builder = MenuBarBuilder::new();
let data = builder
    .menu("File")
        .item(1, "New", 0)
        .item(2, "Open...", 0)
        .separator()
        .item(5, "Quit", 0)
    .end_menu()
    .menu("Edit")
        .item(10, "Cut", 0)
        .item(11, "Copy", 0)
        .item(12, "Paste", 0)
    .end_menu()
    .build();
let menu = MenuBar::set(win.id(), data);
menu.on_item(|e| {
    match e.item_id {
        1 => new_file(),
        2 => open_file(),
        5 => anyui::quit(),
        10 => cut(), 11 => copy(), 12 => paste(),
        _ => {}
    }
});
```

The `MenuItemEvent` struct contains a single field `item_id: u32`.

---

## Dialogs

### FileDialog

Modal file/folder selection dialogs. All methods are static and block until user responds.

```rust
FileDialog::open_file() -> Option<String>              // Select a file
FileDialog::open_folder() -> Option<String>            // Select a folder
FileDialog::save_file(default_name: &str) -> Option<String>  // Save file dialog
FileDialog::create_folder() -> Option<String>          // Create new folder
```

Returns `None` if cancelled, `Some(path)` if confirmed.

### MessageBox

Modal message dialog.

```rust
pub enum MessageBoxType {
    Alert = 0,      // Red exclamation -- errors
    Info = 1,       // Blue "i" -- informational
    Warning = 2,    // Yellow exclamation -- warnings
}

MessageBox::show(msg_type: MessageBoxType, text: &str, button_text: Option<&str>)
// button_text = None uses "OK"
```

---

## Icon System

Load and display icons from files, system icon packs, or raw data.

### IconType

```rust
pub enum IconType {
    Filled,
    Outline,
}
```

### Icon

```rust
pub struct Icon {
    pub pixels: Vec<u32>,   // ARGB pixel buffer
    pub width: u32,
    pub height: u32,
}

// System SVG icons from ico.pak (6000+ Tabler Icons, cached)
Icon::system(name: &str, icon_type: IconType, color: u32, size: u32) -> Option<Self>

// Control icons (from /System/media/icons/controls/, .png then .ico fallback)
Icon::control(name: &str, size: u32) -> Option<Self>

// From file types (loads from /System/media/icons/ via mimetypes.conf)
Icon::for_filetype(ext: &str) -> Option<Self>
Icon::for_filetype_sized(ext: &str, size: u32) -> Option<Self>

// From applications (loads from /System/media/icons/apps/)
Icon::for_application(name: &str) -> Option<Self>
Icon::for_application_sized(name: &str, size: u32) -> Option<Self>

// From files
Icon::load(path: &str, preferred_size: u32) -> Option<Self>
Icon::from_ico_bytes(data: &[u8], preferred_size: u32) -> Option<Self>
Icon::from_bytes(data: &[u8]) -> Option<Self>

// Manipulation
fn recolor(&mut self, color: u32)        // Recolor all non-transparent pixels (preserves alpha)

// Convert to ImageView
fn into_image_view(self, display_w: u32, display_h: u32) -> ImageView
fn apply_to(&self, image_view: &ImageView)
```

`Icon::system()` uses a built-in render cache (128 entries) so repeated calls with the same name/color/size are instant.

---

## Events Reference

### Event Structs

| Struct | Fields | Used by |
|--------|--------|---------|
| `ClickEvent` | `id: u32` | Button, IconButton, Tag, Canvas, TreeView |
| `TextChangedEvent` | `id: u32` + `.text() -> String` | TextField, SearchField, TextArea, TextEditor, AutoCompleteTextField |
| `SubmitEvent` | `id: u32` | TextField, SearchField, AutoCompleteTextField (Enter key) |
| `SelectionChangedEvent` | `id: u32, index: u32` | DataGrid, TreeView, TabBar, SegmentedControl, Sidebar, ContextMenu, DropDown |
| `CheckedChangedEvent` | `id: u32, checked: bool` | Toggle, Checkbox, RadioButton, Expander |
| `ValueChangedEvent` | `id: u32, value: u32` | Slider, Stepper, SplitView |
| `ScrollChangedEvent` | `id: u32, offset: u32` | ScrollView |
| `EventArgs` | `id: u32` | Window (close, resize) |
| `ColorSelectedEvent` | `id: u32, color: u32` | ColorWell |
| `KeyEvent` | `keycode: u32, char_code: u32, modifiers: u32` | Window (on_key_down, on_key_up), TextEditor (on_key_down) |
| `MenuItemEvent` | `item_id: u32` | MenuBar (on_item) |

### KeyEvent

```rust
pub struct KeyEvent {
    pub keycode: u32,        // KEY_* constants or ASCII
    pub char_code: u32,      // Unicode codepoint, 0 for non-printable
    pub modifiers: u32,      // MOD_SHIFT | MOD_CTRL | MOD_ALT
}

impl KeyEvent {
    fn shift(&self) -> bool
    fn ctrl(&self) -> bool
    fn alt(&self) -> bool
}
```

### Standalone Key Functions

```rust
fn get_key_info() -> KeyEvent       // Get key info for current event
fn get_modifiers() -> u32           // Get current modifier state
```

### Event Registration Pattern

```rust
button.on_click(|e: &ClickEvent| {
    // e.id = the button's control ID
});

slider.on_value_changed(|e: &ValueChangedEvent| {
    // e.value = new value (0-100)
});

text_field.on_text_changed(|e: &TextChangedEvent| {
    let text = e.text();  // Queries current content (up to 512 bytes)
});
```

---

## Layout System

### Docking

Controls are laid out in the order added. Each docked control claims its edge; remaining space goes to the next.

```
Window (800x600)
  1. Toolbar   DOCK_TOP    -> claims top 36px strip
  2. Sidebar   DOCK_LEFT   -> claims left 200px of remainder
  3. Status    DOCK_BOTTOM -> claims bottom 24px of remainder
  4. Content   DOCK_FILL   -> fills everything left
```

**Key rule:** Add DOCK_FILL controls last. For multiple DOCK_BOTTOM controls, the first added is at the very bottom.

### Example Layout

```rust
// Toolbar at top
toolbar.set_dock(DOCK_TOP);
toolbar.set_size(800, 36);
win.add(&toolbar);

// Status bar at bottom
status_bar.set_dock(DOCK_BOTTOM);
status_bar.set_size(800, 24);
win.add(&status_bar);

// Content fills remaining space
content.set_dock(DOCK_FILL);
win.add(&content);
```

### Absolute Positioning

Controls with `DOCK_NONE` (default) use manual positioning:

```rust
label.set_position(20, 50);
label.set_size(100, 24);
```

---

## Timer API

Periodic callbacks on the UI thread.

```rust
fn set_timer(interval_ms: u32, f: impl FnMut() + 'static) -> u32
fn kill_timer(timer_id: u32)
```

Example:

```rust
let id = anyui::set_timer(1000, || {
    // runs every second on the UI thread
});
anyui::kill_timer(id);
```

---

## Marshal API

Thread-safe UI access from worker threads. All operations execute asynchronously on the UI thread.

```rust
fn marshal_set_text(id: u32, text: &str)
fn marshal_set_color(id: u32, color: u32)
fn marshal_set_state(id: u32, value: u32)
fn marshal_set_visible(id: u32, visible: bool)
fn marshal_set_position(id: u32, x: i32, y: i32)
fn marshal_set_size(id: u32, w: u32, h: u32)
fn marshal_dispatch(cb: extern "C" fn(u64), userdata: u64)
```

---

## Clipboard API

System clipboard access for copy/paste between applications.

```rust
fn clipboard_set(text: &str)                // Copy text to clipboard
fn clipboard_set_bytes(data: &[u8])         // Copy raw bytes to clipboard
fn clipboard_get(buf: &mut [u8]) -> u32     // Read clipboard; returns bytes read (0 if empty)
```

Example:

```rust
anyui::clipboard_set("Hello");
let mut buf = [0u8; 4096];
let len = anyui::clipboard_get(&mut buf);
let text = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
```

---

## Theme API

Color theming support with dark/light mode.

```rust
// Global theme switching
fn set_theme(light: bool)                   // Switch to light (true) or dark (false) theme
fn get_theme() -> u32                       // Get current theme ID

// Theme module
pub mod theme {
    fn colors() -> &'static ThemeColors     // Get current theme colors (zero-cost pointer read)
    fn set_theme(light: bool)               // Switch theme
    fn is_light() -> bool                   // Check if light mode is active
    fn apply_accent_style(dark_accent: u32, dark_hover: u32, light_accent: u32, light_hover: u32)
                                            // Override accent colors in both palettes

    // Color utilities
    fn darken(color: u32, amount: u32) -> u32    // Darken ARGB color (amount: 0-255)
    fn lighten(color: u32, amount: u32) -> u32   // Lighten ARGB color (amount: 0-255)
    fn with_alpha(color: u32, alpha: u32) -> u32 // Set alpha on ARGB color
}
```

### ThemeColors

```rust
pub struct ThemeColors {
    pub window_bg: u32,           // Main window background
    pub text: u32,                // Default text
    pub text_secondary: u32,      // Secondary/muted text
    pub text_disabled: u32,       // Disabled control text
    pub accent: u32,              // Accent / primary action color
    pub accent_hover: u32,        // Accent hover state
    pub destructive: u32,         // Destructive action color (red)
    pub success: u32,             // Success indicator (green)
    pub warning: u32,             // Warning indicator (yellow)
    pub control_bg: u32,          // Button/control background
    pub control_hover: u32,       // Control hover state
    pub control_pressed: u32,     // Control pressed state
    pub input_bg: u32,            // Text input background
    pub input_border: u32,        // Text input border
    pub input_focus: u32,         // Focused input border
    pub separator: u32,           // Divider/separator color
    pub selection: u32,           // Text selection background
    pub sidebar_bg: u32,          // Sidebar background
    pub card_bg: u32,             // Card background
    pub card_border: u32,         // Card border
    pub badge_red: u32,           // Badge color
    pub toggle_on: u32,           // Toggle on-state
    pub toggle_off: u32,          // Toggle off-state
    pub toggle_thumb: u32,        // Toggle thumb color
    pub scrollbar: u32,           // Scrollbar thumb
    pub scrollbar_track: u32,     // Scrollbar track
    pub check_mark: u32,          // Checkbox/radio checkmark
    pub toolbar_bg: u32,          // Toolbar background
    pub tab_inactive_bg: u32,     // Inactive tab background
    pub tab_hover_bg: u32,        // Tab hover state
    pub tab_border_active: u32,   // Active tab bottom border
    pub editor_bg: u32,           // Text editor background
    pub editor_line_hl: u32,      // Editor current line highlight
    pub editor_selection: u32,    // Editor text selection
    pub alt_row_bg: u32,          // Alternating row background
    pub placeholder_bg: u32,      // Placeholder/loading background
}
```

---

## Window Management

Functions for programmatic window control beyond what the Window struct provides:

```rust
fn minimize_window(win_id: u32)                  // Minimize a window (hide off-screen)
fn move_window(win_id: u32, x: i32, y: i32)     // Move a window to screen position
fn resize_window(win_id: u32, w: u32, h: u32)   // Resize window (SHM + control tree)
fn destroy_window(win_id: u32)                   // Destroy a window and all children
```

These are also available as methods on `Window` (see [Window](#window)).

---

## Fullscreen API

Direct framebuffer access for fullscreen applications (games, video players).

```rust
// On Window:
fn set_fullscreen_capable(&self, auto_enter: bool)   // Mark window as fullscreen-capable
fn on_fullscreen_enter(&self, f: impl FnMut(&EventArgs) + 'static)
fn on_fullscreen_exit(&self, f: impl FnMut(&EventArgs) + 'static)

// Standalone:
fn get_fullscreen_info() -> Option<FullscreenInfo>   // Get FB pointer and dimensions
fn flush_display(x: u32, y: u32, w: u32, h: u32)    // Flush dirty region after direct FB writes
```

### FullscreenInfo

```rust
pub struct FullscreenInfo {
    pub width: u32,     // Screen width in pixels
    pub height: u32,    // Screen height in pixels
    pub stride: u32,    // Pixels per row (may differ from width)
    pub fb_ptr: u32,    // Direct framebuffer pointer (0 if SHM compositing mode)
}
```

When `fb_ptr != 0`, the app can write ARGB pixels directly to GPU VRAM at that address. After writing, call `flush_display()` to tell the GPU to update the screen region. Without `flush_display()`, changes may not be visible on virtualized displays (e.g. SVGA).

---

## Modal Dialogs

```rust
// On Window:
fn set_modal(&self, owner: &Window)      // Make this a modal child of owner
// The modal relationship is automatically cleared when the modal window is destroyed.

// Standalone (advanced):
fn set_modal(modal_id: u32, owner_id: u32)    // Set modal relationship by control IDs
fn clear_modal(modal_id: u32)                  // Clear modal relationship
```

The modal window blocks input to the owner and stays on top until destroyed.

---

## Display & Scaling

System-wide display settings accessible via the `theme` module:

```rust
pub mod theme {
    fn set_font_smoothing(mode: u32)     // 0=none, 1=greyscale AA, 2=subpixel LCD
    fn get_font_smoothing() -> u32

    fn set_scale_factor(percent: u32)    // 100-300 in 25% steps (100=1x, 200=2x)
    fn get_scale_factor() -> u32
}
```

Changes are sent to the compositor via IPC, persisted to `compositor.conf`, and picked up by all running apps immediately.

```rust
// On Window:
fn set_cursor_visible(&self, visible: bool)   // Hide/show mouse cursor for this window
```

---

## Text Measurement

Measure text dimensions using the font engine (useful for layout calculations):

```rust
fn measure_text(text: &str, font_id: u16, font_size: u16) -> (u32, u32)
// Returns (width, height) in pixels
// font_id: 0 = normal, 1 = bold
```

---

## Window Lifecycle

Callbacks for monitoring window open/close events across all applications (used by the compositor/dock):

```rust
fn on_window_opened(f: impl FnMut(u32) + 'static)   // Callback receives app task ID
fn on_window_closed(f: impl FnMut(u32) + 'static)   // Callback receives app task ID
fn focus_by_tid(tid: u32)                             // Bring a window to front by task ID
fn get_compositor_channel() -> u32                    // Get raw compositor IPC channel ID
```

---

## Key Constants

### Keyboard Keys (KEY_*)

```rust
KEY_ENTER = 0x0D, KEY_ESCAPE = 0x1B, KEY_TAB = 0x09, KEY_BACKSPACE = 0x08,
KEY_DELETE = 0x7F, KEY_SPACE = 0x20,

KEY_UP = 0x80, KEY_DOWN = 0x81, KEY_LEFT = 0x82, KEY_RIGHT = 0x83,
KEY_HOME = 0x84, KEY_END = 0x85, KEY_PAGE_UP = 0x86, KEY_PAGE_DOWN = 0x87,

KEY_F1 = 0x90, KEY_F2 = 0x91, KEY_F3 = 0x92, KEY_F4 = 0x93,
KEY_F5 = 0x94, KEY_F6 = 0x95, KEY_F7 = 0x96, KEY_F8 = 0x97,
KEY_F9 = 0x98, KEY_F10 = 0x99, KEY_F11 = 0x9A, KEY_F12 = 0x9B,

KEY_INSERT = 0x9C
```

### Modifier Keys (MOD_*)

```rust
MOD_SHIFT = 0x01
MOD_CTRL  = 0x02
MOD_ALT   = 0x04
```

Use with `KeyEvent.modifiers` or `get_modifiers()`:

```rust
win.on_key_down(|e| {
    if e.ctrl() && e.keycode == b'S' as u32 {
        save_file();
    }
});
```

---

## Utilities

```rust
fn set_blur_behind(window: &impl Widget, radius: u32)     // Frosted glass (0=disable)
fn screen_size() -> (u32, u32)                             // Display dimensions (logical pixels)
fn show_notification(title: &str, message: &str, icon: Option<&[u32; 256]>, timeout_ms: u32)
                                                           // title max 64 bytes, message max 128 bytes, 0=default 5s
```

---

## Syntax Highlighting

TextEditor uses `.syn` files:

```
keywords=if,else,while,for,fn,let,mut,return,...
types=u8,u16,u32,u64,bool,String,Vec,...
builtins=println,print,format,panic,...
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

Place syntax files in `/System/syntax/` for system-wide access.

---

## Frame Pacing & VSync

The anyui event loop implements automatic VSync-driven frame pacing:

1. **Present + track**: Rendering sets `frame_presented` flag
2. **Back-pressure**: Pending frames skip re-rendering
3. **ACK receipt**: `EVT_FRAME_ACK` (0x300B) clears flag, allowing next frame
4. **Safety timeout**: 64ms fallback if ACK is lost

| State | Sleep | Description |
|-------|-------|-------------|
| Frame pending | 2ms | Fast polling for ACK |
| Idle | 16ms | Low CPU when nothing happens |

End-to-end latency: **4-9ms** (vs 18-53ms with fixed timer).

---

## Quick Reference: Controls Overview

| Kind | Control | Type | Description |
|------|---------|------|-------------|
| 0 | Window | Container | Top-level window |
| 1 | View | Container | Generic container |
| 2 | Label | Leaf | Text label |
| 3 | Button | Leaf | Clickable button |
| 4 | TextField | Leaf | Single-line text input |
| 5 | Toggle | Leaf | On/off switch |
| 6 | Checkbox | Leaf | Checkbox with label |
| 7 | Slider | Leaf | Value slider (0-100) |
| 8 | RadioButton | Leaf | Radio button |
| 9 | ProgressBar | Leaf | Progress indicator |
| 10 | Stepper | Leaf | Increment/decrement |
| 11 | SegmentedControl | Leaf | Segment selection |
| 12 | TableView | Container | Simple table |
| 13 | ScrollView | Container | Scrollable container |
| 14 | Sidebar | Container | Navigation sidebar |
| 15 | NavigationBar | Container | Top navigation bar |
| 16 | TabBar | Container | Tab selection |
| 17 | Toolbar | Container | Horizontal toolbar |
| 18 | Card | Container | Rounded card |
| 19 | GroupBox | Container | Titled group |
| 20 | SplitView | Container | Split pane |
| 21 | Divider | Leaf | Separator line |
| 22 | Alert | Container | Alert banner |
| 23 | ContextMenu | Container | Popup menu |
| 24 | Tooltip | Container | Tooltip popup |
| 25 | ImageView | Leaf | Image display |
| 26 | StatusIndicator | Leaf | Status dot |
| 27 | ColorWell | Leaf | Color picker |
| 28 | SearchField | Leaf | Search input |
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
| 41 | RadioGroup | Container | Radio button group |
| 42 | DropDown | Leaf | Drop-down selection list |
| 43 | AutoCompleteTextField | Leaf | Text input with autocomplete |

---

## Quick Reference: Event Mapping

| Control | Events |
|---------|--------|
| Button | `on_click` |
| TextField | `on_text_changed`, `on_submit` |
| TextArea | `on_text_changed` |
| Toggle | `on_checked_changed` |
| Checkbox | `on_checked_changed` |
| RadioButton | `on_checked_changed` |
| Slider | `on_value_changed` |
| Stepper | `on_value_changed` |
| SegmentedControl | `on_active_changed` |
| IconButton | `on_click` |
| Tag | `on_click` |
| Canvas | `on_click`, `on_mouse_down`, `on_mouse_up`, `on_mouse_move`, `on_draw` |
| DataGrid | `on_selection_changed`, `on_submit` |
| TextEditor | `on_text_changed`, `on_key_down` |
| TreeView | `on_selection_changed`, `on_node_clicked`, `on_enter` |
| SearchField | `on_text_changed`, `on_submit` |
| ColorWell | `on_color_selected` |
| ImageButton | `on_click` |
| DropDown | `on_selection_changed` |
| AutoCompleteTextField | `on_text_changed`, `on_submit` |
| Window | `on_close`, `on_resize`, `on_click`, `on_key_down`, `on_key_up`, `on_fullscreen_enter`, `on_fullscreen_exit` |
| SplitView | `on_split_changed` |
| ScrollView | `on_scroll` |
| Sidebar | `on_selection_changed` |
| TabBar | `on_active_changed`, `on_tab_close`, `on_double_click` |
| ContextMenu | `on_item_click` |
| TableView | `on_selection_changed` |
| Expander | `on_toggled` |
| TrayIcon | `on_click` |
| MenuBar | `on_item` |

## Drag & Drop

anyui ships a built-in drag-and-drop framework that works **within a window**
(reorderable lists, tab-reorder, etc.) and **across windows / processes**
(drag a file from a files manager into an editor, drag mail into a folder,
etc.). Both flows use the same controls and callbacks — the framework
transparently routes through the compositor when the cursor crosses window
boundaries.

### Roles

| Role | Setup | Receives |
|------|-------|----------|
| **Source** | `set_draggable(true)` | `EVENT_DRAG_START`, `EVENT_DRAG_END` |
| **Target** | `set_drop_target(true)` + (optional) `set_drop_formats(mask)` | `EVENT_DRAG_ENTER`, `EVENT_DRAG`, `EVENT_DRAG_LEAVE`, `EVENT_DROP` |

A control can be both source and target (used by reorder lists).

### Payload formats

```rust
DND_FORMAT_TEXT      // UTF-8 plain text
DND_FORMAT_URI_LIST  // Newline-separated file:// URIs
DND_FORMAT_FILES     // NUL-separated absolute paths
DND_FORMAT_CUSTOM    // App-defined start (use offsets for subtypes)
DND_FORMAT_ACCEPT_ANY // Sentinel mask: target accepts any non-zero format
```

Build a per-target acceptance mask with `dnd_format_mask(fmt)` (OR several
together for multiple formats), or pass `DND_FORMAT_ACCEPT_ANY`.

### Effects

```rust
DND_EFFECT_COPY  // 1
DND_EFFECT_MOVE  // 2
DND_EFFECT_LINK  // 4
DND_EFFECT_ALL   // COPY | MOVE | LINK
```

The framework intersects source-allowed effects, target-requested effects,
and current modifiers (Ctrl=Copy, Shift=Move, Ctrl+Shift=Link) to pick a
single negotiated effect. Targets read the negotiated effect via
`drag_effect()`.

### Source flow (DRAG_START)

```rust
let card = ui::Card::new();
card.set_draggable(true);
card.on_drag_start(move |_| {
    // Install the payload — also announces the drag to the compositor
    // so the cursor can cross window boundaries.
    ui::drag_set_payload(
        ui::DND_FORMAT_CUSTOM,
        b"my opaque blob",
        ui::DND_EFFECT_MOVE,
    );
});
card.on_drag_end(|_| {
    // Drag finished — completed or cancelled. Optional cleanup.
});
```

Convenience helpers for common formats:

```rust
ui::drag_set_text("hello world");
ui::drag_set_files(&["/home/strati/a.txt", "/home/strati/b.png"],
                   ui::DND_EFFECT_COPY);
```

### Target flow (DRAG_ENTER → DRAG → DROP)

```rust
let sink = ui::Label::new("Drop here");
sink.set_drop_target(true);
sink.set_drop_formats(ui::dnd_format_mask(ui::DND_FORMAT_FILES));

sink.on_drag_enter(|_| {
    ui::drag_accept(ui::DND_EFFECT_COPY);
});
sink.on_drop(|_| {
    let paths = ui::drag_get_files();
    for p in &paths { /* open / import / etc. */ }
});
```

Acceptance is **persistent across pointer moves** as long as the modifiers
don't change. Modifier-aware targets (e.g. Copy vs. Move based on Ctrl)
should also handle `EVENT_DRAG` to re-call `drag_accept` with the new
effect:

```rust
sink.on_event_raw(ui::EVENT_DRAG, |_id, _ev, _ud| {
    ui::drag_accept(ui::DND_EFFECT_ALL); // framework picks based on mods
}, 0);
```

### Cross-window behaviour

When the cursor leaves the source's window, the compositor routes
`EVENT_DRAG_ENTER` / `EVENT_DRAG` / `EVENT_DRAG_LEAVE` / `EVENT_DROP` to
whichever window is under the cursor — including windows of other
processes. Apps don't need any extra code: source registers a payload,
target registers a drop handler, the rest is automatic.

The payload is passed via a small SHM region allocated by the source
(64 KiB cap), mapped read-only by the target during DRAG_ENTER, and
released after DRAG_END. Target apps can read it via the standard
`drag_get_payload`, `drag_get_text`, `drag_get_files` helpers.

### Cancel paths

- **ESC** during a drag cancels it (no DROP fires; `EVENT_DRAG_END` fires
  with `completed=0`).
- **Source window closes** → drag is cancelled, target sees `EVENT_DRAG_LEAVE`.
- **Source process exits** → same.
- **No matching target under cursor on release** → source sees DRAG_END,
  target sees nothing.

### Auto-scroll during drag

Containers can opt in to dragging-near-edge auto-scroll by overriding
`is_drag_autoscroll_target()` and `drag_autoscroll(dx, dy)`. The framework
walks up the ancestor chain from the current target to find the nearest
opt-in container and applies the scroll delta automatically.

### API surface

| Function | Side | Use |
|---|---|---|
| `set_draggable(bool)` | Source | Mark a control as a drag source |
| `set_drop_target(bool)` | Target | Mark a control as a drop target |
| `set_drop_formats(mask)` | Target | Restrict accepted formats |
| `drag_set_payload(fmt, bytes, effects)` | Source | Install payload (in DRAG_START) |
| `drag_set_text(s)`, `drag_set_files(&[...])` | Source | Convenience helpers |
| `drag_set_image(pixels, w, h, hot_x, hot_y) -> bool` | Source | Attach a ghost image that follows the cursor (after `drag_set_payload`) |
| `drag_get_payload() -> (bytes, fmt)` | Target | Read payload (in DRAG_ENTER/DROP) |
| `drag_get_text()`, `drag_get_files()` | Target | Convenience helpers |
| `drag_accept(effects) -> negotiated` | Target | Accept (in DRAG_ENTER / DRAG) |
| `drag_reject()` | Target | Reject explicitly |
| `drag_effect() -> u32` | Either | Currently negotiated effect |
| `drag_format() -> u32` | Either | Current payload format |
| `drag_is_active() -> bool` | Either | Drag in progress? |

### Cursor feedback

While a cross-window drag is active, the compositor renders different
cursor shapes based on the current target acceptance:

| Cursor | When |
|---|---|
| `Move` (open four-arrow) | Hovering over a target that accepted with `MOVE` |
| `DragCopy` (arrow + small `+`) | Negotiated `COPY` (e.g. Ctrl held) |
| `DragLink` (arrow + small chain) | Negotiated `LINK` (Ctrl+Shift held) |
| `DragNoDrop` (arrow + slash circle) | Cursor over no target, or target rejected |

Apps don't have to do anything — `EVT_DRAG_FEEDBACK` from the compositor
drives the cursor. Targets just need to call `drag_accept` with the right
effect bits and the user sees the appropriate cursor.

### Drag images (ghosts)

A drag-image is an optional semi-transparent picture that follows the
cursor while the drag is active. Useful to show "what is being dragged"
across window boundaries, especially when the source was a small clickable
target (e.g. a list row, a tab, a thumbnail).

```rust
on_drag_start(|_| {
    drag_set_payload(DND_FORMAT_FILES, &payload, DND_EFFECT_COPY);
    // 200×30 ARGB8888 ghost, anchored at the centre.
    let pixels: Vec<u32> = build_thumbnail();
    drag_set_image(&pixels, 200, 30, 100, 15);
});
```

Constraints: maximum size is 1024 × 1024. Pixels are ARGB8888, top-left
origin. The compositor maps the SHM read-only and blends it under the
cursor each compose pass; the framework releases the SHM on `EVT_DRAG_END`.

### Pitfalls

- `drag_set_payload` MUST be called in `on_drag_start`, not earlier — the
  drag session doesn't exist before then.
- Drop targets must call `drag_accept` from `on_drag_enter` (and re-call
  from `EVENT_DRAG` if modifier-aware), otherwise DROP is silently dropped.
- A target's `set_drop_formats` mask defaults to `DND_FORMAT_ACCEPT_ANY`
  if not set after `set_drop_target(true)`.
- Cards/Labels are non-interactive by default, so `set_draggable(true)` /
  `set_drop_target(true)` is what makes them participate in hit-testing
  for drag purposes.
