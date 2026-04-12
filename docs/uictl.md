# uictl — UI Automation Tool

`uictl` is a command-line tool for inspecting and automating anyOS GUI applications.
Every app that links `libanyui` exposes an accessibility pipe (`uiacc-<pid>`) that
`uictl` communicates with.

---

## Commands

### `uictl windows`

List all running GUI apps that have accessibility support.

```
PID      PPID   Process              W     H  Title
─────────────────────────────────────────────────────────────────
1234     1      anycode              1200   800  AnyCode
1301     1      calc                  400   500  Calculator
```

Columns: process ID, parent PID, process name, window width, window height, window title.

---

### `uictl tree <pid>`

Dump the full control tree of an app.

```
uictl tree 1234
```

Each line shows one control:
```
  id=1      0,   0 1200× 800  Window                 "AnyCode"
  id=2      0,   0 1200×  36  Toolbar
  id=5      4,   4   80×  28  IconButton             "New"
  ...
```

---

### `uictl props <pid> <id>`

Show all properties of a single control.

```
uictl props 1234 5
```

Output:
```
id:     5
parent: 2
kind:   IconButton
pos:    4,4
size:   80×28
vis:    1
dis:    0
state:  0
text:   New
```

---

### `uictl click <pid> <id>`

Fire a click event on a control.

```
uictl click 1234 5
```

Equivalent to the user clicking the control.

---

### `uictl get-text <pid> <id>`

Get the text content of a control.

```
uictl get-text 1234 42
```

---

### `uictl set-text <pid> <id> [--enter] <text>`

Set the text content of a control.

```
uictl set-text 1234 42 "Hello World"
```

With `--enter`: fires an Enter/submit event after setting the text (simulates the
user typing the text and pressing Enter):

```
uictl set-text 1234 42 --enter "search query"
```

---

### `uictl get-state <pid> <id>`

Get the state of a control (checkbox: 0 or 1, slider: 0–100, etc.).

```
uictl get-state 1234 7
```

---

### `uictl set-state <pid> <id> <val>`

Set the state of a control.

```
uictl set-state 1234 7 1    # check a checkbox
uictl set-state 1234 9 50   # set slider to 50%
```

---

### `uictl find-text <pid> <text>`

Find all controls whose text contains `<text>` (case-insensitive).

```
uictl find-text 1234 "save"
```

Output:
```
  id=23     kind=Button                 text=Save
  id=47     kind=MenuItem               text=Save As…
```

---

### `uictl submit <pid> <id>`

Fire an Enter/submit event on a control without changing its text.
Useful for triggering search fields, forms, or dialogs.

```
uictl submit 1234 42
```

---

### `uictl type-text <pid> <text>`

Send `<text>` as individual keystroke events to the currently focused control
in the target window.  This goes through the control's `handle_key_down()` path,
which fires `EVENT_KEY` and `EVENT_CHANGE` callbacks for each character and
`EVENT_SUBMIT` when Enter is sent — exactly as if the user were typing.

```
uictl type-text 1234 "Hello"
uictl type-text 1234 "search term\n"     # type and press Enter
```

**Escape sequences inside `<text>`:**

| Sequence | Key sent     |
|----------|--------------|
| `\n`     | Enter        |
| `\b`     | Backspace    |
| `\t`     | Tab          |
| `\\`     | Backslash    |

**Difference from `set-text`:**

| Feature       | `set-text`              | `type-text`                          |
|---------------|-------------------------|--------------------------------------|
| Method        | Direct buffer replace   | Keystroke-by-keystroke               |
| Callbacks     | None (silent write)     | EVENT_KEY + EVENT_CHANGE per char    |
| Enter         | Via `--enter` flag      | Via `\n` in text                     |
| Use case      | Fastest, scripting      | App logic / autocomplete / validation|

Use `type-text` when the app reacts to keystrokes (e.g. live search, autocomplete,
form validation).  Use `set-text` when you just want to set a value quickly.

---

### `uictl resize <pid> <w> <h>`

Resize the window to the given logical pixel dimensions.

```
uictl resize 1234 1400 900
```

---

### `uictl move <pid> <x> <y>`

Move the window to screen position `(x, y)`.

```
uictl move 1234 100 50
```

---

### `uictl focus <pid>`

Bring the window to the foreground.

```
uictl focus 1234
```

---

## Accessibility Protocol

All commands communicate over named pipes.  Each GUI app creates a pipe
`uiacc-<pid>` at startup.  `uictl` creates a reply pipe `uiacc-rsp-<my_pid>`,
opens the app pipe, and runs the session:

```
uictl → app:  HELLO\tuiacc-rsp-<my_pid>
app → uictl:  READY\t<title>\t<w>\t<h>
uictl → app:  <command>\n
app → uictl:  <response lines>
uictl → app:  BYE
```

### Commands

```
HELLO <rsp_pipe>           Start session, register reply pipe
TREE                       Dump full control tree
PROPS <id>                 Properties of one control
CLICK <id>                 Fire click event
SET_TEXT <id> <text>       Set text content (tab escaped as \t)
GET_TEXT <id>              Get text content → TEXT <text>
SET_STATE <id> <val>       Set control state
GET_STATE <id>             Get control state → STATE <val>
SUBMIT <id>                Fire Enter/submit event
TYPE_TEXT <text>           Type text as keystrokes into focused control
FIND_TEXT <text>           Find controls by text → MATCH lines + MATCH_END
RESIZE <w> <h>             Resize window
MOVE <x> <y>               Move window
FOCUS                      Raise window to foreground
BYE                        End session
```

### Responses

```
READY <title> <w> <h>
OK
ERR <reason>
CTRL <id> <parent> <kind> <ax> <ay> <w> <h> <vis> <dis> <state> <text>
TREE_END
TEXT <text>
STATE <val>
MATCH <id> <kind> <text>
MATCH_END
```

---

## Examples

```bash
# List all GUI apps
uictl windows

# Inspect the calc app (PID 1301)
uictl tree 1301

# Click the "=" button (id 15)
uictl click 1301 15

# Type a calculation and press Enter
uictl type-text 1301 "3+4\n"

# Set a text field and confirm
uictl set-text 1234 42 --enter "search query"

# Or type it keystroke-by-keystroke (fires autocomplete suggestions)
uictl type-text 1234 "search query\n"

# Resize and move a window
uictl resize 1234 1600 1000
uictl move 1234 0 0
```
