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

/// The number field's id, for reading and moving its cursor.
fn number_id() -> egui::Id {
    egui::Id::new("dialer-number")
}
const KEY_GAP: f32 = 18.0;
/// Seconds the backspace button must be held to clear the whole number.
const HOLD_TO_CLEAR: f64 = 0.6;

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
                        edit_at_cursor(ui, &mut state.number, |text, sel| insert(text, sel, digit));
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
                if erase.long_touched() || erase.secondary_clicked() || held_long(ui, &erase) {
                    edit_at_cursor(ui, &mut state.number, |_, _| (String::new(), 0));
                } else if erase.clicked() {
                    edit_at_cursor(ui, &mut state.number, delete_back);
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

/// True once the mouse has been held down on `response` for
/// [`HOLD_TO_CLEAR`] seconds. `long_touched` only covers touch screens.
fn held_long(ui: &Ui, response: &egui::Response) -> bool {
    if !response.is_pointer_button_down_on() {
        return false;
    }
    let held = ui.input(|i| i.pointer.press_start_time().map_or(0.0, |t| i.time - t));
    if held < HOLD_TO_CLEAR {
        // Wake up when the hold is long enough, even if the mouse is still.
        ui.ctx().request_repaint_after_secs((HOLD_TO_CLEAR - held) as f32);
    }
    held >= HOLD_TO_CLEAR
}

/// Applies a keypad edit at the number field's cursor (or selection), then
/// puts the cursor after the edit and gives the field focus back. With no
/// cursor yet, edits happen at the end.
fn edit_at_cursor(ui: &mut Ui, number: &mut String, edit: impl FnOnce(&str, Option<(usize, usize)>) -> (String, usize)) {
    let id = number_id();
    let mut text_state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    let selection = text_state.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0));
    let (text, cursor) = edit(number, selection);
    *number = text;
    text_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
    text_state.store(ui.ctx(), id);
    ui.memory_mut(|m| m.request_focus(id));
}

/// The selection as a char range clamped to `text`; no cursor means the end.
fn span(text: &str, selection: Option<(usize, usize)>) -> (usize, usize) {
    let len = text.chars().count();
    match selection {
        Some((a, b)) => (a.min(b).min(len), a.max(b).min(len)),
        None => (len, len),
    }
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

/// Inserts `insert` at the cursor (replacing a selection). Returns the new
/// text and the cursor after the insertion.
fn insert(text: &str, selection: Option<(usize, usize)>, insert: &str) -> (String, usize) {
    let (start, end) = span(text, selection);
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..byte_index(text, start)]);
    out.push_str(insert);
    out.push_str(&text[byte_index(text, end)..]);
    (out, start + insert.chars().count())
}

/// Deletes the selection, or the character before the cursor.
fn delete_back(text: &str, selection: Option<(usize, usize)>) -> (String, usize) {
    let (start, end) = span(text, selection);
    let (from, to) = if start == end { (start.saturating_sub(1), end) } else { (start, end) };
    let mut out = text[..byte_index(text, from)].to_owned();
    out.push_str(&text[byte_index(text, to)..]);
    (out, from)
}

/// Returns true when Enter was pressed in the field.
fn number_field(ui: &mut Ui, p: &Palette, state: &mut DialerState) -> bool {
    let size = if state.number.chars().count() > 14 { 22.0 } else { 30.0 };
    let edit = egui::TextEdit::singleline(&mut state.number)
        .id(number_id())
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
    use egui_kittest::kittest::Queryable;

    /// The user's example: 1, 2, click between them, 3 → "132", cursor after 3.
    #[test]
    fn keypad_inserts_at_the_cursor() {
        let (t, c) = insert("", None, "1");
        assert_eq!((t.as_str(), c), ("1", 1));
        let (t, c) = insert(&t, Some((c, c)), "2");
        assert_eq!((t.as_str(), c), ("12", 2));
        let (t, c) = insert(&t, Some((1, 1)), "3");
        assert_eq!((t.as_str(), c), ("132", 2));
        // A selection is replaced.
        assert_eq!(insert("12345", Some((4, 1)), "9"), ("195".to_owned(), 2));
        // A stale cursor past the end is clamped.
        assert_eq!(insert("12", Some((7, 7)), "3"), ("123".to_owned(), 3));
    }

    /// The same example through the real widgets, with pointer clicks.
    #[test]
    fn clicking_between_digits_moves_the_insertion_point() {
        let mut harness = dialer_harness();
        harness.get_by_label("1").click();
        harness.run();
        harness.get_by_label("2").click();
        harness.run();
        assert_eq!(harness.state().number, "12");
        // "12" is centred, so the field's centre is between the digits.
        harness.get_by_label("Number").click();
        harness.run();
        harness.get_by_label("3").click();
        harness.run();
        assert_eq!(harness.state().number, "132");
        let cursor = egui::text_edit::TextEditState::load(&harness.ctx, number_id()).and_then(|s| s.cursor.char_range());
        assert_eq!(cursor.map(|r| r.primary.index.0), Some(2), "cursor after the 3");
    }

    #[test]
    fn backspace_click_deletes_one_and_holding_clears() {
        let mut harness = dialer_harness();
        for key in ["1", "2", "3"] {
            harness.get_by_label(key).click();
            harness.run();
        }
        harness.get_by_label("Delete (hold to clear)").click();
        harness.run();
        assert_eq!(harness.state().number, "12");

        let pos = harness.get_by_label("Delete (hold to clear)").rect().center();
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        harness.hover_at(pos);
        harness.event(button(true));
        harness.step();
        assert_eq!(harness.state().number, "12", "a short press doesn't clear yet");
        harness.run_steps(4); // 1 s at the harness's 0.25 s per step
        assert_eq!(harness.state().number, "", "cleared while still held");
        harness.event(button(false));
        harness.run();
        assert_eq!(harness.state().number, "");
    }

    /// The dialer in a test harness, with the app's fonts and icons.
    fn dialer_harness() -> egui_kittest::Harness<'static, DialerState> {
        let mut ready = false;
        let mut harness = egui_kittest::Harness::builder().with_size([475.0, 760.0]).build_ui_state(
            move |ui, state: &mut DialerState| {
                if !ready {
                    // The fastframe fonts and icons only exist from the next frame.
                    ui.ctx().set_fonts(fastframe_fonts::FontSetup::default().definitions());
                    fastframe_icons::install::<Icon>(ui.ctx());
                    ui.ctx().request_repaint();
                    ready = true;
                    return;
                }
                show(ui, &Palette::dark(), state, true);
            },
            DialerState::default(),
        );
        harness.run();
        harness
    }

    #[test]
    fn backspace_deletes_before_the_cursor() {
        assert_eq!(delete_back("132", Some((2, 2))), ("12".to_owned(), 1));
        assert_eq!(delete_back("132", Some((0, 0))), ("132".to_owned(), 0));
        assert_eq!(delete_back("132", None), ("13".to_owned(), 2));
        assert_eq!(delete_back("12345", Some((1, 4))), ("15".to_owned(), 1));
        assert_eq!(delete_back("+٣٤", Some((2, 2))), ("+٤".to_owned(), 1), "char-based, not bytes");
    }

    #[test]
    fn dial_strings_keep_digits_and_uri_characters() {
        let mut s = String::from("+91 (555) 010-0002\tsip:1234@host;x=y");
        s.retain(allowed);
        assert_eq!(s, "+91555010-0002sip:1234@host;x=y");
    }
}
