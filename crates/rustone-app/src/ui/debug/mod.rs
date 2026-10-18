//! The debug window: a second native window with a vertical navigation
//! rail and live views of RusTone's internals (logs, SIP messages, RTP
//! streams, audio). It opens from Settings, the tray, or Ctrl+Shift+D.

mod audio;
mod logs;
mod media;
mod overview;
mod sip;

use std::time::{Duration, SystemTime};

use egui::{Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;

use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

const RAIL_WIDTH: f32 = 188.0;
const REFRESH: Duration = Duration::from_millis(250);
/// 60 fps for the live audio graphs.
const LIVE_REFRESH: Duration = Duration::from_micros(16_667);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Overview,
    Logs,
    Sip,
    Media,
    Audio,
}

impl Section {
    const ALL: [Section; 5] = [Self::Overview, Self::Logs, Self::Sip, Self::Media, Self::Audio];

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::Dashboard,
            Self::Logs => Icon::Logs,
            Self::Sip => Icon::Network,
            Self::Media => Icon::Activity,
            Self::Audio => Icon::Waveform,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Logs => "Logs",
            Self::Sip => "SIP messages",
            Self::Media => "RTP streams",
            Self::Audio => "Audio",
        }
    }
}

/// State of the debug window, kept while it's open.
#[derive(Debug, Default)]
pub struct DebugState {
    pub open: bool,
    pub section: Section,
    pub logs: logs::LogsState,
    pub sip: sip::SipState,
    pub media: media::MediaState,
    pub audio: audio::AudioState,
}

/// Opens or closes the debug window (and the engine's extra collection).
pub fn set_open(app: &mut App, open: bool) {
    if app.debug.open != open {
        app.debug.open = open;
        app.sip.send(sip_engine::Command::SetDebug(open));
    }
}

/// Shows the debug window while it's open. Called every frame of the main window.
pub fn show(ctx: &egui::Context, app: &mut App) {
    if !app.debug.open {
        return;
    }
    let p = app.palette().clone();
    let builder = egui::ViewportBuilder::default()
        .with_title("RusTone Debug")
        .with_app_id("rustone-debug")
        .with_icon(crate::icons::window_icon())
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([760.0, 480.0]);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("rustone-debug"), builder, |ui, _class| {
        if ui.input(|i| i.viewport().close_requested()) {
            set_open(app, false);
            return;
        }
        draw(ui, app, &p);
        // egui shortens delayed repaints by the predicted frame time, which
        // turns a 16 ms request into back-to-back frames; add it back.
        let predicted = Duration::try_from_secs_f32(ui.input(|i| i.predicted_dt)).unwrap_or_default();
        ui.ctx().request_repaint_after(refresh(app) + predicted);
    });
}

/// How soon to redraw without input. Live audio graphs need a full frame
/// rate; everything else changes at most a few times a second.
fn refresh(app: &App) -> Duration {
    let live_audio = app.debug.section == Section::Audio && app.debug.audio.is_live() && app.sip.debug_feed().is_enabled();
    if live_audio { LIVE_REFRESH } else { REFRESH }
}

fn draw(ui: &mut Ui, app: &mut App, p: &Palette) {
    egui::Panel::left("debug-rail")
        .exact_size(RAIL_WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(10, 12)).stroke(egui::Stroke::new(1.0, p.outline)))
        .show(ui, |ui| rail(ui, app, p));

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin::symmetric(16, 12)))
        .show(ui, |ui| match app.debug.section {
            Section::Overview => overview::show(ui, app, p),
            Section::Logs => logs::show(ui, app, p),
            Section::Sip => sip::show(ui, app, p),
            Section::Media => media::show(ui, app, p),
            Section::Audio => audio::show(ui, app, p),
        });
}

fn rail(ui: &mut Ui, app: &mut App, p: &Palette) {
    ui.horizontal(|ui| {
        let (logo, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        Icon::Bug.image(p.accent, 18.0).paint_at(ui, logo);
        ui.label(egui::RichText::new("RusTone Debug").font(Weight::SemiBold.font_id(15.0)).color(p.text));
    });
    ui.add_space(14.0);
    for section in Section::ALL {
        let selected = app.debug.section == section;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, section.label()));
        let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
        let fill = if selected { p.accent.gamma_multiply(0.18) } else { p.surface_hover.gamma_multiply(hover) };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(8), fill);
        let tint = if selected { p.accent } else { p.secondary };
        section.icon().image(tint, 17.0).paint_at(ui, egui::Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), Vec2::splat(17.0)));
        ui.painter().text(rect.left_center() + vec2(38.0, 0.0), egui::Align2::LEFT_CENTER, section.label(), Weight::Medium.font_id(13.5), if selected { p.text } else { p.secondary });
        if response.clicked() {
            app.debug.section = section;
        }
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        let feed = app.sip.debug_feed();
        let collecting = feed.is_enabled();
        ui.label(egui::RichText::new(if collecting { "● Collecting RTP & audio" } else { "○ RTP & audio paused" }).size(11.5).color(if collecting { p.success } else { p.dim }));
        ui.label(egui::RichText::new(format!("{} SIP messages · {} log lines", feed.messages().len(), feed.logs().len())).size(11.5).color(p.dim));
    });
}

// ---------------------------------------------------------------------------
// Shared helpers

/// Local wall-clock time with milliseconds.
pub fn clock(t: SystemTime) -> String {
    match jiff::Timestamp::try_from(t) {
        Ok(ts) => ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M:%S%.3f").to_string(),
        Err(_) => "--:--:--".into(),
    }
}

pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Monospace)
}

/// A section title with optional actions on the right.
pub fn header(ui: &mut Ui, p: &Palette, title: &str, actions: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(Weight::SemiBold.font_id(18.0)).color(p.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
    ui.add_space(6.0);
}

/// A label/value row in a key-value grid.
pub fn kv(ui: &mut Ui, p: &Palette, key: &str, value: impl Into<String>) {
    ui.label(egui::RichText::new(key).size(12.5).color(p.dim));
    ui.label(egui::RichText::new(value.into()).font(mono(12.5)).color(p.text));
    ui.end_row();
}

/// A titled box for overview and stats cards.
pub fn panel_card<R>(ui: &mut Ui, p: &Palette, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0, p.outline))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(title.to_uppercase()).font(Weight::SemiBold.font_id(11.0)).color(p.dim));
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}
