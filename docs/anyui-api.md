# anyui Controls Framework API Reference

The **anyui** framework is a Windows Forms-inspired UI toolkit providing 41 control types for anyOS GUI applications. It consists of a server-side library (**libanyui**, `.so` at `0x04400000`) compiled into the compositor, and a client-side wrapper (**libanyui_client**) that user programs link against.

**Exports:** 112+ (C ABI, `#[no_mangle]`)
**Client crate:** `libanyui_client`
**Controls:** 41 types (ControlKind 0-40)
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
- [Dialogs](#dialogs)
- [Icon System](#icon-system)
- [Events Reference](#events-reference)
- [Layout System](#layout-system)
- [Timer API](#timer-api)
- [Marshal API (Cross-Thread)](#marshal-api)
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
KIND_DATA_GRID = 38, KIND_TEXT_EDITOR = 39, KIND_TREE_VIEW = 40
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

### Focus & Misc

```rust
fn focus(&self)                          // Set keyboard focus
fn set_tab_index(&self, index: u32)
fn set_context_menu(&self, menu: &impl Widget)
fn remove(&self)                         // Remove from parent
fn from_id(id: u32) -> Self             // Wrap existing control ID
fn id(&self) -> u32                      // Get control ID
```

---

## Container Base Class

Extends Control with:

```rust
fn add(&self, child: &impl Widget)       // Add child control
```

---

## Controls Reference

### Window

Top-level window container.

```rust
Window::new(title: &str, x: i32, y: i32, w: u32, h: u32) -> Self
Window::new_with_flags(title: &str, x: i32, y: i32, w: u32, h: u32, flags: u32) -> Self
// x, y = -1 for auto-placement

fn destroy(&self)
fn on_close(&self, f: impl FnMut(&EventArgs) + 'static)
fn on_resize(&self, f: impl FnMut(&EventArgs) + 'static)
```

**Note:** No `set_title()` method -- title is set only at construction time.

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
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
```

### TextField

Single-line text input.

```rust
TextField::new() -> Self
fn set_placeholder(&self, text: &str)
fn set_prefix_icon(&self, icon_code: u32)
fn set_postfix_icon(&self, icon_code: u32)
fn set_password_mode(&self, enabled: bool)
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

Button with built-in icon.

```rust
IconButton::new(icon_text: &str) -> Self
fn set_icon(&self, icon_id: u32)            // ICON_* constants
fn on_click(&self, f: impl FnMut(&ClickEvent) + 'static)
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
fn on_draw(&self, f: impl FnMut(i32, i32, u32) + 'static)  // Drag events
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
fn set_cell_icon(&self, row: u32, col: u32, pixels: &[u32], w: u32, h: u32)

// Display
fn set_row_height(&self, height: u32)           // Min 16
fn set_header_height(&self, height: u32)        // Min 16

// Selection
fn set_selection_mode(&self, mode: u32)          // SELECTION_SINGLE or SELECTION_MULTI
fn selected_row(&self) -> u32                    // u32::MAX if none
fn set_selected_row(&self, row: u32)             // Also scrolls to row
fn is_row_selected(&self, row: u32) -> bool

// Sorting
fn sort(&self, column: u32, direction: u32)      // SORT_NONE/ASCENDING/DESCENDING

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

// Events
fn on_text_changed(&self, f: impl FnMut(&TextChangedEvent) + 'static)
```

**Keyboard:** Arrow keys, Home/End, Page Up/Down, Backspace, Delete, Tab (inserts spaces), Enter (auto-indent).

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
fn on_active_changed(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
fn on_tab_close(&self, f: impl FnMut(&SelectionChangedEvent) + 'static)
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
```

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

Load and display icons from files or raw data.

```rust
pub struct Icon {
    pub pixels: Vec<u32>,   // ARGB pixel buffer
    pub width: u32,
    pub height: u32,
}

// From file types (loads from /System/media/icons/)
Icon::for_filetype(ext: &str) -> Option<Self>
Icon::for_filetype_sized(ext: &str, size: u32) -> Option<Self>

// From applications (loads from /System/media/icons/apps/)
Icon::for_application(name: &str) -> Option<Self>
Icon::for_application_sized(name: &str, size: u32) -> Option<Self>

// From files
Icon::load(path: &str, preferred_size: u32) -> Option<Self>
Icon::from_ico_bytes(data: &[u8], preferred_size: u32) -> Option<Self>
Icon::from_bytes(data: &[u8]) -> Option<Self>

// Convert to ImageView
fn into_image_view(self, display_w: u32, display_h: u32) -> ImageView
fn apply_to(&self, image_view: &ImageView)
```

---

## Events Reference

### Event Structs

| Struct | Fields | Used by |
|--------|--------|---------|
| `ClickEvent` | `id: u32` | Button, IconButton, Tag, Canvas, TreeView |
| `TextChangedEvent` | `id: u32` + `.text() -> String` | TextField, SearchField, TextArea, TextEditor |
| `SubmitEvent` | `id: u32` | TextField, SearchField (Enter key) |
| `SelectionChangedEvent` | `id: u32, index: u32` | DataGrid, TreeView, TabBar, SegmentedControl, Sidebar, ContextMenu |
| `CheckedChangedEvent` | `id: u32, checked: bool` | Toggle, Checkbox, RadioButton, Expander |
| `ValueChangedEvent` | `id: u32, value: u32` | Slider, Stepper, SplitView |
| `ScrollChangedEvent` | `id: u32, offset: u32` | ScrollView |
| `EventArgs` | `id: u32` | Window (close, resize) |
| `ColorSelectedEvent` | `id: u32, color: u32` | ColorWell |

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

## Utilities

```rust
fn set_blur_behind(window: &impl Widget, radius: u32)     // Frosted glass (0=disable)
fn screen_size() -> (u32, u32)                             // Display dimensions
fn show_notification(title: &str, message: &str, icon: Option<&[u32; 256]>, timeout_ms: u32)
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
| Canvas | `on_click`, `on_mouse_down`, `on_mouse_up`, `on_draw` |
| DataGrid | `on_selection_changed`, `on_submit` |
| TextEditor | `on_text_changed` |
| TreeView | `on_selection_changed`, `on_node_clicked`, `on_enter` |
| SearchField | `on_text_changed`, `on_submit` |
| ColorWell | `on_color_selected` |
| Window | `on_close`, `on_resize` |
| SplitView | `on_split_changed` |
| ScrollView | `on_scroll` |
| Sidebar | `on_selection_changed` |
| TabBar | `on_active_changed`, `on_tab_close` |
| ContextMenu | `on_item_click` |
| TableView | `on_selection_changed` |
| Expander | `on_toggled` |
