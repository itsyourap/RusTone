//! The window's layout: header, page, bottom navigation.

mod dialer;
mod settings;
mod widgets;

use egui::{Align, Layout, Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;

use crate::app::App;
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
}

/// Per-window UI state, reset when the window is rebuilt.
#[derive(Debug, Default)]
pub struct UiState {
    pub page: Page,
    pub dialer: dialer::DialerState,
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
        .show(ui, |ui| match app.ui.page {
            Page::Dialer => dialer::show(ui, &p, &mut app.ui.dialer),
            Page::Calls => widgets::empty_state(ui, &p, Icon::History, "No calls yet", "Calls you make and receive appear here."),
            Page::Contacts => widgets::empty_state(ui, &p, Icon::Users, "No contacts", "Contacts and their presence appear here."),
            Page::Messages => widgets::empty_state(ui, &p, Icon::Message, "No messages", "SIP messages appear here."),
            Page::Settings => settings::show(ui, app, &p),
        });

    if fastframe_titlebar::enabled() {
        fastframe_titlebar::resize(ui);
    }
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
        status_chip(ui, p);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(reserve.inline_width);
            let settings_tint = if app.ui.page == Page::Settings { p.accent } else { p.secondary };
            if widgets::icon_button(ui, p, Icon::Settings, settings_tint, "Settings").clicked() {
                app.ui.page = if app.ui.page == Page::Settings { Page::Dialer } else { Page::Settings };
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

/// Registration status as a small pill. Static until the SIP engine lands (Phase 2).
fn status_chip(ui: &mut Ui, p: &Palette) {
    let text = "Not connected";
    let galley = ui.painter().layout_no_wrap(text.to_owned(), Weight::Medium.font_id(11.5), p.secondary);
    // Leave room for the settings button; show only the dot when narrow.
    let show_text = ui.available_width() > galley.size().x + 90.0;
    let width = if show_text { galley.size().x + 26.0 } else { 20.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(11), p.surface);
    painter.circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 3.5, p.dim);
    if show_text {
        painter.galley(egui::pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0), galley, p.secondary);
    }
    response.on_hover_text(text);
}

fn nav(ui: &mut Ui, app: &mut App, p: &Palette) {
    const TABS: [(Page, Icon, &str); 4] = [
        (Page::Dialer, Icon::Dialpad, "Dialer"),
        (Page::Calls, Icon::History, "Calls"),
        (Page::Contacts, Icon::Users, "Contacts"),
        (Page::Messages, Icon::Message, "Messages"),
    ];
    let full = ui.max_rect();
    let width = full.width() / TABS.len() as f32;
    for (i, (page, icon, label)) in TABS.into_iter().enumerate() {
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
        ui.painter().text(
            egui::pos2(rect.center().x, rect.top() + 46.0),
            egui::Align2::CENTER_CENTER,
            label,
            Weight::Medium.font_id(11.0),
            tint,
        );
    }
}
