//! ASL Manager - one-click Debian setup for Anyos Subsystem for Linux.

#![no_std]
#![no_main]

mod asld;
mod constants;
mod installer;

use alloc::string::String;
use anyos_std::process;
use libanyui_client as anyui;
use libanyui_client::Widget;

use crate::constants::*;

anyos_std::entry!(main);

pub(crate) struct AppState {
    pub(crate) install_btn: anyui::Button,
    pub(crate) terminal_btn: anyui::Button,
    pub(crate) nav_overview: anyui::IconButton,
    pub(crate) nav_performance: anyui::IconButton,
    pub(crate) nav_logs: anyui::IconButton,
    pub(crate) overview_panel: anyui::View,
    pub(crate) performance_panel: anyui::View,
    pub(crate) logs_panel: anyui::View,
    pub(crate) status_label: anyui::Label,
    pub(crate) phase_label: anyui::Label,
    pub(crate) progress_bar: anyui::ProgressBar,
    pub(crate) run_state_label: anyui::Label,
    pub(crate) backend_label: anyui::Label,
    pub(crate) memory_label: anyui::Label,
    pub(crate) vcpu_label: anyui::Label,
    pub(crate) activity_label: anyui::Label,
    pub(crate) activity_bar: anyui::ProgressBar,
    pub(crate) exit_label: anyui::Label,
    pub(crate) log_area: anyui::TextArea,
    pub(crate) log_text: String,
    pub(crate) last_log_seq: u32,
    pub(crate) last_total_exits: u64,
    pub(crate) worker: Option<process::Thread>,
}

anyos_std::global_app_state!(AppState);

fn main() {
    if !anyui::init() {
        anyos_std::println!("[ASL Manager] failed to init libanyui");
        return;
    }
    if !libhttp_client::init() {
        anyos_std::println!("[ASL Manager] failed to init libhttp");
        return;
    }

    let state = build_ui();
    unsafe {
        APP = Some(state);
    }

    installer::refresh_status();

    app().install_btn.on_click(|_| installer::start_install());
    app().terminal_btn.on_click(|_| installer::open_terminal());
    app()
        .nav_overview
        .on_click(|_| show_section(Section::Overview));
    app()
        .nav_performance
        .on_click(|_| show_section(Section::Performance));
    app().nav_logs.on_click(|_| show_section(Section::Logs));

    show_section(Section::Overview);
    let _timer_id = anyui::set_timer(500, || {
        installer::on_timer();
        installer::refresh_runtime_metrics(false);
    });
    anyui::run();
}

fn build_ui() -> AppState {
    let tc = anyui::theme::colors();
    let win = anyui::Window::new("ASL Manager", -1, -1, WIN_W, WIN_H);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    win.add(&toolbar);

    let app_icon = toolbar.add_icon_button("");
    app_icon.set_system_icon("server", anyui::IconType::Outline, 0xFFD9463E, 22);
    app_icon.set_enabled(false);

    let title = toolbar.add_label("ASL Manager");
    title.set_font(1);
    title.set_font_size(14);
    title.set_text_color(tc.text);

    toolbar.add_separator();

    let refresh_btn = toolbar.add_icon_button("Refresh");
    refresh_btn.set_system_icon("refresh", anyui::IconType::Outline, tc.text, 20);
    refresh_btn.on_click(|_| {
        installer::refresh_status();
    });

    let root = anyui::View::new();
    root.set_dock(anyui::DOCK_FILL);
    root.set_color(tc.window_bg);
    win.add(&root);

    let (nav_overview, nav_performance, nav_logs) = build_sidebar(&root, tc);
    let panels = build_content_panels(&root);
    let (install_btn, terminal_btn, status_label, phase_label, progress_bar) =
        build_overview_panel(&panels.overview, tc);
    let runtime = build_performance_panel(&panels.performance, tc);
    let log_area = build_logs_panel(&panels.logs, tc);

    installer::register_controls(
        Widget::id(&phase_label),
        Widget::id(&status_label),
        Widget::id(&progress_bar),
    );

    AppState {
        install_btn,
        terminal_btn,
        nav_overview,
        nav_performance,
        nav_logs,
        overview_panel: panels.overview,
        performance_panel: panels.performance,
        logs_panel: panels.logs,
        status_label,
        phase_label,
        progress_bar,
        run_state_label: runtime.run_state_label,
        backend_label: runtime.backend_label,
        memory_label: runtime.memory_label,
        vcpu_label: runtime.vcpu_label,
        activity_label: runtime.activity_label,
        activity_bar: runtime.activity_bar,
        exit_label: runtime.exit_label,
        log_area,
        log_text: String::from("ASL Manager ready.\n"),
        last_log_seq: 0,
        last_total_exits: 0,
        worker: None,
    }
}

fn build_sidebar(
    root: &anyui::View,
    tc: &anyui::theme::ThemeColors,
) -> (anyui::IconButton, anyui::IconButton, anyui::IconButton) {
    let sidebar = anyui::View::new();
    sidebar.set_position(0, 0);
    sidebar.set_size(SIDEBAR_W, WIN_H - 40);
    sidebar.set_color(tc.sidebar_bg);
    root.add(&sidebar);

    let side_title = anyui::Label::new("ASL");
    side_title.set_position(18, 16);
    side_title.set_size(SIDEBAR_W - 36, 22);
    side_title.set_font(1);
    side_title.set_font_size(18);
    side_title.set_text_color(tc.text);
    side_title.set_color(tc.sidebar_bg);
    sidebar.add(&side_title);

    let side_sub = anyui::Label::new("Debian subsystem");
    side_sub.set_position(18, 42);
    side_sub.set_size(SIDEBAR_W - 36, 18);
    side_sub.set_font_size(11);
    side_sub.set_text_color(tc.text_secondary);
    side_sub.set_color(tc.sidebar_bg);
    sidebar.add(&side_sub);

    let distro_title = anyui::Label::new("DISTRIBUTION");
    distro_title.set_position(18, 78);
    distro_title.set_size(SIDEBAR_W - 36, 18);
    distro_title.set_font_size(10);
    side_title.set_text_color(tc.text_secondary);
    distro_title.set_text_color(tc.text_secondary);
    distro_title.set_color(tc.sidebar_bg);
    sidebar.add(&distro_title);

    let debian_row = anyui::View::new();
    debian_row.set_position(12, 104);
    debian_row.set_size(SIDEBAR_W - 24, 64);
    debian_row.set_color(anyui::theme::darken(tc.sidebar_bg, 2));
    sidebar.add(&debian_row);

    let debian_name = anyui::Label::new("Debian");
    debian_name.set_position(14, 9);
    debian_name.set_size(SIDEBAR_W - 52, 22);
    debian_name.set_font_size(15);
    debian_name.set_font(1);
    debian_name.set_text_color(tc.text);
    debian_name.set_color(anyui::theme::darken(tc.sidebar_bg, 2));
    debian_row.add(&debian_name);

    let debian_sub = anyui::Label::new("stable amd64, headless");
    debian_sub.set_position(14, 34);
    debian_sub.set_size(SIDEBAR_W - 52, 18);
    debian_sub.set_font_size(11);
    debian_sub.set_text_color(tc.text_secondary);
    debian_sub.set_color(anyui::theme::darken(tc.sidebar_bg, 2));
    debian_row.add(&debian_sub);

    let nav_title = anyui::Label::new("SECTIONS");
    nav_title.set_position(18, 198);
    nav_title.set_size(SIDEBAR_W - 36, 18);
    nav_title.set_font_size(10);
    nav_title.set_text_color(tc.text_secondary);
    nav_title.set_color(tc.sidebar_bg);
    sidebar.add(&nav_title);

    let nav_overview = nav_button(&sidebar, 12, 224, "Overview", "layout-dashboard", tc);
    let nav_performance = nav_button(&sidebar, 12, 264, "Performance", "activity", tc);
    let nav_logs = nav_button(&sidebar, 12, 304, "Logs", "terminal", tc);
    (nav_overview, nav_performance, nav_logs)
}

struct ContentPanels {
    overview: anyui::View,
    performance: anyui::View,
    logs: anyui::View,
}

fn build_content_panels(root: &anyui::View) -> ContentPanels {
    let content_x = SIDEBAR_W as i32;
    let content_w = WIN_W - SIDEBAR_W;
    let content_h = WIN_H - 40;

    let overview = anyui::View::new();
    overview.set_position(content_x, 0);
    overview.set_size(content_w, content_h);
    overview.set_color(anyui::theme::colors().window_bg);
    root.add(&overview);

    let performance = anyui::View::new();
    performance.set_position(content_x, 0);
    performance.set_size(content_w, content_h);
    performance.set_color(anyui::theme::colors().window_bg);
    performance.set_visible(false);
    root.add(&performance);

    let logs = anyui::View::new();
    logs.set_position(content_x, 0);
    logs.set_size(content_w, content_h);
    logs.set_color(anyui::theme::colors().window_bg);
    logs.set_visible(false);
    root.add(&logs);

    ContentPanels {
        overview,
        performance,
        logs,
    }
}

fn build_overview_panel(
    root: &anyui::View,
    tc: &anyui::theme::ThemeColors,
) -> (
    anyui::Button,
    anyui::Button,
    anyui::Label,
    anyui::Label,
    anyui::ProgressBar,
) {
    let content_x = 0;
    let content_w = WIN_W - SIDEBAR_W;

    let heading = anyui::Label::new("Debian for ASL");
    heading.set_position(content_x + 24, 24);
    heading.set_size(content_w - 48, 30);
    heading.set_font(1);
    heading.set_font_size(21);
    heading.set_text_color(tc.text);
    heading.set_color(tc.window_bg);
    root.add(&heading);

    let summary = anyui::Label::new(
        "One click downloads verified Debian netboot artifacts, registers ASL, and starts it.",
    );
    summary.set_position(content_x + 24, 58);
    summary.set_size(content_w - 48, 34);
    summary.set_font_size(12);
    summary.set_text_color(tc.text_secondary);
    summary.set_color(tc.window_bg);
    root.add(&summary);

    add_fact(
        root,
        content_x + 24,
        100,
        "Mode",
        "Headless netboot, no X11",
    );
    add_fact(
        root,
        content_x + 196,
        100,
        "Source",
        "deb.debian.org over HTTPS",
    );
    add_fact(root, content_x + 404, 100, "Target", "/System/var/asl");

    let install_btn = anyui::Button::new("Install & Start");
    install_btn.set_position(content_x + 24, 156);
    install_btn.set_size(142, 34);
    install_btn.set_color(tc.accent);
    install_btn.set_text_color(0xFFFFFFFF);
    root.add(&install_btn);

    let terminal_btn = anyui::Button::new("Open Terminal");
    terminal_btn.set_position(content_x + 178, 156);
    terminal_btn.set_size(132, 34);
    terminal_btn.set_enabled(false);
    root.add(&terminal_btn);

    let status_label = anyui::Label::new("Ready.");
    status_label.set_position(content_x + 24, 212);
    status_label.set_size(content_w - 48, 20);
    status_label.set_font_size(12);
    status_label.set_text_color(tc.text);
    status_label.set_color(tc.window_bg);
    root.add(&status_label);

    let phase_label = anyui::Label::new("No installation running.");
    phase_label.set_position(content_x + 24, 238);
    phase_label.set_size(content_w - 48, 20);
    phase_label.set_font_size(12);
    phase_label.set_text_color(tc.text_secondary);
    phase_label.set_color(tc.window_bg);
    root.add(&phase_label);

    let progress_bar = anyui::ProgressBar::new(0);
    progress_bar.set_position(content_x + 24, 270);
    progress_bar.set_size(content_w - 48, 18);
    root.add(&progress_bar);

    let hint = anyui::Label::new("Performance and install logs are available from the left tabs.");
    hint.set_position(content_x + 24, 318);
    hint.set_size(content_w - 48, 20);
    hint.set_font_size(12);
    hint.set_text_color(tc.text_secondary);
    hint.set_color(tc.window_bg);
    root.add(&hint);

    (
        install_btn,
        terminal_btn,
        status_label,
        phase_label,
        progress_bar,
    )
}

struct RuntimeControls {
    run_state_label: anyui::Label,
    backend_label: anyui::Label,
    memory_label: anyui::Label,
    vcpu_label: anyui::Label,
    activity_label: anyui::Label,
    activity_bar: anyui::ProgressBar,
    exit_label: anyui::Label,
}

fn build_performance_panel(root: &anyui::View, tc: &anyui::theme::ThemeColors) -> RuntimeControls {
    let content_w = WIN_W - SIDEBAR_W;

    let heading = anyui::Label::new("Runtime Load");
    heading.set_position(24, 24);
    heading.set_size(content_w - 48, 30);
    heading.set_font(1);
    heading.set_font_size(21);
    heading.set_text_color(tc.text);
    heading.set_color(tc.window_bg);
    root.add(&heading);

    let summary = anyui::Label::new("Live ASL runtime status for the Debian VM.");
    summary.set_position(24, 58);
    summary.set_size(content_w - 48, 24);
    summary.set_font_size(12);
    summary.set_text_color(tc.text_secondary);
    summary.set_color(tc.window_bg);
    root.add(&summary);

    let run_state_label = metric_label(root, 24, 112, "State", "not running", tc);
    let backend_label = metric_label(root, 214, 112, "Backend", "-", tc);
    let memory_label = metric_label(root, 404, 112, "Memory", "-", tc);
    let vcpu_label = metric_label(root, 24, 190, "vCPU", "-", tc);
    let exit_label = metric_label(root, 214, 190, "VM exits", "-", tc);

    let activity_title = anyui::Label::new("Activity");
    activity_title.set_position(24, 286);
    activity_title.set_size(content_w - 48, 18);
    activity_title.set_font_size(11);
    activity_title.set_text_color(tc.text_secondary);
    activity_title.set_color(tc.window_bg);
    root.add(&activity_title);

    let activity_bar = anyui::ProgressBar::new(0);
    activity_bar.set_position(24, 312);
    activity_bar.set_size(content_w - 48, 18);
    root.add(&activity_bar);

    let activity_label = anyui::Label::new("Waiting for VM telemetry...");
    activity_label.set_position(24, 342);
    activity_label.set_size(content_w - 48, 22);
    activity_label.set_font_size(12);
    activity_label.set_text_color(tc.text_secondary);
    activity_label.set_color(tc.window_bg);
    root.add(&activity_label);

    RuntimeControls {
        run_state_label,
        backend_label,
        memory_label,
        vcpu_label,
        activity_label,
        activity_bar,
        exit_label,
    }
}

fn build_logs_panel(root: &anyui::View, tc: &anyui::theme::ThemeColors) -> anyui::TextArea {
    let content_w = WIN_W - SIDEBAR_W;

    let details_title = anyui::Label::new("Install Log");
    details_title.set_position(24, 24);
    details_title.set_size(content_w - 48, 20);
    details_title.set_font(1);
    details_title.set_font_size(21);
    details_title.set_text_color(tc.text);
    details_title.set_color(tc.window_bg);
    root.add(&details_title);

    let summary = anyui::Label::new("Download, validation, ASL registration, and start messages.");
    summary.set_position(24, 58);
    summary.set_size(content_w - 48, 22);
    summary.set_font_size(12);
    summary.set_text_color(tc.text_secondary);
    summary.set_color(tc.window_bg);
    root.add(&summary);

    let log_area = anyui::TextArea::new();
    log_area.set_position(24, 100);
    log_area.set_size(content_w - 48, 390);
    log_area.set_read_only(true);
    log_area.set_font(4);
    log_area.set_font_size(12);
    log_area.set_text("ASL Manager ready.\n");
    root.add(&log_area);

    log_area
}

fn add_fact(root: &anyui::View, x: i32, y: i32, label: &str, value: &str) {
    let tc = anyui::theme::colors();

    let label_ctrl = anyui::Label::new(label);
    label_ctrl.set_position(x, y);
    label_ctrl.set_size(160, 16);
    label_ctrl.set_font_size(10);
    label_ctrl.set_text_color(tc.text_secondary);
    label_ctrl.set_color(tc.window_bg);
    root.add(&label_ctrl);

    let value_ctrl = anyui::Label::new(value);
    value_ctrl.set_position(x, y + 18);
    value_ctrl.set_size(184, 18);
    value_ctrl.set_font_size(12);
    value_ctrl.set_text_color(tc.text);
    value_ctrl.set_color(tc.window_bg);
    root.add(&value_ctrl);
}

fn metric_label(
    root: &anyui::View,
    x: i32,
    y: i32,
    label: &str,
    initial: &str,
    tc: &anyui::theme::ThemeColors,
) -> anyui::Label {
    let label_ctrl = anyui::Label::new(label);
    label_ctrl.set_position(x, y);
    label_ctrl.set_size(160, 16);
    label_ctrl.set_font_size(10);
    label_ctrl.set_text_color(tc.text_secondary);
    label_ctrl.set_color(tc.window_bg);
    root.add(&label_ctrl);

    let value_ctrl = anyui::Label::new(initial);
    value_ctrl.set_position(x, y + 22);
    value_ctrl.set_size(170, 30);
    value_ctrl.set_font(1);
    value_ctrl.set_font_size(18);
    value_ctrl.set_text_color(tc.text);
    value_ctrl.set_color(tc.window_bg);
    root.add(&value_ctrl);
    value_ctrl
}

fn nav_button(
    parent: &anyui::View,
    x: i32,
    y: i32,
    text: &str,
    icon: &str,
    tc: &anyui::theme::ThemeColors,
) -> anyui::IconButton {
    let btn = anyui::IconButton::new(text);
    btn.set_position(x, y);
    btn.set_size(SIDEBAR_W - 24, 32);
    btn.set_color(tc.control_bg);
    btn.set_text_color(tc.text);
    btn.set_system_icon(icon, anyui::IconType::Outline, tc.text_secondary, 18);
    parent.add(&btn);
    btn
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Overview,
    Performance,
    Logs,
}

fn show_section(section: Section) {
    let a = app();
    a.overview_panel.set_visible(section == Section::Overview);
    a.performance_panel
        .set_visible(section == Section::Performance);
    a.logs_panel.set_visible(section == Section::Logs);

    let tc = anyui::theme::colors();
    style_nav(&a.nav_overview, section == Section::Overview, tc);
    style_nav(&a.nav_performance, section == Section::Performance, tc);
    style_nav(&a.nav_logs, section == Section::Logs, tc);

    if section == Section::Performance {
        installer::refresh_runtime_metrics(true);
    }
}

fn style_nav(btn: &anyui::IconButton, active: bool, tc: &anyui::theme::ThemeColors) {
    btn.set_color(if active { tc.selection } else { tc.control_bg });
    btn.set_text_color(if active { 0xFFFFFFFF } else { tc.text });
}
