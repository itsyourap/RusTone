//! SIP messages, like a browser's network tab: numbered cards on the left,
//! the selected message's details on the right, or the whole raw log.

use std::sync::Arc;

use egui::{Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use sip_engine::debug::{MsgDirection, SipMessage};

use super::{clock, header, kv, mono};
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Messages,
    RawLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Detail {
    #[default]
    Overview,
    Headers,
    Body,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Direction {
    #[default]
    All,
    Sent,
    Received,
}

#[derive(Debug)]
pub struct SipState {
    view: View,
    detail: Detail,
    direction: Direction,
    filter: String,
    selected: Option<u64>,
    follow: bool,
    /// Message count at the last frame, to jump to new ones when following.
    seen: usize,
}

impl Default for SipState {
    fn default() -> Self {
        Self { view: View::Messages, detail: Detail::Overview, direction: Direction::All, filter: String::new(), selected: None, follow: true, seen: 0 }
    }
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    let feed = app.sip.debug_feed();
    let all = feed.messages();
    let s = &mut app.debug.sip;
    let needle = s.filter.to_lowercase();
    let messages: Vec<Arc<SipMessage>> = all
        .iter()
        .filter(|m| match s.direction {
            Direction::All => true,
            Direction::Sent => m.direction == MsgDirection::Sent,
            Direction::Received => m.direction == MsgDirection::Received,
        })
        .filter(|m| needle.is_empty() || m.raw.to_lowercase().contains(&needle))
        .cloned()
        .collect();

    header(ui, p, "SIP messages", |ui| {
        if ui.button("Clear").clicked() {
            feed.clear_messages();
            s.selected = None;
        }
        if ui.button("Copy all").on_hover_text("Copy the visible messages as a raw log").clicked() {
            ui.ctx().copy_text(raw_log(&messages));
        }
        ui.checkbox(&mut s.follow, "Follow");
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut s.view, View::Messages, "Messages");
        ui.selectable_value(&mut s.view, View::RawLog, "Raw log");
        ui.separator();
        ui.selectable_value(&mut s.direction, Direction::All, "All");
        ui.selectable_value(&mut s.direction, Direction::Sent, "Sent");
        ui.selectable_value(&mut s.direction, Direction::Received, "Received");
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut s.filter).hint_text("Filter (method, number, Call-ID, header…)").desired_width(300.0));
        ui.label(egui::RichText::new(format!("{} of {}", messages.len(), all.len())).size(12.0).color(p.dim));
    });
    ui.add_space(6.0);

    match s.view {
        View::RawLog => {
            let mut text = raw_log(&messages);
            egui::Frame::new().fill(p.panel).corner_radius(egui::CornerRadius::same(8)).inner_margin(8).show(ui, |ui| {
                egui::ScrollArea::both().auto_shrink(false).stick_to_bottom(s.follow).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).font(mono(12.0)).code_editor().desired_width(f32::INFINITY).interactive(true));
                });
            });
        }
        View::Messages => {
            let selected = s.selected.and_then(|seq| messages.iter().find(|m| m.seq == seq).cloned());
            if selected.is_some() {
                egui::Panel::right("sip-detail")
                    .resizable(true)
                    .default_size(ui.available_width() * 0.5)
                    .min_size(320.0)
                    .frame(egui::Frame::new().inner_margin(egui::Margin { left: 12, ..egui::Margin::ZERO }))
                    .show(ui, |ui| details(ui, p, &mut app.debug.sip, selected.as_deref().expect("checked")));
            }
            let s = &mut app.debug.sip;
            list(ui, p, s, &messages);
        }
    }
}

fn list(ui: &mut Ui, p: &Palette, s: &mut SipState, messages: &[Arc<SipMessage>]) {
    let row_height = 46.0;
    let mut area = egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(s.follow);
    if s.follow && messages.len() != s.seen {
        area = area.vertical_scroll_offset(f32::MAX);
    }
    s.seen = messages.len();
    area.show_rows(ui, row_height, messages.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for m in &messages[range] {
            let selected = s.selected == Some(m.seq);
            if card(ui, p, m, selected, row_height - 4.0).clicked() {
                s.selected = if selected { None } else { Some(m.seq) };
            }
        }
    });
}

fn status_color(p: &Palette, m: &SipMessage) -> egui::Color32 {
    match m.status() {
        Some(100..=199) => p.dim,
        Some(200..=299) => p.success,
        Some(300..=399) => p.warning,
        Some(_) => p.danger,
        None => p.text,
    }
}

fn card(ui: &mut Ui, p: &Palette, m: &SipMessage, selected: bool, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let label = m.label();
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("#{} {label}", m.seq)));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.1);
    let fill = if selected { p.accent.gamma_multiply(0.16) } else { p.panel };
    let painter = ui.painter();
    painter.rect(rect, egui::CornerRadius::same(8), fill, egui::Stroke::new(1.0, if selected { p.accent } else { p.outline.gamma_multiply(0.5 + 0.5 * hover) }), egui::StrokeKind::Inside);

    painter.text(rect.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, format!("{:>4}", m.seq), mono(12.0), p.dim);
    let (icon, tint, dir) = match m.direction {
        MsgDirection::Sent => (Icon::Outgoing, p.accent, "to"),
        MsgDirection::Received => (Icon::Incoming, p.success, "from"),
    };
    icon.image(tint, 16.0).paint_at(ui, egui::Rect::from_center_size(rect.left_center() + vec2(58.0, 0.0), Vec2::splat(16.0)));
    let x = rect.left() + 76.0;
    painter.text(egui::pos2(x, rect.top() + 14.0), egui::Align2::LEFT_CENTER, &label, Weight::SemiBold.font_id(13.5), status_color(p, m));
    let sub = format!("{dir} {} {}", m.transport, m.remote);
    painter.text(egui::pos2(x, rect.top() + 31.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(11.5), p.dim);
    painter.text(rect.right_center() - vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, clock(m.time), mono(11.5), p.dim);
    response
}

fn details(ui: &mut Ui, p: &Palette, s: &mut SipState, m: &SipMessage) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("#{}  {}", m.seq, m.label())).font(Weight::SemiBold.font_id(15.0)).color(status_color(p, m)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Close").clicked() {
                s.selected = None;
            }
            if ui.button("Copy").on_hover_text("Copy the raw message").clicked() {
                ui.ctx().copy_text(m.raw.clone());
            }
        });
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut s.detail, Detail::Overview, "Overview");
        ui.selectable_value(&mut s.detail, Detail::Headers, "Headers");
        ui.selectable_value(&mut s.detail, Detail::Body, "Body");
        ui.selectable_value(&mut s.detail, Detail::Raw, "Raw");
    });
    ui.separator();
    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| match s.detail {
        Detail::Overview => {
            egui::Grid::new("sip-overview").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                kv(ui, p, "Direction", match m.direction {
                    MsgDirection::Sent => "Sent",
                    MsgDirection::Received => "Received",
                });
                kv(ui, p, "Time", clock(m.time));
                kv(ui, p, "Transport", &m.transport);
                kv(ui, p, "Remote", &m.remote);
                kv(ui, p, "Start line", m.start_line());
                for name in ["From", "To", "Call-ID", "CSeq", "Contact", "Via", "User-Agent", "Content-Type", "Expires", "Reason", "P-Asserted-Identity"] {
                    if let Some(value) = m.header(name) {
                        kv(ui, p, name, value);
                    }
                }
                kv(ui, p, "Size", format!("{} bytes", m.raw.len()));
            });
        }
        Detail::Headers => {
            egui::Grid::new("sip-headers").num_columns(2).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                for (name, value) in m.headers() {
                    ui.label(egui::RichText::new(name).font(mono(12.0)).color(p.accent));
                    ui.add(egui::Label::new(egui::RichText::new(value).font(mono(12.0)).color(p.text)).wrap());
                    ui.end_row();
                }
            });
        }
        Detail::Body => {
            let body = m.body();
            if body.trim().is_empty() {
                ui.label(egui::RichText::new("No body").color(p.dim));
            } else {
                let mut text = body.to_owned();
                ui.add(egui::TextEdit::multiline(&mut text).font(mono(12.0)).code_editor().desired_width(f32::INFINITY));
            }
        }
        Detail::Raw => {
            let mut text = m.raw.clone();
            ui.add(egui::TextEdit::multiline(&mut text).font(mono(12.0)).code_editor().desired_width(f32::INFINITY));
        }
    });
}

/// All messages as text, each introduced by a separator line (like pjsua's log).
fn raw_log(messages: &[Arc<SipMessage>]) -> String {
    let mut out = String::new();
    for m in messages {
        let dir = match m.direction {
            MsgDirection::Sent => "Sent to",
            MsgDirection::Received => "Received from",
        };
        out.push_str(&format!("──── #{} {} {} {} at {} ────\n", m.seq, dir, m.transport, m.remote, clock(m.time)));
        out.push_str(&m.raw.replace("\r\n", "\n"));
        if !m.raw.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}
