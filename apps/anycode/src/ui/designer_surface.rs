use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use libanyui_client as ui;
use ui::Widget;

use crate::logic::designer::{DesignerControl, DesignerDocument};
use crate::ui::designer_toolbox;

const SURFACE_H: u32 = 640;
const DESIGNER_CONTENT_W: u32 = 1280;
const DESIGNER_CONTENT_H: u32 = 920;
const FORM_X: i32 = 42;
const FORM_Y: i32 = 38;
const FORM_CONTENT_Y: i32 = FORM_Y + 34;
const HANDLE_SIZE: i32 = 8;

pub const DESIGNER_DRAG_NONE: u32 = 0;
pub const DESIGNER_DRAG_MOVE: u32 = 1;
pub const DESIGNER_DRAG_RESIZE_NW: u32 = 2;
pub const DESIGNER_DRAG_RESIZE_NE: u32 = 3;
pub const DESIGNER_DRAG_RESIZE_SW: u32 = 4;
pub const DESIGNER_DRAG_RESIZE_SE: u32 = 5;

pub struct DesignerSurface {
    pub panel: ui::View,
    _toolbox: ui::TreeView,
    _scroll: ui::ScrollView,
    content: ui::View,
    canvas: ui::Canvas,
    preview_controls: RefCell<Vec<ui::Control>>,
    file_path: String,
    doc: DesignerDocument,
}

impl DesignerSurface {
    pub fn new(file_path: &str, doc: DesignerDocument) -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.editor_bg);

        let toolbox_panel = ui::View::new();
        toolbox_panel.set_dock(ui::DOCK_LEFT);
        toolbox_panel.set_size(210, SURFACE_H);
        toolbox_panel.set_color(tc.sidebar_bg);
        panel.add(&toolbox_panel);

        let toolbox_header = ui::Label::new("Toolbox");
        toolbox_header.set_dock(ui::DOCK_TOP);
        toolbox_header.set_size(210, 34);
        toolbox_header.set_font_size(13);
        toolbox_header.set_text_color(tc.text);
        toolbox_panel.add(&toolbox_header);

        let toolbox = ui::TreeView::new(210, SURFACE_H - 34);
        toolbox.set_dock(ui::DOCK_FILL);
        toolbox.set_indent_width(14);
        toolbox.set_row_height(22);
        toolbox.set_draggable(true);
        toolbox_panel.add(&toolbox);
        let toolbox_root = toolbox.add_root("Controls");
        toolbox.set_node_text_color(toolbox_root, tc.text);
        toolbox.set_node_style(toolbox_root, ui::STYLE_BOLD);
        toolbox.set_expanded(toolbox_root, true);
        let toolbox_nodes = designer_toolbox::populate_toolbox_tree(&toolbox, toolbox_root);

        let drag_nodes = toolbox_nodes.clone();
        toolbox.on_drag_start(move |_| {
            let selected = toolbox.selected();
            let hovered = toolbox.hovered();
            let node = if hovered != u32::MAX {
                hovered
            } else {
                selected
            };
            if let Some(control_name) = designer_toolbox::control_name_for_node(&drag_nodes, node) {
                let payload = alloc::format!("anycode-control:{}", control_name);
                ui::drag_set_text(&payload);
                ui::drag_set_payload(ui::DND_FORMAT_TEXT, payload.as_bytes(), ui::DND_EFFECT_COPY);
            } else {
                ui::drag_set_text("");
            }
        });

        let scroll = ui::ScrollView::new();
        scroll.set_dock(ui::DOCK_FILL);
        scroll.set_color(tc.editor_bg);
        panel.add(&scroll);

        let content = ui::View::new();
        content.set_position(0, 0);
        content.set_size(DESIGNER_CONTENT_W, DESIGNER_CONTENT_H);
        content.set_color(tc.editor_bg);
        scroll.add(&content);

        let canvas = ui::Canvas::new(DESIGNER_CONTENT_W, DESIGNER_CONTENT_H);
        canvas.set_position(0, 0);
        canvas.set_size(DESIGNER_CONTENT_W, DESIGNER_CONTENT_H);
        canvas.set_interactive(true);
        canvas.set_drop_target(true);
        content.add(&canvas);

        let click_path = String::from(file_path);
        canvas.on_mouse_down(move |x, y, _| {
            crate::queue_designer_click(&click_path, x, y);
        });

        let move_path = String::from(file_path);
        canvas.on_mouse_move(move |x, y| {
            crate::queue_designer_mouse_move(&move_path, x, y);
        });

        let up_path = String::from(file_path);
        canvas.on_mouse_up(move |x, y, _| {
            crate::queue_designer_mouse_up(&up_path, x, y);
        });

        let dbl_path = String::from(file_path);
        let dbl_canvas = canvas;
        canvas.on_double_click(move |_| {
            let (x, y, _) = dbl_canvas.get_mouse();
            crate::queue_designer_double_click(&dbl_path, x, y);
        });

        let drop_path = String::from(file_path);
        let drop_canvas = canvas;
        canvas.on_drag_enter(move |_| {
            ui::drag_accept(ui::DND_EFFECT_COPY);
        });

        let drop_path = String::from(drop_path);
        canvas.on_drop(move |_| {
            let (x, y, _) = drop_canvas.get_mouse();
            let mut payload = ui::drag_get_text();
            if payload.is_empty() {
                let (bytes, format) = ui::drag_get_payload();
                if format == ui::DND_FORMAT_TEXT {
                    payload = String::from_utf8_lossy(&bytes).into_owned();
                }
            }
            crate::queue_designer_drop(&drop_path, x, y, &payload);
        });

        let this = Self {
            panel,
            _toolbox: toolbox,
            _scroll: scroll,
            content,
            canvas,
            preview_controls: RefCell::new(Vec::new()),
            file_path: String::from(file_path),
            doc,
        };
        this.render(None);
        this
    }

    pub fn set_visible(&self, visible: bool) {
        self.panel.set_visible(visible);
    }

    pub fn remove(&self) {
        self.panel.remove();
    }

    pub fn render(&self, selected_control: Option<&str>) {
        let tc = ui::theme::colors();
        let canvas_w = self.canvas.get_stride().max(1);
        let canvas_h = self.canvas.get_height().max(1);
        self.canvas.clear(tc.editor_bg);
        draw_grid(&self.canvas, canvas_w, canvas_h, tc.separator);

        let shadow = 0x22000000;
        self.canvas.fill_rect(
            FORM_X + 4,
            FORM_Y + 4,
            self.doc.width,
            self.doc.height,
            shadow,
        );
        self.canvas.fill_rect(
            FORM_X,
            FORM_Y,
            self.doc.width,
            self.doc.height,
            tc.sidebar_bg,
        );
        self.canvas.draw_rect(
            FORM_X,
            FORM_Y,
            self.doc.width,
            self.doc.height,
            tc.separator,
            1,
        );
        self.canvas
            .draw_text(FORM_X + 12, FORM_Y + 8, tc.text, 1, 14, &self.doc.title);
        self.canvas.draw_line(
            FORM_X,
            FORM_Y + 32,
            FORM_X + self.doc.width as i32,
            FORM_Y + 32,
            tc.separator,
        );

        for control in &self.doc.controls {
            draw_control_outline(&self.canvas, &self.doc, control, selected_control, tc);
        }
        self.render_preview_controls();

        self.canvas.draw_text(
            16,
            (canvas_h as i32) - 26,
            tc.text_secondary,
            0,
            11,
            "Designer Preview - drag controls from Toolbox, move/resize selected components, double-click to open event handler",
        );
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    pub fn set_document(&mut self, doc: DesignerDocument, selected_control: Option<&str>) {
        self.doc = doc;
        self.render(selected_control);
    }

    fn render_preview_controls(&self) {
        let old_controls = self.preview_controls.replace(Vec::new());
        for control in old_controls {
            control.remove();
        }

        for control in &self.doc.controls {
            let Some(preview) = create_anyui_preview_control(control) else {
                continue;
            };
            let (abs_x, abs_y, _, _) = self.doc.control_absolute_bounds(&control.name).unwrap_or((
                control.x,
                control.y,
                control.width,
                control.height,
            ));
            preview.set_position(FORM_X + abs_x, FORM_CONTENT_Y + abs_y);
            preview.set_size(control.width, control.height);
            preview.set_enabled(false);
            apply_preview_common_style(&preview, control);
            self.content.add_child(preview.id());
            self.preview_controls.borrow_mut().push(preview);
        }
    }
}

pub fn hit_test_doc(doc: &DesignerDocument, x: i32, y: i32) -> Option<String> {
    for control in doc.controls.iter().rev() {
        let Some((abs_x, abs_y, width, height)) = doc.control_absolute_bounds(&control.name) else {
            continue;
        };
        let left = FORM_X + abs_x;
        let top = FORM_CONTENT_Y + abs_y;
        let right = left + width as i32;
        let bottom = top + height as i32;
        if x >= left && x <= right && y >= top && y <= bottom {
            return Some(control.name.clone());
        }
    }
    None
}

pub fn hit_test_resize_handle(doc: &DesignerDocument, x: i32, y: i32) -> Option<(String, u32)> {
    for control in doc.controls.iter().rev() {
        let Some((abs_x, abs_y, width, height)) = doc.control_absolute_bounds(&control.name) else {
            continue;
        };
        let left = FORM_X + abs_x;
        let top = FORM_CONTENT_Y + abs_y;
        let right = left + width as i32;
        let bottom = top + height as i32;
        let handle = if near_handle(x, y, left, top) {
            DESIGNER_DRAG_RESIZE_NW
        } else if near_handle(x, y, right, top) {
            DESIGNER_DRAG_RESIZE_NE
        } else if near_handle(x, y, left, bottom) {
            DESIGNER_DRAG_RESIZE_SW
        } else if near_handle(x, y, right, bottom) {
            DESIGNER_DRAG_RESIZE_SE
        } else {
            DESIGNER_DRAG_NONE
        };
        if handle != DESIGNER_DRAG_NONE {
            return Some((control.name.clone(), handle));
        }
    }
    None
}

pub fn hit_test_container(doc: &DesignerDocument, x: i32, y: i32) -> Option<String> {
    for control in doc.controls.iter().rev() {
        if !is_container_kind(control.kind.as_str()) {
            continue;
        }
        let Some((abs_x, abs_y, width, height)) = doc.control_absolute_bounds(&control.name) else {
            continue;
        };
        let left = FORM_X + abs_x;
        let top = FORM_CONTENT_Y + abs_y;
        let right = left + width as i32;
        let bottom = if is_paged_kind(control.kind.as_str()) {
            top + height as i32 + paged_content_gap(control) + paged_content_height(control) as i32
        } else {
            top + height as i32
        };
        if x >= left && x <= right && y >= top && y <= bottom {
            return Some(control.name.clone());
        }
    }
    None
}

pub fn hit_test_page_index(doc: &DesignerDocument, x: i32, y: i32) -> Option<u32> {
    for control in doc.controls.iter().rev() {
        if !is_paged_kind(control.kind.as_str()) {
            continue;
        }
        let Some((abs_x, abs_y, width, height)) = doc.control_absolute_bounds(&control.name) else {
            continue;
        };
        let left = FORM_X + abs_x;
        let top = FORM_CONTENT_Y + abs_y;
        let right = left + width as i32;
        let bottom = top + height as i32;
        if x >= left && x <= right && y >= top && y <= bottom {
            let page_count = page_count(control).max(1);
            let tab_width = (width as i32 / page_count as i32).max(1);
            return Some(((x - left).max(0) / tab_width) as u32);
        }
    }
    None
}

pub fn canvas_to_form(x: i32, y: i32) -> (i32, i32) {
    (x - FORM_X, y - FORM_CONTENT_Y)
}

fn near_handle(x: i32, y: i32, hx: i32, hy: i32) -> bool {
    let half = HANDLE_SIZE / 2;
    x >= hx - half && x <= hx + half && y >= hy - half && y <= hy + half
}

fn draw_grid(canvas: &ui::Canvas, width: u32, height: u32, color: u32) {
    let minor = blend_rgb(color, 0x00202020, 50);
    let major = blend_rgb(color, 0x00202020, 82);
    let mut x = 0;
    while x < width as i32 {
        let line_color = if x % 64 == 0 { major } else { minor };
        canvas.draw_line(x, 0, x, height as i32, line_color);
        x += 16;
    }
    let mut y = 0;
    while y < height as i32 {
        let line_color = if y % 64 == 0 { major } else { minor };
        canvas.draw_line(0, y, width as i32, y, line_color);
        y += 16;
    }
}

fn blend_rgb(a: u32, b: u32, percent_a: u32) -> u32 {
    let percent_a = percent_a.min(100);
    let percent_b = 100 - percent_a;
    let ar = (a >> 16) & 0xff;
    let ag = (a >> 8) & 0xff;
    let ab = a & 0xff;
    let br = (b >> 16) & 0xff;
    let bg = (b >> 8) & 0xff;
    let bb = b & 0xff;
    let r = (ar * percent_a + br * percent_b) / 100;
    let g = (ag * percent_a + bg * percent_b) / 100;
    let blue = (ab * percent_a + bb * percent_b) / 100;
    0xff000000 | (r << 16) | (g << 8) | blue
}

fn draw_control_outline(
    canvas: &ui::Canvas,
    doc: &DesignerDocument,
    control: &DesignerControl,
    selected_control: Option<&str>,
    tc: &'static ui::theme::ThemeColors,
) {
    let (abs_x, abs_y, _, _) = doc.control_absolute_bounds(&control.name).unwrap_or((
        control.x,
        control.y,
        control.width,
        control.height,
    ));
    let x = FORM_X + abs_x;
    let y = FORM_CONTENT_Y + abs_y;
    let selected = selected_control == Some(control.name.as_str());
    let border = if selected { tc.accent } else { tc.separator };
    canvas.draw_rect(
        x,
        y,
        control.width,
        control.height,
        border,
        if selected { 2 } else { 1 },
    );
    if is_paged_kind(control.kind.as_str()) {
        let page_y = y + control.height as i32 + paged_content_gap(control);
        let page_h = paged_content_height(control);
        canvas.fill_rect(x, page_y, control.width, page_h, tc.editor_bg);
        canvas.draw_rect(
            x,
            page_y,
            control.width,
            page_h,
            if selected { tc.accent } else { tc.separator },
            1,
        );
        canvas.draw_text(x + 8, page_y + 7, tc.text_secondary, 0, 11, "Page content");
    }
    if selected {
        draw_handles(canvas, x, y, control.width, control.height, tc.accent);
    }
}

fn create_anyui_preview_control(control: &DesignerControl) -> Option<ui::Control> {
    let text = preview_text(control);
    let items = preview_items(control);
    let id = match control.kind.as_str() {
        "Alert" => ui::Alert::new(&text).id(),
        "AutoCompleteTextField" => {
            let c = ui::AutoCompleteTextField::new();
            c.set_placeholder(&text);
            c.id()
        }
        "Badge" => ui::Badge::new(&text).id(),
        "Button" => ui::Button::new(&text).id(),
        "Canvas" => ui::Canvas::new(control.width, control.height).id(),
        "Card" => ui::Card::new().id(),
        "CheckBox" => ui::Checkbox::new(&text).id(),
        "ColorWell" => ui::ColorWell::new().id(),
        "ComboBox" => {
            let c = ui::ComboBox::new();
            c.set_items(&items);
            c.set_placeholder(&text);
            c.id()
        }
        "DataGrid" => {
            let c = ui::DataGrid::new(control.width, control.height);
            c.set_columns(&[
                ui::ColumnDef::new("Property").width(120),
                ui::ColumnDef::new("Value").width(120),
            ]);
            c.id()
        }
        "DatePicker" => ui::DatePicker::new().id(),
        "DateTimePicker" => ui::DateTimePicker::new().id(),
        "Divider" => ui::Divider::new().id(),
        "DropDown" => ui::DropDown::new(&items).id(),
        "Expander" => ui::Expander::new(&text).id(),
        "FlowPanel" => ui::FlowPanel::new().id(),
        "GroupBox" => ui::GroupBox::new(&text).id(),
        "IconButton" => ui::IconButton::new("...").id(),
        "ImageButton" => ui::ImageButton::new(control.width, control.height).id(),
        "ImageView" => ui::ImageView::new(control.width, control.height).id(),
        "Label" => ui::Label::new(&text).id(),
        "LinkLabel" => ui::LinkLabel::new(&text).id(),
        "ListBox" => ui::ListBox::new(&items).id(),
        "NavigationBar" => ui::NavigationBar::new(&text).id(),
        "Panel" => ui::View::new().id(),
        "PlainButton" => ui::PlainButton::new(&text).id(),
        "ProgressBar" => ui::ProgressBar::new(preview_value(control)).id(),
        "RadioButton" => ui::RadioButton::new(&text).id(),
        "RadioGroup" => {
            let c = ui::RadioGroup::new();
            c.set_text(&items);
            c.id()
        }
        "ScrollView" => ui::ScrollView::new().id(),
        "SearchField" => ui::SearchField::new().id(),
        "SegmentedControl" => ui::SegmentedControl::new(&items).id(),
        "Slider" => ui::Slider::new(preview_value(control)).id(),
        "Spinner" => ui::Spinner::new().id(),
        "SplitView" => ui::SplitView::new().id(),
        "StackPanel" => ui::StackPanel::new(ui::ORIENTATION_VERTICAL).id(),
        "StatusIndicator" => ui::StatusIndicator::new(&text).id(),
        "Stepper" => ui::Stepper::new().id(),
        "TabBar" => ui::TabBar::new(&items).id(),
        "TableLayout" => ui::TableLayout::new(2).id(),
        "TableView" => ui::TableView::new().id(),
        "Tag" => ui::Tag::new(&text).id(),
        "TextArea" => {
            let c = ui::TextArea::new();
            c.set_text(&text);
            c.set_read_only(true);
            c.id()
        }
        "TextEditor" => {
            let c = ui::TextEditor::new(control.width, control.height);
            c.set_text(&text);
            c.set_read_only(true);
            c.id()
        }
        "TextField" => {
            let c = ui::TextField::new();
            c.set_text(&text);
            c.set_read_only(true);
            c.id()
        }
        "TimePicker" => ui::TimePicker::new().id(),
        "Toggle" => ui::Toggle::new(false).id(),
        "Toolbar" => ui::Toolbar::new().id(),
        "Tooltip" => ui::Tooltip::new(&text).id(),
        _ => return None,
    };
    Some(ui::Control::from_id(id))
}

fn apply_preview_common_style(preview: &ui::Control, control: &DesignerControl) {
    if let Some(color) = parse_color(&control.property_value("BackgroundColor")) {
        preview.set_color(color);
    }
    if let Some(color) = parse_color(&control.property_value("TextColor")) {
        preview.set_text_color(color);
    }
    if let Some(font_size) = parse_u32(&control.property_value("FontSize")) {
        preview.set_font_size(font_size);
    }
}

fn preview_text(control: &DesignerControl) -> String {
    if !control.text.is_empty() {
        return control.text.clone();
    }
    let value = control.property_value("Text");
    if !value.is_empty() {
        value
    } else {
        String::from(control.kind.as_str())
    }
}

fn preview_items(control: &DesignerControl) -> String {
    let items = control.property_value("Items");
    if !items.is_empty() {
        items
    } else if !control.text.is_empty() {
        control.text.clone()
    } else {
        String::from("Item 1|Item 2")
    }
}

fn preview_value(control: &DesignerControl) -> u32 {
    parse_u32(&control.property_value("Value")).unwrap_or(40)
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

fn parse_color(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix('#'))?;
    u32::from_str_radix(hex, 16).ok()
}

fn is_container_kind(kind: &str) -> bool {
    matches!(
        kind,
        "Card"
            | "Expander"
            | "FlowPanel"
            | "GroupBox"
            | "Panel"
            | "ScrollView"
            | "SegmentedControl"
            | "SplitView"
            | "StackPanel"
            | "TabBar"
            | "TableLayout"
    )
}

fn is_paged_kind(kind: &str) -> bool {
    matches!(kind, "SegmentedControl" | "TabBar")
}

fn page_count(control: &DesignerControl) -> u32 {
    let source = control.property_value("Items");
    let items = if source.is_empty() {
        control.text.as_str()
    } else {
        source.as_str()
    };
    let mut count = 0u32;
    for item in items.split('|') {
        if !item.trim().is_empty() {
            count = count.saturating_add(1);
        }
    }
    count.max(2)
}

fn paged_content_gap(control: &DesignerControl) -> i32 {
    if control.height == 0 {
        0
    } else {
        8
    }
}

fn paged_content_height(control: &DesignerControl) -> u32 {
    control
        .property_value("PageHeight")
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(220)
}

fn draw_handles(canvas: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let right = x + w as i32;
    let bottom = y + h as i32;
    for (hx, hy) in [(x, y), (right, y), (x, bottom), (right, bottom)] {
        canvas.fill_rect(hx - 3, hy - 3, 6, 6, color);
    }
}
