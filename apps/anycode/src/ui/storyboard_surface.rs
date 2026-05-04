use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use libanyui_client as ui;
use ui::Widget;

use crate::logic::{designer, storyboard};

const CANVAS_W: u32 = 1800;
const CANVAS_H: u32 = 1200;

pub struct StoryboardSurface {
    pub panel: ui::View,
    _scroll: ui::ScrollView,
    content: ui::View,
    canvas: ui::Canvas,
    context_menu: ui::ContextMenu,
    zoom: Rc<RefCell<u32>>,
    zoom_label: ui::Label,
    file_path: String,
    doc: Rc<RefCell<storyboard::StoryboardDocument>>,
    drag_source: Rc<RefCell<Option<(String, String)>>>,
    drag_start: Rc<RefCell<Option<(i32, i32)>>>,
    selected_segue: Rc<RefCell<Option<String>>>,
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

        let btn_zoom_out = ui::PlainButton::new("-");
        btn_zoom_out.set_position(380, 4);
        btn_zoom_out.set_size(28, 24);
        btn_zoom_out.set_tooltip("Zoom out");
        header.add(&btn_zoom_out);

        let zoom_label = ui::Label::new("100%");
        zoom_label.set_position(414, 7);
        zoom_label.set_size(52, 18);
        zoom_label.set_font_size(11);
        zoom_label.set_text_color(tc.text_secondary);
        header.add(&zoom_label);

        let btn_zoom_in = ui::PlainButton::new("+");
        btn_zoom_in.set_position(472, 4);
        btn_zoom_in.set_size(28, 24);
        btn_zoom_in.set_tooltip("Zoom in");
        header.add(&btn_zoom_in);

        let btn_zoom_reset = ui::PlainButton::new("100");
        btn_zoom_reset.set_position(506, 4);
        btn_zoom_reset.set_size(42, 24);
        btn_zoom_reset.set_tooltip("Reset zoom");
        header.add(&btn_zoom_reset);

        let hint = ui::Label::new("Drag from a control anchor to another form to create a segue");
        hint.set_position(562, 7);
        hint.set_size(350, 18);
        hint.set_font_size(11);
        hint.set_text_color(tc.text_secondary);
        header.add(&hint);

        let btn_sync = ui::PlainButton::new("Sync Forms");
        btn_sync.set_position(920, 4);
        btn_sync.set_size(92, 24);
        btn_sync.set_tooltip("Add newly created Forms to this Storyboard");
        header.add(&btn_sync);

        let scroll = ui::ScrollView::new();
        scroll.set_dock(ui::DOCK_FILL);
        scroll.set_size(CANVAS_W, CANVAS_H);
        scroll.set_color(tc.editor_bg);
        panel.add(&scroll);

        let content = ui::View::new();
        content.set_position(0, 0);
        content.set_size(CANVAS_W, CANVAS_H);
        content.set_color(tc.editor_bg);
        scroll.add(&content);

        let canvas = ui::Canvas::new(CANVAS_W, CANVAS_H);
        canvas.set_position(0, 0);
        canvas.set_size(CANVAS_W, CANVAS_H);
        canvas.set_interactive(true);
        let context_menu = ui::ContextMenu::new("Delete Segue");
        canvas.set_context_menu(&context_menu);
        content.add(&canvas);
        content.add(&context_menu);

        let zoom = Rc::new(RefCell::new(100u32));

        let surface = Self {
            panel,
            _scroll: scroll,
            content,
            canvas,
            context_menu,
            zoom,
            zoom_label,
            file_path: String::from(file_path),
            doc: Rc::new(RefCell::new(doc)),
            drag_source: Rc::new(RefCell::new(None)),
            drag_start: Rc::new(RefCell::new(None)),
            selected_segue: Rc::new(RefCell::new(None)),
        };
        surface.wire_zoom_buttons(&btn_zoom_out, &btn_zoom_in, &btn_zoom_reset);
        surface.wire_events();
        surface.wire_sync_button(&btn_sync);
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

    pub fn refresh_if_uses_designer(&self, designer_path: &str) -> bool {
        let uses_designer = self
            .doc
            .borrow()
            .scenes
            .iter()
            .any(|scene| scene.designer_path == designer_path);
        if uses_designer {
            self.render();
        }
        uses_designer
    }

    pub fn refresh_from_disk(&self) {
        if let Some(doc) = storyboard::load_storyboard(&self.file_path) {
            *self.doc.borrow_mut() = doc;
        }
        self.render();
    }

    fn wire_events(&self) {
        let file_path = self.file_path.clone();
        let doc_ref = self.doc.clone();
        let drag_ref = self.drag_source.clone();
        let start_ref = self.drag_start.clone();
        let selected_ref = self.selected_segue.clone();
        let file_path_select = self.file_path.clone();
        let canvas_down = self.canvas;
        let canvas_id = self.canvas.id();
        let down_zoom = self.zoom.clone();
        self.canvas.on_mouse_down(move |x, y, button| {
            let zoom = zoom_value(&down_zoom);
            let logical_x = unscale_i32(x, zoom);
            let logical_y = unscale_i32(y, zoom);
            let doc = doc_ref.borrow();
            let is_left_button = button & 0x01 != 0;
            let source = if is_left_button {
                doc.control_anchor_at(logical_x, logical_y)
            } else {
                None
            };
            *start_ref.borrow_mut() = if source.is_some() {
                Some((logical_x, logical_y))
            } else {
                None
            };
            *drag_ref.borrow_mut() = source;
            if drag_ref.borrow().is_some() {
                ui::Control::from_id(canvas_id).set_tooltip("Drag the connector to a target form");
            } else if let Some(segue_id) = segue_at(&doc, logical_x, logical_y) {
                *selected_ref.borrow_mut() = Some(segue_id.clone());
                render_storyboard(&canvas_down, &doc, zoom, Some(&segue_id));
                crate::logic::commands::select_storyboard_segue(&file_path_select, &segue_id);
            } else {
                *selected_ref.borrow_mut() = None;
                render_storyboard(&canvas_down, &doc, zoom, None);
                crate::logic::commands::clear_storyboard_selection(&file_path_select);
            }
        });

        let context_menu = self.context_menu;
        context_menu.on_item_click(|e| {
            if e.index == 0 {
                crate::logic::commands::delete_selected_storyboard_segue();
            }
        });

        let doc_ref_move = self.doc.clone();
        let drag_ref_move = self.drag_source.clone();
        let start_ref_move = self.drag_start.clone();
        let canvas_move = self.canvas;
        let move_zoom = self.zoom.clone();
        self.canvas.on_mouse_move(move |x, y| {
            let Some((from_form, from_control)) = drag_ref_move.borrow().clone() else {
                return;
            };
            let zoom = zoom_value(&move_zoom);
            let logical_x = unscale_i32(x, zoom);
            let logical_y = unscale_i32(y, zoom);
            if let Some((start_x, start_y)) = *start_ref_move.borrow() {
                let dx = logical_x - start_x;
                let dy = logical_y - start_y;
                if dx * dx + dy * dy < 16 {
                    return;
                }
            }
            let doc = doc_ref_move.borrow();
            render_storyboard(&canvas_move, &doc, zoom, None);
            draw_drag_preview(
                &canvas_move,
                &doc,
                &from_form,
                &from_control,
                logical_x,
                logical_y,
                zoom,
            );
        });

        let file_path_up = file_path.clone();
        let doc_ref_up = self.doc.clone();
        let drag_ref_up = self.drag_source.clone();
        let start_ref_up = self.drag_start.clone();
        let canvas_up = self.canvas;
        let canvas_id_up = canvas_up.id();
        let up_zoom = self.zoom.clone();
        self.canvas.on_mouse_up(move |x, y, _button| {
            let Some((from_form, from_control)) = drag_ref_up.borrow_mut().take() else {
                return;
            };
            let zoom = zoom_value(&up_zoom);
            let logical_x = unscale_i32(x, zoom);
            let logical_y = unscale_i32(y, zoom);
            let moved_enough =
                start_ref_up
                    .borrow_mut()
                    .take()
                    .is_some_and(|(start_x, start_y)| {
                        let dx = logical_x - start_x;
                        let dy = logical_y - start_y;
                        dx * dx + dy * dy >= 16
                    });
            {
                let doc = doc_ref_up.borrow();
                render_storyboard(&canvas_up, &doc, zoom, None);
            }
            if !moved_enough {
                return;
            }
            let (target_form, event_options): (Option<String>, String) = {
                let doc = doc_ref_up.borrow();
                let target = doc
                    .scene_at(logical_x, logical_y)
                    .and_then(|idx| doc.scenes.get(idx))
                    .map(|scene| scene.form_name.clone());
                let options = trigger_events_for_source(&doc, &from_form, &from_control);
                (target, options)
            };
            let Some(to_form) = target_form else {
                ui::Control::from_id(canvas_id_up).set_tooltip("Drop on a form to create a segue");
                return;
            };

            let title = format!("Trigger for {}.{}", from_form, from_control);
            let file_path_apply = file_path_up.clone();
            let doc_ref_apply = doc_ref_up.clone();
            let from_form_apply = from_form.clone();
            let from_control_apply = from_control.clone();
            let to_form_apply = to_form.clone();
            crate::ui::storyboard_event_dialog::show(
                &title,
                &event_options,
                move |trigger_event| {
                    let mut doc = doc_ref_apply.borrow_mut();
                    match storyboard::apply_segue(
                        &file_path_apply,
                        &mut doc,
                        &from_form_apply,
                        &from_control_apply,
                        &trigger_event,
                        &to_form_apply,
                    ) {
                        Ok(Some(_)) => {
                            ui::Control::from_id(canvas_id_up).set_tooltip("Segue created");
                            render_storyboard(&canvas_up, &doc, zoom, None);
                            true
                        }
                        Ok(None) => {
                            ui::Control::from_id(canvas_id_up).set_tooltip("Segue already exists");
                            true
                        }
                        Err(err) => {
                            ui::Control::from_id(canvas_id_up).set_tooltip(err);
                            false
                        }
                    }
                },
            );
        });
    }

    fn wire_zoom_buttons(
        &self,
        btn_zoom_out: &ui::PlainButton,
        btn_zoom_in: &ui::PlainButton,
        btn_zoom_reset: &ui::PlainButton,
    ) {
        let out_content = self.content;
        let out_canvas = self.canvas;
        let out_label = self.zoom_label;
        let out_doc = self.doc.clone();
        let out_zoom = self.zoom.clone();
        btn_zoom_out.on_click(move |_| {
            apply_zoom(out_content, out_canvas, out_label, &out_doc, &out_zoom, -10);
        });

        let in_content = self.content;
        let in_canvas = self.canvas;
        let in_label = self.zoom_label;
        let in_doc = self.doc.clone();
        let in_zoom = self.zoom.clone();
        btn_zoom_in.on_click(move |_| {
            apply_zoom(in_content, in_canvas, in_label, &in_doc, &in_zoom, 10);
        });

        let reset_content = self.content;
        let reset_canvas = self.canvas;
        let reset_label = self.zoom_label;
        let reset_doc = self.doc.clone();
        let reset_zoom = self.zoom.clone();
        btn_zoom_reset.on_click(move |_| {
            apply_zoom(
                reset_content,
                reset_canvas,
                reset_label,
                &reset_doc,
                &reset_zoom,
                0,
            );
        });
    }

    fn wire_sync_button(&self, button: &ui::PlainButton) {
        let file_path = self.file_path.clone();
        let doc_ref = self.doc.clone();
        let canvas = self.canvas;
        let content = self.content;
        let zoom_label = self.zoom_label;
        let zoom_ref = self.zoom.clone();
        button.on_click(move |_| {
            let mut doc = doc_ref.borrow_mut();
            match storyboard::sync_document_with_project(&file_path, &mut doc) {
                Ok(added) => {
                    render_storyboard_scaled(
                        content,
                        canvas,
                        zoom_label,
                        &doc,
                        zoom_value(&zoom_ref),
                    );
                    if added == 0 {
                        ui::Control::from_id(canvas.id())
                            .set_tooltip("Storyboard already up to date");
                    } else {
                        ui::Control::from_id(canvas.id())
                            .set_tooltip(&format!("Added {} form(s)", added));
                    }
                }
                Err(err) => ui::Control::from_id(canvas.id()).set_tooltip(err),
            }
        });
    }

    fn render(&self) {
        render_storyboard_scaled(
            self.content,
            self.canvas,
            self.zoom_label,
            &self.doc.borrow(),
            self.zoom_percent(),
        );
    }

    fn zoom_percent(&self) -> u32 {
        zoom_value(&self.zoom)
    }
}

fn trigger_events_for_source(
    doc: &storyboard::StoryboardDocument,
    form_name: &str,
    control_name: &str,
) -> String {
    doc.scenes
        .iter()
        .find(|scene| scene.form_name == form_name)
        .and_then(|scene| designer::load_designer(&scene.designer_path))
        .and_then(|form| {
            form.controls
                .iter()
                .find(|control| control.name == control_name)
                .map(|control| {
                    String::from(storyboard::trigger_event_options_for_control(
                        control.kind.as_str(),
                    ))
                })
        })
        .unwrap_or_else(|| String::from("OnClick|OnChanged|OnSubmit|OnDoubleClick"))
}

fn render_storyboard_scaled(
    content: ui::View,
    canvas: ui::Canvas,
    zoom_label: ui::Label,
    doc: &storyboard::StoryboardDocument,
    zoom: u32,
) {
    let canvas_w = scale_u32(CANVAS_W, zoom);
    let canvas_h = scale_u32(CANVAS_H, zoom);
    content.set_size(canvas_w, canvas_h);
    canvas.set_size(canvas_w, canvas_h);
    zoom_label.set_text(&format!("{}%", zoom));
    render_storyboard(&canvas, doc, zoom, None);
}

fn render_storyboard(
    canvas: &ui::Canvas,
    doc: &storyboard::StoryboardDocument,
    zoom: u32,
    selected_segue: Option<&str>,
) {
    let tc = ui::theme::colors();
    canvas.clear(tc.editor_bg);
    draw_grid(
        canvas,
        scale_u32(CANVAS_W, zoom),
        scale_u32(CANVAS_H, zoom),
        tc.separator,
        zoom,
    );
    for scene in &doc.scenes {
        draw_scene(canvas, scene, zoom);
    }
    for segue in &doc.segues {
        draw_segue(
            canvas,
            doc,
            segue,
            zoom,
            selected_segue == Some(segue.id.as_str()),
        );
    }
}

fn draw_grid(canvas: &ui::Canvas, width: u32, height: u32, color: u32, zoom: u32) {
    let minor = color_with_alpha(color, 0x35);
    let major = color_with_alpha(color, 0x80);
    let mut x = 0;
    let step = scale_i32(20, zoom).max(1);
    let major_step = scale_i32(80, zoom).max(step);
    while x < width as i32 {
        canvas.draw_line(
            x,
            0,
            x,
            height as i32,
            if x % major_step == 0 { major } else { minor },
        );
        x += step;
    }
    let mut y = 0;
    while y < height as i32 {
        canvas.draw_line(
            0,
            y,
            width as i32,
            y,
            if y % major_step == 0 { major } else { minor },
        );
        y += step;
    }
}

fn draw_scene(canvas: &ui::Canvas, scene: &storyboard::StoryboardScene, zoom: u32) {
    let tc = ui::theme::colors();
    let (w, h) = storyboard::scene_size();
    let sx = scale_i32(scene.x, zoom);
    let sy = scale_i32(scene.y, zoom);
    canvas.fill_rect(
        sx,
        sy,
        scale_u32(w, zoom),
        scale_u32(h, zoom),
        tc.sidebar_bg,
    );
    canvas.draw_rect(sx, sy, scale_u32(w, zoom), scale_u32(h, zoom), tc.accent, 2);
    canvas.fill_rect(
        sx,
        sy,
        scale_u32(w, zoom),
        scale_u32(28, zoom),
        tc.toolbar_bg,
    );
    canvas.draw_text(
        scale_i32(scene.x + 10, zoom),
        scale_i32(scene.y + 8, zoom),
        tc.text,
        1,
        scale_font(12, zoom),
        &scene.form_name,
    );

    let Some(doc) = designer::load_designer(&scene.designer_path) else {
        canvas.draw_text(
            scale_i32(scene.x + 10, zoom),
            scale_i32(scene.y + 52, zoom),
            0xffef4444,
            0,
            scale_font(11, zoom),
            "Designer missing",
        );
        return;
    };
    let ox = scene.x + 16;
    let oy = scene.y + 36;
    let form_w = (doc.width / 3).min(w - 32);
    let form_h = (doc.height / 3).min(h - 48);
    let preview_detail = preview_detail_for_zoom(zoom);
    canvas.fill_rect(
        scale_i32(ox, zoom),
        scale_i32(oy, zoom),
        scale_u32(form_w, zoom),
        scale_u32(form_h, zoom),
        tc.control_bg,
    );
    canvas.draw_rect(
        scale_i32(ox, zoom),
        scale_i32(oy, zoom),
        scale_u32(form_w, zoom),
        scale_u32(form_h, zoom),
        tc.separator,
        1,
    );
    for control in &doc.controls {
        let cx = ox + control.x / 3;
        let cy = oy + control.y / 3;
        let cw = (control.width / 3).max(8);
        let ch = (control.height / 3).max(6);
        draw_control_preview(canvas, control, cx, cy, cw, ch, zoom, preview_detail);
        let (ax, ay) = storyboard::control_anchor(scene, control);
        canvas.fill_circle(
            scale_i32(ax, zoom),
            scale_i32(ay, zoom),
            scale_i32(if preview_detail >= 2 { 5 } else { 4 }, zoom).max(3),
            tc.accent,
        );
        if preview_detail >= 2 {
            canvas.draw_text(
                scale_i32(ax + 6, zoom),
                scale_i32(ay - 5, zoom),
                tc.accent,
                0,
                scale_font(8, zoom),
                "event",
            );
        }
    }
}

fn draw_control_preview(
    canvas: &ui::Canvas,
    control: &designer::DesignerControl,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    zoom: u32,
    detail: u32,
) {
    let tc = ui::theme::colors();
    let sx = scale_i32(x, zoom);
    let sy = scale_i32(y, zoom);
    let sw = scale_u32(w, zoom);
    let sh = scale_u32(h, zoom);
    let kind = control.kind.as_str();
    let bg = parse_color(&control.property_value("BackgroundColor"))
        .unwrap_or_else(|| preview_color(kind));
    let fg = parse_color(&control.property_value("TextColor")).unwrap_or(tc.text);
    let border = control_border_color(kind);

    match kind {
        "Label" | "LinkLabel" => {
            canvas.fill_rect(sx, sy, sw, sh, transparent_color(bg, 0x28));
            if detail >= 1 {
                canvas.draw_text(
                    sx + scale_i32(2, zoom),
                    sy + scale_i32(1, zoom),
                    fg,
                    0,
                    scale_font(8, zoom),
                    &preview_text(control),
                );
            }
        }
        "Button" | "PlainButton" | "IconButton" | "ImageButton" => {
            canvas.fill_rect(sx, sy, sw, sh, bg);
            canvas.draw_rect(sx, sy, sw, sh, border, 1);
            if detail >= 1 {
                draw_centered_preview_text(
                    canvas,
                    sx,
                    sy,
                    sw,
                    sh,
                    fg,
                    zoom,
                    &preview_text(control),
                );
            }
        }
        "TextField" | "SearchField" | "AutoCompleteTextField" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff1f2937);
            canvas.draw_rect(sx, sy, sw, sh, 0xff64748b, 1);
            if detail >= 1 {
                canvas.draw_text(
                    sx + scale_i32(5, zoom),
                    sy + scale_i32(3, zoom),
                    0xffcbd5e1,
                    0,
                    scale_font(8, zoom),
                    &preview_placeholder(control),
                );
            }
        }
        "TextArea" | "TextEditor" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff111827);
            canvas.draw_rect(sx, sy, sw, sh, 0xff475569, 1);
            if detail >= 1 {
                draw_preview_lines(canvas, sx, sy, sw, sh, zoom);
            }
        }
        "CheckBox" | "RadioButton" | "Toggle" => {
            canvas.fill_rect(sx, sy, sw, sh, transparent_color(bg, 0x30));
            let mark = scale_i32(9, zoom).max(5);
            if kind == "RadioButton" {
                canvas.fill_circle(sx + mark / 2, sy + mark / 2, mark / 2, 0xff334155);
                canvas.fill_circle(sx + mark / 2, sy + mark / 2, (mark / 3).max(2), 0xff22c55e);
            } else if kind == "Toggle" {
                canvas.fill_rect(sx, sy, scale_u32(24, zoom), scale_u32(10, zoom), 0xff334155);
                canvas.fill_circle(
                    sx + scale_i32(7, zoom),
                    sy + scale_i32(5, zoom),
                    scale_i32(4, zoom).max(2),
                    0xff22c55e,
                );
            } else {
                canvas.draw_rect(sx, sy, mark as u32, mark as u32, 0xff94a3b8, 1);
                canvas.draw_line(
                    sx + 2,
                    sy + mark / 2,
                    sx + mark / 3,
                    sy + mark - 2,
                    0xff22c55e,
                );
                canvas.draw_line(
                    sx + mark / 3,
                    sy + mark - 2,
                    sx + mark - 1,
                    sy + 1,
                    0xff22c55e,
                );
            }
            if detail >= 1 {
                canvas.draw_text(
                    sx + scale_i32(15, zoom),
                    sy,
                    fg,
                    0,
                    scale_font(8, zoom),
                    &preview_text(control),
                );
            }
        }
        "ComboBox" | "DropDown" | "SegmentedControl" | "TabBar" | "RadioGroup" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff334155);
            canvas.draw_rect(sx, sy, sw, sh, 0xff64748b, 1);
            let item_text = preview_items(control).replace('|', "  ");
            if detail >= 1 {
                canvas.draw_text(
                    sx + scale_i32(5, zoom),
                    sy + scale_i32(3, zoom),
                    0xffe2e8f0,
                    0,
                    scale_font(8, zoom),
                    &item_text,
                );
            }
        }
        "ListBox" | "TreeView" | "DataGrid" | "TableView" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff18181b);
            canvas.draw_rect(sx, sy, sw, sh, 0xff7c3aed, 1);
            if detail >= 1 {
                draw_preview_rows(canvas, sx, sy, sw, sh, zoom, kind);
            }
        }
        "GroupBox" | "Panel" | "Card" | "ScrollView" | "SplitView" | "StackPanel" | "FlowPanel"
        | "TableLayout" => {
            canvas.fill_rect(sx, sy, sw, sh, transparent_color(bg, 0x55));
            canvas.draw_rect(sx, sy, sw, sh, 0xff94a3b8, 1);
            if detail >= 1 {
                canvas.draw_text(
                    sx + scale_i32(4, zoom),
                    sy + scale_i32(2, zoom),
                    0xffcbd5e1,
                    0,
                    scale_font(8, zoom),
                    &preview_container_label(control),
                );
            }
        }
        "ProgressBar" | "Slider" | "Stepper" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff27272a);
            let fill_w = (sw / 2).max(1);
            canvas.fill_rect(sx, sy, fill_w, sh, 0xff0ea5e9);
            canvas.draw_rect(sx, sy, sw, sh, 0xff38bdf8, 1);
        }
        "ColorWell" => {
            canvas.fill_rect(sx, sy, sw, sh, bg);
            canvas.draw_rect(sx, sy, sw, sh, 0xffe5e7eb, 1);
        }
        "ImageView" | "Canvas" => {
            canvas.fill_rect(sx, sy, sw, sh, 0xff1e293b);
            canvas.draw_rect(sx, sy, sw, sh, 0xff0ea5e9, 1);
            canvas.draw_line(sx, sy, sx + sw as i32, sy + sh as i32, 0xff64748b);
            canvas.draw_line(sx + sw as i32, sy, sx, sy + sh as i32, 0xff64748b);
        }
        _ => {
            canvas.fill_rect(sx, sy, sw, sh, bg);
            canvas.draw_rect(sx, sy, sw, sh, border, 1);
            if detail >= 1 {
                draw_centered_preview_text(
                    canvas,
                    sx,
                    sy,
                    sw,
                    sh,
                    fg,
                    zoom,
                    &preview_text(control),
                );
            }
        }
    }

    if detail >= 2 {
        canvas.draw_text(
            sx,
            sy + sh as i32 + scale_i32(3, zoom),
            0xff94a3b8,
            0,
            scale_font(7, zoom),
            &control.name,
        );
    }
}

fn draw_segue(
    canvas: &ui::Canvas,
    doc: &storyboard::StoryboardDocument,
    segue: &storyboard::StoryboardSegue,
    zoom: u32,
    selected: bool,
) {
    let Some(from_scene) = doc
        .scenes
        .iter()
        .find(|scene| scene.form_name == segue.from_form)
    else {
        return;
    };
    let Some(to_scene) = doc
        .scenes
        .iter()
        .find(|scene| scene.form_name == segue.to_form)
    else {
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
    let color = if selected {
        0xfffacc15
    } else {
        ui::theme::colors().accent
    };
    draw_curve(
        canvas,
        scale_i32(sx, zoom),
        scale_i32(sy, zoom),
        scale_i32(ex, zoom),
        scale_i32(ey, zoom),
        color,
    );
    canvas.fill_circle(
        scale_i32(ex, zoom),
        scale_i32(ey, zoom),
        scale_i32(5, zoom).max(4),
        color,
    );
    let label = if segue.condition.is_empty() {
        segue.trigger_event.clone()
    } else {
        format!("{} / {}", segue.trigger_event, segue.condition)
    };
    let lx = scale_i32((sx + ex) / 2, zoom);
    let ly = scale_i32((sy + ey) / 2 - 12, zoom);
    if selected {
        canvas.fill_rect(
            lx - scale_i32(4, zoom),
            ly - scale_i32(2, zoom),
            scale_u32(96, zoom),
            scale_u32(16, zoom),
            0xaa1f2937,
        );
    }
    canvas.draw_text(lx, ly, 0xfffacc15, 0, scale_font(10, zoom), &label);
}

fn draw_drag_preview(
    canvas: &ui::Canvas,
    doc: &storyboard::StoryboardDocument,
    from_form: &str,
    from_control: &str,
    mouse_x: i32,
    mouse_y: i32,
    zoom: u32,
) {
    let Some((sx, sy)) = source_anchor(doc, from_form, from_control) else {
        return;
    };
    let target_color = doc
        .scene_at(mouse_x, mouse_y)
        .and_then(|idx| doc.scenes.get(idx))
        .map(|scene| {
            let (w, h) = storyboard::scene_size();
            canvas.draw_rect(
                scale_i32(scene.x - 4, zoom),
                scale_i32(scene.y - 4, zoom),
                scale_u32(w + 8, zoom),
                scale_u32(h + 8, zoom),
                0xff22c55e,
                2,
            );
            0xff22c55e
        })
        .unwrap_or(0xfffacc15);
    draw_curve(
        canvas,
        scale_i32(sx, zoom),
        scale_i32(sy, zoom),
        scale_i32(mouse_x, zoom),
        scale_i32(mouse_y, zoom),
        target_color,
    );
    canvas.fill_circle(
        scale_i32(mouse_x, zoom),
        scale_i32(mouse_y, zoom),
        scale_i32(5, zoom).max(4),
        target_color,
    );
}

fn segue_at(doc: &storyboard::StoryboardDocument, x: i32, y: i32) -> Option<String> {
    for segue in doc.segues.iter().rev() {
        let Some((sx, sy)) = source_anchor(doc, &segue.from_form, &segue.from_control) else {
            continue;
        };
        let Some(to_scene) = doc
            .scenes
            .iter()
            .find(|scene| scene.form_name == segue.to_form)
        else {
            continue;
        };
        let (w, h) = storyboard::scene_size();
        let ex = to_scene.x + w as i32 / 2;
        let ey = to_scene.y + h as i32 / 2;
        let label_x = (sx + ex) / 2;
        let label_y = (sy + ey) / 2 - 12;
        if x >= label_x - 8 && x <= label_x + 112 && y >= label_y - 6 && y <= label_y + 18 {
            return Some(segue.id.clone());
        }
        if distance_to_line_sq(x, y, sx, sy, ex, ey) <= 144 {
            return Some(segue.id.clone());
        }
    }
    None
}

fn distance_to_line_sq(px: i32, py: i32, ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
    let abx = (bx - ax) as i64;
    let aby = (by - ay) as i64;
    let apx = (px - ax) as i64;
    let apy = (py - ay) as i64;
    let len_sq = abx * abx + aby * aby;
    if len_sq == 0 {
        let dx = px - ax;
        let dy = py - ay;
        return dx * dx + dy * dy;
    }
    let t = ((apx * abx + apy * aby) * 1024 / len_sq).clamp(0, 1024);
    let cx = ax as i64 + abx * t / 1024;
    let cy = ay as i64 + aby * t / 1024;
    let dx = px as i64 - cx;
    let dy = py as i64 - cy;
    (dx * dx + dy * dy).min(i32::MAX as i64) as i32
}

fn source_anchor(
    doc: &storyboard::StoryboardDocument,
    from_form: &str,
    from_control: &str,
) -> Option<(i32, i32)> {
    let scene = doc
        .scenes
        .iter()
        .find(|scene| scene.form_name == from_form)?;
    let form = designer::load_designer(&scene.designer_path)?;
    let control = form
        .controls
        .iter()
        .find(|control| control.name == from_control)?;
    Some(storyboard::control_anchor(scene, control))
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

fn control_border_color(kind: &str) -> u32 {
    match kind {
        "Button" | "PlainButton" | "IconButton" | "ImageButton" => 0xff60a5fa,
        "TextField" | "TextArea" | "TextEditor" | "SearchField" => 0xff94a3b8,
        "CheckBox" | "RadioButton" | "Toggle" => 0xff22c55e,
        "DataGrid" | "TableView" | "TreeView" | "ListBox" => 0xffa78bfa,
        "GroupBox" | "Panel" | "Card" | "StackPanel" | "FlowPanel" => 0xff94a3b8,
        _ => 0xff71717a,
    }
}

fn preview_detail_for_zoom(zoom: u32) -> u32 {
    if zoom >= 150 {
        2
    } else if zoom >= 95 {
        1
    } else {
        0
    }
}

fn preview_text(control: &designer::DesignerControl) -> String {
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

fn preview_placeholder(control: &designer::DesignerControl) -> String {
    let placeholder = control.property_value("Placeholder");
    if !placeholder.is_empty() {
        placeholder
    } else {
        preview_text(control)
    }
}

fn preview_items(control: &designer::DesignerControl) -> String {
    let items = control.property_value("Items");
    if !items.is_empty() {
        items
    } else if !control.text.is_empty() {
        control.text.clone()
    } else {
        String::from("Item 1|Item 2")
    }
}

fn preview_container_label(control: &designer::DesignerControl) -> String {
    let text = preview_text(control);
    if text == control.kind.as_str() {
        text
    } else {
        format!("{}  {}", control.kind.as_str(), text)
    }
}

fn draw_centered_preview_text(
    canvas: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
    zoom: u32,
    text: &str,
) {
    canvas.draw_text(
        x + scale_i32(4, zoom),
        y + ((h as i32 - scale_i32(10, zoom)) / 2).max(1),
        color,
        0,
        scale_font(8, zoom),
        text,
    );
    let _ = w;
}

fn draw_preview_lines(canvas: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, zoom: u32) {
    let mut yy = y + scale_i32(5, zoom);
    let step = scale_i32(8, zoom).max(4);
    let right = x + w as i32 - scale_i32(6, zoom);
    let mut idx = 0;
    while yy < y + h as i32 - step && idx < 5 {
        let inset = scale_i32(if idx % 2 == 0 { 6 } else { 18 }, zoom);
        canvas.draw_line(x + inset, yy, right, yy, 0xff64748b);
        yy += step;
        idx += 1;
    }
}

fn draw_preview_rows(canvas: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, zoom: u32, kind: &str) {
    let row_h = scale_i32(10, zoom).max(5);
    let mut yy = y + row_h;
    if kind == "DataGrid" || kind == "TableView" {
        canvas.fill_rect(x, y, w, row_h as u32, 0xff312e81);
    }
    let mut idx = 0;
    while yy < y + h as i32 && idx < 5 {
        canvas.draw_line(x, yy, x + w as i32, yy, 0xff3f3f46);
        yy += row_h;
        idx += 1;
    }
}

fn color_with_alpha(color: u32, alpha: u32) -> u32 {
    (alpha << 24) | (color & 0x00ffffff)
}

fn transparent_color(color: u32, alpha: u32) -> u32 {
    (alpha << 24) | (color & 0x00ffffff)
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

fn apply_zoom(
    content: ui::View,
    canvas: ui::Canvas,
    zoom_label: ui::Label,
    doc_ref: &Rc<RefCell<storyboard::StoryboardDocument>>,
    zoom_ref: &Rc<RefCell<u32>>,
    delta: i32,
) {
    let next = if delta == 0 {
        100
    } else {
        (zoom_value(zoom_ref) as i32 + delta).clamp(50, 200) as u32
    };
    *zoom_ref.borrow_mut() = next;
    render_storyboard_scaled(content, canvas, zoom_label, &doc_ref.borrow(), next);
}

fn zoom_value(zoom: &Rc<RefCell<u32>>) -> u32 {
    (*zoom.borrow()).clamp(50, 200)
}

fn scale_i32(value: i32, zoom: u32) -> i32 {
    ((value as i64 * zoom as i64) / 100) as i32
}

fn unscale_i32(value: i32, zoom: u32) -> i32 {
    ((value as i64 * 100) / zoom.max(1) as i64) as i32
}

fn scale_u32(value: u32, zoom: u32) -> u32 {
    ((value as u64 * zoom as u64) / 100).max(1) as u32
}

fn scale_font(value: u32, zoom: u32) -> u16 {
    scale_u32(value, zoom).max(8).min(32) as u16
}
