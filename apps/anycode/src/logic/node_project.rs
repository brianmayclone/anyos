use alloc::format;
use alloc::string::String;

use crate::util::path;

pub fn package_json(project_name: &str) -> String {
    format!(
        "{{\n  \"name\": \"{}\",\n  \"version\": \"0.1.0\",\n  \"type\": \"commonjs\",\n  \"private\": true,\n  \"main\": \"src/main.js\",\n  \"scripts\": {{\n    \"start\": \"node src/main.js\",\n    \"lint\": \"eslint src\",\n    \"test\": \"node src/main.js --self-test\"\n  }},\n  \"dependencies\": {{\n    \"@anyos/anyui\": \"0.1.0\"\n  }},\n  \"devDependencies\": {{\n    \"eslint\": \"^8.57.1\"\n  }}\n}}\n",
        project_name
    )
}

pub fn ensure_support_files(project_root: &str) -> Result<(), &'static str> {
    ensure_dir(&format!("{}/src", project_root))?;
    ensure_dir(&format!("{}/src/types", project_root))?;
    write_new(
        &format!("{}/jsconfig.json", project_root),
        br#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "checkJs": true,
    "allowSyntheticDefaultImports": true,
    "baseUrl": "."
  },
  "include": [
    "src/**/*.js",
    "src/types/**/*.d.ts",
    "node_modules/@anyos/**/*.d.ts"
  ]
}
"#,
    )?;
    write_new(
        &format!("{}/.eslintrc.json", project_root),
        br#"{
  "env": {
    "es2022": true,
    "node": true
  },
  "extends": "eslint:recommended",
  "parserOptions": {
    "ecmaVersion": "latest",
    "sourceType": "script"
  },
  "rules": {
    "no-unused-vars": ["warn", { "argsIgnorePattern": "^_" }],
    "no-undef": "error"
  }
}
"#,
    )?;
    write_new(
        &format!("{}/src/types/anyos-anyui.d.ts", project_root),
        anyui_types().as_bytes(),
    )?;
    Ok(())
}

fn ensure_dir(dir: &str) -> Result<(), &'static str> {
    if path::is_directory(dir) {
        return Ok(());
    }
    if anyos_std::fs::mkdir(dir) == u32::MAX {
        Err("Could not create Node project folder")
    } else {
        Ok(())
    }
}

fn write_new(path: &str, data: &[u8]) -> Result<(), &'static str> {
    if crate::util::path::exists(path) {
        return Ok(());
    }
    anyos_std::fs::write_bytes(path, data).map_err(|_| "Could not write Node project support file")
}

fn anyui_types() -> &'static str {
    r#"declare module "@anyos/anyui" {
  export const DOCK_NONE: number;
  export const DOCK_TOP: number;
  export const DOCK_BOTTOM: number;
  export const DOCK_LEFT: number;
  export const DOCK_RIGHT: number;
  export const DOCK_FILL: number;
  export const ORIENTATION_VERTICAL: number;
  export const ORIENTATION_HORIZONTAL: number;

  export interface AnyuiEvent {
    id: number;
    controlId: number;
    type: number;
  }

  export type EventHandler = (event: AnyuiEvent) => void;

  export interface ThemeColors {
    editorBg: number;
    text: number;
    accent: number;
  }

  export const theme: {
    colors(): ThemeColors;
  };

  export function createApp(): Control;
  export function run(): void;

  export class Control {
    readonly __anyuiKind: string;
    readonly __anyuiId?: number;
    add(child: Control): this;
    setPosition(x: number, y: number): this;
    setSize(width: number, height: number): this;
    getPosition(): { x: number; y: number };
    getSize(): { width: number; height: number };
    setColor(color: number | string): this;
    setText(text: string): this;
    getText(): string;
    setDock(dock: number): this;
    setMargin(left: number, top: number, right: number, bottom: number): this;
    setPadding(left: number, top: number, right: number, bottom: number): this;
    setOrientation(orientation: number): this;
    setAutoSize(enabled: boolean): this;
    setMinSize(width: number, height: number): this;
    setMaxSize(width: number, height: number): this;
    setState(value: number): this;
    getState(): number;
    setVisible(visible: boolean): this;
    setEnabled(enabled: boolean): this;
    setFontSize(size: number): this;
    setTextColor(color: number | string): this;
    setStyle(key: number, value: number): this;
    setTooltip(text: string): this;
    setTabIndex(index: number): this;
    setPlaceholder(text: string): this;
    setPasswordMode(enabled: boolean): this;
    setReadOnly(enabled: boolean): this;
    selectAll(): this;
    setCursor(pos: number): this;
    setSelection(start: number, end: number): this;
    setMaxLength(maxLength: number): this;
    setItems(items: string): this;
    setSelectedIndex(index: number | null | undefined): this;
    setSuggestions(items: string): this;
    setEditable(editable: boolean): this;
    setSplitRatio(ratio: number): this;
    setScrollOffsets(x: number, y: number): this;
    setSelectedColor(color: number | string): this;
    setDraggable(enabled: boolean): this;
    setDropTarget(enabled: boolean): this;
    openPopup(): this;
    remove(): this;
    focus(): this;
    bringToFront(): this;
    onClick(handler: EventHandler): this;
    onDoubleClick(handler: EventHandler): this;
    onFocus(handler: EventHandler): this;
    onBlur(handler: EventHandler): this;
    onContextMenu(handler: EventHandler): this;
    onMouseEnter(handler: EventHandler): this;
    onMouseLeave(handler: EventHandler): this;
    onMouseDown(handler: EventHandler): this;
    onMouseUp(handler: EventHandler): this;
    onDragStart(handler: EventHandler): this;
    onDragEnter(handler: EventHandler): this;
    onDragLeave(handler: EventHandler): this;
    onDrop(handler: EventHandler): this;
    onDragEnd(handler: EventHandler): this;
    onTextChanged(handler: EventHandler): this;
    onSelectionChanged(handler: EventHandler): this;
    onActiveChanged(handler: EventHandler): this;
    onCheckedChanged(handler: EventHandler): this;
    onValueChanged(handler: EventHandler): this;
    onChanged(handler: EventHandler): this;
    onColorSelected(handler: EventHandler): this;
    onSubmit(handler: EventHandler): this;
    onEnter(handler: EventHandler): this;
  }

  export class Window extends Control {
    constructor(title?: string, x?: number, y?: number, width?: number, height?: number);
  }

  export class View extends Control {}
  export class Button extends Control { constructor(text?: string); }
  export class PlainButton extends Control { constructor(text?: string); }
  export class IconButton extends Control { constructor(text?: string); }
  export class ImageButton extends Control { constructor(width?: number, height?: number); }
  export class Label extends Control { constructor(text?: string); }
  export class LinkLabel extends Control { constructor(text?: string); }
  export class TextField extends Control {}
  export class TextArea extends Control {}
  export class TextEditor extends Control { constructor(width?: number, height?: number); }
  export class SearchField extends Control {}
  export class AutoCompleteTextField extends Control {}
  export class Checkbox extends Control { constructor(text?: string); }
  export class RadioButton extends Control { constructor(text?: string); }
  export class RadioGroup extends Control {}
  export class ComboBox extends Control {}
  export class DropDown extends Control { constructor(items?: string); }
  export class ListBox extends Control { constructor(items?: string); }
  export class TreeView extends Control { constructor(width?: number, height?: number); }
  export class DataGrid extends Control { constructor(width?: number, height?: number); }
  export class TableView extends Control {}
  export class ColorWell extends Control {}
  export class DatePicker extends Control {}
  export class DateTimePicker extends Control {}
  export class TimePicker extends Control {}
  export class Divider extends Control {}
  export class Expander extends Control { constructor(text?: string); }
  export class FlowPanel extends Control {}
  export class GroupBox extends Control { constructor(text?: string); }
  export class ImageView extends Control { constructor(width?: number, height?: number); }
  export class NavigationBar extends Control { constructor(text?: string); }
  export class ProgressBar extends Control { constructor(value?: number); }
  export class ScrollView extends Control {}
  export class SegmentedControl extends Control { constructor(items?: string); }
  export class Slider extends Control { constructor(value?: number); }
  export class Spinner extends Control {}
  export class SplitView extends Control {}
  export class StackPanel extends Control { constructor(orientation?: number); }
  export class StatusIndicator extends Control { constructor(text?: string); }
  export class Stepper extends Control {}
  export class TabBar extends Control { constructor(items?: string); }
  export class TableLayout extends Control { constructor(columns?: number); }
  export class Tag extends Control { constructor(text?: string); }
  export class Toggle extends Control { constructor(on?: boolean); }
  export class Toolbar extends Control {}
  export class Tooltip extends Control { constructor(text?: string); }
  export class Alert extends Control { constructor(text?: string); }
  export class Badge extends Control { constructor(text?: string); }
  export class Canvas extends Control { constructor(width?: number, height?: number); }
  export class Card extends Control {}
}
"#
}
