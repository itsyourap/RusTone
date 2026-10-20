//! The Calls page: call history with filters.

use egui::{Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use rustone_core::contacts::Contact;
use rustone_core::history::{CallKind, CallRecord, format_duration};

use super::widgets;
use super::{Page, contacts};
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Missed,
    Incoming,
    Outgoing,
}

impl Filter {
    fn keeps(self, kind: CallKind) -> bool {
        match self {
            Self::All => true,
            Self::Missed => matches!(kind, CallKind::Missed | CallKind::Elsewhere),
            Self::Incoming => kind == CallKind::Incoming,
            Self::Outgoing => kind == CallKind::Outgoing,
        }
    }
}

#[derive(Debug, Default)]
pub struct HistoryView {
    pub query: String,
    pub filter: Filter,
    pub expanded: Option<i64>,
    pub confirm_clear: bool,
}

enum RowAction {
    Toggle,
    Call(String),
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    if app.phonebook.dialog_open() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some(message) = app.phonebook.poll_dialog() {
        app.notify(message);
    }

    ui.horizontal(|ui| {
        ui.add_sized(
            vec2((ui.available_width() - 38.0).max(80.0), 34.0),
            egui::TextEdit::singleline(&mut app.ui.history.query).hint_text("Search calls").margin(egui::Margin::symmetric(10, 8)),
        );
        let more = widgets::icon_button(ui, p, Icon::More, p.text, "More");
        egui::Popup::menu(&more).show(|ui| {
            if ui.button("Export history (.csv)…").clicked() {
                app.phonebook.pick_export_history();
                ui.close();
            }
            let text = if app.ui.history.confirm_clear { "Click again to clear the history" } else { "Clear history" };
            if ui.add(egui::Button::new(egui::RichText::new(text).color(p.danger))).clicked() {
                if app.ui.history.confirm_clear {
                    if let Err(e) = app.phonebook.clear_calls() {
                        app.notify(format!("Couldn't clear the history: {e}"));
                    }
                    app.ui.history.confirm_clear = false;
                    ui.close();
                } else {
                    app.ui.history.confirm_clear = true;
                }
            }
        });
    });
    ui.add_space(6.0);
    widgets::segmented(
        ui,
        p,
        &mut app.ui.history.filter,
        &[(Filter::All, None, "All"), (Filter::Missed, None, "Missed"), (Filter::Incoming, None, "In"), (Filter::Outgoing, None, "Out")],
    );
    ui.add_space(6.0);

    if app.phonebook.calls.is_empty() {
        widgets::empty_state(ui, p, Icon::History, "No calls yet", "Calls you make and receive appear here.");
        return;
    }
    let (query, filter) = (app.ui.history.query.trim().to_lowercase(), app.ui.history.filter);
    let rows: Vec<(CallRecord, Option<Contact>)> = app
        .phonebook
        .calls
        .iter()
        .filter(|r| filter.keeps(r.kind))
        .map(|r| (r.clone(), app.phonebook.lookup(&r.number).cloned()))
        .filter(|(r, c)| {
            let title = title(r, c.as_ref()).to_lowercase();
            query.split_whitespace().all(|w| title.contains(w) || r.number.to_lowercase().contains(w))
        })
        .collect();
    if rows.is_empty() {
        ui.label(egui::RichText::new("No calls match").color(p.dim));
        return;
    }

    let now = jiff::Zoned::now();
    let mut action: Option<(i64, RowAction)> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (record, contact) in &rows {
            let expanded = app.ui.history.expanded == Some(record.id);
            if let Some(a) = call_row(ui, p, record, contact.as_ref(), expanded, &now) {
                action = Some((record.id, a));
            }
            if expanded && let Some(a) = row_actions(ui, app, p, record, contact.as_ref()) {
                action = Some((record.id, a));
            }
        }
    });
    match action {
        Some((id, RowAction::Toggle)) => app.ui.history.expanded = if app.ui.history.expanded == Some(id) { None } else { Some(id) },
        Some((_, RowAction::Call(number))) => {
            super::dial(app, &number);
            app.ui.page = Page::Dialer;
        }
        None => {}
    }
}

/// The contact's name, else the SIP display name, else the number.
fn title(r: &CallRecord, contact: Option<&Contact>) -> String {
    match contact {
        Some(c) => c.title(),
        None if !r.name.trim().is_empty() => r.name.trim().to_owned(),
        None => r.number.clone(),
    }
}

/// "14:05" today, "Yesterday 14:05", "12 Oct, 14:05" this year, else "12 Oct 2025".
fn when(time: i64, now: &jiff::Zoned) -> String {
    let Ok(ts) = jiff::Timestamp::from_second(time) else { return String::new() };
    let t = ts.to_zoned(now.time_zone().clone());
    let days = (now.date() - t.date()).get_days();
    match days {
        0 => t.strftime("%H:%M").to_string(),
        1 => t.strftime("Yesterday %H:%M").to_string(),
        _ if t.year() == now.year() => t.strftime("%-d %b, %H:%M").to_string(),
        _ => t.strftime("%-d %b %Y").to_string(),
    }
}

fn call_row(ui: &mut Ui, p: &Palette, r: &CallRecord, contact: Option<&Contact>, expanded: bool, now: &jiff::Zoned) -> Option<RowAction> {
    let title = title(r, contact);
    let (icon, tint, kind) = match r.kind {
        CallKind::Outgoing => (Icon::Outgoing, p.secondary, "Outgoing"),
        CallKind::Incoming => (Icon::Incoming, p.success, "Incoming"),
        CallKind::Missed => (Icon::PhoneMissed, p.danger, "Missed"),
        CallKind::Elsewhere => (Icon::PhoneMissed, p.warning, "Answered elsewhere"),
    };
    let mut detail = vec![when(r.time, now)];
    if title != r.number {
        detail.insert(0, r.number.clone());
    }
    let duration = format_duration(r.duration);
    if !duration.is_empty() {
        detail.push(duration);
    }
    let detail = detail.join(" · ");

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{kind} call, {title}, {detail}")));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered() || expanded, 0.12);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(10), p.surface_hover.gamma_multiply(hover));
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(20.0));
    icon.image(tint, 20.0).paint_at(ui, icon_rect);
    let text_x = rect.left() + 44.0;
    let width = rect.right() - 8.0 - text_x;
    let name_color = if r.kind == CallKind::Missed { p.danger } else { p.text };
    let name = widgets::elided(ui, title, Weight::Medium.font_id(14.0), name_color, width);
    ui.painter().galley(egui::pos2(text_x, rect.top() + 9.0), name, name_color);
    let sub = widgets::elided(ui, detail, Weight::Regular.font_id(12.0), p.secondary, width);
    ui.painter().galley(egui::pos2(text_x, rect.top() + 29.0), sub, p.secondary);
    let response = response.on_hover_text(&r.info);
    if response.double_clicked() {
        Some(RowAction::Call(r.number.clone()))
    } else if response.clicked() {
        Some(RowAction::Toggle)
    } else {
        None
    }
}

fn row_actions(ui: &mut Ui, app: &mut App, p: &Palette, r: &CallRecord, contact: Option<&Contact>) -> Option<RowAction> {
    let mut action = None;
    egui::Frame::new().inner_margin(egui::Margin { left: 44, right: 0, top: 2, bottom: 0 }).show(ui, |ui| {
        if !r.info.is_empty() {
            ui.label(egui::RichText::new(&r.info).size(12.0).color(p.dim));
        }
        ui.horizontal_wrapped(|ui| {
            if ui.add(egui::Button::new(egui::RichText::new("Call back").color(p.on_accent)).fill(p.success)).clicked() {
                action = Some(RowAction::Call(r.number.clone()));
            }
            match contact {
                Some(c) => {
                    if ui.button("Edit contact").clicked() {
                        app.ui.contact_draft = Some(contacts::Draft::new(c.clone()));
                        app.ui.page = Page::Contact;
                    }
                }
                None => {
                    if ui.button("Add to contacts").clicked() {
                        let new = Contact { name: r.name.trim().to_owned(), number: r.number.clone(), ..Contact::default() };
                        app.ui.contact_draft = Some(contacts::Draft::new(new));
                        app.ui.page = Page::Contact;
                    }
                }
            }
            if ui.button("Copy number").clicked() {
                ui.ctx().copy_text(r.number.clone());
                app.notify(format!("Copied {}", r.number));
            }
            if ui.add(egui::Button::new(egui::RichText::new("Delete").color(p.danger))).clicked() {
                match app.phonebook.delete_call(r.id) {
                    Ok(()) => app.ui.history.expanded = None,
                    Err(e) => app.notify(format!("Couldn't delete the call: {e}")),
                }
            }
        });
    });
    ui.add_space(6.0);
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_naturally() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(5));
        let now = jiff::civil::date(2026, 10, 20).at(15, 0, 0, 0).to_zoned(tz.clone()).unwrap();
        let at = |d: jiff::civil::Date, h, m| d.at(h, m, 0, 0).to_zoned(tz.clone()).unwrap().timestamp().as_second();
        assert_eq!(when(at(jiff::civil::date(2026, 10, 20), 14, 5), &now), "14:05");
        assert_eq!(when(at(jiff::civil::date(2026, 10, 19), 9, 30), &now), "Yesterday 09:30");
        assert_eq!(when(at(jiff::civil::date(2026, 3, 2), 9, 30), &now), "2 Mar, 09:30");
        assert_eq!(when(at(jiff::civil::date(2025, 12, 31), 9, 30), &now), "31 Dec 2025");
    }

    #[test]
    fn filters_keep_their_kinds() {
        assert!(Filter::Missed.keeps(CallKind::Elsewhere) && !Filter::Missed.keeps(CallKind::Incoming));
        assert!(Filter::All.keeps(CallKind::Outgoing) && Filter::Outgoing.keeps(CallKind::Outgoing));
    }
}
