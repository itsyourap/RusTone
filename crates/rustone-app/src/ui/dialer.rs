//! The keypad dialer. Calling is wired up in Phase 3; the call buttons are
//! disabled until the account is registered.

use egui::{Align, Layout, Ui, Vec2};
use fastframe_fonts::Weight;

use super::widgets;
use crate::icons::Icon;
use crate::theme::Palette;

const KEYS: [(&str, &str); 12] = [
    ("1", ""),
    ("2", "ABC"),
    ("3", "DEF"),
    ("4", "GHI"),
    ("5", "JKL"),
    ("6", "MNO"),
    ("7", "PQRS"),
    ("8", "TUV"),
    ("9", "WXYZ"),
    ("*", ""),
    ("0", "+"),
    ("#", ""),
];
const KEY_SIZE: f32 = 64.0;
const KEY_GAP: f32 = 18.0;

#[derive(Debug, Default)]
pub struct DialerState {
    pub number: String,
}

/// What the user asked the dialer for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Call(String),
    /// A keypad key, for local tone feedback.
    Key(String),
}

/// Characters a dial string may contain (digits, DTMF symbols, a leading +),
/// plus what a SIP URI needs when typed or pasted.
fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || "*#+@.:-_;=".contains(c)
}

pub fn show(ui: &mut Ui, p: &Palette, state: &mut DialerState, registered: bool) -> Option<Action> {
    let mut action = None;

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        let keypad_height = 4.0 * KEY_SIZE + 3.0 * KEY_GAP;
        let content_height = 64.0 + 24.0 + keypad_height + 28.0 + KEY_SIZE;
        ui.add_space(((ui.available_height() - content_height) / 2.0).max(8.0));

        if number_field(ui, p, state) && registered && !state.number.is_empty() {
            action = Some(Action::Call(state.number.clone()));
        }
        ui.add_space(24.0);

        for row in KEYS.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = KEY_GAP;
                centre_row(ui, 3);
                for (digit, letters) in row {
                    if widgets::dial_key(ui, p, KEY_SIZE, digit, letters).clicked() {
                        state.number.push_str(digit);
                        action = Some(Action::Key((*digit).to_owned()));
                    }
                }
            });
            ui.add_space(KEY_GAP - ui.spacing().item_spacing.y);
        }

        ui.add_space(28.0 - KEY_GAP);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            centre_row(ui, 3);
            let reason = if registered { "" } else { " (not connected)" };
            ui.add_enabled_ui(registered && !state.number.is_empty(), |ui| {
                // Video calls arrive in Phase 7.
                ui.add_enabled_ui(false, |ui| {
                    widgets::round_icon_button(ui, KEY_SIZE, p.surface, Icon::Video, p.text, "Video call (coming later)");
                });
                if widgets::round_icon_button(ui, KEY_SIZE, p.success, Icon::Phone, p.on_accent, &format!("Call{reason}")).clicked() {
                    action = Some(Action::Call(state.number.clone()));
                }
            });
            ui.add_enabled_ui(!state.number.is_empty(), |ui| {
                let erase = widgets::round_icon_button(ui, KEY_SIZE, p.window, Icon::Backspace, p.secondary, "Delete (hold to clear)");
                if erase.long_touched() || erase.secondary_clicked() {
                    state.number.clear();
                } else if erase.clicked() {
                    state.number.pop();
                }
            });
        });
    });
    action
}

/// Indents a row of `keys` keypad buttons so it's centred.
fn centre_row(ui: &mut Ui, keys: usize) {
    let width = keys as f32 * KEY_SIZE + (keys - 1) as f32 * KEY_GAP;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
}

/// Returns true when Enter was pressed in the field.
fn number_field(ui: &mut Ui, p: &Palette, state: &mut DialerState) -> bool {
    let size = if state.number.chars().count() > 14 { 22.0 } else { 30.0 };
    let edit = egui::TextEdit::singleline(&mut state.number)
        .font(Weight::Medium.font_id(size))
        .hint_text(egui::RichText::new("Enter a number").font(Weight::Regular.font_id(22.0)).color(p.dim))
        .horizontal_align(Align::Center)
        .frame(egui::Frame::NONE)
        .desired_width(ui.available_width().min(320.0))
        .min_size(Vec2::new(0.0, 64.0))
        .vertical_align(Align::Center);
    let response = ui.add(edit);
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label("Number");
        node.add_action(egui::accesskit::Action::SetValue);
    });
    // Screen readers and automation can set the number directly.
    let set_value = ui.input(|i| {
        i.accesskit_action_requests(response.id, egui::accesskit::Action::SetValue)
            .filter_map(|r| match &r.data {
                Some(egui::accesskit::ActionData::Value(v)) => Some(v.to_string()),
                _ => None,
            })
            .last()
    });
    if let Some(value) = set_value {
        state.number = value;
        state.number.retain(allowed);
    }
    if response.changed() {
        state.number.retain(allowed);
    }
    let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    // Typing anywhere in the dialer goes into the number.
    if !response.has_focus() && ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
    entered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_strings_keep_digits_and_uri_characters() {
        let mut s = String::from("+91 (555) 010-0002\tsip:1234@host;x=y");
        s.retain(allowed);
        assert_eq!(s, "+91555010-0002sip:1234@host;x=y");
    }
}
