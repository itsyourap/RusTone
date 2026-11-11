//! Settings' "Import and export" section: the configuration as a JSON file,
//! with the accounts the user picks (none for settings only).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;

use egui::Ui;
use rustone_core::config_file::{self, ConfigFile};

use super::widgets::{self, card};
use crate::app::App;
use crate::theme::Palette;

/// Where the section is: idle, choosing what to export or import, or
/// waiting for a file dialog.
#[derive(Debug, Default)]
pub enum Transfer {
    #[default]
    Idle,
    /// Choosing the accounts to export (by id).
    Export { selected: BTreeSet<u32> },
    /// A file dialog is open; its result arrives here.
    Waiting(mpsc::Receiver<Outcome>),
    /// A file was read; choosing the accounts to import (by index).
    Import { path: PathBuf, file: Box<ConfigFile>, selected: BTreeSet<usize> },
}

/// What a file dialog's thread came back with.
#[derive(Debug)]
pub enum Outcome {
    Cancelled,
    /// Written, with how many accounts.
    Exported(PathBuf, usize),
    /// Read and understood.
    Read(PathBuf, Box<ConfigFile>),
    Failed(String),
}

/// The section's state and its last result.
#[derive(Debug, Default)]
pub struct TransferState {
    pub mode: Transfer,
    /// The last result, shown under the buttons (error, or done).
    pub status: Option<(String, bool)>,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

pub fn section(ui: &mut Ui, app: &mut App, p: &Palette) {
    // A finished dialog.
    if let Transfer::Waiting(rx) = &app.ui.transfer.mode {
        match rx.try_recv() {
            Ok(outcome) => {
                let state = &mut app.ui.transfer;
                state.mode = Transfer::Idle;
                match outcome {
                    Outcome::Cancelled => {}
                    Outcome::Exported(path, n) => {
                        let what = if n == 0 { "settings".to_owned() } else { format!("settings and {n} account{}", if n == 1 { "" } else { "s" }) };
                        state.status = Some((format!("Exported {what} to {}.", file_name(&path)), false));
                    }
                    Outcome::Read(path, file) => {
                        let selected = (0..file.accounts.len()).collect();
                        state.mode = Transfer::Import { path, file, selected };
                    }
                    Outcome::Failed(e) => state.status = Some((e, true)),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            Err(mpsc::TryRecvError::Disconnected) => app.ui.transfer.mode = Transfer::Idle,
        }
    }

    card(ui, p, |ui| match &app.ui.transfer.mode {
        Transfer::Idle | Transfer::Waiting(_) => idle(ui, app, p),
        Transfer::Export { .. } => export(ui, app, p),
        Transfer::Import { .. } => import(ui, app, p),
    });
}

fn idle(ui: &mut Ui, app: &mut App, p: &Palette) {
    let waiting = matches!(app.ui.transfer.mode, Transfer::Waiting(_));
    widgets::note(ui, egui::RichText::new("All settings, codec order and speed dial included, and the accounts you choose, in a JSON file to keep or to set up another computer.").size(12.0).color(p.dim));
    ui.horizontal(|ui| {
        if ui.add_enabled(!waiting, egui::Button::new("Export…")).clicked() {
            app.ui.transfer = TransferState { mode: Transfer::Export { selected: BTreeSet::new() }, status: None };
        }
        if ui.add_enabled(!waiting, egui::Button::new("Import…")).clicked() {
            let (tx, rx) = mpsc::channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let outcome = match rfd::FileDialog::new().set_title("Import RusTone settings").add_filter("RusTone settings", &["json"]).pick_file() {
                    None => Outcome::Cancelled,
                    Some(path) => match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| config_file::parse(&text)) {
                        Ok(file) => Outcome::Read(path, Box::new(file)),
                        Err(e) => Outcome::Failed(format!("Couldn't import {}: {e}.", file_name(&path))),
                    },
                };
                let _ = tx.send(outcome);
                ctx.request_repaint();
            });
            app.ui.transfer = TransferState { mode: Transfer::Waiting(rx), status: None };
        }
    });
    if let Some((text, error)) = &app.ui.transfer.status {
        widgets::note(ui, egui::RichText::new(text).size(12.0).color(if *error { p.danger } else { p.success }));
    }
}

/// One account's checkbox row: its name, then its user and server. Both
/// are cut short with "…" rather than widening the card past its column.
fn account_check(ui: &mut Ui, p: &Palette, checked: &mut bool, title: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(checked, "").on_hover_text(title);
        let name = ui.add(egui::Label::new(egui::RichText::new(title).size(13.5).color(p.text)).truncate().sense(egui::Sense::click()));
        if name.clicked() {
            *checked = !*checked;
        }
        ui.add(egui::Label::new(egui::RichText::new(detail).size(12.0).color(p.dim)).truncate()).on_hover_text(detail);
    });
}

fn export(ui: &mut Ui, app: &mut App, p: &Palette) {
    let Transfer::Export { selected } = &mut app.ui.transfer.mode else { return };
    widgets::note(ui, egui::RichText::new("Export the settings and these accounts:").size(13.0).color(p.secondary));
    if app.settings.accounts.is_empty() {
        widgets::note(ui, egui::RichText::new("No accounts: the settings only.").size(12.0).color(p.dim));
    }
    for account in &app.settings.accounts {
        let mut checked = selected.contains(&account.id);
        let host = if account.server.is_empty() { &account.domain } else { &account.server };
        account_check(ui, p, &mut checked, account.title(), &format!("{} · {host}", account.username));
        if checked {
            selected.insert(account.id);
        } else {
            selected.remove(&account.id);
        }
    }
    if !selected.is_empty() {
        widgets::note(ui, egui::RichText::new("Their passwords are saved in the file as plain text: keep it somewhere safe.").size(12.0).color(p.warning));
    }
    let mut cancel = false;
    let mut save = false;
    ui.horizontal(|ui| {
        save = ui.button("Save to file…").clicked();
        cancel = ui.button("Cancel").clicked();
    });
    if cancel {
        app.ui.transfer.mode = Transfer::Idle;
    } else if save {
        let ids: Vec<u32> = selected.iter().copied().collect();
        let count = ids.len();
        let text = config_file::export(&app.settings, &ids, &format!("RusTone {}", env!("CARGO_PKG_VERSION")));
        let (tx, rx) = mpsc::channel();
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let outcome = match rfd::FileDialog::new().set_title("Export RusTone settings").add_filter("RusTone settings", &["json"]).set_file_name("RusTone.settings.json").save_file() {
                None => Outcome::Cancelled,
                Some(path) => match std::fs::write(&path, text) {
                    Ok(()) => Outcome::Exported(path, count),
                    Err(e) => Outcome::Failed(format!("Couldn't write {}: {e}.", file_name(&path))),
                },
            };
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
        app.ui.transfer.mode = Transfer::Waiting(rx);
    }
}

fn import(ui: &mut Ui, app: &mut App, p: &Palette) {
    let Transfer::Import { path, file, selected } = &mut app.ui.transfer.mode else { return };
    widgets::note(ui, egui::RichText::new(format!("Import {}", file_name(path))).size(13.0).color(p.secondary));
    if !file.exported_by.is_empty() {
        widgets::note(ui, egui::RichText::new(format!("Exported by {}", file.exported_by)).size(12.0).color(p.dim));
    }
    widgets::note(ui, egui::RichText::new("Its settings replace yours. Your accounts stay; add these:").size(12.0).color(p.dim));
    if file.accounts.is_empty() {
        widgets::note(ui, egui::RichText::new("The file has no accounts: the settings only.").size(12.0).color(p.dim));
    }
    for (i, exported) in file.accounts.iter().enumerate() {
        let a = &exported.account;
        let mut checked = selected.contains(&i);
        let host = if a.server.is_empty() { &a.domain } else { &a.server };
        let replaces = app.settings.accounts.iter().any(|mine| config_file::same_account(mine, a));
        let detail = format!("{} · {host}{}", a.username, if replaces { " · updates yours" } else { "" });
        account_check(ui, p, &mut checked, a.title(), &detail);
        if checked {
            selected.insert(i);
        } else {
            selected.remove(&i);
        }
    }
    let mut cancel = false;
    let mut apply = false;
    ui.horizontal(|ui| {
        apply = ui.button("Import").clicked();
        cancel = ui.button("Cancel").clicked();
    });
    if cancel {
        app.ui.transfer.mode = Transfer::Idle;
    } else if apply {
        let selected: Vec<usize> = selected.iter().copied().collect();
        let count = selected.len();
        let name = file_name(path);
        let plan = config_file::plan_import(&app.settings, file, &selected);
        let titlebar_changed = plan.settings.ui.custom_titlebar != app.settings.ui.custom_titlebar;
        app.apply_import(plan);
        if titlebar_changed {
            // Window decorations are fixed when the window is created.
            app.reopen_window(ui.ctx());
        }
        let what = if count == 0 { "settings".to_owned() } else { format!("settings and {count} account{}", if count == 1 { "" } else { "s" }) };
        app.ui.transfer = TransferState { mode: Transfer::Idle, status: Some((format!("Imported {what} from {name}."), false)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A long address is cut short instead of widening the card past its
    /// column (the narrowest column is 300 points).
    #[test]
    fn account_rows_fit_a_narrow_column() {
        let width = std::rc::Rc::new(std::cell::Cell::new(0.0f32));
        let seen = width.clone();
        let harness = crate::ui::test_support::harness([300.0, 80.0], false, move |ui, checked| {
            let row = ui.scope(|ui| {
                account_check(ui, &Palette::dark(), checked, "A rather long account name", "+910000000000@a.very.long.ims.domain.example.test · sip.example.test");
            });
            seen.set(row.response.rect.width());
        });
        assert!(width.get() <= 300.0, "row is {} wide", width.get());
        drop(harness);
    }
}
