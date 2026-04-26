use alloc::format;
use libanyui_client as ui;
use ui::Widget;

use crate::app;

const DLG_W: u32 = 820;
const DLG_H: u32 = 560;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();
    let Some(project) = app().current_project.as_ref() else {
        return;
    };

    let win = ui::Window::new(t("Project Properties"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 64);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new("Project Properties");
    title.set_position(22, 12);
    title.set_size(420, 24);
    title.set_font_size(18);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(&format!("{} - {}", project.name, project.root));
    subtitle.set_position(22, 38);
    subtitle.set_size(740, 18);
    subtitle.set_font_size(11);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let tabs = ui::TabBar::new(
        "Application|Build|Run|Debug|Dependencies|Connected Services|Designer/UI|AI/Codex",
    );
    tabs.set_dock(ui::DOCK_TOP);
    tabs.set_size(DLG_W, 34);
    tabs.set_color(tc.toolbar_bg);
    tabs.set_style(ui::STYLE_ACTIVE_BG, tc.editor_bg);
    tabs.set_style(ui::STYLE_ACTIVE_TEXT, tc.text);
    tabs.set_style(ui::STYLE_INACTIVE_BG, tc.toolbar_bg);
    tabs.set_style(ui::STYLE_INACTIVE_TEXT, tc.text_secondary);
    tabs.set_style(ui::STYLE_HOVER_BG, tc.sidebar_bg);
    tabs.set_style(ui::STYLE_ACCENT, tc.accent);
    tabs.set_style(ui::STYLE_RADIUS, 6);
    win.add(&tabs);

    let pages = [
        page("Application", application_text()),
        page("Build", build_text()),
        page("Run", run_text()),
        page("Debug", debug_text()),
        page("Dependencies", dependencies_text()),
        page("Connected Services", connected_services_text()),
        page("Designer/UI", designer_text()),
        page("AI/Codex", ai_text()),
    ];
    for page in &pages {
        win.add(page);
    }
    tabs.connect_panels(&[
        &pages[0], &pages[1], &pages[2], &pages[3], &pages[4], &pages[5], &pages[6], &pages[7],
    ]);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 52);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_close = ui::Button::new("Close");
    btn_close.set_position((DLG_W as i32) - 112, 11);
    btn_close.set_size(90, 30);
    btn_close.set_color(tc.control_bg);
    footer.add(&btn_close);
    btn_close.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}

fn page(title: &str, body: alloc::string::String) -> ui::View {
    let tc = ui::theme::colors();
    let page = ui::View::new();
    page.set_dock(ui::DOCK_FILL);
    page.set_color(tc.editor_bg);

    let label = ui::Label::new(title);
    label.set_position(24, 18);
    label.set_size(360, 22);
    label.set_font_size(15);
    label.set_text_color(tc.text);
    page.add(&label);

    let editor = ui::TextEditor::new(740, 330);
    editor.set_position(24, 54);
    editor.set_text(&body);
    page.add(&editor);
    page
}

fn application_text() -> alloc::string::String {
    let s = app();
    let Some(project) = s.current_project.as_ref() else {
        return alloc::string::String::new();
    };
    let startup_project = s
        .solution
        .as_ref()
        .map(|solution| solution.startup_project.as_str())
        .unwrap_or(project.name.as_str());
    let startup_form = s
        .solution
        .as_ref()
        .map(|solution| solution.startup_form.as_str())
        .unwrap_or("");
    let project_count = s
        .solution
        .as_ref()
        .map(|solution| solution.project_count(project))
        .unwrap_or(1);
    format!(
        "Name: {}\nRoot: {}\nType: {}\nWorkspace: {}\nProjects: {}\nStartup project: {}\nStartup form: {}\nMetadata: .anycode-workspace\n",
        project.name,
        project.root,
        project.project_type.display_name(),
        if project.is_workspace { "yes" } else { "no" },
        project_count,
        startup_project,
        if startup_form.is_empty() { "not set" } else { startup_form }
    )
}

fn build_text() -> alloc::string::String {
    let s = app();
    let Some(project) = s.current_project.as_ref() else {
        return alloc::string::String::new();
    };
    let build_order = s
        .solution
        .as_ref()
        .map(|solution| solution.build_order.join(" -> "))
        .unwrap_or_else(|| project.name.clone());
    format!(
        "Configuration: {}\nTargets: {}\nBuild order: {}\nToolchain: ccargo/crust/anyrc from confd settings\n",
        project.active_configuration.display_name(),
        project.target_count(),
        build_order
    )
}

fn run_text() -> alloc::string::String {
    let s = app();
    let Some(project) = s.current_project.as_ref() else {
        return alloc::string::String::new();
    };
    let startup_run_config = s
        .solution
        .as_ref()
        .map(|solution| solution.startup_run_config.as_str())
        .unwrap_or("");
    format!(
        "Run configurations: {}\nStartup run config: {}\nToolbar selected run config is reflected from Project metadata.\n",
        project.run_configs.len()
            + project
                .cargo_projects
                .iter()
                .map(|p| p.run_configs.len())
                .sum::<usize>(),
        if startup_run_config.is_empty() {
            "not set"
        } else {
            startup_run_config
        }
    )
}

fn debug_text() -> alloc::string::String {
    let s = app();
    format!(
        "Breakpoints: {}\nSession: {:?}\nPanels: Call Stack, Registers, Memory and Disassembly are available in Run and Debug.\n",
        s.debug_session.breakpoint_count(),
        s.debug_session.status
    )
}

fn dependencies_text() -> alloc::string::String {
    let s = app();
    let Some(project) = s.current_project.as_ref() else {
        return alloc::string::String::new();
    };
    let deps = crate::logic::crates::dependencies_for_project(project);
    format!(
        "Crates: {}\nInstalled/Browse/Updates dialog: available\nSecurity/license checks: pending registry metadata backend\nWorkspace consolidation: pending\n",
        deps.len()
    )
}

fn connected_services_text() -> alloc::string::String {
    let s = app();
    let Some(project) = s.current_project.as_ref() else {
        return alloc::string::String::new();
    };
    let services = crate::logic::connected_services::services_for_project(project);
    let mut out = format!("Connected services: {}\n", services.len());
    for service in &services {
        let preview = crate::logic::connected_services::preview_service(project, service);
        out.push_str(&format!(
            "\n{} ({})\n{}\nFiles:\n",
            service.name,
            service.kind.display_name(),
            preview.summary
        ));
        for file in &preview.files {
            out.push_str(&format!("  {}\n", file));
        }
        out.push_str("Operations:\n");
        for operation in &preview.operations {
            out.push_str(&format!(
                "  {} {} -> {}\n",
                operation.method, operation.path, operation.name
            ));
        }
    }
    out
}

fn designer_text() -> alloc::string::String {
    alloc::string::String::from(
        "Designer metadata: .Designer files\nProperty grid: typed layout properties in progress\nEvents: double-click handler generation active\nUndo/Redo, alignment and resources: pending\n",
    )
}

fn ai_text() -> alloc::string::String {
    alloc::string::String::from(
        "Codex provider: configured through AI settings/confd\nPatch preview: required for mutating agent tasks\nConnected Services can later expose AI-assisted client generation and review.\n",
    )
}
