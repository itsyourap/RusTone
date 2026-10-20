//! The contacts page and the contact editor
//! (its Add/Edit dialog).

use egui::{Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use rustone_core::contacts::Contact;
use sip_engine::{Presence, PresenceState};

use super::widgets::{self, card, section_title, setting_row};
use super::Page;
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

/// The list's state.
#[derive(Debug, Default)]
pub struct ContactsView {
    pub query: String,
    /// The contact whose actions are showing.
    pub expanded: Option<i64>,
    /// Delete was clicked once for this contact.
    pub confirm_delete: Option<i64>,
}

/// A contact being edited. `contact.id == 0` means a new one.
#[derive(Debug, Default)]
pub struct Draft {
    pub contact: Contact,
    /// The name as last composed from first and last name: while the name
    /// still equals it, editing those fields keeps composing it.
    composed: String,
    /// The phone as last seen: while the number equals it, it follows the phone.
    last_phone: String,
    confirm_overwrite: bool,
    confirm_delete: bool,
    error: Option<String>,
}

impl Draft {
    pub fn new(contact: Contact) -> Self {
        let composed = compose(&contact.first_name, &contact.last_name);
        let last_phone = contact.phone.clone();
        Self { contact, composed, last_phone, ..Self::default() }
    }

    /// Auto-fill: name from first + last name, number from phone.
    fn follow_fields(&mut self) {
        let c = &mut self.contact;
        let composed = compose(&c.first_name, &c.last_name);
        if c.name.trim() == self.composed {
            c.name = composed.clone();
        }
        self.composed = composed;
        if c.number.trim() == self.last_phone.trim() {
            c.number = c.phone.clone();
        }
        self.last_phone = c.phone.clone();
    }
}

/// "First Last" (just one if they're the same).
fn compose(first: &str, last: &str) -> String {
    let (first, last) = (first.trim(), last.trim());
    if first == last { first.to_owned() } else { format!("{first} {last}").trim().to_owned() }
}

enum RowAction {
    None,
    Toggle,
    Call(String),
    Star,
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    // Finish an import or export once its file dialog closes.
    if app.phonebook.dialog_open() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some(message) = app.phonebook.poll_dialog() {
        app.notify(message);
    }

    ui.horizontal(|ui| {
        let buttons = 2.0 * 38.0;
        ui.add_sized(
            vec2((ui.available_width() - buttons).max(80.0), 34.0),
            egui::TextEdit::singleline(&mut app.ui.contacts.query).hint_text("Search contacts").margin(egui::Margin::symmetric(10, 8)),
        );
        if widgets::icon_button(ui, p, Icon::Plus, p.text, "Add contact").clicked() {
            app.ui.contact_draft = Some(Draft::new(Contact::default()));
            app.ui.page = Page::Contact;
        }
        let more = widgets::icon_button(ui, p, Icon::More, p.text, "More");
        egui::Popup::menu(&more).show(|ui| {
            if ui.button("Import contacts (.vcf)…").clicked() {
                app.phonebook.pick_import();
                ui.close();
            }
            if ui.button("Export contacts (.vcf)…").clicked() {
                app.phonebook.pick_export();
                ui.close();
            }
        });
    });
    ui.add_space(6.0);

    let query = app.ui.contacts.query.trim().to_owned();
    let shown: Vec<Contact> = app.phonebook.contacts.iter().filter(|c| c.matches(&query)).cloned().collect();
    if app.phonebook.contacts.is_empty() {
        widgets::empty_state(ui, p, Icon::Users, "No contacts", "Add contacts with + or import a .vcf file from the ⋯ menu.");
        return;
    }
    if shown.is_empty() {
        ui.label(egui::RichText::new(format!("No contacts match “{query}”")).color(p.dim));
        return;
    }

    let mut action: Option<(Contact, RowAction)> = None;
    let pickup_code = app.settings.calls.pickup_code.trim().to_owned();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for contact in &shown {
            let expanded = app.ui.contacts.expanded == Some(contact.id);
            let presence = contact.presence.then(|| app.sip.presence.get(contact.number.trim()).cloned()).flatten();
            let row = contact_row(ui, p, contact, presence.as_ref(), expanded);
            match row {
                RowAction::None => {}
                other => action = Some((contact.clone(), other)),
            }
            let ringing = presence.as_ref().is_some_and(|pr| pr.state == PresenceState::Ringing);
            let pickup = (ringing && !pickup_code.is_empty()).then(|| pickup_number(&pickup_code, &contact.number));
            if expanded && let Some(a) = row_actions(ui, app, p, contact, pickup) {
                action = Some((contact.clone(), a));
            }
        }
    });

    match action {
        Some((c, RowAction::Toggle)) => {
            app.ui.contacts.expanded = if app.ui.contacts.expanded == Some(c.id) { None } else { Some(c.id) };
            app.ui.contacts.confirm_delete = None;
        }
        Some((_, RowAction::Call(number))) => {
            super::dial(app, &number);
            app.ui.page = Page::Dialer;
        }
        Some((mut c, RowAction::Star)) => {
            c.starred = !c.starred;
            if let Err(e) = app.phonebook.save(&c) {
                app.notify(format!("Couldn't save {}: {e}", c.title()));
            }
        }
        Some((_, RowAction::None)) | None => {}
    }
}

/// The presence dot's colour and words; `None` while unknown.
fn presence_look(p: &Palette, state: PresenceState) -> Option<(egui::Color32, &'static str)> {
    match state {
        PresenceState::Unknown => None,
        PresenceState::Offline => Some((p.dim, "Offline")),
        PresenceState::Online => Some((p.success, "Online")),
        PresenceState::Away => Some((p.warning, "Away")),
        PresenceState::Busy => Some((p.danger, "Busy")),
        PresenceState::OnThePhone => Some((p.danger, "On the phone")),
        PresenceState::Ringing => Some((p.danger, "Ringing")),
    }
}

/// Call pickup: the code in front of the user part (`**101`).
fn pickup_number(code: &str, number: &str) -> String {
    let n = number.trim();
    let n = n.strip_prefix("sip:").unwrap_or(n);
    format!("{code}{n}")
}

fn contact_row(ui: &mut Ui, p: &Palette, c: &Contact, presence: Option<&Presence>, expanded: bool) -> RowAction {
    let title = c.title();
    let look = presence.and_then(|pr| presence_look(p, pr.state));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let label = match look {
        Some((_, words)) => format!("{title}, {}, {words}", c.number),
        None => format!("{title}, {}", c.number),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered() || expanded, 0.12);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(10), p.surface_hover.gamma_multiply(hover));
    }
    let avatar = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(36.0));
    widgets::avatar_at(ui, p, avatar, &title);
    if let (Some(pr), Some((color, _))) = (presence, look) {
        // Ringing blinks every 500 ms.
        let lit = if pr.state == PresenceState::Ringing {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0
        } else {
            true
        };
        let dot = avatar.right_bottom() - vec2(5.0, 5.0);
        ui.painter().circle_filled(dot, 6.5, p.window);
        if lit {
            ui.painter().circle_filled(dot, 4.5, color);
        }
    }
    let text_x = rect.left() + 54.0;
    let text_width = rect.right() - 44.0 - text_x;
    let name = widgets::elided(ui, title.clone(), Weight::Medium.font_id(14.0), p.text, text_width);
    ui.painter().galley(egui::pos2(text_x, rect.top() + 9.0), name, p.text);
    let subtitle = match look {
        Some((_, words)) => format!("{} · {words}", c.number),
        None => c.number.clone(),
    };
    let number = widgets::elided(ui, subtitle, Weight::Regular.font_id(12.0), p.secondary, text_width);
    ui.painter().galley(egui::pos2(text_x, rect.top() + 29.0), number, p.secondary);

    let star_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 22.0, rect.center().y), Vec2::splat(34.0));
    let star_label = if c.starred { format!("Unstar {title}") } else { format!("Star {title}") };
    let tint = if c.starred { p.warning } else { p.dim.gamma_multiply(if response.hovered() { 1.0 } else { 0.0 }) };
    let starred = ui.put(star_rect, |ui: &mut Ui| widgets::icon_button(ui, p, Icon::Star, tint, &star_label)).clicked();
    if c.starred {
        filled_star(ui, star_rect.center(), 8.0, p.warning);
    }

    let mut picked = None;
    response.context_menu(|ui| {
        for (label, number) in call_targets(c) {
            if ui.button(label).clicked() {
                picked = Some(number);
            }
        }
    });
    if let Some(number) = picked {
        RowAction::Call(number)
    } else if starred {
        RowAction::Star
    } else if response.double_clicked() {
        RowAction::Call(c.number.clone())
    } else if response.clicked() {
        RowAction::Toggle
    } else {
        RowAction::None
    }
}

/// A filled five-pointed star (the icon is only an outline).
fn filled_star(ui: &Ui, center: egui::Pos2, radius: f32, color: egui::Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..10 {
        let r = if i % 2 == 0 { radius } else { radius * 0.45 };
        let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        mesh.colored_vertex(center + r * egui::vec2(angle.cos(), angle.sin()), color);
    }
    // A fan from the centre: the star is star-shaped around it.
    for i in 1..=10u32 {
        mesh.add_triangle(0, i, i % 10 + 1);
    }
    ui.painter().add(mesh);
}

/// "Call", plus "Call phone"/"Call mobile" when they differ from the number.
fn call_targets(c: &Contact) -> Vec<(String, String)> {
    let mut targets = vec![(format!("Call {}", c.number.trim()), c.number.trim().to_owned())];
    for (kind, number) in [("phone", &c.phone), ("mobile", &c.mobile)] {
        let number = number.trim();
        if !number.is_empty() && number != c.number.trim() {
            targets.push((format!("Call {kind} {number}"), number.to_owned()));
        }
    }
    targets
}

/// The expanded row's buttons.
fn row_actions(ui: &mut Ui, app: &mut App, p: &Palette, c: &Contact, pickup: Option<String>) -> Option<RowAction> {
    let mut action = None;
    // Lined up under the name, wrapping as needed.
    egui::Frame::new().inner_margin(egui::Margin { left: 54, right: 0, top: 2, bottom: 0 }).show(ui, |ui| ui.horizontal_wrapped(|ui| {
        if let Some(number) = pickup
            && ui.add(egui::Button::new(egui::RichText::new("Pick up").color(p.on_accent)).fill(p.warning)).on_hover_text(format!("Dial {number}")).clicked()
        {
            action = Some(RowAction::Call(number));
        }
        for (label, number) in call_targets(c) {
            if ui.add(egui::Button::new(egui::RichText::new(label).color(p.on_accent)).fill(p.success)).clicked() {
                action = Some(RowAction::Call(number));
            }
        }
        if ui.button("Copy number").clicked() {
            ui.ctx().copy_text(c.number.trim().to_owned());
            app.notify(format!("Copied {}", c.number.trim()));
        }
        if ui.button("Edit").clicked() {
            app.ui.contact_draft = Some(Draft::new(c.clone()));
            app.ui.page = Page::Contact;
        }
        let confirming = app.ui.contacts.confirm_delete == Some(c.id);
        let text = if confirming { "Click again to delete" } else { "Delete" };
        if ui.add(egui::Button::new(egui::RichText::new(text).color(p.danger))).clicked() {
            if confirming {
                match app.phonebook.delete(c.id) {
                    Ok(()) => app.ui.contacts.expanded = None,
                    Err(e) => app.notify(format!("Couldn't delete {}: {e}", c.title())),
                }
                app.ui.contacts.confirm_delete = None;
            } else {
                app.ui.contacts.confirm_delete = Some(c.id);
            }
        }
    }));
    ui.add_space(6.0);
    action
}

/// The contact editor page.
pub fn editor(ui: &mut Ui, app: &mut App, p: &Palette) {
    let Some(mut draft) = app.ui.contact_draft.take() else {
        app.ui.page = Page::Contacts;
        return;
    };
    let mut close = false;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, p, Icon::Back, p.text, "Back to contacts").clicked() {
                close = true;
            }
            ui.heading(if draft.contact.id == 0 { "New contact" } else { "Edit contact" });
        });
        ui.add_space(8.0);
        let c = &mut draft.contact;
        section_title(ui, p, "Contact");
        card(ui, p, |ui| {
            widgets::text_field(ui, p, "Name", "Shown in lists and calls", &mut c.name);
            widgets::text_field(ui, p, "Number", "Number, extension or SIP address to call", &mut c.number);
            setting_row(ui, p, "Presence", Some("Show when this contact is busy or on the phone"), |ui| {
                widgets::toggle(ui, p, &mut c.presence, "Presence");
            });
            setting_row(ui, p, "Starred", Some("Listed first"), |ui| {
                widgets::toggle(ui, p, &mut c.starred, "Starred");
            });
        });
        ui.add_space(12.0);
        section_title(ui, p, "Details");
        card(ui, p, |ui| {
            ui.columns(2, |cols| {
                widgets::text_field(&mut cols[0], p, "First name", "", &mut c.first_name);
                widgets::text_field(&mut cols[1], p, "Last name", "", &mut c.last_name);
            });
            ui.columns(2, |cols| {
                widgets::text_field(&mut cols[0], p, "Phone", "", &mut c.phone);
                widgets::text_field(&mut cols[1], p, "Mobile", "", &mut c.mobile);
            });
            widgets::text_field(ui, p, "E-mail", "", &mut c.email);
            widgets::text_field(ui, p, "Address", "", &mut c.address);
            ui.columns(3, |cols| {
                widgets::text_field(&mut cols[0], p, "City", "", &mut c.city);
                widgets::text_field(&mut cols[1], p, "State", "", &mut c.state);
                widgets::text_field(&mut cols[2], p, "Postal code", "", &mut c.zip);
            });
            widgets::text_field(ui, p, "Comment", "", &mut c.comment);
        });
        draft.follow_fields();

        ui.add_space(12.0);
        if let Some(error) = &draft.error {
            ui.label(egui::RichText::new(error).color(p.danger));
        }
        ui.columns(2, |cols| {
            let save_text = if draft.confirm_overwrite { "Overwrite" } else { "Save" };
            if widgets::primary_button(&mut cols[0], p, save_text, p.accent).clicked() {
                match save(app, &mut draft) {
                    Ok(true) => close = true,
                    Ok(false) => {}
                    Err(e) => draft.error = Some(e),
                }
            }
            if cols[1].add_sized([cols[1].available_width(), 36.0], egui::Button::new("Cancel")).clicked() {
                close = true;
            }
        });
        if draft.contact.id != 0 {
            ui.add_space(8.0);
            let text = if draft.confirm_delete { "Click again to delete this contact" } else { "Delete contact" };
            if widgets::primary_button(ui, p, text, p.danger).clicked() {
                if draft.confirm_delete {
                    match app.phonebook.delete(draft.contact.id) {
                        Ok(()) => close = true,
                        Err(e) => draft.error = Some(e),
                    }
                } else {
                    draft.confirm_delete = true;
                }
            }
        }
        ui.add_space(12.0);
    });
    if close {
        app.ui.page = Page::Contacts;
    } else {
        app.ui.contact_draft = Some(draft);
    }
}

/// Validates and saves: the number is required, and
/// taking another contact's number needs a second click, which replaces
/// that contact. Returns whether it was saved.
fn save(app: &mut App, draft: &mut Draft) -> Result<bool, String> {
    let mut c = draft.contact.clone();
    for field in [
        &mut c.name, &mut c.number, &mut c.first_name, &mut c.last_name, &mut c.phone, &mut c.mobile, &mut c.email, &mut c.address, &mut c.city,
        &mut c.state, &mut c.zip, &mut c.comment,
    ] {
        *field = field.trim().to_owned();
    }
    if c.number.is_empty() {
        return Err("Enter a number to call.".into());
    }
    let duplicate = app.phonebook.with_number(&c.number).filter(|other| other.id != c.id).map(|other| other.id);
    if let Some(other) = duplicate {
        if !draft.confirm_overwrite {
            draft.confirm_overwrite = true;
            draft.error = Some("Another contact has this number. Save again to replace it.".into());
            return Ok(false);
        }
        app.phonebook.delete(other)?;
    }
    if c.name.is_empty() {
        c.name = compose(&c.first_name, &c.last_name);
    }
    app.phonebook.save(&c)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers_follow_the_fields() {
        let mut d = Draft::new(Contact::default());
        d.contact.first_name = "Ada".into();
        d.follow_fields();
        d.contact.last_name = "Lovelace".into();
        d.follow_fields();
        assert_eq!(d.contact.name, "Ada Lovelace");
        d.contact.name = "Countess".into();
        d.contact.first_name = "Augusta".into();
        d.follow_fields();
        assert_eq!(d.contact.name, "Countess", "a typed name stays");

        d.contact.phone = "1002".into();
        d.follow_fields();
        assert_eq!(d.contact.number, "1002");
        d.contact.number = "sip:ada@pbx".into();
        d.contact.phone = "1003".into();
        d.follow_fields();
        assert_eq!(d.contact.number, "sip:ada@pbx", "a typed number stays");
        assert_eq!(compose("Ada", "Ada"), "Ada");
    }

    #[test]
    fn rows_show_presence() {
        use egui_kittest::kittest::Queryable;
        let ada = Contact { id: 1, name: "Ada".into(), number: "101".into(), presence: true, ..Contact::default() };
        let ringing = Presence { number: "101".into(), state: PresenceState::Ringing, note: "Ringing".into() };
        let harness = crate::ui::test_support::harness([400.0, 80.0], (), move |ui, _| {
            contact_row(ui, &Palette::dark(), &ada, Some(&ringing), false);
        });
        harness.get_by_label("Ada, 101, Ringing");
        let unknown = Presence { number: "101".into(), state: PresenceState::Unknown, note: String::new() };
        let ada = Contact { id: 1, name: "Ada".into(), number: "101".into(), presence: true, ..Contact::default() };
        let harness = crate::ui::test_support::harness([400.0, 80.0], (), move |ui, _| {
            contact_row(ui, &Palette::dark(), &ada, Some(&unknown), false);
        });
        harness.get_by_label("Ada, 101");
    }

    #[test]
    fn pickup_dials_the_code_before_the_number() {
        assert_eq!(pickup_number("**", "101"), "**101");
        assert_eq!(pickup_number("*8", "sip:101@pbx"), "*8101@pbx");
    }

    #[test]
    fn call_targets_skip_repeats() {
        let c = Contact { number: "1002".into(), phone: "1002".into(), mobile: "98765".into(), ..Contact::default() };
        assert_eq!(call_targets(&c).iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["1002", "98765"]);
    }
}
