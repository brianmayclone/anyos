# anyOS WebView Library (libwebview) API Reference

The **libwebview** library is a complete HTML/CSS/JS rendering engine that parses web content and produces real libanyui controls (Labels, Views, ImageViews, TextFields, etc.) positioned by a CSS layout engine. It is used by the Surf web browser application.

**Type:** Rust `no_std` library (statically linked, not a DLL)
**Location:** `libs/libwebview/`
**Dependencies:** `libjs` (JavaScript engine), `libanyui_client` (UI controls), `anyos_std`

---

## Table of Contents

- [Getting Started](#getting-started)
- [WebView API](#webview-api)
- [Callbacks](#callbacks)
- [Image Cache](#image-cache)
- [Form Handling](#form-handling)
- [JavaScript Integration](#javascript-integration)
- [HTML Parser](#html-parser)
- [CSS Engine](#css-engine)
- [Layout Engine](#layout-engine)
- [DOM API](#dom-api)
- [Debug Logging](#debug-logging)

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libanyui_client = { path = "../../libs/libanyui_client" }
libwebview = { path = "../../libs/libwebview" }
```

### Minimal Example

```rust
use libwebview::WebView;
use libanyui_client as ui;

let mut wv = WebView::new(800, 600);
parent_view.add(wv.scroll_view());
wv.scroll_view().set_dock(ui::DOCK_FILL);
wv.set_html("<h1>Hello World</h1><p>This is rendered with real controls.</p>");
```

### Architecture

libwebview is a pipeline of four stages:

1. **HTML Parser** (`html::parse`) -- Tokenizes HTML and builds an arena-based DOM tree
2. **CSS Engine** (`css::parse_stylesheet` + `style::resolve_styles`) -- Parses CSS, resolves cascade and specificity, computes per-node styles
3. **Layout Engine** (`layout::layout`) -- Produces a tree of `LayoutBox`es with absolute positions and sizes
4. **Renderer** (`renderer::Renderer`) -- Maps layout boxes to real libanyui controls inside a ScrollView

JavaScript execution happens after rendering via the `js::JsRuntime`, which uses libjs to run `<script>` tags and provides a native DOM API.

---

## WebView API

### `WebView::new(w: u32, h: u32) -> WebView`

Create a new WebView with the given initial viewport dimensions. Internally creates a `ScrollView` containing a content `View` with a white background.

### `scroll_view(&self) -> &ScrollView`

Returns the ScrollView container. Add this to your window or parent view.

```rust
let wv = WebView::new(800, 600);
window.add(wv.scroll_view());
wv.scroll_view().set_dock(ui::DOCK_FILL);
```

### `content_view(&self) -> &View`

Returns the inner content View. All rendered controls are children of this view. The content view height grows to match the full document height.

### `set_html(html: &str)`

Parse HTML content and render it. This runs the full pipeline: HTML parse, CSS resolve, layout, render controls, and execute `<script>` tags. Call `set_url()` before this method so JavaScript has the correct `window.location`.

### `set_url(url: &str)`

Set the current page URL. Must be called before `set_html()` so that the JS environment has the correct `window.location` / `document.location` values when scripts run.

### `add_stylesheet(css_text: &str)`

Add an external CSS stylesheet (as raw text). Applied on the next `set_html()` or `relayout()` call. Multiple stylesheets can be added and they stack in order.

### `clear_stylesheets()`

Remove all previously added external stylesheets.

### `add_image(src: &str, pixels: Vec<u32>, w: u32, h: u32)`

Add a decoded image to the internal cache. The `src` string should match the `src` attribute of `<img>` elements in the HTML. The image will appear on the next render or `relayout()`. Pixels are ARGB8888 format.

### `get_title() -> Option<String>`

Return the page title from the current DOM (the text content of the first `<title>` element). Returns `None` if no DOM is loaded or no title element exists.

### `total_height() -> i32`

Return the total document height in pixels. Updated after each `set_html()` or `relayout()`. Fixed-position elements are excluded from this calculation.

### `resize(w: u32, h: u32)`

Resize the viewport and re-layout. If a DOM is loaded, triggers a full relayout at the new width.

### `relayout()`

Re-run layout and rendering with the current DOM and stylesheets. Call this after adding images or stylesheets to update the display without re-parsing HTML.

### `tick(delta_ms: u64) -> bool`

Advance CSS animations/transitions and JS timers (setTimeout, setInterval, requestAnimationFrame) by `delta_ms` milliseconds. Returns `true` if any animation changed the document (a relayout was performed). Call at ~60 fps when pages may have running animations.

```rust
// In a 60fps timer callback:
if webview.tick(16) {
    // Layout changed due to animation
}
```

### `clear()`

Clear all content: removes all rendered controls, resets the DOM, and sets height to zero.

### `dom() -> Option<&Dom>`

Access the current DOM tree (read-only). Returns `None` if no HTML has been loaded.

### `link_url_for(control_id: u32) -> Option<&str>`

Look up the link URL for a control ID. Used in link click callbacks to determine which URL the user clicked.

### `js_runtime() -> &mut JsRuntime`

Access the JavaScript runtime. Used for evaluating additional scripts, reading pending WebSocket operations, or setting cookies.

### `js_console() -> &[String]`

Get console output from JavaScript execution (lines logged via `console.log`, `console.warn`, `console.error`).

---

## Callbacks

### `set_link_callback(cb: Callback, userdata: u64)`

Register a callback invoked when the user clicks a hyperlink. The callback receives the control ID of the clicked label. Use `link_url_for(control_id)` to resolve the URL.

```rust
extern "C" fn on_link_click(ctrl_id: u32, _evt: u32, userdata: u64) {
    let wv = get_webview();
    if let Some(url) = wv.link_url_for(ctrl_id) {
        navigate_to(url);
    }
}
wv.set_link_callback(on_link_click, 0);
```

### `set_submit_callback(cb: Callback, userdata: u64)`

Register a callback invoked when the user clicks a form submit button. The callback receives the control ID of the clicked button. Use `form_action_for()` and `collect_form_data()` to process the submission.

---

## Form Handling

### `form_controls() -> &[FormControl]`

Get all rendered form controls on the current page. Each `FormControl` contains:

| Field | Type | Description |
|-------|------|-------------|
| `control_id` | `u32` | The libanyui control ID |
| `node_id` | `usize` | The DOM node ID |
| `kind` | `FormFieldKind` | Field type |
| `name` | `String` | Input name attribute |

### FormFieldKind

| Variant | HTML Element |
|---------|-------------|
| `TextInput` | `<input type="text">` |
| `Password` | `<input type="password">` |
| `Submit` | `<input type="submit">` |
| `Checkbox` | `<input type="checkbox">` |
| `Radio` | `<input type="radio">` |
| `Hidden` | `<input type="hidden">` |
| `ButtonEl` | `<button>` |
| `Textarea` | `<textarea>` |

### `is_submit_button(control_id: u32) -> bool`

Check if a control ID belongs to a submit button (Submit or ButtonEl).

### `form_action_for(control_id: u32) -> Option<(String, String)>`

Find the form action URL and method for a submit button click. Walks up the DOM from the button to find the parent `<form>` and reads its `action` and `method` attributes. Returns `(action_url, method)` where method is uppercased (e.g. "GET", "POST").

### `collect_form_data(control_id: u32) -> Vec<(String, String)>`

Collect all name=value pairs from the form containing the given submit button. Reads current values from live libanyui TextFields and Checkboxes. Only controls with a `name` attribute are included. Checkboxes and radio buttons are included only when checked.

---

## Image Cache

The `ImageCache` stores decoded ARGB8888 pixel data keyed by URL string. Images are added via `WebView::add_image()` and used during layout to determine `<img>` element dimensions and during rendering to display the image.

```rust
// Decode image externally, then add to cache:
wv.add_image("https://example.com/photo.jpg", pixels, 640, 480);
wv.relayout(); // Re-render to show the image
```

---

## JavaScript Integration

libwebview integrates with **libjs** (a JavaScript engine) to execute `<script>` tags and provide a browser-like DOM API. All DOM objects are created as native JsObject instances in Rust with native function methods -- no JS injection.

### Script Execution

Scripts are executed automatically after `set_html()`. The JS runtime provides:

- `console.log()`, `console.warn()`, `console.error()` -- output available via `js_console()`
- `setTimeout()`, `setInterval()`, `clearTimeout()`, `clearInterval()` -- advanced by `tick()`
- `requestAnimationFrame()` -- advanced by `tick()`

### Window API

The `window` global object provides:

| Property/Method | Description |
|----------------|-------------|
| `window.document` | Reference to the document object |
| `window.location` | URL location object (protocol, hostname, port, pathname, search, hash) |
| `window.navigator` | Navigator with userAgent, language, platform |
| `window.screen` | Screen dimensions and color depth |
| `window.innerWidth` / `innerHeight` | Viewport dimensions |
| `window.localStorage` | Persistent key-value storage (backed by filesystem) |
| `window.sessionStorage` | Session key-value storage (in-memory) |
| `setTimeout(fn, ms)` | Schedule a one-shot timer |
| `setInterval(fn, ms)` | Schedule a repeating timer |
| `requestAnimationFrame(fn)` | Schedule next-frame callback |
| `alert()`, `confirm()`, `prompt()` | Dialog stubs |
| `atob()`, `btoa()` | Base64 encode/decode |
| `encodeURIComponent()` / `decodeURIComponent()` | URI encoding |
| `JSON.parse()` / `JSON.stringify()` | JSON serialization |

### Document API

The `document` object provides:

| Method | Description |
|--------|-------------|
| `getElementById(id)` | Find element by ID attribute |
| `getElementsByClassName(name)` | Find elements by class name |
| `getElementsByTagName(tag)` | Find elements by tag name |
| `querySelector(sel)` | CSS selector query (first match) |
| `querySelectorAll(sel)` | CSS selector query (all matches) |
| `createElement(tag)` | Create a new DOM element |
| `createTextNode(text)` | Create a new text node |
| `createDocumentFragment()` | Create a document fragment |

Additional properties: `document.title`, `document.cookie`, `document.location`, `document.body`, `document.documentElement`, `document.readyState`, `document.head`.

### Element API

Each DOM element object provides:

| Property/Method | Description |
|----------------|-------------|
| `tagName` | Uppercase tag name (e.g. "DIV") |
| `id`, `className` | ID and class attributes |
| `textContent` | Get/set text content |
| `innerHTML` | Get/set inner HTML (parsed on set) |
| `style` | Inline style object |
| `getAttribute(name)` | Read attribute value |
| `setAttribute(name, value)` | Set attribute value |
| `removeAttribute(name)` | Remove attribute |
| `hasAttribute(name)` | Check attribute existence |
| `appendChild(child)` | Append child node |
| `removeChild(child)` | Remove child node |
| `insertBefore(new, ref)` | Insert before reference node |
| `parentNode` / `parentElement` | Parent references |
| `children` / `childNodes` | Child collections |
| `firstChild` / `lastChild` | First/last child |
| `firstElementChild` / `lastElementChild` | First/last element child |
| `nextSibling` / `previousSibling` | Adjacent siblings (all nodes) |
| `nextElementSibling` / `previousElementSibling` | Adjacent siblings (elements only) |
| `querySelector(sel)` / `querySelectorAll(sel)` | Scoped CSS queries |
| `classList` | ClassList object (add, remove, toggle, contains) |
| `nodeType` | Node type constant (1=element, 3=text) |
| `addEventListener(type, fn)` | Register event listener (click, input, change, submit) |

### XMLHttpRequest

Implements the standard XHR lifecycle. `send()` performs the HTTP request synchronously via the host's HTTP stack, then fires state change callbacks.

```javascript
var xhr = new XMLHttpRequest();
xhr.open("GET", "/api/data");
xhr.onreadystatechange = function() {
    if (xhr.readyState === 4 && xhr.status === 200) {
        var data = JSON.parse(xhr.responseText);
    }
};
xhr.send();
```

### Fetch API

`fetch(url, options)` performs HTTP requests and returns a Promise-like result. Since the Promise implementation is synchronous, requests are made immediately.

```javascript
fetch("/api/data")
    .then(function(response) { return response.json(); })
    .then(function(data) { console.log(data); });
```

### WebSocket

`new WebSocket(url[, protocols])` creates a WebSocket connection. The host application (surf) handles the TCP connection and RFC 6455 handshake. Messages are exchanged via pending mutation queues.

```javascript
var ws = new WebSocket("wss://echo.example.com");
ws.onopen = function() { ws.send("Hello"); };
ws.onmessage = function(e) { console.log(e.data); };
```

### localStorage / sessionStorage

`localStorage` is persistent -- backed by a file at `/tmp/surf_ls_<origin>.dat` using a simple tab-separated key-value format. `sessionStorage` is in-memory only. Both implement the standard Storage interface: `getItem()`, `setItem()`, `removeItem()`, `clear()`, `key()`, `length`.

---

## HTML Parser

### `html::parse(html: &str) -> Dom`

Parse a complete HTML document into an arena-based DOM tree. Handles:

- Entity decoding (numeric `&#NNN;` / `&#xHH;` and 150+ named entities)
- Void elements (self-closing: `<br>`, `<hr>`, `<img>`, `<input>`, `<meta>`, `<link>`, etc.)
- Auto-closing (`<p>` closed by block elements, `<li>` by `<li>`, `<td>` by `<td>`, `<tr>` by `<tr>`)
- Implicit `<html>`, `<head>`, `<body>` structure
- Raw text elements (`<script>`, `<style>` content not parsed as HTML)
- Whitespace collapsing (except inside `<pre>`)
- Error recovery for malformed HTML
- Comments and doctypes (skipped)

### `html::parse_fragment(html: &str) -> Dom`

Parse an HTML fragment (for `innerHTML`). No implicit html/head/body wrapping. Returns a DOM whose root is a synthetic container.

### Supported HTML Elements

**Document structure:** html, head, title, body, style, link, meta, script, noscript, template

**Headings:** h1, h2, h3, h4, h5, h6

**Content sectioning:** div, section, header, footer, nav, main, article, aside, hgroup, address

**Text content:** p, br, hr, pre, blockquote, figure, figcaption, details, summary, dialog

**Inline text semantics:** a, span, em, strong, b, i, u, s, code, mark, small, sub, sup, kbd, samp, var, abbr, cite, dfn, q, time, del, ins, bdi, bdo, data, ruby, rt, rp, wbr

**Lists:** ul, ol, li, dl, dt, dd

**Tables:** table, thead, tbody, tfoot, tr, th, td, caption, colgroup, col

**Forms:** form, input, button, textarea, select, option, optgroup, label, fieldset, legend, datalist, output, progress, meter

**Media/embedded:** img, audio, video, source, track, canvas, svg, iframe, embed, object, param, picture, map, area

**Deprecated (still parsed):** center, font, nobr, tt

---

## CSS Engine

### `css::parse_stylesheet(css: &str) -> Stylesheet`

Parse a CSS stylesheet into rules, @media rules, and @keyframes blocks.

### Selectors

| Selector Type | Syntax | Example |
|--------------|--------|---------|
| Tag | `tag` | `div`, `p`, `a` |
| Class | `.class` | `.header`, `.active` |
| ID | `#id` | `#main`, `#nav` |
| Universal | `*` | `*` |
| Descendant | `A B` | `div p` |
| Child | `A > B` | `ul > li` |
| Adjacent sibling | `A + B` | `h1 + p` |
| General sibling | `A ~ B` | `h1 ~ p` |
| Attribute exists | `[attr]` | `[disabled]` |
| Attribute exact | `[attr=val]` | `[type="text"]` |
| Attribute contains | `[attr~=val]` | `[class~="item"]` |
| Attribute prefix | `[attr^=val]` | `[href^="https"]` |
| Attribute suffix | `[attr$=val]` | `[src$=".png"]` |
| Attribute substring | `[attr*=val]` | `[class*="col"]` |
| Attribute dash-match | `[attr\|=val]` | `[lang\|="en"]` |

### Pseudo-Classes

`:hover`, `:active`, `:focus`, `:visited`, `:first-child`, `:last-child`, `:nth-child(n)`, `:nth-last-child(n)`, `:first-of-type`, `:last-of-type`, `:not(selector)`, `:empty`, `:checked`, `:disabled`, `:enabled`, `:root`

### @media Queries

Responsive media queries with conditions:

```css
@media (min-width: 768px) { ... }
@media (max-width: 600px) { ... }
@media (min-height: 400px) { ... }
@media (max-height: 800px) { ... }
@media (prefers-color-scheme: dark) { ... }
```

Viewport dimensions are passed during style resolution for correct evaluation.

### @keyframes

CSS keyframe animations are fully supported:

```css
@keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
}
.element {
    animation: fade-in 0.3s ease-in-out;
}
```

### CSS Properties

**Display:** display (block, inline, inline-block, list-item, flex, inline-flex, grid, inline-grid, none, table-row, table-cell)

**Box model:** width, height, min-width, max-width, min-height, max-height, margin (and individual sides), padding (and individual sides), border (and individual sides), border-radius, border-color, border-width, border-style, box-sizing

**Typography:** color, font-size, font-weight, font-style, text-align, text-decoration, text-indent, text-transform, line-height, vertical-align, white-space

**Background:** background-color, background

**Positioning:** position (static, relative, absolute, fixed), top, right, bottom, left, z-index

**Flexbox:** flex-direction, flex-wrap, justify-content, align-items, align-self, align-content, flex-grow, flex-shrink, flex-basis, flex (shorthand), gap, row-gap, column-gap, order

**Grid:** grid-template-columns, grid-template-rows, grid-auto-columns, grid-auto-rows, grid-auto-flow, grid-column, grid-column-start, grid-column-end, grid-row, grid-row-start, grid-row-end, grid-area, justify-items

**Float:** float, clear

**Lists:** list-style-type (disc, circle, square, decimal, none)

**Visual:** opacity, visibility, cursor, overflow, overflow-x, overflow-y

**Table:** border-collapse, border-spacing, table-layout

**Transitions:** transition (shorthand), transition-property, transition-duration, transition-timing-function, transition-delay

**Animations:** animation (shorthand), animation-name, animation-duration, animation-timing-function, animation-delay, animation-iteration-count, animation-direction, animation-fill-mode, animation-play-state

**Custom properties:** `--name` (CSS variables via `var(--name)`)

### CSS Values

| Type | Examples |
|------|---------|
| Keywords | `auto`, `none`, `bold`, `center`, `flex` |
| Colors | `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `rgb()`, `rgba()`, named colors |
| Lengths | `px`, `em`, `rem`, `%`, `vw`, `vh` |
| Numbers | `0`, `1.5`, `100` |

### Style Resolution

Cascade order: initial values, browser defaults (DEFAULT_CSS), author rules (by specificity), inline styles. Inheritable properties that are not explicitly set are inherited from the parent node. `!important` declarations are supported.

---

## Layout Engine

The layout engine takes a DOM tree and per-node computed styles and produces a tree of `LayoutBox`es with absolute positions and sizes.

### Layout Modes

**Block layout** (`layout/block.rs`) -- Standard block formatting context. Elements stack vertically, margins collapse, percentage widths resolve against the containing block.

**Inline layout** (`layout/inline.rs`) -- Text and inline elements flow left to right, wrapping to new line boxes when the available width is exceeded. Handles word breaking, text measurement via libanyui, and inline-block elements.

**Flexbox layout** (`layout/flex.rs`) -- CSS Flexible Box Layout. Supports:
- `flex-direction`: row, row-reverse, column, column-reverse
- `flex-wrap`: nowrap, wrap, wrap-reverse
- `justify-content`: flex-start, flex-end, center, space-between, space-around, space-evenly
- `align-items` / `align-self`: flex-start, flex-end, center, stretch, baseline
- `flex-grow`, `flex-shrink`, `flex-basis`
- `order` for item reordering
- `gap` between items

**Grid layout** (`layout/grid.rs`) -- CSS Grid Layout. Supports:
- Explicit track sizing via `grid-template-columns` / `grid-template-rows`
- Track units: `px`, `fr`, `%`, `auto`
- Explicit item placement with `grid-column-start/end` and `grid-row-start/end`
- `span N` for items spanning multiple tracks
- Auto-placement (row-major scanning)
- `column-gap` / `row-gap`
- `justify-items` / `align-items` within cells
- Limitations: no named grid lines, no `grid-template-areas`, no `minmax()`, no subgrid

**Table layout** (`layout/table.rs`) -- HTML table layout. Supports:
- Automatic column width distribution
- `colspan` attribute
- `cellpadding` / `cellspacing` attributes
- `width` attribute on table, td, th
- `align` and `valign` attributes
- `<thead>`, `<tbody>`, `<tfoot>` section grouping
- `<caption>` element
- `border` attribute

### Positioning

- **Static** -- Normal flow
- **Relative** -- Offset from normal position
- **Absolute** -- Positioned relative to nearest positioned ancestor
- **Fixed** -- Positioned relative to the viewport (excluded from document height)

### Float

CSS `float: left` / `float: right` is supported. Floated elements are removed from normal flow and positioned to the left or right of their container. Subsequent content wraps around floats. The `clear` property advances content past floated elements.

### LayoutBox

Each layout box contains:

| Field | Type | Description |
|-------|------|-------------|
| `node_id` | `Option<NodeId>` | DOM node this box represents |
| `box_type` | `BoxType` | Block, Inline, InlineBlock, Anonymous, LineBox |
| `x`, `y` | `i32` | Position relative to parent |
| `width`, `height` | `i32` | Dimensions in pixels |
| `margin`, `padding` | `Edges` | Box model edges (top, right, bottom, left) |
| `text` | `Option<String>` | Text content for text runs |
| `font_size` | `i32` | Font size in pixels |
| `bold`, `italic` | `bool` | Font style flags |
| `color`, `bg_color` | `u32` | ARGB8888 text and background colors |
| `border_width`, `border_color`, `border_radius` | `i32`/`u32` | Border properties |
| `link_url` | `Option<String>` | Hyperlink URL for `<a>` elements |
| `image_src` | `Option<String>` | Image URL for `<img>` elements |
| `form_field` | `Option<FormFieldKind>` | Form field type for input elements |
| `overflow_hidden` | `bool` | Clip children to box bounds |
| `visibility_hidden` | `bool` | Invisible but occupies space |
| `opacity` | `i32` | 0..255 opacity value |
| `is_fixed` | `bool` | Position:fixed (viewport-relative) |

---

## DOM API

The DOM tree uses an arena-based flat `Vec<DomNode>` structure. Nodes are referenced by `NodeId` (a `usize` index), avoiding recursive Box/Rc trees.

### Dom

| Method | Description |
|--------|-------------|
| `Dom::new()` | Create an empty DOM |
| `add_node(node_type, parent) -> NodeId` | Add a node, returns its ID |
| `get(id) -> &DomNode` | Get node by ID |
| `get_mut(id) -> &mut DomNode` | Get mutable node by ID |
| `attr(id, name) -> Option<&str>` | Read attribute (case-insensitive) |
| `tag(id) -> Option<Tag>` | Get tag of element node |
| `text_content(id) -> String` | Collect all descendant text |
| `find_body() -> Option<NodeId>` | Find the `<body>` element |
| `find_title() -> Option<String>` | Get `<title>` text content |
| `set_attr(id, name, value)` | Set or add an attribute |
| `remove_attr(id, name)` | Remove an attribute |
| `set_text(id, text)` | Replace children with a text node |
| `append_child(parent, child)` | Move a child under a new parent |
| `remove_child(parent, child)` | Remove a child from a parent |
| `insert_before(parent, new, ref)` | Insert before a reference node |
| `adopt_children_from(parent, fragment)` | Copy nodes from a parsed fragment |

### DomNode

```rust
pub struct DomNode {
    pub node_type: NodeType,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

pub enum NodeType {
    Element { tag: Tag, attrs: Vec<Attr> },
    Text(String),
}
```

---

## Debug Logging

Enable the `debug_surf` feature to get detailed pipeline tracing:

```toml
[features]
debug_surf = ["libwebview/debug_surf"]
```

When enabled, the `debug_surf!()` macro prints timing and memory information at each pipeline stage (HTML parse, CSS resolve, layout, render, JS execute), including stack pointer and heap positions.

---

## Browser Default Stylesheet

libwebview applies a built-in default stylesheet before any author styles:

```css
body { margin: 8px; font-size: 16px; color: #000; }
h1 { font-size: 32px; font-weight: bold; margin: 21px 0; }
h2 { font-size: 24px; font-weight: bold; margin: 19px 0; }
h3 { font-size: 19px; font-weight: bold; margin: 18px 0; }
h4 { font-size: 16px; font-weight: bold; margin: 21px 0; }
h5 { font-size: 13px; font-weight: bold; margin: 22px 0; }
h6 { font-size: 11px; font-weight: bold; margin: 24px 0; }
p { margin: 16px 0; }
ul, ol { margin: 16px 0; padding-left: 40px; }
li { margin: 4px 0; }
a { color: #0066cc; text-decoration: underline; }
pre, code { font-family: monospace; }
pre { margin: 16px 0; padding: 8px; background: #f5f5f5; }
blockquote { margin: 16px 0; padding-left: 16px; border-left: 4px solid #ddd; }
hr { margin: 16px 0; border: none; border-top: 1px solid #ccc; }
table { border-collapse: collapse; }
td, th { padding: 4px 8px; }
img { max-width: 100%; }
strong, b { font-weight: bold; }
em, i { font-style: italic; }
```

## Usage in Surf Browser

The Surf web browser (`apps/surf/`) is the primary consumer. Typical flow:

```rust
// Create tab with WebView
let mut wv = WebView::new(800, 600);
parent.add(wv.scroll_view());
wv.scroll_view().set_dock(ui::DOCK_FILL);

// Register callbacks
wv.set_link_callback(on_link_click, 0);
wv.set_submit_callback(on_form_submit, 0);

// Load a page
wv.clear_stylesheets();
wv.set_url("https://example.com");
wv.js_runtime().set_cookies(&cookie_header);
wv.set_html(&html_body);

// Read console output
for line in wv.js_console() {
    println!("[JS] {}", line);
}

// Get page title
let title = wv.get_title().unwrap_or_else(|| String::from("Untitled"));

// External CSS (fetched separately)
wv.add_stylesheet(&css_text);
wv.relayout();

// Images (decoded separately)
wv.add_image("https://example.com/img.png", pixels, w, h);
wv.relayout();

// Animation tick (called at 60fps)
wv.tick(16);

// Viewport resize
wv.resize(new_width, new_height);

// WebSocket handling
let connects = wv.js_runtime().take_ws_connects();
let sends = wv.js_runtime().take_ws_sends();
let closes = wv.js_runtime().take_ws_closes();
```
