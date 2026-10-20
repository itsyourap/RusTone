//! The call screen: the current call, its controls, the in-call keypad and
//! transfer, and the other calls.

use egui::{Align, Layout, Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use sip_engine::{CallId, CallState, MediaState};

use super::widgets;
use crate::app::App;
use crate::calls::Call;
use crate::icons::Icon;
use crate::theme::Palette;

const DTMF_KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"];

/// Panels that can open over the call controls.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum CallPanel {
    #[default]
    None,
    Keypad {
        sent: String,
    },
    Transfer {
        target: String,
    },
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    let Some(call) = app.sip.calls.current_call().cloned() else { return };
    others_strip(ui, app, call.id());
    let compact = app.ui.call_panel != CallPanel::None;

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(if compact { 4.0 } else { 12.0 });
        party_header(ui, p, &call, &caller(app, &call), compact);
        ui.add_space(if compact { 8.0 } else { 16.0 });

        if call.ended.is_some() {
            return;
        }
        if call.is_ringing_in() {
            ringing_controls(ui, app, p, &call);
            return;
        }
        match app.ui.call_panel.clone() {
            CallPanel::Keypad { sent } => keypad(ui, app, p, &call, sent),
            CallPanel::Transfer { target } => transfer(ui, app, p, &call, target),
            CallPanel::None => controls(ui, app, p, &call),
        }
        ui.add_space(20.0);
        if widgets::round_icon_button(ui, 64.0, p.danger, Icon::PhoneOff, p.on_accent, "Hang up").clicked() {
            app.act(|c| c.hangup(call.id()));
        }
        ui.add_space(8.0);
    }));
}

/// A slim bar on other pages leading back to the call.
pub fn return_bar(ui: &mut Ui, app: &mut App, p: &Palette) -> bool {
    let Some(call) = app.sip.calls.current_call().cloned() else { return false };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Return to the call"));
    let fill = if call.is_ringing_in() { p.warning } else { p.success };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(10), fill.gamma_multiply(0.18));
    let text = format!("{}  ·  {}", caller(app, &call), call.status_text());
    ui.painter().text(rect.left_center() + vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, text, Weight::Medium.font_id(13.0), p.text);
    ui.painter().text(rect.right_center() - vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, "Return to call", Weight::SemiBold.font_id(12.0), p.accent);
    response.clicked()
}

fn others_strip(ui: &mut Ui, app: &mut App, current: CallId) {
    let others: Vec<Call> = app.sip.calls.visible().filter(|c| c.id() != current).cloned().collect();
    if others.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for other in others {
            let text = format!("{} · {}", caller(app, &other), other.status_text());
            let button = egui::Button::new(egui::RichText::new(text).size(12.5)).corner_radius(egui::CornerRadius::same(14));
            if ui.add(button).on_hover_text("Switch to this call").clicked() {
                app.act(|c| c.switch_to(other.id()));
                app.ui.call_panel = CallPanel::None;
            }
        }
    });
}

/// The name to show for a call: the contact's
/// name, else the SIP display name, else the number.
pub fn caller(app: &App, call: &Call) -> String {
    let party = call.party();
    match app.phonebook.lookup(&party.user) {
        Some(contact) => contact.title(),
        None => party.title().to_owned(),
    }
}

fn party_header(ui: &mut Ui, p: &Palette, call: &Call, title: &str, compact: bool) {
    let party = call.party();
    // With a panel open, its space goes to the panel instead of the avatar.
    if !compact {
        avatar(ui, p, title, 88.0);
        ui.add_space(10.0);
    }
    ui.add(egui::Label::new(egui::RichText::new(title).font(Weight::SemiBold.font_id(22.0)).color(p.text)).truncate());
    if title != party.user {
        ui.label(egui::RichText::new(&party.user).size(14.0).color(p.secondary));
    }
    let status_color = if call.ended.is_some() { p.danger } else { p.secondary };
    ui.label(egui::RichText::new(call.status_text()).font(Weight::Medium.font_id(15.0)).color(status_color));
    if let Some(q) = call.quality.as_ref().filter(|_| call.ended.is_none() && call.info.media == MediaState::Active) {
        quality_line(ui, p, q);
    }
    if let Some(from) = &call.info.diversion {
        ui.label(egui::RichText::new(format!("Forwarded from {from}")).size(12.0).color(p.dim));
    }
    if let Some(transfer) = &call.transfer {
        ui.label(egui::RichText::new(transfer).size(12.0).color(p.dim));
    }
}

/// The quality icon as a coloured dot and a word, with the codec:
/// MOS above 3 is good, above 2 fair, otherwise poor.
fn quality_line(ui: &mut Ui, p: &Palette, q: &sip_engine::CallQuality) {
    let (word, color) = quality_word(q.mos, p);
    let text = format!("{word} · {}", q.codec);
    let galley = ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), p.secondary);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, galley.size().y.max(12.0)), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Call quality: {text}")));
    ui.painter().circle_filled(egui::pos2(rect.left() + 4.0, rect.center().y), 4.0, color);
    ui.painter().galley(egui::pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0), galley, p.secondary);
    response.on_hover_text(format!("Estimated quality (MOS) {:.1} of 4.5", q.mos));
}

fn quality_word(mos: f32, p: &Palette) -> (&'static str, egui::Color32) {
    if mos <= 2.0 {
        ("Poor", p.danger)
    } else if mos <= 3.0 {
        ("Fair", p.warning)
    } else {
        ("Good", p.success)
    }
}

fn avatar(ui: &mut Ui, p: &Palette, title: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    widgets::avatar_at(ui, p, rect, title);
}

fn ringing_controls(ui: &mut Ui, app: &mut App, p: &Palette, call: &Call) {
    ui.add_space(24.0);
    // "Answer with Video" when the caller offers video.
    let video = call.info.remote_video && app.settings.features.video;
    ui.horizontal(|ui| {
        let gap = if video { 36.0 } else { 72.0 };
        let buttons = if video { 3.0 } else { 2.0 };
        ui.add_space(((ui.available_width() - buttons * 68.0 - (buttons - 1.0) * gap) / 2.0).max(0.0));
        if widgets::round_icon_button(ui, 68.0, p.danger, Icon::PhoneOff, p.on_accent, "Decline").clicked() {
            app.act(|c| c.decline(call.id()));
        }
        ui.add_space(gap);
        if video {
            if widgets::round_icon_button(ui, 68.0, p.success, Icon::Video, p.on_accent, "Answer with video").clicked() {
                app.act(|c| c.answer(call.id(), true));
            }
            ui.add_space(gap);
        }
        if widgets::round_icon_button(ui, 68.0, p.success, Icon::Phone, p.on_accent, "Answer").clicked() {
            app.act(|c| c.answer(call.id(), false));
        }
    });
    ui.add_space(16.0);
    if !call.silenced && ui.add(egui::Button::new("Ignore").frame(false)).on_hover_text("Stop ringing without answering").clicked() {
        let cfg = app.settings.calls.clone();
        app.act(|c| c.ignore(call.id(), &cfg));
    }
}

/// A round toggle button with a caption underneath.
fn control(ui: &mut Ui, p: &Palette, icon: Icon, label: &str, active: bool, enabled: bool) -> bool {
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(76.0, 84.0), Layout::top_down(Align::Center), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let (fill, tint) = if active { (p.text, p.window) } else { (p.surface, p.text) };
            clicked = widgets::round_icon_button(ui, 56.0, fill, icon, tint, label).clicked();
            ui.label(egui::RichText::new(label).size(12.0).color(p.secondary));
        });
    });
    clicked
}

fn controls(ui: &mut Ui, app: &mut App, p: &Palette, call: &Call) {
    let id = call.id();
    let confirmed = call.info.state == CallState::Confirmed;
    let held = matches!(call.info.media, MediaState::LocalHold | MediaState::None) && confirmed;
    let calls = &app.sip.calls;
    let (mic_muted, speaker_muted) = (calls.mic_muted, calls.speaker_muted);

    let width = 3.0 * 76.0 + 2.0 * 12.0;
    let transfer = app.settings.features.transfer;
    for row in 0..2 {
        // Without call transfer the second row has two buttons, still centred.
        let buttons = if row == 1 && !transfer { 2.0 } else { 3.0 };
        let row_width = buttons * 76.0 + (buttons - 1.0) * 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.add_space(((ui.available_width() - row_width) / 2.0).max(0.0));
            if row == 0 {
                if control(ui, p, if mic_muted { Icon::MicOff } else { Icon::Mic }, if mic_muted { "Unmute" } else { "Mute" }, mic_muted, true) {
                    app.act(|c| c.toggle_mic());
                }
                if control(ui, p, Icon::Dialpad, "Keypad", false, confirmed) {
                    app.ui.call_panel = CallPanel::Keypad { sent: String::new() };
                }
                if control(ui, p, if speaker_muted { Icon::SpeakerOff } else { Icon::Speaker }, "Speaker", speaker_muted, true) {
                    app.act(|c| c.toggle_speaker());
                }
            } else {
                if control(ui, p, if held { Icon::Play } else { Icon::Pause }, if held { "Resume" } else { "Hold" }, held, confirmed) {
                    app.act(|c| c.toggle_hold(id));
                }
                if transfer && control(ui, p, Icon::Transfer, "Transfer", false, confirmed) {
                    app.ui.call_panel = CallPanel::Transfer { target: String::new() };
                }
                if control(ui, p, Icon::AddCall, "Add call", false, true) {
                    app.ui.adding_call = true;
                }
            }
        });
    }
    ui.add_space(8.0);
    levels(ui, app, p, width);
}

/// Speaker and microphone levels: moving one while
/// muted unmutes it.
fn levels(ui: &mut Ui, app: &mut App, p: &Palette, width: f32) {
    let (mic_muted, speaker_muted) = (app.sip.calls.mic_muted, app.sip.calls.speaker_muted);
    let meters = app.sip.meters();
    let (speaker_level, mic_level) = (meters.speaker(), meters.mic());
    // The engine samples every 100 ms.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    let audio = &mut app.settings.audio;
    let (mut speaker, mut mic) = (audio.volume_output, audio.volume_input);
    let mut out = (widgets::LevelResponse::default(), widgets::LevelResponse::default());
    ui.allocate_ui_with_layout(vec2(width, 72.0), Layout::top_down(Align::Min), |ui| {
        out.0 = widgets::level_slider(ui, p, (Icon::Speaker, Icon::SpeakerOff), "Speaker", &mut speaker, speaker_muted, Some(speaker_level));
        out.1 = widgets::level_slider(ui, p, (Icon::Mic, Icon::MicOff), "Microphone", &mut mic, mic_muted, Some(mic_level));
    });
    let (speaker_out, mic_out) = out;
    if speaker_out.changed || mic_out.changed {
        audio.volume_output = speaker;
        audio.volume_input = mic;
        app.mark_settings_dirty();
    }
    if speaker_out.mute_clicked || (speaker_out.changed && speaker_muted) {
        app.act(|c| c.toggle_speaker());
    }
    if mic_out.mute_clicked || (mic_out.changed && mic_muted) {
        app.act(|c| c.toggle_mic());
    }
}

fn keypad(ui: &mut Ui, app: &mut App, p: &Palette, call: &Call, mut sent: String) {
    if !sent.is_empty() {
        ui.label(egui::RichText::new(sent.clone()).font(Weight::Medium.font_id(20.0)).color(p.text));
    }
    ui.add_space(4.0);
    let (size, gap) = (52.0, 12.0);
    for row in DTMF_KEYS.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(((ui.available_width() - 3.0 * size - 2.0 * gap) / 2.0).max(0.0));
            for digit in row {
                if widgets::dial_key(ui, p, size, digit, "").clicked() {
                    sent.push_str(digit);
                    let method = app.settings.calls.dtmf_method;
                    app.act(|c| c.dtmf(call.id(), digit, method));
                }
            }
        });
    }
    // Typed digits are sent too.
    let typed: String = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None })
            .collect::<String>()
    });
    let typed: String = typed.chars().filter(|c| c.is_ascii_digit() || *c == '*' || *c == '#').collect();
    if !typed.is_empty() {
        sent.push_str(&typed);
        let method = app.settings.calls.dtmf_method;
        app.act(|c| c.dtmf(call.id(), &typed, method));
    }
    ui.add_space(8.0);
    let hide = ui.button("Hide keypad").clicked();
    app.ui.call_panel = if hide { CallPanel::None } else { CallPanel::Keypad { sent } };
}

fn transfer(ui: &mut Ui, app: &mut App, p: &Palette, call: &Call, mut target: String) {
    let mut close = false;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width().min(320.0));
        ui.label(egui::RichText::new("Transfer to").font(Weight::SemiBold.font_id(15.0)).color(p.text));
        let response = widgets::text_field(ui, p, "Number or SIP address", "e.g. 1002", &mut target);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(6.0);
        let ready = !target.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new("Transfer").min_size(vec2(ui.available_width(), 34.0))).clicked() || (enter && ready) {
            app.act(|c| c.transfer(call.id(), target.trim()));
            close = true;
        }
        let others: Vec<Call> = app.sip.calls.visible().filter(|c| c.id() != call.id() && c.is_live()).cloned().collect();
        if !others.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Or connect with another call").size(12.0).color(p.secondary));
            for other in others {
                let label = format!("Connect with {}", caller(app, &other));
                if ui.add(egui::Button::new(label).min_size(vec2(ui.available_width(), 32.0))).clicked() {
                    app.act(|c| c.attended_transfer(call.id(), other.id()));
                    close = true;
                }
            }
        }
        ui.add_space(6.0);
        if ui.button("Cancel").clicked() {
            close = true;
        }
    });
    app.ui.call_panel = if close { CallPanel::None } else { CallPanel::Transfer { target } };
}
