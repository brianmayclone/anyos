use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use eframe::egui;
use crate::theme;

const MAX_LOG_ENTRIES: usize = 2000;

#[derive(Clone, Debug)]
pub struct DiagEntry {
    pub timestamp_ms: u64,
    pub category: DiagCategory,
    pub message: String,
    pub repeat_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiagCategory {
    Info,
    IoPort,
    Mmio,
    Interrupt,
    CpuState,
    Error,
}

impl DiagCategory {
    fn label(&self) -> &'static str {
        match self {
            DiagCategory::Info => "INFO",
            DiagCategory::IoPort => "I/O",
            DiagCategory::Mmio => "MMIO",
            DiagCategory::Interrupt => "IRQ",
            DiagCategory::CpuState => "CPU",
            DiagCategory::Error => "ERR",
        }
    }

    fn color(&self) -> egui::Color32 {
        match self {
            DiagCategory::Info => egui::Color32::from_rgb(180, 180, 180),
            DiagCategory::IoPort => egui::Color32::from_rgb(100, 180, 255),
            DiagCategory::Mmio => egui::Color32::from_rgb(100, 220, 180),
            DiagCategory::Interrupt => egui::Color32::from_rgb(255, 200, 100),
            DiagCategory::CpuState => egui::Color32::from_rgb(200, 150, 255),
            DiagCategory::Error => egui::Color32::from_rgb(255, 100, 100),
        }
    }
}

/// Shared log buffer between the VM thread and the UI.
#[derive(Clone)]
pub struct DiagLog {
    inner: Arc<Mutex<DiagLogInner>>,
}

struct DiagLogInner {
    entries: VecDeque<DiagEntry>,
    start_time: std::time::Instant,
    /// Counters for summary
    io_count: u64,
    mmio_count: u64,
    irq_count: u64,
    exit_count: u64,
    /// Debug port (0x402) accumulated text output
    debug_text: String,
    /// WHP debug output (IOAPIC, exits, APIC events)
    whp_text: String,
}

impl DiagLog {
    pub fn new() -> Self {
        DiagLog {
            inner: Arc::new(Mutex::new(DiagLogInner {
                entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
                start_time: std::time::Instant::now(),
                io_count: 0,
                mmio_count: 0,
                irq_count: 0,
                exit_count: 0,
                debug_text: String::new(),
                whp_text: String::new(),
            })),
        }
    }

    pub fn log(&self, category: DiagCategory, message: String) {
        if let Ok(mut inner) = self.inner.lock() {
            let timestamp_ms = inner.start_time.elapsed().as_millis() as u64;

            match category {
                DiagCategory::IoPort => inner.io_count += 1,
                DiagCategory::Mmio => inner.mmio_count += 1,
                DiagCategory::Interrupt => inner.irq_count += 1,
                _ => {}
            }
            inner.exit_count += 1;

            // Deduplicate consecutive identical messages
            if let Some(last) = inner.entries.back_mut() {
                if last.category == category && last.message == message {
                    last.repeat_count += 1;
                    last.timestamp_ms = timestamp_ms;
                    return;
                }
            }

            inner.entries.push_back(DiagEntry { timestamp_ms, category, message, repeat_count: 1 });
            if inner.entries.len() > MAX_LOG_ENTRIES {
                inner.entries.pop_front();
            }
        }
    }

    pub fn entries(&self) -> Vec<DiagEntry> {
        self.inner.lock().map(|i| i.entries.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn counters(&self) -> (u64, u64, u64, u64) {
        self.inner.lock().map(|i| (i.exit_count, i.io_count, i.mmio_count, i.irq_count)).unwrap_or_default()
    }

    pub fn append_debug_text(&self, text: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.debug_text.push_str(text);
        }
    }

    pub fn debug_text(&self) -> String {
        self.inner.lock().map(|i| i.debug_text.clone()).unwrap_or_default()
    }

    pub fn append_whp_text(&self, text: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.whp_text.push_str(text);
            // Cap at 256KB to avoid unbounded growth
            if inner.whp_text.len() > 256 * 1024 {
                let drain = inner.whp_text.len() - 128 * 1024;
                inner.whp_text.drain(..drain);
            }
        }
    }

    pub fn whp_text(&self) -> String {
        self.inner.lock().map(|i| i.whp_text.clone()).unwrap_or_default()
    }

    pub fn clear_whp_text(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.whp_text.clear();
        }
    }

    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.clear();
        }
    }

    /// Export all VM log entries as plain text.
    pub fn export_vm_log(&self) -> String {
        let entries = self.entries();
        let mut out = String::with_capacity(entries.len() * 80);
        for e in &entries {
            let secs = e.timestamp_ms as f64 / 1000.0;
            let repeat = if e.repeat_count > 1 { format!(" x{}", e.repeat_count) } else { String::new() };
            out.push_str(&format!("{:>9.3} {:?} {}{}\n", secs, e.category, e.message, repeat));
        }
        out
    }

    /// Export the BIOS debug port text (port 0x402 output).
    pub fn export_bios_log(&self) -> String {
        self.debug_text()
    }
}

/// The diagnostics window shown alongside the VM display.
pub struct DiagnosticsWindow {
    pub open: bool,
    auto_scroll: bool,
    filter_io: bool,
    filter_mmio: bool,
    filter_irq: bool,
    filter_cpu: bool,
    filter_err: bool,
    filter_info: bool,
    pub vm_name: String,
    active_tab: DiagTab,
}

#[derive(Clone, Copy, PartialEq)]
enum DiagTab {
    Log,
    BiosLog,
    WhpDebug,
}

impl DiagnosticsWindow {
    pub fn new(vm_name: &str) -> Self {
        Self {
            open: true,
            auto_scroll: true,
            filter_io: true,
            filter_mmio: true,
            filter_irq: true,
            filter_cpu: true,
            filter_err: true,
            filter_info: true,
            vm_name: vm_name.to_string(),
            active_tab: DiagTab::Log,
        }
    }

    fn is_visible(&self, cat: &DiagCategory) -> bool {
        match cat {
            DiagCategory::Info => self.filter_info,
            DiagCategory::IoPort => self.filter_io,
            DiagCategory::Mmio => self.filter_mmio,
            DiagCategory::Interrupt => self.filter_irq,
            DiagCategory::CpuState => self.filter_cpu,
            DiagCategory::Error => self.filter_err,
        }
    }

    /// Render diagnostics content directly into a `Ui` (for use in a separate viewport).
    pub fn show_contents(&mut self, ui: &mut egui::Ui, log: &DiagLog) {
        // Tab bar
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.active_tab, DiagTab::Log, "VM Log");
            ui.selectable_value(&mut self.active_tab, DiagTab::BiosLog, "BIOS Log");
            ui.selectable_value(&mut self.active_tab, DiagTab::WhpDebug, "WHP Debug");
        });
        ui.separator();

        match self.active_tab {
            DiagTab::Log => self.show_log_tab(ui, log),
            DiagTab::BiosLog => self.show_bios_log_tab(ui, log),
            DiagTab::WhpDebug => self.show_whp_debug_tab(ui, log),
        }
    }

    /// Legacy: show as egui::Window inside the main viewport (fallback).
    pub fn show(&mut self, ctx: &egui::Context, log: &DiagLog) {
        if !self.open { return; }

        let title = format!("Diagnostics - {}", self.vm_name);
        let mut still_open = self.open;

        egui::Window::new(title)
            .open(&mut still_open)
            .default_size([600.0, 400.0])
            .min_width(400.0)
            .min_height(200.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.show_contents(ui, log);
            });

        self.open = still_open;
    }

    fn show_log_tab(&mut self, ui: &mut egui::Ui, log: &DiagLog) {
        // Summary bar
        let (exits, ios, mmios, irqs) = log.counters();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Exits: {}", exits)).small().color(egui::Color32::GRAY));
            ui.separator();
            ui.label(egui::RichText::new(format!("I/O: {}", ios)).small().color(DiagCategory::IoPort.color()));
            ui.separator();
            ui.label(egui::RichText::new(format!("MMIO: {}", mmios)).small().color(DiagCategory::Mmio.color()));
            ui.separator();
            ui.label(egui::RichText::new(format!("IRQ: {}", irqs)).small().color(DiagCategory::Interrupt.color()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    log.clear();
                }
                ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
            });
        });

        // Filter bar
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.checkbox(&mut self.filter_info, egui::RichText::new("INFO").small().color(DiagCategory::Info.color()));
            ui.checkbox(&mut self.filter_io, egui::RichText::new("I/O").small().color(DiagCategory::IoPort.color()));
            ui.checkbox(&mut self.filter_mmio, egui::RichText::new("MMIO").small().color(DiagCategory::Mmio.color()));
            ui.checkbox(&mut self.filter_irq, egui::RichText::new("IRQ").small().color(DiagCategory::Interrupt.color()));
            ui.checkbox(&mut self.filter_cpu, egui::RichText::new("CPU").small().color(DiagCategory::CpuState.color()));
            ui.checkbox(&mut self.filter_err, egui::RichText::new("ERR").small().color(DiagCategory::Error.color()));
        });

        ui.separator();

        // Log entries
        let entries = log.entries();
        let row_height = 16.0;
        let filtered: Vec<&DiagEntry> = entries.iter().filter(|e| self.is_visible(&e.category)).collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(self.auto_scroll)
            .show_rows(ui, row_height, filtered.len(), |ui, range| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                for entry in &filtered[range] {
                    ui.horizontal(|ui| {
                        let ts = format!("{:>8.3}", entry.timestamp_ms as f64 / 1000.0);
                        ui.label(egui::RichText::new(ts).small().color(egui::Color32::from_rgb(120, 120, 120)));
                        ui.label(egui::RichText::new(format!("{:<4}", entry.category.label())).small().color(entry.category.color()));
                        ui.label(egui::RichText::new(&entry.message).small());
                        if entry.repeat_count > 1 {
                            ui.label(egui::RichText::new(format!(" x{}", entry.repeat_count)).small().color(egui::Color32::from_rgb(255, 200, 100)));
                        }
                    });
                }
            });
    }

    fn show_whp_debug_tab(&self, ui: &mut egui::Ui, log: &DiagLog) {
        ui.horizontal(|ui| {
            if ui.small_button("Clear").clicked() {
                log.clear_whp_text();
            }
        });
        ui.separator();
        let text = log.whp_text();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                ui.label(egui::RichText::new(&text).color(egui::Color32::from_rgb(220, 200, 150)));
            });
    }

    fn show_bios_log_tab(&self, ui: &mut egui::Ui, log: &DiagLog) {
        let text = log.debug_text();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                ui.label(egui::RichText::new(&text).color(egui::Color32::from_rgb(200, 220, 255)));
            });
    }
}
