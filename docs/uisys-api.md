# anyOS UI System (uisys) API Reference

The **uisys** DLL is a shared library providing 31 macOS-inspired dark-themed UI components for anyOS GUI applications. It exports 84 functions via a C ABI function pointer table at virtual address `0x04000000`.

**DLL Address:** `0x04000000`
**Version:** 1
**Exports:** 84
**Client crate:** `uisys_client`

---

## Table of Contents

- [Getting Started](#getting-started)
- [Core Types](#core-types)
- [Components](#components)
  - [Label](#1-label)
  - [Button](#2-button)
  - [Toggle](#3-toggle)
  - [Checkbox](#4-checkbox)
  - [Radio Button](#5-radio-button)
  - [Slider](#6-slider)
  - [Stepper](#7-stepper)
  - [TextField](#8-textfield)
  - [SearchField](#9-searchfield)
  - [TextArea](#10-textarea)
  - [SegmentedControl](#11-segmentedcontrol)
  - [TabBar](#12-tabbar)
  - [Sidebar](#13-sidebar)
  - [NavigationBar](#14-navigationbar)
  - [Toolbar](#15-toolbar)
  - [TableView](#16-tableview)
  - [ScrollView](#17-scrollview)
  - [SplitView](#18-splitview)
  - [ContextMenu](#19-contextmenu)
  - [Card](#20-card)
  - [GroupBox](#21-groupbox)
  - [Badge](#22-badge)
  - [Tag](#23-tag)
  - [ProgressBar](#24-progressbar)
  - [Tooltip](#25-tooltip)
  - [StatusIndicator](#26-statusindicator)
  - [ColorWell](#27-colorwell)
  - [ImageView](#28-imageview)
  - [IconButton](#29-iconbutton)
  - [Divider](#30-divider)
  - [Alert](#31-alert)
- [v2 API Extensions](#v2-api-extensions)
  - [GPU Acceleration](#gpu-acceleration)
  - [Anti-Aliased Drawing](#anti-aliased-drawing)
  - [Font Rendering](#font-rendering)
  - [Shadow Effects](#shadow-effects)
  - [Blur Effects](#blur-effects)
- [Controls Module](#controls-module)
- [Design Patterns](#design-patterns)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../stdlib" }
uisys_client = { path = "../../programs/dll/uisys_client" }
```

### Minimal GUI Program

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use uisys_client::*;

anyos_std::entry!(main);

fn main() {
    let win = ui::window::create("My App", 100, 100, 400, 300);
    if win == u32::MAX { return; }

    let mut btn = button::UiButton::new(20, 20, 120, 32, types::ButtonStyle::Primary);
    let mut event = [0u32; 5];

    loop {
        while ui::window::get_event(win, &mut event) == 1 {
            let ev = types::UiEvent::from_raw(&event);
            if ev.event_type == 0 { return; } // window closed
            if btn.handle_event(&ev) {
                println!("Button clicked!");
            }
        }

        // Clear background
        ui::window::fill_rect(win, 0, 0, 400, 300, types::WINDOW_BG);

        // Draw UI
        btn.render(win, "Click Me");
        ui::window::present(win);

        process::yield_cpu();
    }
}
```

---

## Core Types

### Color Palette

All colors are 32-bit ARGB (`0xAARRGGBB`).

| Constant | Value | Usage |
|----------|-------|-------|
| `WINDOW_BG` | `0xFF1E1E1E` | Window background |
| `TEXT` | `0xFFE6E6E6` | Primary text |
| `TEXT_SECONDARY` | `0xFF969696` | Secondary/dimmed text |
| `TEXT_DISABLED` | `0xFF5A5A5A` | Disabled text |
| `ACCENT` | `0xFF007AFF` | Primary blue (buttons, sliders) |
| `ACCENT_HOVER` | `0xFF0A84FF` | Blue on hover |
| `DESTRUCTIVE` | `0xFFFF3B30` | Red (delete, errors) |
| `SUCCESS` | `0xFF30D158` | Green (success states) |
| `WARNING` | `0xFFFFD60A` | Yellow (warnings) |
| `CONTROL_BG` | `0xFF3C3C3C` | Control backgrounds |
| `SEPARATOR` | `0xFF3D3D3D` | Lines and borders |
| `SIDEBAR_BG` | `0xFF252525` | Sidebar background |
| `CARD_BG` | `0xFF2A2A2A` | Card/container background |

### Enums

#### ButtonStyle

| Variant | Value | Appearance |
|---------|-------|------------|
| `Default` | 0 | Gray background |
| `Primary` | 1 | Blue background (accent color) |
| `Destructive` | 2 | Red background |
| `Plain` | 3 | Text-only, no background |

#### ButtonState

| Variant | Value | Description |
|---------|-------|-------------|
| `Normal` | 0 | Default state |
| `Hover` | 1 | Mouse hovering |
| `Pressed` | 2 | Mouse button down |
| `Disabled` | 3 | Grayed out, not interactive |

#### CheckboxState

| Variant | Value | Description |
|---------|-------|-------------|
| `Unchecked` | 0 | Empty checkbox |
| `Checked` | 1 | Checkmark shown |
| `Indeterminate` | 2 | Dash/minus shown |

#### TextAlign

| Variant | Value |
|---------|-------|
| `Left` | 0 |
| `Center` | 1 |
| `Right` | 2 |

#### FontSize

| Variant | Value (pixels) |
|---------|----------------|
| `Small` | 11 |
| `Normal` | 13 |
| `Large` | 17 |
| `Title` | 24 |

#### StatusKind

| Variant | Value | Color |
|---------|-------|-------|
| `Online` | 0 | Green |
| `Warning` | 1 | Yellow |
| `Error` | 2 | Red |
| `Offline` | 3 | Gray |

### UiEvent

```rust
pub struct UiEvent {
    pub event_type: u32,   // EVENT_KEY_DOWN, EVENT_MOUSE_DOWN, etc.
    pub p1: u32,           // Mouse: x coordinate; Key: key code
    pub p2: u32,           // Mouse: y coordinate; Key: character value
    pub p3: u32,
    pub p4: u32,
}
```

**Helper methods:**
- `from_raw(raw: &[u32; 5]) -> UiEvent`
- `is_mouse_down() -> bool`
- `is_mouse_up() -> bool`
- `is_mouse_move() -> bool`
- `is_key_down() -> bool`
- `mouse_pos() -> (i32, i32)`
- `key_code() -> u32`
- `char_val() -> u32`
- `modifiers() -> u32`

### Key Codes

| Constant | Value |
|----------|-------|
| `KEY_ENTER` | `0x100` |
| `KEY_BACKSPACE` | `0x101` |
| `KEY_TAB` | `0x102` |
| `KEY_ESCAPE` | `0x103` |
| `KEY_SPACE` | `0x104` |
| `KEY_UP` | `0x105` |
| `KEY_DOWN` | `0x106` |
| `KEY_LEFT` | `0x107` |
| `KEY_RIGHT` | `0x108` |
| `KEY_DELETE` | `0x120` |
| `KEY_HOME` | `0x121` |
| `KEY_END` | `0x122` |
| `KEY_PAGE_UP` | `0x123` |
| `KEY_PAGE_DOWN` | `0x124` |

---

## Components

### 1. Label

Text rendering with multiple styles and alignment.

#### Raw Functions

```rust
// Single-line label
label::label(win, x, y, text, color, font_size, align);

// Measure text dimensions
let (w, h) = label::label_measure(text, font_size);

// Label with ellipsis truncation when exceeding max_width
label::label_ellipsis(win, x, y, text, color, font_size, max_width);

// Multi-line text with wrapping
label::label_multiline(win, x, y, text, color, font_size, max_width, line_spacing);
```

#### Example

```rust
use uisys_client::{label, types::*};

// Title text, centered
label::label(win, 200, 10, "Settings", TEXT, FontSize::Title as u32, TextAlign::Center as u32);

// Secondary description
label::label(win, 20, 50, "Configure your preferences below.", TEXT_SECONDARY,
    FontSize::Normal as u32, TextAlign::Left as u32);
```

---

### 2. Button

Interactive clickable button with multiple styles.

#### Stateful Component

```rust
pub struct UiButton {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub style: ButtonStyle,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h, style)` | `UiButton` | Create button |
| `render(&self, win, text)` | -- | Draw button |
| `handle_event(&mut self, event) -> bool` | `true` on click | Process events |

#### Example

```rust
let mut save_btn = button::UiButton::new(20, 250, 100, 32, ButtonStyle::Primary);
let mut cancel_btn = button::UiButton::new(130, 250, 100, 32, ButtonStyle::Default);

// In event loop:
if save_btn.handle_event(&ev) { /* save */ }
if cancel_btn.handle_event(&ev) { /* cancel */ }

// In render:
save_btn.render(win, "Save");
cancel_btn.render(win, "Cancel");
```

---

### 3. Toggle

On/off switch control (like iOS toggle).

#### Stateful Component

```rust
pub struct UiToggle {
    pub x: i32, pub y: i32,
    pub on: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, initial)` | `UiToggle` | Create toggle |
| `render(&self, win)` | -- | Draw toggle |
| `handle_event(&mut self, event) -> Option<bool>` | `Some(new_state)` on toggle | Process events |

---

### 4. Checkbox

Three-state checkbox with label.

#### Stateful Component

```rust
pub struct UiCheckbox {
    pub x: i32, pub y: i32,
    pub checked: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y)` | `UiCheckbox` | Create unchecked |
| `render(&self, win, text)` | -- | Draw with label |
| `handle_event(&mut self, event) -> Option<bool>` | `Some(new_state)` on click | Process events |

---

### 5. Radio Button

Radio button group for single selection.

#### Stateful Component

```rust
pub struct UiRadioGroup {
    pub x: i32, pub y: i32,
    pub spacing: i32,      // Vertical spacing (default 28)
    pub selected: usize,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y)` | `UiRadioGroup` | Create group |
| `render(&self, win, items: &[&str])` | -- | Draw all options |
| `handle_event(&mut self, event, num_items) -> Option<usize>` | `Some(index)` on selection | Process events |

#### Example

```rust
let mut radio = radio::UiRadioGroup::new(20, 80);

// In event loop:
if let Some(idx) = radio.handle_event(&ev, 3) {
    println!("Selected option {}", idx);
}

// In render:
radio.render(win, &["Small", "Medium", "Large"]);
```

---

### 6. Slider

Horizontal value slider with drag support.

#### Stateful Component

```rust
pub struct UiSlider {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub min: u32, pub max: u32, pub value: u32,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h, min, max, value)` | `UiSlider` | Create slider |
| `render(&self, win)` | -- | Draw slider |
| `handle_event(&mut self, event) -> Option<u32>` | `Some(new_value)` on change | Supports drag |

---

### 7. Stepper

Increment/decrement control with +/- buttons.

#### Stateful Component

```rust
pub struct UiStepper {
    pub x: i32, pub y: i32,
    pub value: i32,
    pub min: i32, pub max: i32,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, value, min, max)` | `UiStepper` | Create stepper |
| `render(&self, win)` | -- | Draw stepper |
| `handle_event(&mut self, event) -> Option<i32>` | `Some(new_value)` on +/- | Process events |

---

### 8. TextField

Single-line text input with cursor, selection, and placeholder.

#### Stateful Component

```rust
pub struct UiTextField {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub cursor: u32,
    pub focused: bool,
    pub password: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h)` | `UiTextField` | Create text field |
| `text(&self) -> &str` | Current text | Get contents |
| `set_text(&mut self, s)` | -- | Set contents |
| `clear(&mut self)` | -- | Clear text |
| `render(&self, win, placeholder)` | -- | Draw with placeholder |
| `handle_event(&mut self, event) -> bool` | `true` if text changed | Handles typing, backspace, arrows, home/end |

**Additional methods:**
- `has_selection() -> bool` -- Check if text is selected
- `selection_range() -> (usize, usize)` -- Get selection start/end
- `select_all(&mut self)` -- Select all text
- `selected_text(&self) -> &str` -- Get selected text

**Supported keys:** Printable ASCII (0x20-0x7E), BACKSPACE, DELETE, LEFT, RIGHT, HOME, END

**v2 Features:** The `textfield_ex` raw function supports password mode (dots instead of text) and text selection highlighting (sel_start, sel_end). The `UiTextField` struct has a `password: bool` field. Double-click selects all text.

#### Example

```rust
let mut name_field = textfield::UiTextField::new(20, 50, 200, 28);

// In event loop:
if name_field.handle_event(&ev) {
    println!("Text: {}", name_field.text());
}

// In render:
label::label(win, 20, 30, "Name:", TEXT, 16, 0);
name_field.render(win, "Enter your name");
```

---

### 9. SearchField

Search input with magnifying glass icon.

Same API as `UiTextField` but renders with a search icon prefix. Useful for filter/search bars.

```rust
let mut search = searchfield::UiSearchField::new(20, 10, 250, 28);
search.render(win, "Search...");
```

---

### 10. TextArea

Multi-line text editing with optional line numbers.

#### Raw Functions

```rust
// flags bit 0: show line numbers
textarea::textarea(win, x, y, w, h, text, cursor, scroll, flags);
textarea::textarea_hit_test(x, y, w, h, mx, my) -> bool;
```

---

### 11. SegmentedControl

Mutually exclusive segment selection (like iOS segmented control).

#### Stateful Component

```rust
pub struct UiSegmentedControl {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub selected: usize,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h)` | `UiSegmentedControl` | Create control |
| `render(&self, win, items: &[&str])` | -- | Draw segments |
| `handle_event(&mut self, event, count) -> Option<usize>` | `Some(index)` on select | Process events |

#### Example

```rust
let mut seg = segmented::UiSegmentedControl::new(20, 10, 300, 28);
seg.selected = 0;

// In render:
seg.render(win, &["General", "Display", "Network"]);
```

---

### 12. TabBar

Horizontal tab selection bar.

#### Stateful Component

```rust
pub struct UiTabBar {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub selected: usize,
}
```

Same pattern as SegmentedControl -- `render(win, items)` and `handle_event(event, count)`.

---

### 13. Sidebar

Vertical navigation menu with header and selectable items.

#### Stateful Component

```rust
pub struct UiSidebar {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub selected: usize,
    pub header_h: u32,   // Default 28
    pub item_h: u32,     // Default 32
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h)` | `UiSidebar` | Create sidebar |
| `render(&self, win, header, items: &[&str])` | -- | Draw sidebar |
| `handle_event(&mut self, event, count) -> Option<usize>` | `Some(index)` on select | Process events |

#### Example

```rust
let mut sidebar = sidebar::UiSidebar::new(0, 0, 180, 300);

sidebar.render(win, "Settings", &["General", "Display", "Sound", "Network"]);

if let Some(idx) = sidebar.handle_event(&ev, 4) {
    // Switch to section idx
}
```

---

### 14. NavigationBar

Top navigation bar with title and optional back button.

#### Stateful Component

```rust
pub struct UiNavbar {
    pub x: i32, pub y: i32,
    pub w: u32,
    pub show_back: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w)` | `UiNavbar` | Create navbar |
| `render(&self, win, title)` | -- | Draw navbar |
| `handle_event(&self, event) -> bool` | `true` if back clicked | Check back button |

---

### 15. Toolbar

Horizontal toolbar with buttons.

#### Raw Functions

```rust
// Draw toolbar background
toolbar::toolbar(win, x, y, w, h);

// Draw toolbar button
toolbar::toolbar_button(win, x, y, w, h, text, state);
```

#### Stateful Component

```rust
pub struct UiToolbarButton {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub enabled: bool,
}
```

---

### 16. TableView

Scrollable table with rows, headers, and row selection.

#### Stateful Component

```rust
pub struct UiTableView {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub row_h: u32,              // Default 28
    pub selected: Option<usize>,
    pub scroll: u32,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h)` | `UiTableView` | Create table |
| `render_bg(&self, win, num_rows)` | -- | Draw background and selection |
| `render_row(&self, win, index, text)` | -- | Draw one row |
| `handle_event(&mut self, event, num_rows) -> Option<usize>` | `Some(row)` on select | Process events |

#### Example

```rust
let mut table = tableview::UiTableView::new(20, 50, 360, 200);
let items = ["File A", "File B", "File C"];

// In render:
table.render_bg(win, items.len());
for (i, item) in items.iter().enumerate() {
    table.render_row(win, i, item);
}
```

---

### 17. ScrollView

Vertical scrollbar with thumb tracking.

#### Stateful Component

```rust
pub struct UiScrollbar {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub content_h: u32,
    pub scroll: u32,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h, content_h)` | `UiScrollbar` | Create scrollbar |
| `max_scroll() -> u32` | Maximum scroll | Content height - visible height |
| `render(&self, win)` | -- | Draw scrollbar |
| `handle_event(&mut self, event) -> Option<u32>` | `Some(new_scroll)` on change | Drag support |

---

### 18. SplitView

Vertical divider between two panes with draggable separator.

#### Stateful Component

```rust
pub struct UiSplitView {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub split_x: u32,
    pub min_left: u32,
    pub min_right: u32,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, w, h, split_x)` | `UiSplitView` | Create split view |
| `render(&self, win)` | -- | Draw divider |
| `handle_event(&mut self, event) -> Option<u32>` | `Some(new_split_x)` on drag | Process events |

---

### 19. ContextMenu

Popup menu with items and separators.

#### Stateful Component

```rust
pub struct UiContextMenu {
    pub x: i32, pub y: i32,
    pub w: u32,
    pub item_h: u32,   // Default 28
    pub visible: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(w)` | `UiContextMenu` | Create hidden menu |
| `show(&mut self, x, y)` | -- | Show at position |
| `hide(&mut self)` | -- | Hide menu |
| `render(&self, win, items: &[&str])` | -- | Draw if visible |
| `handle_event(&mut self, event, count) -> Option<usize>` | `Some(index)` on select | Auto-hides on select |

#### Example

```rust
let mut menu = contextmenu::UiContextMenu::new(150);

// On right-click:
if ev.is_mouse_down() {
    let (mx, my) = ev.mouse_pos();
    menu.show(mx, my);
}

if let Some(idx) = menu.handle_event(&ev, 3) {
    match idx {
        0 => { /* Cut */ }
        1 => { /* Copy */ }
        2 => { /* Paste */ }
        _ => {}
    }
}

menu.render(win, &["Cut", "Copy", "Paste"]);
```

---

### 20. Card

Rounded container for grouped content.

```rust
// Draw a card background with rounded corners
card::card(win, x, y, w, h);

// Then draw content on top
label::label(win, x + 16, y + 12, "Card Title", TEXT, FontSize::Normal as u32, 0);
```

---

### 21. GroupBox

Container with a title label.

```rust
groupbox::groupbox(win, x, y, w, h, "Settings Group");
```

---

### 22. Badge

Notification badge (count or dot).

```rust
// Badge with count
badge::badge(win, x, y, 5);  // Shows "5"

// Small dot badge (no number)
badge::badge_dot(win, x, y);
```

---

### 23. Tag

Chip/tag with optional close button.

#### Stateful Component

```rust
pub struct UiTag {
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub bg: u32, pub fg: u32,
    pub show_close: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, bg, fg)` | `UiTag` | Create tag |
| `render(&self, win, text)` | -- | Draw tag |
| `handle_event(&self, event) -> bool` | `true` on body click | Tag clicked |
| `handle_close(&self, event) -> bool` | `true` on X click | Close button clicked |

---

### 24. ProgressBar

Determinate or indeterminate progress indicator.

```rust
// Determinate (0-100%)
progress::progress(win, x, y, w, h, 75);

// Indeterminate (animated)
progress::progress_indeterminate(win, x, y, w, h);
```

---

### 25. Tooltip

Floating tooltip text.

```rust
// Show on hover
tooltip::tooltip(win, mouse_x, mouse_y - 24, "Save document");
```

---

### 26. StatusIndicator

Colored status dot with label.

```rust
use uisys_client::types::StatusKind;

status_indicator::status_indicator(win, x, y, StatusKind::Online, "Connected");
status_indicator::status_indicator(win, x, y + 20, StatusKind::Error, "Disconnected");
```

---

### 27. ColorWell

Color preview swatch.

```rust
colorwell::colorwell(win, x, y, 24, 0xFF007AFF);  // Blue swatch
```

---

### 28. ImageView

Render ARGB pixel data with scaling.

```rust
// pixels: &[u32] of ARGB values, img_w x img_h
imageview::imageview(win, x, y, display_w, display_h, &pixels, img_w, img_h);
```

---

### 29. IconButton

Circular or square colored button.

#### Stateful Component

```rust
pub struct UiIconButton {
    pub x: i32, pub y: i32,
    pub size: u32,
    pub shape: u8,     // 0 = circle, 1 = square
    pub color: u32,
    pub enabled: bool,
}
```

| Method | Returns | Description |
|--------|---------|-------------|
| `new(x, y, size, color)` | `UiIconButton` | Create (circle by default) |
| `render(&self, win)` | -- | Draw button |
| `handle_event(&self, event) -> bool` | `true` on click | Process events |

---

### 30. Divider

Visual separator lines.

```rust
// Horizontal line
divider::divider_h(win, x, y, width);

// Vertical line
divider::divider_v(win, x, y, height);
```

---

### 31. Alert

Modal alert dialog panel with title, message, and action buttons.

#### Raw Functions

```rust
// Draw alert panel background with title and message
alert::alert(win, x, y, w, h, "Title", "Message text here");

// Draw alert button (same style as regular buttons)
alert::alert_button(win, x, y, w, h, "OK", ButtonStyle::Primary, ButtonState::Normal);
```

The alert component draws a centered panel with title in bold, message text below, and buttons at the bottom. Use in combination with a semi-transparent overlay for modal behavior.

---

## v2 API Extensions

These functions were added in the v2 export table update (exports 70-84).

### GPU Acceleration

```rust
// Check if GPU acceleration (VMware SVGA II) is available
let accel = uisys_client::gpu_has_accel();
```

### Anti-Aliased Drawing

```rust
// Fill a rounded rectangle with anti-aliased edges
uisys_client::fill_rounded_rect_aa(win, x, y, w, h, radius, color);
```

Produces smoother corners than the standard `fill_rounded_rect` by using sub-pixel alpha blending at edges.

### Font Rendering

```rust
// Draw text with a specific loaded font and size
uisys_client::draw_text_with_font(win, x, y, color, size, font_id, "Hello");

// Measure text dimensions with a specific font
let (w, h) = uisys_client::font_measure(font_id, size, "Hello");
```

These functions use the libfont DLL for TrueType rendering instead of the built-in bitmap font.

### Shadow Effects

Draw drop shadows behind UI elements. These operate on raw pixel buffers (for compositor use).

```rust
use uisys_client::*;

// Rectangle shadow
draw_shadow_rect_buf(&mut pixels, fb_w, fb_h, x, y, w, h, offset_x, offset_y, spread, alpha);

// Rounded rectangle shadow
draw_shadow_rounded_rect_buf(&mut pixels, fb_w, fb_h, x, y, w, h, radius, offset_x, offset_y, spread, alpha);

// Oval/ellipse shadow
draw_shadow_oval_buf(&mut pixels, fb_w, fb_h, cx, cy, rx, ry, offset_x, offset_y, spread, alpha);
```

**Parameters:**
- `offset_x`, `offset_y` -- Shadow offset from the element
- `spread` -- Shadow blur spread radius
- `alpha` -- Shadow opacity (0-255)

### Blur Effects

Apply box blur to regions of a pixel buffer (for frosted glass, background blur).

```rust
use uisys_client::*;

// Blur a rectangular region
blur_rect_buf(&mut pixels, fb_w, fb_h, x, y, w, h, radius, passes);

// Blur a rounded rectangle region
blur_rounded_rect_buf(&mut pixels, fb_w, fb_h, x, y, w, h, corner_radius, blur_radius, passes);
```

**Parameters:**
- `radius` -- Blur kernel radius (larger = more blurry)
- `passes` -- Number of blur iterations (2-3 recommended for quality)

---

## Controls Module

Load system control icons from the icon set.

```rust
use uisys_client::controls;

if let Some(icon) = controls::load_control_icon("close", 16) {
    // icon.pixels: Vec<u32> (ARGB8888)
    // icon.width, icon.height: u32
}
```

The `ControlIcon` struct contains decoded ARGB pixel data ready for blitting.

---

## Design Patterns

### Component Architecture

Every interactive component follows the same pattern:

1. **Raw functions** -- Direct FFI wrappers for rendering and hit testing
2. **Stateful `Ui*` struct** -- Manages position, value, and interaction state

```rust
// 1. Create the component
let mut toggle = toggle::UiToggle::new(20, 50, false);

// 2. Handle events (returns Some/true when state changes)
if let Some(new_state) = toggle.handle_event(&ev) {
    println!("Toggle is now {}", if new_state { "ON" } else { "OFF" });
}

// 3. Render
toggle.render(win);
```

### Event Flow

```
Window Event Queue
    |
    v
ui::window::get_event()
    |
    v
UiEvent::from_raw()
    |
    v
component.handle_event(&ev)  -->  State change?  -->  Update app state
    |
    v
component.render(win)
    |
    v
ui::window::present(win)
```

### Coordinate System

- **Origin**: Top-left corner of the window content area
- **X**: Increases rightward
- **Y**: Increases downward
- **Coordinates**: Signed `i32` (allows off-screen positioning)

### String Handling

All string parameters are copied to a 256-byte buffer with NUL termination internally. Maximum string length is 255 characters.

### Color Format

32-bit ARGB throughout: `0xAARRGGBB`

- Alpha `0xFF` = fully opaque (required for most components)
- Use the predefined palette constants for consistent theming

### Rendering Order

Components are drawn in painter's order (last drawn = on top). For popup menus or tooltips, render them last:

```rust
// Draw base UI first
sidebar.render(win, "Menu", &items);
table.render_bg(win, rows.len());

// Draw popups on top
menu.render(win, &menu_items);
tooltip::tooltip(win, tx, ty, "Help text");

// Always present last
ui::window::present(win);
```

### Full Application Example

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use uisys_client::*;

anyos_std::entry!(main);

fn main() {
    let win = ui::window::create("Settings", 100, 80, 500, 400);
    if win == u32::MAX { return; }

    // Create components
    let mut sidebar = sidebar::UiSidebar::new(0, 0, 150, 400);
    let mut dark_mode = toggle::UiToggle::new(280, 60, true);
    let mut volume = slider::UiSlider::new(280, 120, 180, 20, 0, 100, 75);
    let mut save_btn = button::UiButton::new(280, 350, 100, 32, types::ButtonStyle::Primary);

    let sections = ["General", "Display", "Sound"];
    let mut event = [0u32; 5];

    loop {
        while ui::window::get_event(win, &mut event) == 1 {
            let ev = types::UiEvent::from_raw(&event);
            if ev.event_type == 0 { return; }

            sidebar.handle_event(&ev, sections.len());
            dark_mode.handle_event(&ev);
            volume.handle_event(&ev);

            if save_btn.handle_event(&ev) {
                println!("Settings saved!");
            }
        }

        // Clear
        ui::window::fill_rect(win, 0, 0, 500, 400, types::WINDOW_BG);

        // Draw
        sidebar.render(win, "Settings", &sections);

        label::label(win, 170, 20, "General", types::TEXT,
            types::FontSize::Title as u32, types::TextAlign::Left as u32);

        divider::divider_h(win, 170, 48, 310);

        label::label(win, 170, 60, "Dark Mode", types::TEXT, 16, 0);
        dark_mode.render(win);

        label::label(win, 170, 120, "Volume", types::TEXT, 16, 0);
        volume.render(win);

        save_btn.render(win, "Save");

        ui::window::present(win);
        process::yield_cpu();
    }
}
```
