use alloc::cell::RefCell;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::logic::{designer, storyboard};

const CANVAS_W: u32 = 1800;
const CANVAS_H: u32 = 1200;

pub struct StoryboardSurface {
    pub panel: ui::View,
    canvas: ui::Canvas,
    file_path: String,
    doc: Rc<RefCell<storyboard::StoryboardDocument>>,
    drag_source: Rc<RefCell<Option<(String, String)>>>,
}

impl StoryboardSurface {
    pub fn new(file_path: &str, doc: storyboard::StoryboardDocument) -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.editor_bg);

        let header = ui::View::new();
        header.set_dock(ui::DOCK_TOP);
        header.set_size(700, 34);
        header.set_color(tc.toolbar_bg);
        panel.add(&header);

        let title = ui::Label::new(&format!("Storyboard: {}", doc.name));
        title.set_position(12, 7);
        title.set_size(360, 18);
        title.set_font_size(12);
        title.set_text_color(tc.text);
        header.add(&title);

        let hint = ui::Label::new("Drag from a control anchor to another form to create a segue");
        hint.set_position(380, 7);
        hint.set_size(520, 18);
        hint.set_font_size(11);
        hint.set_text_color(tc.text_secondary);
        header.add(&hint);

        let scroll = ui::ScrollView::new(CANVAS_W, CANVAS_H);
        scroll.set_dock(ui::DOCK_FILL);
        scroll.set_color(tc.editor_bg);
        panel.add(&scroll);

        let canvas = ui::Canvas::new(CANVAS_W, CANVAS_H);
        canvas.set_interactive(true);
        scroll.add(&canvas);

        let surface = Self {
            panel,
            canvas,
            file_path: String::from(file_path),
            doc: Rc::new(RefCell::new(doc)),
            drag_source: Rc::new(RefCell::new(None)),
        };
        surface.wire_events();
        surface.render();
        surface
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    pub fn set_visible(&self, visible: bool) {
        self.panel.set_visible(visible);
    }

    pub fn remove(&self) {
        self.panel.remove();
    }

    fn wire_events(&self) {
        let file_path = self.file_path.clone();
        let doc_ref = self.doc.clone();
        let drag_ref = self.drag_source.clone();
        let canvas_id = self.canvas.id();
        self.canvas.on_mouse_down(move |x, y, _button| {
            let source = doc_ref.borrow().control_anchor_at(x, y);
            *drag_ref.borrow_mut() = source;
            ui::Control::from_id(canvas_id).set_tooltip("Release over another form");
        });

        let file_path_up = file_path.clone();
        let doc_ref_up = self.doc.clone();
        let drag_ref_up = self.drag_source.clone();
        let canvas_up = self.canvas;
        let canvas_id_up = canvas_up.id();
        self.canvas.on_mouse_up(move |x, y, _button| {
            let Some((from_form, from_control)) = drag_ref_up.borrow_mut().take() else {
                return;
            };
            let target_form = {
                let doc = doc_ref_up.borrow();
                doc.scene_at(x, y)
                    .and_then(|idx| doc.scenes.get(idx))
                    .map(|scene| scene.form_name.clone())
            };
            let Some(to_form) = target_form else {
                return;
            };
            let mut doc = doc_ref_up.borrow_mut();
            match storyboard::apply_segue(
                &file_path_up,
                &mut doc,
                &from_form,
                &from_control,
                &to_form,
            ) {
                Ok(Some(_)) => {
                    ui::Control::from_id(canvas_id_up).set_tooltip("Segue created");
                    render_storyboard(&canvas_up, &doc);
                }
                Ok(None) => {
                    ui::Control::from_id(canvas_id_up).set_tooltip("Segue already exists");
                }
                Err(err) => {
                    ui::Control::from_id(canvas_id_up).set_tooltip(err);
                }
            }
        });
    }

    fn render(&self) {
        render_storyboard(&self.canvas, &self.doc.borrow());
    }
}

fn render_storyboard(canvas: &ui::Canvas, doc: &storyboard::StoryboardDocument) {
    let tc = ui::theme::colors();
    canvas.clear(tc.editor_bg);
    draw_grid(canvas, tc.separator);
    for segue in &doc.segues {
        draw_segue(canvas, doc, segue);
    }
    for scene in &doc.scenes {
        draw_scene(canvas, scene);
    }
}

fn draw_grid(canvas: &ui::Canvas, color: u32) {
    let minor = color_with_alpha(color, 0x35);
    let major = color_with_alpha(color, 0x80);
    let mut x = 0;
    while x < CANVAS_W as i32 {
        canvas.draw_line(x, 0, x, CANVAS_H as i32, if x % 80 == 0 { major } else { minor });
        x += 20;
    }
    let mut y = 0;
    while y < CANVAS_H as i32 {
        canvas.draw_line(0, y, CANVAS_W as i32, y, if y % 80 == 0 { major } else { minor });
        y += 20;
    }
}

fn draw_scene(canvas: &ui::Canvas, scene: &storyboard::StoryboardScene) {
    let tc = ui::theme::colors();
    let (w, h) = storyboard::scene_size();
    canvas.fill_rect(scene.x, scene.y, w, h, tc.sidebar_bg);
    canvas.draw_rect(scene.x, scene.y, w, h, tc.accent, 2);
    canvas.fill_rect(scene.x, scene.y, w, 28, tc.toolbar_bg);
    canvas.draw_text(scene.x + 10, scene.y + 8, tc.text, 1, 12, &scene.form_name);

    let Some(doc) = designer::load_designer(&scene.designer_path) else {
        canvas.draw_text(scene.x + 10, scene.y + 52, tc.error, 0, 11, "Designer missing");
        return;
    };
    let ox = scene.x + 16;
    let oy = scene.y + 36;
    let form_w = (doc.width / 3).min(w - 32);
    let form_h = (doc.height / 3).min(h - 48);
    canvas.fill_rect(ox, oy, form_w, form_h, tc.control_bg);
    canvas.draw_rect(ox, oy, form_w, form_h, tc.separator, 1);
    for control in &doc.controls {
        let cx = ox + control.x / 3;
        let cy = oy + control.y / 3;
        let cw = (control.width / 3).max(8);
        let ch = (control.height / 3).max(6);
        canvas.fill_rect(cx, cy, cw, ch, preview_color(control.kind.as_str()));
        canvas.draw_rect(cx, cy, cw, ch, tc.separator, 1);
        let (ax, ay) = storyboard::control_anchor(scene, control);
        canvas.fill_circle(ax, ay, 4, tc.accent);
    }
}

fn draw_segue(
    canvas: &ui::Canvas,
    doc: &storyboard::StoryboardDocument,
    segue: &storyboard::StoryboardSegue,
) {
    let Some(from_scene) = doc.scenes.iter().find(|scene| scene.form_name == segue.from_form)
    else {
        return;
    };
    let Some(to_scene) = doc.scenes.iter().find(|scene| scene.form_name == segue.to_form) else {
        return;
    };
    let Some(form) = designer::load_designer(&from_scene.designer_path) else {
        return;
    };
    let Some(control) = form
        .controls
        .iter()
        .find(|control| control.name == segue.from_control)
    else {
        return;
    };
    let (sx, sy) = storyboard::control_anchor(from_scene, control);
    let (w, h) = storyboard::scene_size();
    let ex = to_scene.x + w as i32 / 2;
    let ey = to_scene.y + h as i32 / 2;
    draw_curve(canvas, sx, sy, ex, ey, ui::theme::colors().accent);
    canvas.fill_circle(ex, ey, 5, ui::theme::colors().accent);
    if !segue.condition.is_empty() {
        canvas.draw_text((sx + ex) / 2, (sy + ey) / 2 - 12, 0xfffacc15, 0, 10, &segue.condition);
    }
}

fn draw_curve(canvas: &ui::Canvas, sx: i32, sy: i32, ex: i32, ey: i32, color: u32) {
    let c1x = sx + 90;
    let c1y = sy;
    let c2x = ex - 90;
    let c2y = ey;
    let mut px = sx;
    let mut py = sy;
    for step in 1..=24 {
        let t = step as i32;
        let inv = 24 - t;
        let x = (inv * inv * inv * sx
            + 3 * inv * inv * t * c1x
            + 3 * inv * t * t * c2x
            + t * t * t * ex)
            / (24 * 24 * 24);
        let y = (inv * inv * inv * sy
            + 3 * inv * inv * t * c1y
            + 3 * inv * t * t * c2y
            + t * t * t * ey)
            / (24 * 24 * 24);
        canvas.draw_thick_line(px, py, x, y, color, 2);
        px = x;
        py = y;
    }
}

fn preview_color(kind: &str) -> u32 {
    match kind {
        "Button" | "PlainButton" | "IconButton" | "ImageButton" => 0xff2563eb,
        "TextField" | "TextArea" | "TextEditor" | "SearchField" => 0xff374151,
        "Label" | "LinkLabel" => 0xff52525b,
        "CheckBox" | "RadioButton" | "Toggle" => 0xff16a34a,
        "DataGrid" | "TableView" | "TreeView" | "ListBox" => 0xff7c3aed,
        "GroupBox" | "Panel" | "Card" | "StackPanel" | "FlowPanel" => 0xff475569,
        _ => 0xff3f3f46,
    }
}

fn color_with_alpha(color: u32, alpha: u32) -> u32 {
    (alpha << 24) | (color & 0x00ffffff)
}
