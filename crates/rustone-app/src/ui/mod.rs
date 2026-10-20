//! The window's layout: header, page, bottom navigation.

mod accounts;
mod call;
mod contacts;
mod history;
mod messages;
pub mod debug;
mod dialer;
mod settings;
#[cfg(test)]
mod test_support;
mod widgets;

use egui::{Align, Layout, Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;

use crate::app::App;
use rustone_core::account::Account;
use crate::sip::Status;
use crate::icons::Icon;
use crate::theme::Palette;

const HEADER_HEIGHT: f32 = 48.0;
const NAV_HEIGHT: f32 = 62.0;
/// Narrowest header content before the caption buttons push it down.
const MIN_HEADER_CONTENT: f32 = 180.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Dialer,
    Calls,
    Contacts,
    Messages,
    Settings,
    /// The account editor (reached from Settings).
    Account,
    /// The contact editor (reached from Contacts).
    Contact,
}

/// Per-window UI state, reset when the window is rebuilt.
#[derive(Debug, Default)]
pub struct UiState {
    pub page: Page,
    pub dialer: dialer::DialerState,
    pub account_draft: Option<accounts::Draft>,
    /// Showing the dialer during a call ("Add call").
    pub adding_call: bool,
    pub call_panel: call::CallPanel,
    pub contacts: contacts::ContactsView,
    pub history: history::HistoryView,
    pub messages: messages::MessagesView,
    pub contact_draft: Option<contacts::Draft>,
}

pub fn draw(ui: &mut Ui, app: &mut App) {
    let p = app.palette().clone();

    egui::Panel::top("header")
        .exact_size(HEADER_HEIGHT)
        .frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| header(ui, app, &p));

    egui::Panel::bottom("nav")
        .exact_size(NAV_HEIGHT)
        .frame(egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.outline)))
        .show(ui, |ui| nav(ui, app, &p));

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin::symmetric(16, 8)))
        .show(ui, |ui| {
            // A page whose feature was turned off isn't shown.
            if matches!(app.ui.page, Page::Messages) && !app.settings.features.messages {
                app.ui.page = Page::Dialer;
            }
            let page = app.ui.page;
            let in_call_view = page == Page::Dialer && !app.ui.adding_call;
            // Elsewhere during a call, a bar leads back to it.
            if app.sip.calls.has_calls() && !in_call_view && call::return_bar(ui, app, &p) {
                app.ui.page = Page::Dialer;
                app.ui.adding_call = false;
                return;
            }
            match page {
                Page::Dialer if app.sip.calls.has_calls() && !app.ui.adding_call => call::show(ui, app, &p),
                Page::Dialer => {
                    let can_call = app.status().is_online() || app.settings.local_account.enabled;
                    match dialer::show(ui, &p, &mut app.ui.dialer, can_call) {
                        Some(dialer::Action::Call(target)) => dial(app, &target),
                        Some(dialer::Action::Key(digit)) => app.sip.send(sip_engine::Command::PlayLocalDigits(digit)),
                        None => {}
                    }
                }
                Page::Calls => {
                    app.missed_seen();
                    history::show(ui, app, &p);
                }
                Page::Contacts => contacts::show(ui, app, &p),
                Page::Messages => messages::show(ui, app, &p),
                Page::Settings => settings::show(ui, app, &p),
                Page::Account => accounts::editor(ui, app, &p),
                Page::Contact => contacts::editor(ui, app, &p),
            }
            notice(ui, app, &p);
        });

    if fastframe_titlebar::enabled() {
        fastframe_titlebar::resize(ui);
    }
}

/// The latest notice (failed call, import result...) as a toast at the
/// bottom of the page; it fades after a few seconds.
fn notice(ui: &mut Ui, app: &App, p: &Palette) {
    let Some((text, _)) = &app.sip.calls.notice else { return };
    let max = ui.max_rect();
    let galley = widgets::elided(ui, text.clone(), Weight::Medium.font_id(13.0), p.window, max.width() - 48.0);
    let size = galley.size() + vec2(28.0, 18.0);
    let rect = egui::Rect::from_center_size(egui::pos2(max.center().x, max.bottom() - size.y / 2.0 - 8.0), size);
    let painter = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("notice")));
    painter.rect_filled(rect, egui::CornerRadius::same(10), p.text);
    painter.galley(rect.min + vec2(14.0, 9.0), galley, p.window);
    // Keep drawing so the notice goes away on time.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
}

/// Places a call from the dialer. Uses the local account for IP addresses
/// when it's on, or when there's no usable main account.
fn dial(app: &mut App, target: &str) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    let local = app.use_local_account(target);
    app.act(|c| c.dial(target, local));
    app.ui.adding_call = false;
    app.ui.call_panel = call::CallPanel::None;
}

fn header(ui: &mut Ui, app: &mut App, p: &Palette) {
    let rect = ui.max_rect();
    let titlebar = fastframe_titlebar::enabled();
    if titlebar {
        fastframe_titlebar::drag(ui, rect);
    }
    let room = fastframe_titlebar::footprint(ui.ctx(), HEADER_HEIGHT);
    let reserve = room.reserve(rect.width(), MIN_HEADER_CONTENT);

    ui.horizontal_centered(|ui| {
        ui.add_space(room.left);
        let (logo, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        Icon::Phone.image(p.accent, 18.0).paint_at(ui, logo);
        ui.label(egui::RichText::new("RusTone").font(Weight::SemiBold.font_id(16.0)).color(p.text));
        if let Some(copied) = status_chip(ui, p, &app.status(), app.settings.active()) {
            app.notify(format!("Copied {copied}"));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(reserve.inline_width);
            let in_settings = matches!(app.ui.page, Page::Settings | Page::Account);
            let settings_tint = if in_settings { p.accent } else { p.secondary };
            if widgets::icon_button(ui, p, Icon::Settings, settings_tint, "Settings").clicked() {
                app.ui.page = if in_settings { Page::Dialer } else { Page::Settings };
            }
        });
    });

    if titlebar {
        fastframe_titlebar::controls(
            ui,
            &fastframe_titlebar::Style { icon: p.secondary, button: p.surface_hover, header_height: HEADER_HEIGHT },
            &fastframe_titlebar::Labels::default(),
        );
    }
}

/// Registration status as a small pill; the account, its number, its SIP
/// address and any error on hover. Right-click copies the number or the
/// address; returns what was copied.
fn status_chip(ui: &mut Ui, p: &Palette, status: &Status, account: Option<&Account>) -> Option<String> {
    let (dot, text) = match status {
        Status::Online | Status::Ready => (p.success, p.secondary),
        Status::Starting | Status::Connecting => (p.warning, p.secondary),
        Status::Error(_) => (p.danger, p.danger),
        Status::NoAccount | Status::Offline => (p.dim, p.secondary),
    };
    let label = status.label();
    let galley = ui.painter().layout_no_wrap(label.to_owned(), Weight::Medium.font_id(11.5), text);
    // Leave room for the settings button; show only the dot when narrow.
    let show_text = ui.available_width() > galley.size().x + 90.0;
    let width = if show_text { galley.size().x + 26.0 } else { 20.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(11), p.surface);
    let pulse = matches!(status, Status::Starting | Status::Connecting);
    let alpha = if pulse {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        0.55 + 0.45 * (ui.input(|i| i.time) * 4.0).sin().abs() as f32
    } else {
        1.0
    };
    painter.circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 3.5, dot.gamma_multiply(alpha));
    if show_text {
        painter.galley(egui::pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0), galley, text);
    }
    let hover = status_hover(status, account);
    let response = response.on_hover_text(hover);
    let mut copied = None;
    if let Some(account) = account {
        response.context_menu(|ui| {
            widgets::menu(ui, |ui| {
                for (label, value) in [("Copy phone number", account.number().to_owned()), ("Copy full SIP URI", account.sip_uri())] {
                    if ui.add_enabled(!value.is_empty(), egui::Button::new(label)).clicked() {
                        ui.ctx().copy_text(value.clone());
                        copied = Some(value);
                        ui.close();
                    }
                }
            });
        });
    }
    copied
}

/// A count on a tab: missed calls (red), unread messages (accent).
fn count_badge(ui: &Ui, p: &Palette, center: egui::Pos2, count: u32, fill: egui::Color32) {
    let text = if count > 99 { "99+".to_owned() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, Weight::SemiBold.font_id(10.5), p.on_accent);
    let size = vec2((galley.size().x + 8.0).max(17.0), 17.0);
    let rect = egui::Rect::from_min_size(egui::pos2(center.x - 8.5, center.y - 8.5), size);
    ui.painter().rect(rect, egui::CornerRadius::same(9), fill, egui::Stroke::new(2.0, p.panel), egui::StrokeKind::Outside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.on_accent);
}

/// The status pill's hover text: "Home · Online", the account's number,
/// its SIP address, and the error when there is one.
fn status_hover(status: &Status, account: Option<&Account>) -> String {
    let Some(account) = account.filter(|_| *status != Status::NoAccount) else {
        return match status {
            Status::NoAccount => "Add an account in Settings".to_owned(),
            Status::Error(e) => e.clone(),
            s => s.label().to_owned(),
        };
    };
    let mut lines = vec![format!("{} · {}", account.title(), status.label())];
    for line in [account.number().to_owned(), account.sip_uri()] {
        if !line.is_empty() && line != "sip:" {
            lines.push(line);
        }
    }
    if let Status::Error(e) = status {
        lines.push(e.clone());
    }
    lines.join("\n")
}

fn nav(ui: &mut Ui, app: &mut App, p: &Palette) {
    const TABS: [(Page, Icon, &str); 4] = [
        (Page::Dialer, Icon::Dialpad, "Dialer"),
        (Page::Calls, Icon::History, "Calls"),
        (Page::Contacts, Icon::Users, "Contacts"),
        (Page::Messages, Icon::Message, "Messages"),
    ];
    let messages = app.settings.features.messages;
    let tabs: Vec<_> = TABS.into_iter().filter(|(page, _, _)| messages || *page != Page::Messages).collect();
    let full = ui.max_rect();
    let width = full.width() / tabs.len() as f32;
    for (i, (page, icon, label)) in tabs.into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(full.left() + width * i as f32, full.top()), vec2(width, full.height()));
        let response = ui.interact(rect, ui.id().with(label), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, app.ui.page == page, label));
        if response.clicked() {
            app.ui.page = page;
        }
        let selected = app.ui.page == page;
        let hovered = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
        let tint = if selected { p.accent } else { widgets::lerp_color(p.dim, p.text, hovered) };

        // A pill behind the selected tab's icon.
        let icon_center = egui::pos2(rect.center().x, rect.top() + 22.0);
        let pill_t = ui.ctx().animate_bool_with_time(response.id.with("pill"), selected, 0.15);
        if pill_t > 0.0 {
            let pill = egui::Rect::from_center_size(icon_center, vec2(52.0 * (0.6 + 0.4 * pill_t), 28.0));
            ui.painter().rect_filled(pill, egui::CornerRadius::same(14), p.accent.gamma_multiply(0.16 * pill_t));
        }
        Icon::image(icon, tint, 20.0).paint_at(ui, egui::Rect::from_center_size(icon_center, Vec2::splat(20.0)));
        let (count, fill) = match page {
            Page::Calls => (app.missed, p.danger),
            Page::Messages => (app.phonebook.unread(), p.accent),
            _ => (0, p.accent),
        };
        if count > 0 {
            count_badge(ui, p, icon_center + vec2(12.0, -9.0), count, fill);
        }
        ui.painter().text(
            egui::pos2(rect.center().x, rect.top() + 46.0),
            egui::Align2::CENTER_CENTER,
            label,
            Weight::Medium.font_id(11.0),
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_hover_names_the_account_its_number_and_address() {
        let home = Account { label: "Home".into(), username: "+910000000000@sip.example.com".into(), ..Account::default() };
        assert_eq!(status_hover(&Status::Online, Some(&home)), "Home · Online\n+910000000000\nsip:+910000000000@sip.example.com");
        assert_eq!(
            status_hover(&Status::Error("403 Forbidden".into()), Some(&home)),
            "Home · Error\n+910000000000\nsip:+910000000000@sip.example.com\n403 Forbidden"
        );
        assert_eq!(status_hover(&Status::NoAccount, None), "Add an account in Settings");
    }

    #[test]
    fn right_click_copies_the_number_or_address() {
        use egui_kittest::kittest::Queryable;
        for (item, expected) in [("Copy phone number", "+910000000000"), ("Copy full SIP URI", "sip:+910000000000@sip.example.com")] {
            let mut harness = test_support::harness([300.0, 160.0], None::<String>, |ui, copied| {
                let home = Account { label: "Home".into(), username: "+910000000000@sip.example.com".into(), ..Account::default() };
                if let Some(c) = status_chip(ui, &Palette::dark(), &Status::Online, Some(&home)) {
                    *copied = Some(c);
                }
            });
            harness.get_by_label("Online").click_secondary();
            harness.run_steps(2);
            harness.get_by_label(item).click();
            harness.run_steps(2);
            assert_eq!(harness.state().as_deref(), Some(expected));
        }
    }
}
