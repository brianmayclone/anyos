//! anyTrace — interactive debugger, profiler & process inspector for anyOS.
//!
//! Architecture:
//!   logic/ — Business logic (no UI imports)
//!   ui/    — UI layer (libanyui_client widgets)
//!   util/  — Shared utilities (hex formatting, register names)
//!
//! Event model:
//!   Uses anyui::run() (blocking event loop) with event callbacks
//!   and timers. Debug events are polled via a 100ms timer.

#![no_std]
#![no_main]

mod logic;
mod ui;
mod util;

use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;
use anyui::Widget;

use crate::logic::{debugger, breakpoints, sampler, snapshots, traces, process_list, unwinder, disasm};
use crate::ui::{
    toolbar, process_tree, registers_view, stack_view, disasm_view,
    memory_view, timeline_view, output_panel, snapshot_view, trace_view, status_bar,
};

// ════════════════════════════════════════════════════════════════
//  Global application state (single-threaded, UI-thread only)
// ════════════════════════════════════════════════════════════════

struct AppState {
    // Logic
    debugger: debugger::DebugSession,
    breakpoints: breakpoints::BreakpointManager,
    sampler: sampler::Sampler,
    snapshots: snapshots::SnapshotStore,
    traces: traces::TraceStore,
    process_list: alloc::vec::Vec<process_list::ProcessEntry>,
    // UI
    toolbar: toolbar::DebugToolbar,
    process_tree: process_tree::ProcessTreeView,
    registers_view: registers_view::RegistersView,
    stack_view: stack_view::StackView,
    disasm_view: disasm_view::DisasmView,
    memory_view: memory_view::MemoryView,
    timeline_view: timeline_view::TimelineView,
    output_panel: output_panel::OutputPanel,
    snapshot_view: snapshot_view::SnapshotView,
    trace_view: trace_view::TraceView,
    status_bar: status_bar::StatusBar,
    // Timer IDs
    poll_timer_id: u32,
    proclist_timer_id: u32,
    status_timer_id: u32,
}

static mut APP: Option<AppState> = None;

/// Access the global app state. Safe because all callbacks run on the UI thread.
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("app not initialized") }
}

// ════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════

anyos_std::entry!(main);

fn main() {
    if !anyui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let tc = anyui::theme::colors();

    // ── Window ──
    let win = anyui::Window::new("anyTrace", -1, -1, 1400, 900);

    // ── Toolbar (DOCK_TOP) ──
    let tb = toolbar::DebugToolbar::new(&win);
    win.add(&tb.toolbar);

    // ── Status bar (DOCK_BOTTOM) ──
    let status = status_bar::StatusBar::new(&win);
    status.view.set_dock(anyui::DOCK_BOTTOM);
    win.add(&status.view);

    // ── Main split: process tree (left) | debug panels (right) ──
    let main_split = anyui::SplitView::new();
    main_split.set_dock(anyui::DOCK_FILL);
    main_split.set_split_ratio(20);
    main_split.set_min_split(15);
    main_split.set_max_split(35);
    win.add(&main_split);

    // ── Left pane: Process/Thread TreeView ──
    let left_container = anyui::View::new();
    left_container.set_color(tc.sidebar_bg);
    let ptree = process_tree::ProcessTreeView::new(&left_container);
    left_container.add(&ptree.tree);
    main_split.add(&left_container);

    // ── Right pane: vertical split (top: main tabs | bottom: secondary tabs) ──
    let right_split = anyui::SplitView::new();
    right_split.set_orientation(anyui::ORIENTATION_VERTICAL);
    right_split.set_split_ratio(60);
    right_split.set_min_split(30);
    right_split.set_max_split(80);
    main_split.add(&right_split);

    // ── Top tab bar: Disassembly | Registers | Memory | Snapshots ──
    let top_container = anyui::View::new();
    top_container.set_dock(anyui::DOCK_FILL);

    let top_tabs = anyui::TabBar::new("Disassembly|Registers|Memory|Snapshots");
    top_tabs.set_dock(anyui::DOCK_TOP);
    top_container.add(&top_tabs);

    let disasm_v = disasm_view::DisasmView::new(&top_container);
    top_container.add(&disasm_v.editor);

    let mut regs_v = registers_view::RegistersView::new(&top_container);
    top_container.add(&regs_v.grid);

    let mem_v = memory_view::MemoryView::new(&top_container);
    top_container.add(&mem_v.editor);

    let snap_v = snapshot_view::SnapshotView::new(&top_container);
    top_container.add(&snap_v.grid);

    // Manual tab switching (heterogeneous control types)
    {
        let ids = [disasm_v.editor.id(), regs_v.grid.id(), mem_v.editor.id(), snap_v.grid.id()];
        // Initially show first panel, hide others
        for i in 1..ids.len() {
            anyui::Control::from_id(ids[i]).set_visible(false);
        }
        top_tabs.on_active_changed(move |e| {
            let active = e.index as usize;
            for (i, &id) in ids.iter().enumerate() {
                anyui::Control::from_id(id).set_visible(i == active);
            }
        });
    }

    right_split.add(&top_container);

    // ── Bottom tab bar: Call Stack | Timeline | Output | Traces ──
    let bottom_container = anyui::View::new();
    bottom_container.set_dock(anyui::DOCK_FILL);

    let bottom_tabs = anyui::TabBar::new("Call Stack|Timeline|Output|Traces");
    bottom_tabs.set_dock(anyui::DOCK_TOP);
    bottom_container.add(&bottom_tabs);

    let stack_v = stack_view::StackView::new(&bottom_container);
    bottom_container.add(&stack_v.tree);

    let timeline_v = timeline_view::TimelineView::new(&bottom_container);
    bottom_container.add(&timeline_v.canvas);

    let output_p = output_panel::OutputPanel::new(&bottom_container);
    bottom_container.add(&output_p.text_area);

    let trace_v = trace_view::TraceView::new(&bottom_container);
    bottom_container.add(&trace_v.grid);

    // Manual tab switching (heterogeneous control types)
    {
        let ids = [stack_v.tree.id(), timeline_v.canvas.id(), output_p.text_area.id(), trace_v.grid.id()];
        for i in 1..ids.len() {
            anyui::Control::from_id(ids[i]).set_visible(false);
        }
        bottom_tabs.on_active_changed(move |e| {
            let active = e.index as usize;
            for (i, &id) in ids.iter().enumerate() {
                anyui::Control::from_id(id).set_visible(i == active);
            }
        });
    }

    right_split.add(&bottom_container);

    // ── Initialize global state ──
    let initial_procs = process_list::poll_processes();
    unsafe {
        APP = Some(AppState {
            debugger: debugger::DebugSession::new(),
            breakpoints: breakpoints::BreakpointManager::new(),
            sampler: sampler::Sampler::new(),
            snapshots: snapshots::SnapshotStore::new(),
            traces: traces::TraceStore::new(),
            process_list: initial_procs,
            toolbar: tb,
            process_tree: ptree,
            registers_view: regs_v,
            stack_view: stack_v,
            disasm_view: disasm_v,
            memory_view: mem_v,
            timeline_view: timeline_v,
            output_panel: output_p,
            snapshot_view: snap_v,
            trace_view: trace_v,
            status_bar: status,
            poll_timer_id: 0,
            proclist_timer_id: 0,
            status_timer_id: 0,
        });
    }

    // ── Initial process tree ──
    {
        let s = app();
        s.process_tree.refresh(&s.process_list);
        s.output_panel.log("anyTrace started.");
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring
    // ════════════════════════════════════════════════════════════════

    // ── Toolbar: Attach ──
    app().toolbar.btn_attach.on_click(|_| {
        on_attach();
    });

    // ── Toolbar: Detach ──
    app().toolbar.btn_detach.on_click(|_| {
        on_detach();
    });

    // ── Toolbar: Suspend ──
    app().toolbar.btn_suspend.on_click(|_| {
        on_suspend();
    });

    // ── Toolbar: Resume ──
    app().toolbar.btn_resume.on_click(|_| {
        on_resume();
    });

    // ── Toolbar: Step Into ──
    app().toolbar.btn_step_into.on_click(|_| {
        on_step_into();
    });

    // ── Toolbar: Step Over (same as step into for now) ──
    app().toolbar.btn_step_over.on_click(|_| {
        on_step_into();
    });

    // ── Toolbar: Step Out (resume until return — simplified) ──
    app().toolbar.btn_step_out.on_click(|_| {
        on_resume();
    });

    // ── Toolbar: Snapshot ──
    app().toolbar.btn_snapshot.on_click(|_| {
        on_snapshot();
    });

    // ── Process tree: double-click to attach ──
    app().process_tree.tree.on_selection_changed(|_| {
        // Selection changed — no action needed until attach button clicked
    });

    // ── Timers ──

    // Poll timer (100ms): debug events only
    app().poll_timer_id = anyui::set_timer(100, poll_timer_callback);

    // Process list timer (2000ms): refresh process tree
    app().proclist_timer_id = anyui::set_timer(2000, proclist_timer_callback);

    // Status timer (1000ms): uptime
    app().status_timer_id = anyui::set_timer(1000, status_timer_callback);

    // ════════════════════════════════════════════════════════════════
    //  Run the event loop (blocking, ~60fps, fires timers + events)
    // ════════════════════════════════════════════════════════════════

    anyui::run();
}

// ════════════════════════════════════════════════════════════════
//  Action handlers
// ════════════════════════════════════════════════════════════════

/// Attach to the selected process in the process tree.
fn on_attach() {
    let s = app();
    let tid = s.process_tree.selected_tid(&s.process_list);
    if tid == 0 {
        s.output_panel.log("No process selected.");
        return;
    }
    if s.debugger.attach(tid) {
        s.output_panel.log(&format!("Attached to TID {}.", tid));
        s.status_bar.set_state(&format!("Attached to TID {}", tid));
        update_all_views();
    } else {
        s.output_panel.log(&format!("Failed to attach to TID {}.", tid));
    }
    update_toolbar_state();
}

/// Detach from the current target.
fn on_detach() {
    let s = app();
    let tid = s.debugger.target_tid;
    s.breakpoints.clear_all(tid);
    s.debugger.detach();
    s.output_panel.log("Detached.");
    s.status_bar.set_state("Detached");
    update_toolbar_state();
}

/// Suspend the running target.
fn on_suspend() {
    let s = app();
    if s.debugger.suspend() {
        s.output_panel.log("Target suspended.");
        update_all_views();
    }
    update_toolbar_state();
}

/// Resume the suspended target.
fn on_resume() {
    let s = app();
    if s.debugger.resume() {
        s.output_panel.log("Target resumed.");
    }
    update_toolbar_state();
}

/// Single-step one instruction.
fn on_step_into() {
    let s = app();
    if s.debugger.step_into() {
        s.output_panel.log("Single step...");
    }
    update_toolbar_state();
}

/// Take a snapshot of the current state.
fn on_snapshot() {
    let s = app();
    if !s.debugger.is_suspended() {
        return;
    }
    let tid = s.debugger.target_tid;
    let label = format!("Snap @ RIP={}", crate::util::format::hex64(s.debugger.regs.rip));
    let idx = s.snapshots.take(tid, &s.debugger.regs, &label);
    s.output_panel.log(&format!("Snapshot #{} taken.", idx));
    s.snapshot_view.update(&s.snapshots.snapshots);
}

// ════════════════════════════════════════════════════════════════
//  View update helpers
// ════════════════════════════════════════════════════════════════

/// Update all debug views after a state change (attach, suspend, step).
fn update_all_views() {
    let s = app();
    if !s.debugger.is_suspended() {
        return;
    }

    let tid = s.debugger.target_tid;
    let regs = &s.debugger.regs;

    // Registers
    let regs_copy = *regs;
    s.registers_view.update(&regs_copy);

    // Disassembly: read 256 bytes at RIP and decode
    let rip = regs.rip;
    let mut code = [0u8; 256];
    let read = s.debugger.read_mem(rip, &mut code);
    if read > 0 {
        s.disasm_view.update(&code[..read], rip, rip);
    }

    // Memory: show stack area (RSP - 64)
    let rsp = regs.rsp;
    let mem_addr = if rsp >= 64 { rsp - 64 } else { 0 };
    let mut mem_buf = [0u8; 256];
    let mem_read = s.debugger.read_mem(mem_addr, &mut mem_buf);
    if mem_read > 0 {
        s.memory_view.update(mem_addr, &mem_buf[..mem_read]);
    }

    // Call stack
    let frames = unwinder::unwind(tid, rip, regs.rbp, 32);
    s.stack_view.update(&frames);

    // Record trace if active
    if s.traces.active {
        let mut instr_code = [0u8; 15];
        let instr_read = s.debugger.read_mem(rip, &mut instr_code);
        if instr_read > 0 {
            if let Some(instr) = disasm::decode(&instr_code[..instr_read], rip) {
                s.traces.record(tid, rip, instr.mnemonic_str(), instr.operands_str());
                s.trace_view.update(&s.traces.entries);
            }
        }
    }

    // Record profiler sample if active
    if s.sampler.active {
        s.sampler.record(tid, rip);
        s.timeline_view.update(&s.sampler.samples, 0, 200);
    }
}

/// Update toolbar button enabled states.
fn update_toolbar_state() {
    let s = app();
    s.toolbar.update_state(s.debugger.is_attached(), s.debugger.is_suspended());
}

// ════════════════════════════════════════════════════════════════
//  Timer callbacks
// ════════════════════════════════════════════════════════════════

/// Poll timer (100ms): check for debug events.
fn poll_timer_callback() {
    let s = app();

    // Poll debug events
    if s.debugger.is_attached() && !s.debugger.is_suspended() {
        if s.debugger.poll_event() {
            if let Some(ref event) = s.debugger.last_event {
                let etype = event.event_type;
                let addr = event.addr;
                match etype {
                    1 => s.output_panel.log(&format!("Breakpoint hit at {}", crate::util::format::hex64(addr))),
                    2 => s.output_panel.log(&format!("Single step at {}", crate::util::format::hex64(addr))),
                    3 => s.output_panel.log("Target exited."),
                    _ => {}
                }
            }
            update_all_views();
            update_toolbar_state();
        }
    }
}

/// Process list timer (2000ms): refresh process tree.
fn proclist_timer_callback() {
    let s = app();
    s.process_list = process_list::poll_processes();
    s.process_tree.refresh(&s.process_list);
}

/// Status timer (1000ms): update uptime display.
fn status_timer_callback() {
    let s = app();
    s.status_bar.update_uptime();

    // Update status bar state text
    if s.debugger.is_attached() {
        let tid = s.debugger.target_tid;
        let state_str = if s.debugger.is_suspended() { "Suspended" } else { "Running" };
        s.status_bar.set_state(&format!("TID {} — {}", tid, state_str));
    }
}
