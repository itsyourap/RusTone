//! The settings window: three tabs whose
//! sections are laid out in as many columns as the window is wide, so each
//! fits without scrolling; and the account editor.

use egui::Ui;
use rustone_core::settings::{AutoAnswerWhen, DenyIncoming, DtmfMethod, ThemePreference};

use super::widgets::{self, card, section_title, setting_row, setting_stack};
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;


fn users_directory(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let response = widgets::text_field(
            ui,
            p,
            "Directory URL",
            "https://… (JSON, XML, Yealink or CSV); {username} and %s fill in the account",
            &mut app.settings.sip.users_directory,
        );
        if response.changed() {
            app.mark_settings_dirty();
        }
        let status = match &app.directory.status {
            Some(s) => s.clone(),
            None if app.settings.sip.users_directory.trim().is_empty() => "Its contacts are added and kept up to date; yours are never changed.".to_owned(),
            None => "Loading…".to_owned(),
        };
        ui.label(egui::RichText::new(status).size(12.0).color(p.dim));
    });
}

fn calls(ui: &mut Ui, app: &mut App, p: &Palette) {
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
        setting_stack(ui, p, "Reject calls", Some("Incoming calls refused with 486 Busy"), |ui| {
            egui::ComboBox::from_id_salt("deny-incoming")
                .width(ui.available_width())
                .selected_text(deny_label(calls.deny_incoming))
                .show_ui(ui, |ui| {
                    for mode in [DenyIncoming::Off, DenyIncoming::All, DenyIncoming::OtherUser, DenyIncoming::OtherDomain, DenyIncoming::RemoteDomain, DenyIncoming::OtherUserOrDomain] {
                        changed |= ui.selectable_value(&mut calls.deny_incoming, mode, deny_label(mode)).changed();
                    }
                });
        });
        ui.separator();
        changed |= seconds_row(ui, p, "Hang up calls after", Some("0 for never"), &mut calls.auto_hangup, 0..=86_400);
        ui.add_space(4.0);
        ui.separator();
        for (value, title, description) in [
            (&mut calls.dnd, "Do not disturb", Some("Refuse incoming calls. Also under the status pill and in the tray")),
            (&mut calls.local_dtmf, "Call sounds", Some("Keypad tones, call-waiting beep and hang-up sound")),
            (&mut calls.call_waiting, "Call waiting", Some("Accept a second call during a call")),
            (&mut calls.bring_to_front, "Show window for incoming calls", None),
            (&mut calls.media_keys, "Media keys and headset buttons", Some("Play answers or holds, stop hangs up, during calls")),
            (&mut calls.headset, "USB headset controls", Some("Hook switch answers and hangs up, mute and redial buttons, headset lights")),
        ] {
            setting_row(ui, p, title, description, |ui| {
                changed |= widgets::toggle(ui, p, value, title).changed();
            });
        }
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn auto_answer(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let calls = &mut app.settings.calls;
        let mut changed = false;
        for (value, title, description) in [
            (&mut calls.auto_answer, "Answer every call", Some("Also under the status pill and in the tray")),
            (&mut calls.auto_answer_on_request, "Answer when the caller asks", Some("X-AUTOANSWER or Call-Info answer-after (intercom, paging)")),
        ] {
            setting_row(ui, p, title, description, |ui| {
                changed |= widgets::toggle(ui, p, value, title).changed();
            });
        }
        ui.separator();
        changed |= seconds_row(ui, p, "Ring first for", Some("0 answers at once"), &mut calls.auto_answer_delay, 0..=600);
        changed |= widgets::text_field(ui, p, "Only these callers", "Everyone (e.g. 100;2*;+9133*)", &mut calls.auto_answer_numbers).changed();
        ui.add_space(4.0);
        setting_stack(ui, p, "While in another call", None, |ui| {
            egui::ComboBox::from_id_salt("auto-answer-when")
                .width(ui.available_width())
                .selected_text(auto_answer_when_label(calls.auto_answer_when))
                .show_ui(ui, |ui| {
                    for when in [AutoAnswerWhen::NoOtherCalls, AutoAnswerWhen::OthersOnHold, AutoAnswerWhen::Always] {
                        changed |= ui.selectable_value(&mut calls.auto_answer_when, when, auto_answer_when_label(when)).changed();
                    }
                });
        });
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn forwarding(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let calls = &mut app.settings.calls;
        let mut changed = false;
        setting_row(ui, p, "Forward calls", Some("Sends callers on with a 302. Also under the status pill and in the tray"), |ui| {
            changed |= widgets::toggle(ui, p, &mut calls.forwarding, "Forward calls").changed();
        });
        ui.separator();
        changed |= widgets::text_field(ui, p, "Forward to", "Number or SIP address", &mut calls.forwarding_number).changed();
        ui.add_space(4.0);
        changed |= seconds_row(ui, p, "Ring first for", Some("0 forwards without ringing"), &mut calls.forwarding_delay, 0..=600);
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn speed_dial_section(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| speed_dial(ui, app, p));
}

fn recording(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let recording = &mut app.settings.recording;
        let mut changed = false;
        for (value, title, description) in [
            (&mut recording.button, "Record button", Some("Show Record in the call controls")),
            (&mut recording.auto, "Record every call", Some("Starts when a call is answered")),
        ] {
            setting_row(ui, p, title, description, |ui| {
                changed |= widgets::toggle(ui, p, value, title).changed();
            });
        }
        ui.separator();
        let shown = recording.dir().display().to_string();
        ui.label(egui::RichText::new("Folder").size(12.0).color(p.secondary));
        ui.add(egui::Label::new(egui::RichText::new(&shown).size(12.5).color(p.text)).wrap());
        ui.horizontal(|ui| {
            if ui.button("Change…").clicked()
                && let Some(folder) = rfd::FileDialog::new().set_title("Recordings folder").set_directory(recording.dir()).pick_folder()
            {
                recording.folder = folder.display().to_string();
                changed = true;
            }
            if !recording.folder.is_empty() && ui.button("Use default").clicked() {
                recording.folder.clear();
                changed = true;
            }
            if ui.button("Open").clicked() {
                let dir = recording.dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        ui.label(egui::RichText::new("Recordings are WAV files with both sides of the call.").size(12.0).color(p.dim));
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn feature_codes(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let calls = &mut app.settings.calls;
        let mut changed = false;
        ui.label(egui::RichText::new("Codes your PBX understands, sent before a number.").size(12.0).color(p.dim));
        changed |= widgets::text_field(ui, p, "Call pickup (empty for off)", "**", &mut calls.pickup_code).changed();
        ui.add_space(6.0);
        setting_row(ui, p, "Transfer with a code", Some("Dial the code and number in the call instead of a SIP transfer"), |ui| {
            changed |= widgets::toggle(ui, p, &mut calls.use_transfer_code, "Transfer with a code").changed();
        });
        if calls.use_transfer_code {
            changed |= widgets::text_field(ui, p, "Transfer code", "##", &mut calls.transfer_code).changed();
        }
        ui.separator();
        setting_row(ui, p, "Attended transfer with a code", Some("Adds an attended transfer button to the transfer panel"), |ui| {
            changed |= widgets::toggle(ui, p, &mut calls.use_attended_code, "Attended transfer with a code").changed();
        });
        if calls.use_attended_code {
            changed |= widgets::text_field(ui, p, "Attended transfer code", "*2", &mut calls.attended_code).changed();
        }
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn command_hooks(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        let hooks = &mut app.settings.hooks;
        let set = [&hooks.outgoing_call, &hooks.incoming_call, &hooks.call_ring, &hooks.call_answer, &hooks.call_answer_video, &hooks.call_start, &hooks.call_busy, &hooks.call_end]
            .iter()
            .filter(|h| !h.trim().is_empty())
            .count();
        let summary = match set {
            0 => "Programs to run on call events. None set.".to_owned(),
            1 => "Programs to run on call events. 1 set.".to_owned(),
            n => format!("Programs to run on call events. {n} set."),
        };
        // Folded away: eight fields most people never use.
        let open = app.ui.hooks_open;
        setting_row(ui, p, &summary, None, |ui| {
            if ui.button(if open { "Done" } else { "Edit hooks" }).clicked() {
                app.ui.hooks_open = !open;
            }
        });
        if !open {
            return;
        }
        ui.label(egui::RichText::new("{number}, {name}, {callid}, {direction}, {duration} and {time} are filled in; without them the number is added at the end.").size(12.0).color(p.dim));
        let mut changed = false;
        for (value, label) in [
            (&mut hooks.outgoing_call, "Outgoing call"),
            (&mut hooks.incoming_call, "Incoming call"),
            (&mut hooks.call_ring, "Ringing"),
            (&mut hooks.call_answer, "Answered (incoming)"),
            (&mut hooks.call_answer_video, "Answered with video"),
            (&mut hooks.call_start, "Call started"),
            (&mut hooks.call_busy, "Busy or declined"),
            (&mut hooks.call_end, "Call ended"),
        ] {
            changed |= widgets::text_field(ui, p, label, "Command line", value).changed();
            ui.add_space(4.0);
        }
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn features(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        ui.label(egui::RichText::new("Turn off what your provider doesn't support.").size(12.0).color(p.dim));
        let features = &mut app.settings.features;
        let mut changed = false;
        for (value, title, description) in [
            (&mut features.presence, "Presence", "See when contacts are busy or on the phone (SUBSCRIBE)"),
            (&mut features.messages, "Messages", "Send and receive text messages (MESSAGE)"),
            (&mut features.transfer, "Call transfer", "Pass a call on to another number (REFER)"),
            (&mut features.video, "Video calls", "Make and answer calls with video"),
        ] {
            setting_row(ui, p, title, Some(description), |ui| {
                changed |= widgets::toggle(ui, p, value, title).changed();
            });
        }
        if changed {
            app.mark_settings_dirty();
        }
    });
}

fn appearance(ui: &mut Ui, app: &mut App, p: &Palette) {
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
}

fn behaviour(ui: &mut Ui, app: &mut App, p: &Palette) {
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
        ui.separator();
        start_with_windows(ui, app, p);
        ui.separator();
        let mut links = *app.ui.links_handled.get_or_insert_with(crate::urlhandler::registered);
        setting_row(ui, p, "Open sip:, tel: and callto: links", Some("Calls start in RusTone from browsers and other apps"), |ui| {
            if widgets::toggle(ui, p, &mut links, "Open sip:, tel: and callto: links").changed() {
                match crate::urlhandler::set(links) {
                    Ok(()) => app.ui.links_handled = Some(crate::urlhandler::registered()),
                    Err(e) => app.notify(format!("Couldn't change the link handler: {e}")),
                }
            }
        });
        if links && ui.link(egui::RichText::new("Another app still opens them? Choose RusTone in Windows' Default apps").size(12.0)).clicked() {
            let _ = std::process::Command::new("explorer").arg("ms-settings:defaultapps").spawn();
        }
    });
}

fn developer(ui: &mut Ui, app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        setting_row(ui, p, "Debug window", Some("Logs, SIP messages, RTP statistics and audio graphs (Ctrl+Shift+D)"), |ui| {
            let label = if app.debug.open { "Close" } else { "Open" };
            if ui.button(label).clicked() {
                let open = !app.debug.open;
                crate::ui::debug::set_open(app, open);
            }
        });
    });
}

fn about(ui: &mut Ui, _app: &mut App, p: &Palette) {
    card(ui, p, |ui| {
        setting_row(ui, p, &format!("RusTone {}", env!("CARGO_PKG_VERSION")), Some("pjproject 2.17 · egui · fastframe"), |_| {});
    });
}

/// A number of seconds on one row with its title.
fn seconds_row(ui: &mut Ui, p: &Palette, title: &str, description: Option<&str>, value: &mut u32, range: std::ops::RangeInclusive<u32>) -> bool {
    let mut changed = false;
    setting_row(ui, p, title, description, |ui| {
        changed = ui.add(egui::DragValue::new(value).range(range).suffix(" s").speed(1.0)).on_hover_text(title).changed();
    });
    changed
}

fn auto_answer_when_label(when: AutoAnswerWhen) -> &'static str {
    match when {
        AutoAnswerWhen::NoOtherCalls => "Don't answer",
        AutoAnswerWhen::OthersOnHold => "Answer if the others are on hold",
        AutoAnswerWhen::Always => "Answer anyway",
    }
}

/// The speed-dial buttons: a list to edit, remove or add to.
fn speed_dial(ui: &mut Ui, app: &mut App, p: &Palette) {
    use rustone_core::settings::{Shortcut, ShortcutAction};
    ui.label(egui::RichText::new("Buttons under the dialer's keypad. With presence on, a dot shows the number's state and a press picks up its ringing call.").size(12.0).color(p.dim));
    if let Some((index, mut draft)) = app.ui.shortcut_draft.take() {
        let mut keep = true;
        widgets::text_field(ui, p, "Label", "e.g. Reception", &mut draft.label);
        widgets::text_field(ui, p, "Number", "Number, SIP address or digits", &mut draft.number);
        widgets::text_field(ui, p, "Second number (optional)", "Pressed again, uses this one", &mut draft.number2);
        ui.add_space(4.0);
        setting_stack(ui, p, "Action", None, |ui| {
            egui::ComboBox::from_id_salt("shortcut-action").width(ui.available_width()).selected_text(shortcut_label(draft.action)).show_ui(ui, |ui| {
                for action in [
                    ShortcutAction::Call,
                    ShortcutAction::VideoCall,
                    ShortcutAction::Message,
                    ShortcutAction::Dtmf,
                    ShortcutAction::Transfer,
                    ShortcutAction::AttendedTransfer,
                    ShortcutAction::Conference,
                ] {
                    ui.selectable_value(&mut draft.action, action, shortcut_label(action));
                }
            });
        });
        setting_row(ui, p, "Presence (BLF)", Some("Watch this number"), |ui| {
            widgets::toggle(ui, p, &mut draft.presence, "Presence (BLF)");
        });
        ui.horizontal(|ui| {
            let ready = !draft.number.trim().is_empty() || matches!(draft.action, ShortcutAction::Transfer);
            if ui.add_enabled(ready, egui::Button::new("Save")).clicked() {
                match index {
                    Some(i) if i < app.settings.shortcuts.len() => app.settings.shortcuts[i] = draft.clone(),
                    _ => app.settings.shortcuts.push(draft.clone()),
                }
                app.mark_settings_dirty();
                keep = false;
            }
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
        if keep {
            app.ui.shortcut_draft = Some((index, draft));
        }
        return;
    }
    let mut remove = None;
    for (i, shortcut) in app.settings.shortcuts.iter().enumerate() {
        ui.horizontal(|ui| {
            let title = if shortcut.label.trim().is_empty() { shortcut.number.clone() } else { shortcut.label.clone() };
            ui.label(egui::RichText::new(title).color(p.text));
            ui.label(egui::RichText::new(format!("{} · {}", shortcut_label(shortcut.action), shortcut.number)).size(12.0).color(p.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, p, Icon::Trash, p.danger, &format!("Remove {}", shortcut.label)).clicked() {
                    remove = Some(i);
                }
                if widgets::icon_button(ui, p, Icon::Pencil, p.secondary, &format!("Edit {}", shortcut.label)).clicked() {
                    app.ui.shortcut_draft = Some((Some(i), shortcut.clone()));
                }
            });
        });
    }
    if let Some(i) = remove {
        app.settings.shortcuts.remove(i);
        app.ui.shortcuts_toggled.clear();
        app.mark_settings_dirty();
    }
    if ui.button("Add a button").clicked() {
        app.ui.shortcut_draft = Some((None, Shortcut::default()));
    }
}

fn shortcut_label(action: rustone_core::settings::ShortcutAction) -> &'static str {
    use rustone_core::settings::ShortcutAction as A;
    match action {
        A::Call => "Call",
        A::VideoCall => "Video call",
        A::Message => "Message",
        A::Dtmf => "Send digits",
        A::Transfer => "Transfer",
        A::AttendedTransfer => "Attended transfer",
        A::Conference => "Add to conference",
    }
}

/// "Start with Windows": off, with the window, or in the tray.
fn start_with_windows(ui: &mut Ui, app: &mut App, p: &Palette) {
    use fastframe_autostart::{Start, State};
    let state = app.start_at_login.state();
    if state == Some(State::Unsupported) {
        return;
    }
    let label = |start: Option<Start>| match start {
        None => "Off",
        Some(Start::Window) => "With the window",
        Some(Start::Hidden) => "In the tray",
    };
    setting_stack(ui, p, "Start with Windows", Some("When you sign in"), |ui| {
        let current = state.and_then(State::start);
        let text = if state.is_none() { "Checking…" } else { label(current) };
        ui.add_enabled_ui(state.is_some(), |ui| {
            egui::ComboBox::from_id_salt("start-with-windows").width(ui.available_width()).selected_text(text).show_ui(ui, |ui| {
                for start in [None, Some(Start::Window), Some(Start::Hidden)] {
                    if ui.selectable_label(current == start, label(start)).clicked() && current != start {
                        app.start_at_login.set(start);
                    }
                }
            });
        });
    });
}

fn deny_label(mode: DenyIncoming) -> &'static str {
    match mode {
        DenyIncoming::Off => "None",
        DenyIncoming::All => "All calls",
        DenyIncoming::OtherUser => "Calls to another user",
        DenyIncoming::OtherDomain => "Calls to another domain",
        DenyIncoming::RemoteDomain => "Calls from another domain",
        DenyIncoming::OtherUserOrDomain => "Calls to another user or domain",
    }
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
    let mut preview_ring = false;
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
        ui.separator();
        setting_stack(ui, p, "Ringtone volume", None, |ui| {
            let level = widgets::level_slider(ui, p, (Icon::Bell, Icon::BellOff), "Ringtone volume", &mut audio.volume_ring, false, None);
            changed |= level.changed;
            preview_ring |= level.released;
        });
        ui.separator();
        for (value, title, description) in [
            (&mut audio.mic_amplification, "Microphone boost", Some("The upper half of the microphone slider amplifies")),
            (&mut audio.sw_level_adjustment, "Software microphone level", Some("Leave the device's own volume alone")),
        ] {
            setting_row(ui, p, title, description, |ui| {
                changed |= widgets::toggle(ui, p, value, title).changed();
            });
        }
    });
    if changed {
        app.mark_settings_dirty();
    }
    // Play the ringtone once at the new volume.
    if preview_ring && !app.sip.calls.has_calls() {
        let level = f32::from(app.settings.audio.volume_ring.min(100)) / 100.0;
        let path = app.sip.calls.sounds.ringtone.clone();
        app.sip.send(sip_engine::Command::PlaySound(sip_engine::Sound { path, looping: false, level, ring: true }));
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

/// EC, VAD, Opus stereo and forceCodec (the checkboxes under the
/// codec lists). All but forceCodec restart the engine once no call is live.
fn processing(ui: &mut Ui, app: &mut App, p: &Palette, available: &[String]) {
    let sip = &mut app.settings.sip;
    let mut changed = false;
    let has_opus = available.iter().any(|c| c.to_ascii_lowercase().starts_with("opus/"));
    let mut rows = vec![
        (&mut sip.ec, "Echo cancellation", Some("Stops the other side hearing themselves through your speaker")),
        (&mut sip.vad, "Silence suppression", Some("Sends less data while nobody talks (VAD)")),
        (&mut sip.force_codec, "Use my codec order for incoming calls", Some("Otherwise the caller's preferred codec wins")),
    ];
    if has_opus {
        rows.push((&mut sip.opus_stereo, "Opus in stereo", Some("Turns echo cancellation off")));
    }
    for (value, title, description) in rows {
        setting_row(ui, p, title, description, |ui| {
            changed |= widgets::toggle(ui, p, value, title).changed();
        });
    }
    // Saving warns the same way.
    let stereo = sip.audio_codecs.split_whitespace().any(|c| !c.to_ascii_lowercase().starts_with("opus/") && c.ends_with("/2"));
    if stereo && sip.ec {
        ui.label(egui::RichText::new("Echo cancellation turns the stereo codecs mono.").size(12.0).color(p.warning));
    }
    if changed {
        app.mark_settings_dirty();
    }
}

/// Camera, bitrate and a preview. Changes apply
/// when the engine restarts, once no call is live.
fn video(ui: &mut Ui, app: &mut App, p: &Palette) {
    let cameras: Vec<String> = app.sip.video_devices.iter().filter(|d| d.capture).map(|d| d.name.clone()).collect();
    let mut changed = false;
    card(ui, p, |ui| {
        setting_stack(ui, p, "Camera", None, |ui| {
            let camera = &mut app.settings.video.camera;
            let missing = !camera.is_empty() && !cameras.contains(camera) && !cameras.is_empty();
            let selected = match (camera.is_empty(), missing) {
                (true, _) => "System default".to_owned(),
                (false, true) => format!("{camera} (not connected)"),
                (false, false) => camera.clone(),
            };
            let combo = egui::ComboBox::from_id_salt("camera").width(ui.available_width()).selected_text(selected).truncate().show_ui(ui, |ui| {
                changed |= ui.selectable_value(camera, String::new(), "System default").changed();
                for name in &cameras {
                    changed |= ui.selectable_value(camera, name.clone(), name).changed();
                }
            });
            ui.ctx().accesskit_node_builder(combo.response.id, |node| node.set_label("Camera"));
        });
        ui.separator();
        changed |= widgets::number_field(ui, p, "Bitrate (kbit/s)", &mut app.settings.video.bitrate_kbps, 0..=8000, " kbit/s").changed();
        ui.label(egui::RichText::new("0 keeps H.264's default (256 kbit/s).").size(12.0).color(p.dim));
        ui.separator();
        setting_row(ui, p, "Preview", Some("Check the camera"), |ui| {
            let label = if app.sip.preview { "Stop" } else { "Show" };
            if ui.button(label).clicked() {
                app.sip.preview = !app.sip.preview;
                app.sip.send(sip_engine::Command::SetPreview(app.sip.preview));
            }
        });
        if app.sip.preview {
            let frames = app.sip.video();
            crate::ui::video::preview(ui, &mut app.ui.video, &frames, p);
        }
        if app.sip.calls.live_count() > 0 {
            ui.label(egui::RichText::new("Changes apply after the current call.").size(12.0).color(p.warning));
        }
    });
    if changed {
        app.mark_settings_dirty();
    }
}

// ---------------------------------------------------------------------------
// The settings window: tabs of sections, in as many columns as fit.

/// The settings sections, in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Accounts,
    Calls,
    AutoAnswer,
    Forwarding,
    FeatureCodes,
    SpeedDial,
    AudioDevices,
    AudioCodecs,
    AudioProcessing,
    Video,
    Recording,
    UsersDirectory,
    Features,
    Behaviour,
    Appearance,
    CommandHooks,
    Developer,
    About,
}

/// The settings window's tabs; each fits its window without scrolling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    #[default]
    General,
    Calls,
    AudioVideo,
}

impl Tab {
    const ALL: [Self; 3] = [Self::General, Self::Calls, Self::AudioVideo];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Calls => "Calls",
            Self::AudioVideo => "Audio & video",
        }
    }
}

impl Section {
    fn tab(self) -> Tab {
        match self {
            Self::Accounts | Self::UsersDirectory | Self::Features | Self::Behaviour | Self::Appearance | Self::Developer | Self::About => Tab::General,
            Self::Calls | Self::AutoAnswer | Self::Forwarding | Self::FeatureCodes | Self::SpeedDial | Self::Recording | Self::CommandHooks => Tab::Calls,
            Self::AudioDevices | Self::AudioCodecs | Self::AudioProcessing | Self::Video => Tab::AudioVideo,
        }
    }

    const ALL: [Self; 18] = [
        Self::Accounts,
        Self::Calls,
        Self::AutoAnswer,
        Self::Forwarding,
        Self::FeatureCodes,
        Self::SpeedDial,
        Self::AudioDevices,
        Self::AudioCodecs,
        Self::AudioProcessing,
        Self::Video,
        Self::Recording,
        Self::UsersDirectory,
        Self::Features,
        Self::Behaviour,
        Self::Appearance,
        Self::CommandHooks,
        Self::Developer,
        Self::About,
    ];

    fn title(self) -> Option<&'static str> {
        Some(match self {
            // The accounts list has its own heading.
            Self::Accounts => return None,
            Self::Calls => "Calls",
            Self::AutoAnswer => "Auto-answer",
            Self::Forwarding => "Forwarding",
            Self::FeatureCodes => "Feature codes",
            Self::SpeedDial => "Speed dial",
            Self::AudioDevices => "Audio devices",
            Self::AudioCodecs => "Audio codecs",
            Self::AudioProcessing => "Audio processing",
            Self::Video => "Video",
            Self::Recording => "Recording",
            Self::UsersDirectory => "Users directory",
            Self::Features => "Features",
            Self::Behaviour => "Behaviour",
            Self::Appearance => "Appearance",
            Self::CommandHooks => "Command hooks",
            Self::Developer => "Developer",
            Self::About => "About",
        })
    }

    /// A first guess at its height, until it has been drawn once.
    fn estimate(self) -> f32 {
        match self {
            Self::Calls | Self::CommandHooks => 560.0,
            Self::AudioCodecs => 460.0,
            Self::AutoAnswer | Self::AudioDevices | Self::Behaviour => 340.0,
            Self::Video | Self::Recording | Self::Features | Self::FeatureCodes | Self::AudioProcessing => 260.0,
            Self::Accounts | Self::Forwarding | Self::Appearance => 200.0,
            Self::SpeedDial | Self::UsersDirectory => 140.0,
            Self::Developer | Self::About => 80.0,
        }
    }

    fn draw(self, ui: &mut Ui, app: &mut App, p: &Palette) {
        if let Some(title) = self.title() {
            section_title(ui, p, title);
        }
        match self {
            Self::Accounts => super::accounts::list(ui, app, p),
            Self::Calls => calls(ui, app, p),
            Self::AutoAnswer => auto_answer(ui, app, p),
            Self::Forwarding => forwarding(ui, app, p),
            Self::FeatureCodes => feature_codes(ui, app, p),
            Self::SpeedDial => speed_dial_section(ui, app, p),
            Self::AudioDevices => audio_devices(ui, app, p),
            Self::AudioCodecs => codecs(ui, app, p),
            Self::AudioProcessing => {
                let available: Vec<String> = app.sip.codecs.iter().map(|c| c.id.clone()).collect();
                card(ui, p, |ui| processing(ui, app, p, &available));
            }
            Self::Video => video(ui, app, p),
            Self::Recording => recording(ui, app, p),
            Self::UsersDirectory => users_directory(ui, app, p),
            Self::Features => features(ui, app, p),
            Self::Behaviour => behaviour(ui, app, p),
            Self::Appearance => appearance(ui, app, p),
            Self::CommandHooks => command_hooks(ui, app, p),
            Self::Developer => developer(ui, app, p),
            Self::About => about(ui, app, p),
        }
    }
}

/// Narrowest a column gets.
const COLUMN_MIN: f32 = 300.0;
const COLUMN_GAP: f32 = 20.0;

/// Which section goes in which column. Placed by measured heights and then
/// kept, so sections don't move while their contents change; placed again
/// when the number of columns changes.
#[derive(Debug, Default)]
pub struct Layout {
    columns: Vec<Vec<Section>>,
    heights: std::collections::HashMap<Section, f32>,
    /// Placed with measured heights (not estimates).
    settled: bool,
}

impl Layout {
    fn place(&mut self, sections: &[Section], count: usize) {
        let mut columns = vec![Vec::new(); count];
        let mut filled = vec![0.0f32; count];
        for &section in sections {
            // The shortest column so far takes it (masonry, in order).
            let (i, _) = filled.iter().enumerate().fold((0, f32::MAX), |best, (i, &h)| if h < best.1 { (i, h) } else { best });
            columns[i].push(section);
            filled[i] += self.heights.get(&section).copied().unwrap_or_else(|| section.estimate()) + COLUMN_GAP;
        }
        self.columns = columns;
    }
}

/// A tab's sections, in columns as the width allows.
fn sections(ui: &mut Ui, app: &mut App, p: &Palette, tab: Tab) {
    let shown: Vec<Section> = Section::ALL.into_iter().filter(|s| s.tab() == tab && (*s != Section::Video || app.settings.features.video)).collect();
    let width = ui.available_width();
    let count = (((width + COLUMN_GAP) / (COLUMN_MIN + COLUMN_GAP)).floor() as usize).clamp(1, shown.len());
    let mut layout = app.ui.settings_layout.remove(&tab).unwrap_or_default();
    let measured = shown.iter().all(|s| layout.heights.contains_key(s));
    let flat: Vec<Section> = layout.columns.iter().flatten().copied().collect();
    let same_sections = flat.len() == shown.len() && shown.iter().all(|s| flat.contains(s));
    if layout.columns.len() != count || !same_sections {
        layout.place(&shown, count);
        layout.settled = measured;
    } else if !layout.settled && measured {
        layout.place(&shown, count);
        layout.settled = true;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.columns(count, |cols| {
            for (col, sections) in cols.iter_mut().zip(&layout.columns) {
                for &section in sections {
                    let drawn = col.scope(|ui| section.draw(ui, app, p));
                    layout.heights.insert(section, drawn.response.rect.height());
                    col.add_space(COLUMN_GAP);
                }
            }
        });
    });
    app.ui.settings_layout.insert(tab, layout);
}

/// The settings window's size when it opens, in points.
const DEFAULT_SIZE: [f32; 2] = [990.0, 618.0];

/// The settings window's id, to open or bring it forward.
pub fn window_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("rustone-settings")
}

/// Shows the settings window while it's open: the sections, or the account
/// editor while an account is being edited. Called every frame of the main
/// window, from `logic`.
pub fn show_window(ctx: &egui::Context, app: &mut App) {
    if !app.ui.settings_open {
        return;
    }
    let p = app.palette().clone();
    // The size chosen for it (three columns at 125%), smaller on a
    // screen that can't hold it.
    let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1600.0, 900.0));
    let size = [DEFAULT_SIZE[0].min(monitor.x * 0.95), DEFAULT_SIZE[1].min(monitor.y * 0.9)];
    let builder = egui::ViewportBuilder::default()
        .with_title("RusTone Settings")
        .with_app_id("rustone-settings")
        .with_icon(crate::icons::window_icon())
        .with_inner_size(size)
        .with_min_inner_size([420.0, 480.0]);
    ctx.show_viewport_immediate(window_id(), builder, |ui, _class| {
        if ui.input(|i| i.viewport().close_requested()) {
            app.ui.settings_open = false;
            return;
        }
        let frame = egui::Frame::new().fill(p.window).inner_margin(egui::Margin::symmetric(20, 16));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if app.ui.account_draft.is_some() {
                // The editor, in a column of its own width.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_max_width(560.0);
                    super::accounts::editor(ui, app, &p);
                });
            } else {
                ui.horizontal(|ui| {
                    ui.heading("Settings");
                    ui.add_space(24.0);
                    // Tabs, each fitting the window.
                    let mut tab = app.ui.settings_tab;
                    for t in Tab::ALL {
                        let selected = tab == t;
                        let text = egui::RichText::new(t.label()).size(14.0).color(if selected { p.accent } else { p.secondary });
                        if ui.add(egui::Button::new(text).selected(selected).min_size(egui::vec2(0.0, 30.0))).clicked() {
                            tab = t;
                        }
                    }
                    app.ui.settings_tab = tab;
                });
                ui.add_space(10.0);
                let tab = app.ui.settings_tab;
                sections(ui, app, &p, tab);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_go_to_the_shortest_column_in_order() {
        let mut layout = Layout::default();
        for (section, height) in [(Section::Calls, 500.0), (Section::AutoAnswer, 300.0), (Section::Forwarding, 200.0), (Section::SpeedDial, 100.0)] {
            layout.heights.insert(section, height);
        }
        layout.place(&[Section::Calls, Section::AutoAnswer, Section::Forwarding, Section::SpeedDial], 2);
        // Calls fills the first column; the others stack in the second until it's as tall.
        assert_eq!(layout.columns, vec![vec![Section::Calls, Section::SpeedDial], vec![Section::AutoAnswer, Section::Forwarding]]);
        layout.place(&[Section::Calls, Section::AutoAnswer], 1);
        assert_eq!(layout.columns, vec![vec![Section::Calls, Section::AutoAnswer]]);
    }

    #[test]
    fn every_section_has_a_tab() {
        for tab in Tab::ALL {
            assert!(Section::ALL.iter().any(|s| s.tab() == tab), "{tab:?} is empty");
        }
    }
}
