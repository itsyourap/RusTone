//! Logs: pjsua's log and RusTone's own, filterable, newest at the bottom.

use egui::Ui;
use sip_engine::debug::{LogLine, LogSource};

use super::{clock, header, mono};
use crate::app::App;
use crate::theme::Palette;

#[derive(Debug)]
pub struct LogsState {
    /// Show lines at this level or more severe (1 error .. 5 trace).
    max_level: u8,
    pjsua: bool,
    rustone: bool,
    filter: String,
    follow: bool,
    seen: usize,
}

impl Default for LogsState {
    fn default() -> Self {
        Self { max_level: 4, pjsua: true, rustone: true, filter: String::new(), follow: true, seen: 0 }
    }
}

fn level_name(level: u8) -> &'static str {
    match level {
        ..=1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        4 => "DEBUG",
        _ => "TRACE",
    }
}

fn level_color(p: &Palette, level: u8) -> egui::Color32 {
    match level {
        ..=1 => p.danger,
        2 => p.warning,
        3 => p.text,
        _ => p.dim,
    }
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    let feed = app.sip.debug_feed();
    let s = &mut app.debug.logs;
    let all = feed.logs();
    let needle = s.filter.to_lowercase();
    let lines: Vec<&LogLine> = all
        .iter()
        .map(AsRef::as_ref)
        .filter(|l| l.level <= s.max_level)
        .filter(|l| match l.source {
            LogSource::Pjsua => s.pjsua,
            LogSource::App => s.rustone,
        })
        .filter(|l| needle.is_empty() || l.text.to_lowercase().contains(&needle))
        .collect();

    header(ui, p, "Logs", |ui| {
        if ui.button("Clear").clicked() {
            feed.clear_logs();
        }
        if ui.button("Copy").on_hover_text("Copy the visible lines").clicked() {
            let text: String = lines.iter().map(|l| format_line(l) + "\n").collect();
            ui.ctx().copy_text(text);
        }
        ui.checkbox(&mut s.follow, "Follow");
    });
    ui.horizontal(|ui| {
        ui.label("Level");
        egui::ComboBox::from_id_salt("log-level").selected_text(level_name(s.max_level)).show_ui(ui, |ui| {
            for level in 1..=5 {
                ui.selectable_value(&mut s.max_level, level, format!("{} and above", level_name(level)));
            }
        });
        ui.checkbox(&mut s.pjsua, "pjsua");
        ui.checkbox(&mut s.rustone, "RusTone");
        ui.add(egui::TextEdit::singleline(&mut s.filter).hint_text("Filter…").desired_width(260.0));
        ui.label(egui::RichText::new(format!("{} of {} lines", lines.len(), all.len())).size(12.0).color(p.dim));
    });
    ui.add_space(4.0);

    egui::Frame::new().fill(p.panel).corner_radius(egui::CornerRadius::same(8)).inner_margin(8).show(ui, |ui| {
        let row_height = 18.0;
        let mut area = egui::ScrollArea::both().auto_shrink(false).stick_to_bottom(s.follow);
        if s.follow && lines.len() != s.seen {
            area = area.vertical_scroll_offset(f32::MAX);
        }
        s.seen = lines.len();
        area.show_rows(ui, row_height, lines.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in &lines[range] {
                let mut job = egui::text::LayoutJob::default();
                let mut part = |text: &str, color: egui::Color32| {
                    job.append(text, 0.0, egui::TextFormat { font_id: mono(12.0), color, ..Default::default() });
                };
                let source = match line.source {
                    LogSource::Pjsua => "pjsua",
                    LogSource::App => "rustone",
                };
                part(&format!("{}  ", clock(line.time)), p.dim);
                part(&format!("{:<5}  ", level_name(line.level)), level_color(p, line.level));
                part(&format!("{source:<5}  "), p.secondary);
                part(&line.text, level_color(p, line.level));
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), row_height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(job).extend());
                });
            }
        });
    });
}

fn format_line(l: &LogLine) -> String {
    let source = match l.source {
        LogSource::Pjsua => "pjsua",
        LogSource::App => "rustone",
    };
    format!("{} {:<5} {source:<5} {}", clock(l.time), level_name(l.level), l.text)
}
