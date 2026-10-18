//! The settings page. Phase 1 covers appearance and window behaviour; the
//! full settings arrive in Phase 8.

use egui::Ui;
use rustone_core::settings::ThemePreference;

use super::widgets::{self, card, section_title, setting_row, setting_stack};
use crate::app::App;
use crate::icons::Icon;
use crate::theme::Palette;

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        ui.heading("Settings");
        ui.add_space(8.0);

        section_title(ui, p, "Appearance");
        card(ui, p, |ui| {
            setting_stack(ui, p, "Theme", Some("System follows the Windows light/dark setting"), |ui| {
                let options = [
                    (ThemePreference::System, Icon::Monitor, "System"),
                    (ThemePreference::Light, Icon::Sun, "Light"),
                    (ThemePreference::Dark, Icon::Moon, "Dark"),
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
        section_title(ui, p, "About");
        card(ui, p, |ui| {
            setting_row(ui, p, &format!("RusTone {}", env!("CARGO_PKG_VERSION")), Some("pjproject 2.17 · egui · fastframe"), |_| {});
        });
        ui.add_space(8.0);
    });
}
