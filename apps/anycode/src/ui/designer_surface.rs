use alloc::string::String;
use libanyui_client as ui;

use crate::logic::designer::{DesignerControl, DesignerDocument};

const SURFACE_W: u32 = 960;
const SURFACE_H: u32 = 640;
const FORM_X: i32 = 42;
const FORM_Y: i32 = 38;

pub struct DesignerSurface {
    pub panel: ui::View,
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

        let canvas = ui::Canvas::new(SURFACE_W, SURFACE_H);
        canvas.set_dock(ui::DOCK_FILL);
        canvas.set_interactive(true);
        panel.add(&canvas);

        let click_path = String::from(file_path);
        canvas.on_mouse_down(move |x, y, _| {
            crate::logic::commands::select_designer_control_at(&click_path, x, y);
        });

        let dbl_path = String::from(file_path);
        let dbl_canvas = canvas;
        canvas.on_double_click(move |_| {
            let (x, y, _) = dbl_canvas.get_mouse();
            crate::logic::commands::designer_double_click_at(&dbl_path, x, y);
        });

        let this = Self {
            panel,
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
        self.canvas.clear(tc.editor_bg);
        draw_grid(&self.canvas, tc.separator);

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
            (SURFACE_H as i32) - 26,
            tc.text_secondary,
            0,
            11,
            "Designer Preview - click controls to inspect, double-click to open event handler",
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
        let top = FORM_Y + 34 + control.y;
        let right = left + control.width as i32;
        let bottom = top + control.height as i32;
        if x >= left && x <= right && y >= top && y <= bottom {
            return Some(control.name.clone());
        }
    }
    None
}

fn draw_grid(canvas: &ui::Canvas, color: u32) {
    let mut x = 0;
    while x < SURFACE_W as i32 {
        canvas.draw_line(x, 0, x, SURFACE_H as i32, color);
        x += 16;
    }
    let mut y = 0;
    while y < SURFACE_H as i32 {
        canvas.draw_line(0, y, SURFACE_W as i32, y, color);
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
    let y = FORM_Y + 34 + control.y;
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
