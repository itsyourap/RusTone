//! The account list (in Settings) and the account editor.

use egui::{Sense, Ui, Vec2, vec2};
use fastframe_fonts::Weight;
use rustone_core::account::{Account, SrtpMode, Transport};

use super::widgets::{self, card, section_title, setting_row};
use super::Page;
use crate::app::App;
use crate::icons::Icon;
use crate::sip::Status;
use crate::theme::Palette;

/// An account being edited. `account.id == 0` means a new account.
#[derive(Debug, Default)]
pub struct Draft {
    pub account: Account,
    pub show_password: bool,
    pub confirm_delete: bool,
    pub error: Option<String>,
}

impl Draft {
    fn validate(&self) -> Result<(), String> {
        let a = &self.account;
        if a.username.trim().is_empty() {
            return Err("Enter a username.".into());
        }
        if a.server.trim().is_empty() && a.domain.trim().is_empty() {
            return Err("Enter a SIP server or a domain.".into());
        }
        Ok(())
    }
}

/// The Accounts section of the Settings page.
pub fn list(ui: &mut Ui, app: &mut App, p: &Palette) {
    section_title(ui, p, "Accounts");
    let status = app.status();
    card(ui, p, |ui| {
        let mut select = None;
        let mut edit = None;
        for account in &app.settings.accounts {
            let active = app.settings.active_account == Some(account.id);
            match account_row(ui, p, account, active, active.then_some(&status)) {
                RowAction::Select => select = Some(account.id),
                RowAction::Edit => edit = Some(account.id),
                RowAction::None => {}
            }
        }
        if let Some(id) = select {
            app.settings.active_account = Some(id);
            app.mark_settings_dirty();
        }
        if let Some(id) = edit
            && let Some(account) = app.settings.account(id)
        {
            app.ui.account_draft = Some(Draft { account: account.clone(), ..Draft::default() });
            app.ui.page = Page::Account;
        }
        if app.settings.accounts.is_empty() {
            ui.label(egui::RichText::new("No accounts yet. Add your SIP account to make and receive calls.").size(13.0).color(p.dim));
        }
        ui.add_space(4.0);
        if ui.add(egui::Button::new(egui::RichText::new("＋  Add account").font(Weight::Medium.font_id(14.0)))).clicked() {
            app.ui.account_draft = Some(Draft::default());
            app.ui.page = Page::Account;
        }
    });

    ui.add_space(12.0);
    card(ui, p, |ui| {
        let mut enabled = app.settings.local_account.enabled;
        setting_row(ui, p, "Call by IP address", Some("A local account without a server, listening on port 5060"), |ui| {
            if widgets::toggle(ui, p, &mut enabled, "Call by IP address").changed() {
                app.settings.local_account.enabled = enabled;
                app.mark_settings_dirty();
            }
        });
    });
}

enum RowAction {
    None,
    Select,
    Edit,
}

fn account_row(ui: &mut Ui, p: &Palette, account: &Account, active: bool, status: Option<&Status>) -> RowAction {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
    let label = format!("{}{}", account.title(), if active { " (active)" } else { "" });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, active, &label));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
    let painter = ui.painter();
    if hover > 0.0 {
        painter.rect_filled(rect.expand2(vec2(6.0, 0.0)), egui::CornerRadius::same(8), p.surface_hover.gamma_multiply(hover));
    }
    // Radio mark.
    let radio = egui::pos2(rect.left() + 10.0, rect.center().y);
    painter.circle_stroke(radio, 8.0, egui::Stroke::new(1.5, if active { p.accent } else { p.dim }));
    if active {
        painter.circle_filled(radio, 4.5, p.accent);
    }
    let text_x = rect.left() + 28.0;
    painter.text(egui::pos2(text_x, rect.top() + 15.0), egui::Align2::LEFT_CENTER, account.title(), Weight::Medium.font_id(14.0), p.text);
    let transport = match account.transport {
        Transport::Udp => "UDP",
        Transport::Tcp => "TCP",
        Transport::Tls => "TLS",
    };
    let host = if account.server.is_empty() { &account.domain } else { &account.server };
    let (detail, color) = match status {
        Some(Status::Error(e)) => (e.clone(), p.danger),
        Some(s) => (format!("{} · {transport} · {host}", s.label()), if s.is_online() { p.success } else { p.dim }),
        None => (format!("{transport} · {host}"), p.dim),
    };
    let detail_rect = egui::Rect::from_min_max(egui::pos2(text_x, rect.top() + 26.0), egui::pos2(rect.right() - 44.0, rect.bottom() - 4.0));
    let galley = widgets::elided(ui, detail, Weight::Regular.font_id(12.0), color, detail_rect.width());
    ui.painter().galley(detail_rect.min, galley, color);

    let edit_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(34.0));
    let edited = ui.put(edit_rect, |ui: &mut Ui| widgets::icon_button(ui, p, Icon::Pencil, p.secondary, &format!("Edit {}", account.title()))).clicked();
    if edited {
        RowAction::Edit
    } else if response.clicked() && !active {
        RowAction::Select
    } else {
        RowAction::None
    }
}

/// The account editor page.
pub fn editor(ui: &mut Ui, app: &mut App, p: &Palette) {
    let Some(mut draft) = app.ui.account_draft.take() else {
        app.ui.page = Page::Settings;
        return;
    };
    let mut close = false;

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, p, Icon::Back, p.text, "Back to settings").clicked() {
                close = true;
            }
            ui.heading(if draft.account.id == 0 { "New account" } else { "Edit account" });
        });
        ui.add_space(8.0);
        let a = &mut draft.account;

        section_title(ui, p, "Account");
        card(ui, p, |ui| {
            widgets::text_field(ui, p, "Account name", "Optional, e.g. Office", &mut a.label);
            widgets::text_field(ui, p, "SIP server", "Registrar, e.g. sip.example.com", &mut a.server);
            widgets::text_field(ui, p, "SIP proxy", "Optional outbound proxy(s), space-separated", &mut a.proxy);
            widgets::text_field(ui, p, "Username", "User or number", &mut a.username);
            widgets::text_field(ui, p, "Domain", "Added to numbers without @", &mut a.domain);
            widgets::password_field(ui, p, "Password", &mut a.password, &mut draft.show_password);
            widgets::text_field(ui, p, "Login", "Authentication user, if not the username", &mut a.auth_id);
            widgets::text_field(ui, p, "Display name", "Optional", &mut a.display_name);
        });

        ui.add_space(12.0);
        section_title(ui, p, "Connection");
        card(ui, p, |ui| {
            ui.label(egui::RichText::new("Transport").size(12.0).color(p.secondary));
            widgets::segmented(ui, p, &mut a.transport, &[(Transport::Udp, None, "UDP"), (Transport::Tcp, None, "TCP"), (Transport::Tls, Some(Icon::Lock), "TLS")]);
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Media encryption (SRTP)").size(12.0).color(p.secondary));
            egui::ComboBox::from_id_salt("srtp")
                .width(ui.available_width())
                .selected_text(srtp_label(a.srtp))
                .show_ui(ui, |ui| {
                    for mode in [SrtpMode::Disabled, SrtpMode::Optional, SrtpMode::Mandatory, SrtpMode::Dtls, SrtpMode::DtlsSdes] {
                        ui.selectable_value(&mut a.srtp, mode, srtp_label(mode));
                    }
                });
            widgets::text_field(ui, p, "Public address", "Optional, used when STUN is off", &mut a.public_addr);
            ui.columns(2, |cols| {
                widgets::number_field(&mut cols[0], p, "Register refresh", &mut a.register_refresh, 10..=86400, " s");
                widgets::number_field(&mut cols[1], p, "Keep-alive", &mut a.keep_alive, 0..=3600, " s");
            });
            ui.separator();
            for (value, title, description) in [
                (&mut a.ice, "ICE", Some("NAT traversal for media")),
                (&mut a.allow_rewrite, "Allow IP rewrite", Some("Use the address the server sees")),
                (&mut a.disable_session_timer, "Disable session timers", None),
                (&mut a.publish, "Publish presence", None),
            ] {
                setting_row(ui, p, title, description, |ui| {
                    widgets::toggle(ui, p, value, title);
                });
            }
        });

        ui.add_space(12.0);
        section_title(ui, p, "Calling");
        card(ui, p, |ui| {
            widgets::text_field(ui, p, "Voicemail number", "Dialled by the voicemail button; enables MWI", &mut a.voicemail_number);
            widgets::text_field(ui, p, "Dialing prefix", "Added to dialled numbers", &mut a.dialing_prefix);
            widgets::text_field(ui, p, "Dial plan", "Rules, |-separated", &mut a.dial_plan);
            setting_row(ui, p, "Hide caller ID", None, |ui| {
                widgets::toggle(ui, p, &mut a.hide_cid, "Hide caller ID");
            });
        });

        ui.add_space(12.0);
        if let Some(error) = &draft.error {
            ui.label(egui::RichText::new(error).color(p.danger));
        }
        ui.columns(2, |cols| {
            if widgets::primary_button(&mut cols[0], p, "Save", p.accent).clicked() {
                match draft.validate() {
                    Ok(()) => {
                        app.save_account(draft.account.clone());
                        close = true;
                    }
                    Err(e) => draft.error = Some(e),
                }
            }
            if cols[1].add_sized([cols[1].available_width(), 36.0], egui::Button::new("Cancel")).clicked() {
                close = true;
            }
        });
        if draft.account.id != 0 {
            ui.add_space(8.0);
            let text = if draft.confirm_delete { "Click again to delete this account" } else { "Delete account" };
            if widgets::primary_button(ui, p, text, p.danger).clicked() {
                if draft.confirm_delete {
                    app.delete_account(draft.account.id);
                    close = true;
                } else {
                    draft.confirm_delete = true;
                }
            }
        }
        ui.add_space(12.0);
    });

    if close {
        app.ui.page = Page::Settings;
    } else {
        app.ui.account_draft = Some(draft);
    }
}

fn srtp_label(mode: SrtpMode) -> &'static str {
    match mode {
        SrtpMode::Disabled => "Disabled",
        SrtpMode::Optional => "Optional (SDES)",
        SrtpMode::Mandatory => "Mandatory (SDES)",
        SrtpMode::Dtls => "DTLS-SRTP",
        SrtpMode::DtlsSdes => "DTLS-SRTP, then SDES",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_need_user_and_host() {
        let mut d = Draft::default();
        assert!(d.validate().is_err());
        d.account.username = "1001".into();
        assert!(d.validate().is_err());
        d.account.domain = "pbx.local".into();
        assert!(d.validate().is_ok());
    }
}
