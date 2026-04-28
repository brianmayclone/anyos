use alloc::string::String;
use libanyui_client as anyui;

use crate::logic::{
    ai, build, config, debug_backend, debug_session, diagnostic_pipeline, diagnostics,
    file_manager, git, plugin, project, solution, symbol_index, tasks, test_explorer,
};
use crate::ui::{
    activity_bar, ai_panel, command_palette, editor_view, extensions_panel, git_panel,
    inspector_panel, output_panel, problems_panel, run_panel, search_panel, sidebar, status_bar,
    symbols_panel, welcome_tab,
};

pub struct AppState {
    // Core
    pub file_mgr: file_manager::FileManager,
    pub config: config::Config,
    pub current_project: Option<project::Project>,
    pub solution: Option<solution::SolutionMetadata>,

    // Subsystems
    pub task_mgr: tasks::TaskManager,
    pub test_explorer: test_explorer::TestExplorerState,
    pub diagnostics: diagnostics::DiagnosticSet,
    pub symbol_index: symbol_index::SymbolIndex,
    pub active_completions: alloc::vec::Vec<crate::logic::intellisense::CompletionItem>,
    pub active_completion_prefix: String,
    pub debug_backend: debug_backend::DebugBackend,
    pub debug_session: debug_session::DebugSession,
    pub plugin_mgr: plugin::PluginManager,
    pub ai_client: ai::AiClient,

    // Build
    pub build_process: Option<build::BuildProcess>,
    pub active_task_category: Option<tasks::TaskCategory>,
    pub build_rules: build::BuildRules,
    pub build_timer_id: u32,
    pub debug_timer_id: u32,
    pub build_output_buffer: String,

    // Live analysis
    pub live_check_process: Option<build::BuildProcess>,
    pub live_check: diagnostic_pipeline::LiveCheckState,

    // Git
    pub git_state: git::GitState,
    pub git_process: Option<git::GitProcess>,
    pub git_pending_op: Option<git::GitOp>,
    pub git_timer_id: u32,

    // UI panels
    pub editor_view: editor_view::EditorView,
    pub side_editor_view: editor_view::EditorView,
    pub welcome: welcome_tab::WelcomeTab,
    pub sidebar: sidebar::Sidebar,
    pub git_panel: git_panel::GitPanel,
    pub search_panel: search_panel::SearchPanel,
    pub run_panel: run_panel::RunPanel,
    pub symbols_panel: symbols_panel::SymbolsPanel,
    pub ai_panel: ai_panel::AiPanel,
    pub extensions_panel: extensions_panel::ExtensionsPanel,
    pub output: output_panel::OutputPanel,
    pub problems_panel: problems_panel::ProblemsPanel,
    pub inspector_panel: inspector_panel::InspectorPanel,
    pub status: status_bar::StatusBar,
    pub activity_bar: activity_bar::ActivityBar,
    pub command_palette: command_palette::CommandPalette,
    pub editor_groups_split: anyui::SplitView,
    pub side_file_mgr: file_manager::FileManager,
    pub split_visible: bool,
    pub panel_ids: [u32; 7],

    // Toolbar and global action IDs
    pub toolbar_save_id: u32,
    pub toolbar_save_all_id: u32,
    pub toolbar_build_id: u32,
    pub toolbar_run_config_button_id: u32,
    pub toolbar_run_id: u32,
    pub toolbar_debug_id: u32,
    pub toolbar_debug_continue_id: u32,
    pub toolbar_debug_pause_id: u32,
    pub toolbar_debug_step_id: u32,
    pub toolbar_stop_id: u32,
    pub run_config_dropdown_id: u32,
    pub debug_profile_dropdown_id: u32,

    // Designer interaction state
    pub selected_designer_file: String,
    pub selected_designer_control: String,
    pub pending_designer_event_file: String,
    pub pending_designer_event_x: i32,
    pub pending_designer_event_y: i32,
    pub pending_designer_event_kind: u32,
    pub pending_designer_event_payload: String,
    pub designer_event_timer_id: u32,
    pub designer_drag_file: String,
    pub designer_drag_control: String,
    pub designer_drag_mode: u32,
    pub designer_drag_start_x: i32,
    pub designer_drag_start_y: i32,
    pub designer_drag_orig_x: i32,
    pub designer_drag_orig_y: i32,
    pub designer_drag_orig_w: u32,
    pub designer_drag_orig_h: u32,
    pub designer_drag_moved: bool,
}
