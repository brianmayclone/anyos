use alloc::string::String;
use libanyui_client as ui;

use crate::logic::designer::{DesignerControl, DesignerDocument};
use crate::ui::designer_toolbox;

const SURFACE_W: u32 = 960;
const SURFACE_H: u32 = 640;
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
    canvas: ui::Canvas,
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
                ui::drag_set_text(&alloc::format!("anycode-control:{}", control_name));
            } else {
                ui::drag_set_text("");
            }
        });

        let canvas = ui::Canvas::new(SURFACE_W, SURFACE_H);
        canvas.set_dock(ui::DOCK_FILL);
        canvas.set_interactive(true);
        canvas.set_drop_target(true);
        panel.add(&canvas);

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
        canvas.on_drop(move |_| {
            let (x, y, _) = drop_canvas.get_mouse();
            let payload = ui::drag_get_text();
            crate::queue_designer_drop(&drop_path, x, y, &payload);
        });

        let this = Self {
            panel,
            _toolbox: toolbox,
            canvas,
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
            draw_control(&self.canvas, control, selected_control, tc);
        }

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
}

pub fn hit_test_doc(doc: &DesignerDocument, x: i32, y: i32) -> Option<String> {
    for control in doc.controls.iter().rev() {
        let left = FORM_X + control.x;
        let top = FORM_CONTENT_Y + control.y;
        let right = left + control.width as i32;
        let bottom = top + control.height as i32;
        if x >= left && x <= right && y >= top && y <= bottom {
            return Some(control.name.clone());
        }
    }
    None
}

pub fn hit_test_resize_handle(doc: &DesignerDocument, x: i32, y: i32) -> Option<(String, u32)> {
    for control in doc.controls.iter().rev() {
        let left = FORM_X + control.x;
        let top = FORM_CONTENT_Y + control.y;
        let right = left + control.width as i32;
        let bottom = top + control.height as i32;
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

pub fn canvas_to_form(x: i32, y: i32) -> (i32, i32) {
    (x - FORM_X, y - FORM_CONTENT_Y)
}

fn near_handle(x: i32, y: i32, hx: i32, hy: i32) -> bool {
    let half = HANDLE_SIZE / 2;
    x >= hx - half && x <= hx + half && y >= hy - half && y <= hy + half
}

fn draw_grid(canvas: &ui::Canvas, width: u32, height: u32, color: u32) {
    let mut x = 0;
    while x < width as i32 {
        canvas.draw_line(x, 0, x, height as i32, color);
        x += 16;
    }
    let mut y = 0;
    while y < height as i32 {
        canvas.draw_line(0, y, width as i32, y, color);
        y += 16;
    }
}

fn draw_control(
    canvas: &ui::Canvas,
    control: &DesignerControl,
    selected_control: Option<&str>,
    tc: &'static ui::theme::ThemeColors,
) {
    let x = FORM_X + control.x;
    let y = FORM_CONTENT_Y + control.y;
    let selected = selected_control == Some(control.name.as_str());
    let fill = match control.kind.as_str() {
        "Label" => tc.sidebar_bg,
        "Panel" => tc.editor_bg,
        _ => tc.control_bg,
    };
    let border = if selected { tc.accent } else { tc.separator };
    canvas.fill_rect(x, y, control.width, control.height, fill);
    canvas.draw_rect(
        x,
        y,
        control.width,
        control.height,
        border,
        if selected { 2 } else { 1 },
    );
    if !control.text.is_empty() {
        canvas.draw_text(x + 7, y + 7, tc.text, 0, 12, &control.text);
    } else {
        canvas.draw_text(
            x + 7,
            y + 7,
            tc.text_secondary,
            0,
            11,
            control.kind.as_str(),
        );
    }
    if selected {
        draw_handles(canvas, x, y, control.width, control.height, tc.accent);
    }
}

fn draw_handles(canvas: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let right = x + w as i32;
    let bottom = y + h as i32;
    for (hx, hy) in [(x, y), (right, y), (x, bottom), (right, bottom)] {
        canvas.fill_rect(hx - 3, hy - 3, 6, 6, color);
    }
}
