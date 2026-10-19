//! The settings page: accounts, appearance and window behaviour. The full
//! Settings arrive in Phase 8.

use egui::Ui;
use rustone_core::settings::{DtmfMethod, ThemePreference};

use super::widgets::{self, card, section_title, setting_row, setting_stack};
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        ui.heading("Settings");
        ui.add_space(8.0);

        super::accounts::list(ui, app, p);
        ui.add_space(12.0);

        section_title(ui, p, "Audio devices");
        audio_devices(ui, app, p);
        ui.add_space(12.0);

        section_title(ui, p, "Audio codecs");
        codecs(ui, app, p);
        ui.add_space(12.0);

        section_title(ui, p, "Calls");
        card(ui, p, |ui| {
            let calls = &mut app.settings.calls;
            let mut changed = false;
            setting_stack(ui, p, "DTMF", Some("How keypad digits are sent during a call"), |ui| {
                egui::ComboBox::from_id_salt("dtmf-method")
                    .width(ui.available_width())
                    .selected_text(dtmf_label(calls.dtmf_method))
                    .show_ui(ui, |ui| {
                        for method in [DtmfMethod::Auto, DtmfMethod::Rfc2833, DtmfMethod::InBand, DtmfMethod::SipInfo] {
                            changed |= ui.selectable_value(&mut calls.dtmf_method, method, dtmf_label(method)).changed();
                        }
                    });
            });
            ui.separator();
            for (value, title, description) in [
                (&mut calls.local_dtmf, "Call sounds", Some("Keypad tones, call-waiting beep and hang-up sound")),
                (&mut calls.call_waiting, "Call waiting", Some("Accept a second call during a call")),
                (&mut calls.bring_to_front, "Show window for incoming calls", None),
            ] {
                setting_row(ui, p, title, description, |ui| {
                    changed |= widgets::toggle(ui, p, value, title).changed();
                });
            }
            if changed {
                app.mark_settings_dirty();
            }
        });
        ui.add_space(12.0);

        section_title(ui, p, "Appearance");
        card(ui, p, |ui| {
            setting_stack(ui, p, "Theme", Some("System follows the Windows light/dark setting"), |ui| {
                let options = [
                    (ThemePreference::System, Some(Icon::Monitor), "System"),
                    (ThemePreference::Light, Some(Icon::Sun), "Light"),
                    (ThemePreference::Dark, Some(Icon::Moon), "Dark"),
                ];
                if widgets::segmented(ui, p, &mut app.settings.ui.theme, &options) {
                    app.mark_settings_dirty();
                }
            });
            ui.separator();
            let mut custom = app.settings.ui.custom_titlebar;
            setting_row(ui, p, "RusTone title bar", Some("Draw a modern title bar instead of the system frame"), |ui| {
                if widgets::toggle(ui, p, &mut custom, "RusTone title bar").changed() {
                    app.settings.ui.custom_titlebar = custom;
                    app.mark_settings_dirty();
                    // Window decorations are fixed when the window is created.
                    app.reopen_window(ui.ctx());
                }
            });
        });

        ui.add_space(12.0);
        section_title(ui, p, "Behaviour");
        card(ui, p, |ui| {
            let mut close_to_tray = app.settings.ui.close_to_tray;
            setting_row(ui, p, "Keep running in the tray", Some("Closing the window keeps RusTone ready for calls"), |ui| {
                if widgets::toggle(ui, p, &mut close_to_tray, "Keep running in the tray").changed() {
                    app.settings.ui.close_to_tray = close_to_tray;
                    app.mark_settings_dirty();
                }
            });
            ui.separator();
            let mut start_minimized = app.settings.ui.start_minimized;
            setting_row(ui, p, "Start minimised", Some("Open in the tray without showing the window"), |ui| {
                if widgets::toggle(ui, p, &mut start_minimized, "Start minimised").changed() {
                    app.settings.ui.start_minimized = start_minimized;
                    app.mark_settings_dirty();
                }
            });
        });

        ui.add_space(12.0);
        section_title(ui, p, "Developer");
        card(ui, p, |ui| {
            setting_row(ui, p, "Debug window", Some("Logs, SIP messages, RTP statistics and audio graphs (Ctrl+Shift+D)"), |ui| {
                let label = if app.debug.open { "Close" } else { "Open" };
                if ui.button(label).clicked() {
                    let open = !app.debug.open;
                    crate::ui::debug::set_open(app, open);
                }
            });
        });

        ui.add_space(12.0);
        section_title(ui, p, "About");
        card(ui, p, |ui| {
            setting_row(ui, p, &format!("RusTone {}", env!("CARGO_PKG_VERSION")), Some("pjproject 2.17 · egui · fastframe"), |_| {});
        });
        ui.add_space(8.0);
    });
}

fn dtmf_label(method: DtmfMethod) -> &'static str {
    match method {
        DtmfMethod::Auto => "Automatic (RFC 2833, else in-band)",
        DtmfMethod::Rfc2833 => "RFC 2833",
        DtmfMethod::InBand => "In-band tones",
        DtmfMethod::SipInfo => "SIP INFO",
    }
}

/// Enabled codecs in priority order (offered in that order), then the
/// others the engine has. Changes restart the engine once no call is live.
fn codecs(ui: &mut Ui, app: &mut App, p: &Palette) {
    use rustone_core::codecs;
    let available: Vec<String> = app.sip.codecs.iter().map(|c| c.id.clone()).collect();
    let rows = codecs::rows(&app.settings.sip.audio_codecs, &available);
    let enabled_count = rows.iter().filter(|r| r.enabled).count();
    let mut new_list = None;
    card(ui, p, |ui| {
        ui.label(egui::RichText::new("Offered in this order.").size(12.0).color(p.dim));
        ui.add_space(4.0);
        for (i, row) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut on = row.enabled;
                let name = codecs::label(&row.id);
                if widgets::toggle(ui, p, &mut on, &name).changed() {
                    new_list = Some(codecs::set_enabled(&app.settings.sip.audio_codecs, &available, &row.id, on));
                }
                let color = if row.enabled { p.text } else { p.dim };
                ui.label(egui::RichText::new(name).color(color)).on_hover_text(&row.id);
                if row.enabled {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(i + 1 < enabled_count, |ui| {
                            if widgets::icon_button(ui, p, Icon::Down, p.secondary, "Lower priority").clicked() {
                                new_list = Some(codecs::move_codec(&app.settings.sip.audio_codecs, &row.id, false));
                            }
                        });
                        ui.add_enabled_ui(i > 0, |ui| {
                            if widgets::icon_button(ui, p, Icon::Up, p.secondary, "Higher priority").clicked() {
                                new_list = Some(codecs::move_codec(&app.settings.sip.audio_codecs, &row.id, true));
                            }
                        });
                    });
                }
            });
        }
        if available.is_empty() {
            ui.label(egui::RichText::new("The full codec list appears once the SIP engine has started.").size(12.0).color(p.dim));
        }
        if app.sip.calls.live_count() > 0 {
            ui.label(egui::RichText::new("Changes apply after the current call.").size(12.0).color(p.warning));
        }
    });
    if let Some(list) = new_list {
        if list.is_empty() {
            app.sip.calls.notice = Some(("Keep at least one codec enabled".into(), std::time::Instant::now()));
        } else {
            app.settings.sip.audio_codecs = list;
            app.mark_settings_dirty();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceKind {
    Input,
    Output,
}

/// Microphone, speaker and ring device, by name.
fn audio_devices(ui: &mut Ui, app: &mut App, p: &Palette) {
    let devices = app.sip.audio_devices.clone();
    let audio = &mut app.settings.audio;
    let mut changed = false;
    card(ui, p, |ui| {
        for (i, (value, title, description, kind)) in [
            (&mut audio.input_device, "Microphone", None, DeviceKind::Input),
            (&mut audio.output_device, "Speaker", Some("Call audio"), DeviceKind::Output),
            (&mut audio.ring_device, "Ring device", Some("Plays the ringtone while no call is connected"), DeviceKind::Output),
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                ui.separator();
            }
            setting_stack(ui, p, title, description, |ui| {
                changed |= device_picker(ui, title, value, &devices, kind);
            });
        }
        if devices.is_empty() {
            ui.label(egui::RichText::new("The device list appears once the SIP engine has started.").size(12.0).color(p.dim));
        }
    });
    if changed {
        app.mark_settings_dirty();
    }
}

/// A device combo: "System default", then the devices of `kind`. A saved
/// device that isn't connected stays selected (and is used when it's back).
fn device_picker(ui: &mut Ui, id: &str, value: &mut String, devices: &[sip_engine::AudioDevice], kind: DeviceKind) -> bool {
    let fits = |d: &&sip_engine::AudioDevice| match kind {
        DeviceKind::Input => d.inputs > 0,
        DeviceKind::Output => d.outputs > 0,
    };
    let mut names: Vec<&str> = devices.iter().filter(fits).map(|d| d.name.as_str()).collect();
    names.dedup();
    let missing = !value.is_empty() && !names.contains(&value.as_str()) && !devices.is_empty();
    let selected = match (value.is_empty(), missing) {
        (true, _) => "System default".to_owned(),
        (false, true) => format!("{value} (not connected)"),
        (false, false) => value.clone(),
    };
    let mut changed = false;
    let combo = egui::ComboBox::from_id_salt(("audio-device", id)).width(ui.available_width()).selected_text(selected).truncate().show_ui(ui, |ui| {
        changed |= ui.selectable_value(value, String::new(), "System default").changed();
        for name in names {
            changed |= ui.selectable_value(value, name.to_owned(), name).changed();
        }
    });
    ui.ctx().accesskit_node_builder(combo.response.id, |node| node.set_label(id));
    changed
}
