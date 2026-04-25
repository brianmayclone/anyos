use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::app;

const DLG_W: u32 = 560;
const DLG_H: u32 = 390;
const LABEL_X: i32 = 24;
const FIELD_X: i32 = 160;
const FIELD_W: u32 = 360;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();

    let defaults = default_values();
    let win = ui::Window::new(t("Run Configurations"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 58);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let stripe = ui::View::new();
    stripe.set_dock(ui::DOCK_TOP);
    stripe.set_size(DLG_W, 3);
    stripe.set_color(tc.success);
    header.add(&stripe);

    let title = ui::Label::new(t("Run Configurations"));
    title.set_position(24, 16);
    title.set_font_size(18);
    title.set_text_color(tc.text);
    header.add(&title);

    let desc = ui::Label::new(t("Stored in Cargo.toml metadata for the workspace."));
    desc.set_position(24, 39);
    desc.set_font_size(11);
    desc.set_text_color(tc.text_secondary);
    header.add(&desc);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(tc.editor_bg);
    win.add(&content);

    let mut y = 18;
    let name = add_text_row(&content, t("Name"), &defaults.name, y, tc);
    y += 42;

    let target = add_text_row(&content, t("Cargo Target"), &defaults.target, y, tc);
    y += 42;

    add_label(&content, t("Target Kind"), y + 5, tc);
    let kind = ui::DropDown::new("Binary|Example|Test|Bench");
    kind.set_position(FIELD_X, y);
    kind.set_size(150, 28);
    kind.set_state(defaults.kind_index);
    content.add(&kind);

    add_label(&content, t("Profile"), y + 47, tc);
    let profile = ui::DropDown::new("Debug|Release");
    profile.set_position(FIELD_X, y + 42);
    profile.set_size(150, 28);
    profile.set_state(defaults.profile_index);
    content.add(&profile);
    y += 84;

    let args = add_text_row(&content, t("Executable Args"), &defaults.args, y, tc);
    y += 42;

    let working_dir = add_text_row(&content, t("Working Dir"), &defaults.working_dir, y, tc);
    y += 42;

    let package = add_text_row(&content, t("Package"), &defaults.package, y, tc);

    let hint = ui::Label::new(t("Package can stay empty for single-crate Cargo projects."));
    hint.set_position(FIELD_X, y + 31);
    hint.set_size(FIELD_W, 18);
    hint.set_font_size(10);
    hint.set_text_color(tc.text_secondary);
    content.add(&hint);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 54);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_save = ui::Button::new(t("Save"));
    btn_save.set_size(88, 30);
    btn_save.set_position((DLG_W as i32) - 196, 12);
    btn_save.set_color(tc.success);
    footer.add(&btn_save);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_size(88, 30);
    btn_cancel.set_position((DLG_W as i32) - 100, 12);
    btn_cancel.set_color(tc.control_bg);
    footer.add(&btn_cancel);

    let name_id = name.id();
    let target_id = target.id();
    let kind_id = kind.id();
    let profile_id = profile.id();
    let args_id = args.id();
    let working_dir_id = working_dir.id();
    let package_id = package.id();

    btn_save.on_click(move |_| {
        crate::logic::commands::save_run_configuration(
            read_string(name_id),
            read_string(target_id),
            ui::Control::from_id(kind_id).get_state(),
            ui::Control::from_id(profile_id).get_state(),
            read_string(args_id),
            read_string(working_dir_id),
            read_string(package_id),
        );
        ui::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui::Control::from_id(win_id).set_visible(false);
    });
}

struct RunDefaults {
    name: String,
    target: String,
    kind_index: u32,
    profile_index: u32,
    args: String,
    working_dir: String,
    package: String,
}

fn default_values() -> RunDefaults {
    let s = app();
    if let Some(ref project) = s.current_project {
        if let Some(cfg) = project.run_configs.first() {
            return RunDefaults {
                name: cfg.name.clone(),
                target: cfg.target.clone(),
                kind_index: match cfg.kind {
                    crate::logic::project::TargetKind::Example => 1,
                    crate::logic::project::TargetKind::Test => 2,
                    crate::logic::project::TargetKind::Bench => 3,
                    _ => 0,
                },
                profile_index: match cfg.profile {
                    crate::logic::project::BuildConfiguration::Release => 1,
                    _ => 0,
                },
                args: cfg.args.clone(),
                working_dir: cfg.working_dir.clone(),
                package: cfg.package.clone(),
            };
        }
        for cargo_project in &project.cargo_projects {
            if let Some(cfg) = cargo_project.run_configs.first() {
                return RunDefaults {
                    name: cfg.name.clone(),
                    target: cfg.target.clone(),
                    kind_index: match cfg.kind {
                        crate::logic::project::TargetKind::Example => 1,
                        crate::logic::project::TargetKind::Test => 2,
                        crate::logic::project::TargetKind::Bench => 3,
                        _ => 0,
                    },
                    profile_index: match cfg.profile {
                        crate::logic::project::BuildConfiguration::Release => 1,
                        _ => 0,
                    },
                    args: cfg.args.clone(),
                    working_dir: cfg.working_dir.clone(),
                    package: cargo_project.name.clone(),
                };
            }
        }
        if let Some(target) = project.runnable_targets().first() {
            return RunDefaults {
                name: format!("Run {}", target.name),
                target: target.name.clone(),
                kind_index: if target.kind == crate::logic::project::TargetKind::Example {
                    1
                } else {
                    0
                },
                profile_index: 0,
                args: String::new(),
                working_dir: String::from("."),
                package: String::new(),
            };
        }
        for cargo_project in &project.cargo_projects {
            if let Some(target) = cargo_project.targets.iter().find(|target| {
                target.kind == crate::logic::project::TargetKind::Binary
                    || target.kind == crate::logic::project::TargetKind::Example
            }) {
                return RunDefaults {
                    name: format!("Run {} / {}", cargo_project.name, target.name),
                    target: target.name.clone(),
                    kind_index: if target.kind == crate::logic::project::TargetKind::Example {
                        1
                    } else {
                        0
                    },
                    profile_index: 0,
                    args: String::new(),
                    working_dir: String::from("."),
                    package: cargo_project.name.clone(),
                };
            }
        }
    }
    RunDefaults {
        name: String::from("Run"),
        target: String::new(),
        kind_index: 0,
        profile_index: 0,
        args: String::new(),
        working_dir: String::from("."),
        package: String::new(),
    }
}

fn add_text_row(
    parent: &ui::View,
    label: &str,
    value: &str,
    y: i32,
    tc: &'static ui::theme::ThemeColors,
) -> ui::TextField {
    add_label(parent, label, y + 5, tc);
    let field = ui::TextField::new();
    field.set_position(FIELD_X, y);
    field.set_size(FIELD_W, 28);
    field.set_color(tc.control_bg);
    field.set_text_color(tc.text);
    field.set_text(value);
    parent.add(&field);
    field
}

fn add_label(parent: &ui::View, label: &str, y: i32, tc: &'static ui::theme::ThemeColors) {
    let l = ui::Label::new(label);
    l.set_position(LABEL_X, y);
    l.set_size(120, 18);
    l.set_font_size(12);
    l.set_text_color(tc.text);
    parent.add(&l);
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}
