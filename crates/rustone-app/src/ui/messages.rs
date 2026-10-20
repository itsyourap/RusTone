//! The Messages page: conversations, and one conversation with its calls.

use std::collections::HashMap;
use std::time::Instant;

use egui::{Align, Layout, Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use rustone_core::history::{CallKind, CallRecord, format_duration};
use rustone_core::messages::{MessageRecord, MessageState};

use super::{Page, widgets};
use crate::app::App;
use crate::icons::Icon;
use crate::phonebook::conversation_key;
use crate::theme::Palette;

#[derive(Debug, Default)]
pub struct MessagesView {
    /// The conversation on screen (its key), or the list.
    pub open: Option<String>,
    /// Unsent text per conversation.
    drafts: HashMap<String, String>,
    /// The "To" box of a new conversation.
    new_to: Option<String>,
    /// Delete was chosen once for this conversation.
    confirm_delete: Option<String>,
    /// The open conversation's messages, with the revision they were read at.
    cache: Option<(String, u64, Vec<MessageRecord>)>,
    /// When a conversation was last on screen (no notification for it then).
    pub seen: Option<(String, Instant)>,
}

impl MessagesView {
    /// Shows the conversation with `number`.
    pub fn open(&mut self, number: &str) {
        self.open = Some(conversation_key(number));
        self.new_to = None;
    }
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    if app.settings.calls.disable_messaging {
        widgets::empty_state(ui, p, Icon::Message, "Messages are off", "Turn text messages on in Settings › Calls.");
        return;
    }
    match app.ui.messages.open.clone() {
        Some(peer) => conversation(ui, app, p, &peer),
        None => list(ui, app, p),
    }
}

/// The name for a conversation: a contact's, else the sender's display name, else the number.
fn title(app: &App, peer: &str, name: &str) -> String {
    match app.phonebook.lookup(peer) {
        Some(contact) => contact.title(),
        None if !name.trim().is_empty() => name.trim().to_owned(),
        None => peer.to_owned(),
    }
}

fn list(ui: &mut Ui, app: &mut App, p: &Palette) {
    ui.horizontal(|ui| {
        ui.heading("Messages");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, p, Icon::Plus, p.text, "New message").clicked() {
                app.ui.messages.new_to = Some(String::new());
            }
        });
    });
    if let Some(mut to) = app.ui.messages.new_to.take() {
        let mut keep = true;
        ui.horizontal(|ui| {
            let field = ui.add_sized(
                vec2((ui.available_width() - 80.0).max(80.0), 34.0),
                egui::TextEdit::singleline(&mut to).hint_text("Number or SIP address").margin(egui::Margin::symmetric(10, 8)),
            );
            field.request_focus();
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled(!to.trim().is_empty(), egui::Button::new("Start")).clicked() || enter) && !to.trim().is_empty() {
                app.ui.messages.open(to.trim());
                keep = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                keep = false;
            }
        });
        if keep && app.ui.messages.open.is_none() {
            app.ui.messages.new_to = Some(to);
        }
    }
    ui.add_space(6.0);

    let conversations = app.phonebook.conversations.clone();
    if conversations.is_empty() {
        widgets::empty_state(ui, p, Icon::Message, "No messages", "Start one with + or from a contact.");
        return;
    }
    let now = jiff::Zoned::now();
    let mut open = None;
    let mut delete = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for c in &conversations {
            let title = title(app, &c.peer, &c.last.name);
            let preview = format!("{}{}", if c.last.outgoing { "You: " } else { "" }, c.last.text.lines().next().unwrap_or_default());
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
            let unread = if c.unread > 0 { format!(", {} unread", c.unread) } else { String::new() };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{title}{unread}: {preview}")));
            let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
            if hover > 0.0 {
                ui.painter().rect_filled(rect, egui::CornerRadius::same(10), p.surface_hover.gamma_multiply(hover));
            }
            widgets::avatar_at(ui, p, egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(38.0)), &title);
            let x = rect.left() + 56.0;
            let time = super::history::when(c.last.time, &now);
            let time_galley = ui.painter().layout_no_wrap(time, Weight::Regular.font_id(11.5), p.dim);
            let width = rect.right() - 12.0 - time_galley.size().x - 8.0 - x;
            let bold = c.unread > 0;
            let name = widgets::elided(ui, title.clone(), if bold { Weight::SemiBold.font_id(14.0) } else { Weight::Medium.font_id(14.0) }, p.text, width);
            ui.painter().galley(egui::pos2(x, rect.top() + 9.0), name, p.text);
            ui.painter().galley(egui::pos2(rect.right() - 12.0 - time_galley.size().x, rect.top() + 11.0), time_galley, p.dim);
            let preview_color = if bold { p.text } else { p.secondary };
            let preview = widgets::elided(ui, preview, Weight::Regular.font_id(12.5), preview_color, rect.right() - 44.0 - x);
            ui.painter().galley(egui::pos2(x, rect.top() + 31.0), preview, preview_color);
            if c.unread > 0 {
                let text = if c.unread > 99 { "99+".to_owned() } else { c.unread.to_string() };
                let g = ui.painter().layout_no_wrap(text, Weight::SemiBold.font_id(10.5), p.on_accent);
                let badge = egui::Rect::from_center_size(egui::pos2(rect.right() - 22.0, rect.top() + 38.0), vec2((g.size().x + 10.0).max(18.0), 18.0));
                ui.painter().rect_filled(badge, egui::CornerRadius::same(9), p.accent);
                ui.painter().galley(badge.center() - g.size() / 2.0, g, p.on_accent);
            }
            response.context_menu(|ui| {
                widgets::menu(ui, |ui| {
                    let confirming = app.ui.messages.confirm_delete.as_deref() == Some(c.peer.as_str());
                    let text = if confirming { "Click again to delete" } else { "Delete conversation" };
                    if ui.add(egui::Button::new(egui::RichText::new(text).color(p.danger))).clicked() {
                        if confirming {
                            delete = Some(c.peer.clone());
                            ui.close();
                        } else {
                            app.ui.messages.confirm_delete = Some(c.peer.clone());
                        }
                    }
                });
            });
            if response.clicked() {
                open = Some(c.peer.clone());
            }
        }
    });
    if let Some(peer) = delete {
        if let Err(e) = app.phonebook.delete_conversation(&peer) {
            app.notify(format!("Couldn't delete the conversation: {e}"));
        }
        app.ui.messages.confirm_delete = None;
    }
    if let Some(peer) = open {
        app.ui.messages.open = Some(peer);
    }
}

/// A message or a call, in time order.
enum Item {
    Message(MessageRecord),
    Call(CallRecord),
}

impl Item {
    fn time(&self) -> i64 {
        match self {
            Self::Message(m) => m.time,
            Self::Call(c) => c.time,
        }
    }
}

fn conversation(ui: &mut Ui, app: &mut App, p: &Palette, peer: &str) {
    // Read the conversation again only when messages changed.
    let revision = app.phonebook.revision;
    let fresh = app.ui.messages.cache.as_ref().is_some_and(|(cached, rev, _)| cached == peer && *rev == revision);
    if !fresh {
        app.ui.messages.cache = Some((peer.to_owned(), revision, app.phonebook.messages_with(peer)));
    }
    app.phonebook.mark_read(peer);
    app.ui.messages.seen = Some((peer.to_owned(), Instant::now()));
    app.conversation_seen(peer);
    let messages = app.ui.messages.cache.as_ref().map(|(_, _, m)| m.clone()).unwrap_or_default();
    let name = messages.iter().rev().find(|m| !m.name.is_empty()).map(|m| m.name.clone()).unwrap_or_default();
    let title = title(app, peer, &name);

    // Header: back, who, call.
    let mut call = false;
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, p, Icon::Back, p.text, "Back to messages").clicked() {
            app.ui.messages.open = None;
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        widgets::avatar_at(ui, p, rect, &title);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(egui::Label::new(egui::RichText::new(&title).font(Weight::SemiBold.font_id(15.0)).color(p.text)).truncate());
            if title != peer {
                ui.label(egui::RichText::new(peer).size(12.0).color(p.secondary));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            call = widgets::icon_button(ui, p, Icon::Phone, p.success, &format!("Call {title}")).clicked();
        });
    });
    if call {
        super::dial(app, peer);
        app.ui.page = Page::Dialer;
        return;
    }
    if app.ui.messages.open.is_none() {
        return;
    }
    ui.separator();

    // The peer's calls, from the history, among the messages.
    let mut items: Vec<Item> = messages.into_iter().map(Item::Message).collect();
    items.extend(app.phonebook.calls.iter().filter(|c| conversation_key(&c.number) == peer).cloned().map(Item::Call));
    items.sort_by_key(Item::time);

    let mut draft = app.ui.messages.drafts.remove(peer).unwrap_or_default();
    let mut send = None;
    let mut retry = None;
    egui::Panel::bottom("compose").frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 2 })).show(ui, |ui| {
        ui.horizontal(|ui| {
            let id = ui.id().with("compose-text");
            // Enter sends; Shift+Enter starts a new line.
            let enter = ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let field = ui.add_sized(
                vec2((ui.available_width() - 42.0).max(80.0), 36.0),
                egui::TextEdit::multiline(&mut draft).id(id).desired_rows(1).hint_text("Message").margin(egui::Margin::symmetric(10, 8)),
            );
            if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                field.request_focus();
            }
            // Screen readers and automation can name and fill the box.
            ui.ctx().accesskit_node_builder(field.id, |node| {
                node.set_label("Message");
                node.add_action(egui::accesskit::Action::SetValue);
            });
            let set = ui.input(|i| {
                i.accesskit_action_requests(field.id, egui::accesskit::Action::SetValue)
                    .filter_map(|r| match &r.data {
                        Some(egui::accesskit::ActionData::Value(v)) => Some(v.to_string()),
                        _ => None,
                    })
                    .last()
            });
            if let Some(value) = set {
                draft = value;
            }
            let ready = !draft.trim().is_empty();
            let tint = if ready { p.accent } else { p.dim };
            let clicked = ui.add_enabled_ui(ready, |ui| widgets::icon_button(ui, p, Icon::Send, tint, "Send")).inner.clicked();
            if (clicked || enter) && ready {
                send = Some(draft.trim().to_owned());
            }
        });
    });

    egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
            if items.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.label(egui::RichText::new("No messages yet. Say hello!").color(p.dim)));
            }
            let now = jiff::Zoned::now();
            let mut last_day = None;
            for item in &items {
                let day = day_label(item.time(), &now);
                if last_day.as_ref() != Some(&day) {
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(&day).size(11.5).color(p.dim)));
                    last_day = Some(day);
                }
                match item {
                    Item::Message(m) => {
                        if let Some(r) = bubble(ui, p, m) {
                            retry = Some(r);
                        }
                    }
                    Item::Call(c) => call_line(ui, p, c),
                }
            }
            ui.add_space(4.0);
        });
    });

    if let Some(text) = send {
        app.send_message(peer, &text);
        draft.clear();
    }
    if let Some(m) = retry {
        app.retry_message(m.id, peer, &m.text);
    }
    if !draft.is_empty() {
        app.ui.messages.drafts.insert(peer.to_owned(), draft);
    }
}

/// "Today", "Yesterday", or the date.
fn day_label(time: i64, now: &jiff::Zoned) -> String {
    let Ok(ts) = jiff::Timestamp::from_second(time) else { return String::new() };
    let t = ts.to_zoned(now.time_zone().clone());
    match (now.date() - t.date()).get_days() {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ if t.year() == now.year() => t.strftime("%-d %B").to_string(),
        _ => t.strftime("%-d %B %Y").to_string(),
    }
}

fn clock(time: i64) -> String {
    jiff::Timestamp::from_second(time).map(|t| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string()).unwrap_or_default()
}

/// One message as a bubble: ours on the right, theirs on the left. Returns
/// the message when Retry was clicked.
fn bubble(ui: &mut Ui, p: &Palette, m: &MessageRecord) -> Option<MessageRecord> {
    let mut retry = None;
    let max = ui.available_width() * 0.78;
    let (fill, text_color, align) = if m.outgoing { (p.accent, p.on_accent, Align::Max) } else { (p.surface, p.text, Align::Min) };
    ui.with_layout(Layout::top_down(align), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        egui::Frame::new().fill(fill).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(max);
            ui.add(egui::Label::new(egui::RichText::new(&m.text).size(14.0).color(text_color)).wrap());
        });
        let status = match m.state {
            MessageState::Sending => format!("{} · Sending…", clock(m.time)),
            MessageState::Failed => format!("{} · Not delivered{}", clock(m.time), if m.reason.is_empty() { String::new() } else { format!(": {}", m.reason) }),
            _ => clock(m.time),
        };
        let color = if m.state == MessageState::Failed { p.danger } else { p.dim };
        ui.horizontal(|ui| {
            if m.outgoing {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if m.state == MessageState::Failed && ui.add(egui::Button::new(egui::RichText::new("Retry").size(12.0)).small()).clicked() {
                        retry = Some(m.clone());
                    }
                    ui.label(egui::RichText::new(&status).size(11.0).color(color));
                });
            } else {
                ui.label(egui::RichText::new(&status).size(11.0).color(color));
            }
        });
        ui.add_space(4.0);
    });
    retry
}

/// A call with the peer, as a quiet line between the messages.
fn call_line(ui: &mut Ui, p: &Palette, c: &CallRecord) {
    let (what, color) = match c.kind {
        CallKind::Outgoing => ("Outgoing call", p.secondary),
        CallKind::Incoming => ("Incoming call", p.secondary),
        CallKind::Missed => ("Missed call", p.danger),
        CallKind::Elsewhere => ("Answered elsewhere", p.secondary),
    };
    let duration = format_duration(c.duration);
    let text = if duration.is_empty() { format!("{what} · {}", clock(c.time)) } else { format!("{what} · {duration} · {}", clock(c.time)) };
    ui.vertical_centered(|ui| ui.label(egui::RichText::new(text).size(12.0).color(color)));
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_read_naturally() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(5));
        let now = jiff::civil::date(2026, 10, 20).at(15, 0, 0, 0).to_zoned(tz.clone()).unwrap();
        let at = |d: jiff::civil::Date| d.at(9, 0, 0, 0).to_zoned(tz.clone()).unwrap().timestamp().as_second();
        assert_eq!(day_label(at(jiff::civil::date(2026, 10, 20)), &now), "Today");
        assert_eq!(day_label(at(jiff::civil::date(2026, 10, 19)), &now), "Yesterday");
        assert_eq!(day_label(at(jiff::civil::date(2026, 3, 2)), &now), "2 March");
        assert_eq!(day_label(at(jiff::civil::date(2025, 3, 2)), &now), "2 March 2025");
    }
}
