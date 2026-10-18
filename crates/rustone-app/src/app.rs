//! The resident application: everything that outlives the window.
//!
//! fastframe-shell drops the window when it closes to the tray and builds a
//! new one (with a fresh egui context) when it's shown again, so all state
//! lives here. Per-window state is reset in `window_gone`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fastframe_shell::{Closed, Headless, Resident, Waker};
use fastframe_tray::{Event, Tray};
use rustone_core::secrets::{Keyring, MemoryStore, SecretStore};
use rustone_core::settings::{Settings, WindowGeometry};

use crate::netwatch::NetWatch;
use crate::sip::{Sip, Status};
use crate::theme::{self, Palette};
use crate::tray;
use crate::ui::UiState;

/// Where the app keeps its files.
pub struct Paths {
    pub settings: PathBuf,
    /// The opt-in full SIP log.
    pub sip_log: PathBuf,
}

const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Requests from other launches (`fastframe-instance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Show,
}

pub type Requests = Arc<Mutex<Vec<Request>>>;

pub struct App {
    pub settings: Settings,
    settings_path: PathBuf,
    settings_dirty: bool,
    last_save: Instant,
    files: fastframe_file::Writer,
    pub tray: Option<Tray>,
    /// The status the tray currently shows.
    tray_status: Option<Status>,
    requests: Requests,
    pub sip: Sip,
    secrets: Box<dyn SecretStore>,
    power: fastframe_power::Power,
    net: NetWatch,
    rendering: fastframe_text::TextRendering,

    /// What happens when the window goes away.
    next: Closed,
    quit_requested: bool,
    wants_show: bool,

    // Per-window state, reset in `window_gone`.
    pub ui: UiState,
    applied_palette: Option<Palette>,
    transition: fastframe_theme::Transition,
    recovery_checked: bool,
}

impl App {
    pub fn new(mut settings: Settings, paths: Paths, requests: Requests, waker: &Waker) -> Self {
        let secrets: Box<dyn SecretStore> = match Keyring::init("RusTone") {
            Ok(store) => Box::new(store),
            Err(e) => {
                log::warn!("credential store unavailable ({e}); passwords won't be remembered");
                Box::new(MemoryStore::default())
            }
        };
        for account in &mut settings.accounts {
            match secrets.get(&account.secret_key()) {
                Ok(password) => account.password = password.unwrap_or_default(),
                Err(e) => log::warn!("cannot read the password for account {}: {e}", account.id),
            }
        }
        let sip = Sip::spawn(&settings, paths.sip_log, waker);
        let (power_wake, net_wake) = (waker.clone(), waker.clone());
        let wake = waker.clone();
        Self {
            settings,
            settings_path: paths.settings,
            settings_dirty: false,
            last_save: Instant::now(),
            files: fastframe_file::Writer::new(),
            tray: tray::spawn(move || wake.wake()),
            tray_status: None,
            requests,
            sip,
            secrets,
            power: fastframe_power::Power::spawn(move || power_wake.wake()),
            net: NetWatch::spawn(move || net_wake.wake()),
            rendering: fastframe_text::detect(),
            next: Closed::Quit,
            quit_requested: false,
            wants_show: false,
            ui: UiState::default(),
            applied_palette: None,
            transition: fastframe_theme::Transition::default(),
            recovery_checked: false,
        }
    }

    /// Fonts, image loaders and icons for a new window's context.
    pub fn attach(&mut self, ctx: &egui::Context) {
        let mut fonts = fastframe_fonts::FontSetup::default().definitions();
        self.rendering.apply_to(&mut fonts);
        ctx.set_fonts(fonts);
        egui_extras::install_image_loaders(ctx);
        fastframe_icons::install::<crate::icons::Icon>(ctx);
        fastframe_titlebar::install(ctx);
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
    }

    pub fn palette(&self) -> &Palette {
        self.applied_palette.as_ref().unwrap_or(&const { Palette::dark() })
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Opens the window again with new window options (e.g. title bar style).
    pub fn reopen_window(&mut self, ctx: &egui::Context) {
        self.next = Closed::Reopen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub fn quit(&mut self, ctx: Option<&egui::Context>) {
        self.quit_requested = true;
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Whether closing the window should keep RusTone running in the tray.
    fn can_hide(&self) -> bool {
        self.settings.ui.close_to_tray && self.tray.as_ref().is_some_and(Tray::is_shown)
    }

    pub fn status(&self) -> Status {
        self.sip.status(&self.settings)
    }

    /// Saves an account (new when its id is 0) and its password. Returns its id.
    pub fn save_account(&mut self, mut account: rustone_core::account::Account) -> u32 {
        let id = if account.id == 0 {
            self.settings.add_account(account.clone())
        } else {
            self.settings.update_account(account.clone());
            account.id
        };
        account.id = id;
        if let Err(e) = self.secrets.set(&account.secret_key(), &account.password) {
            log::warn!("cannot store the password for account {id}: {e}");
        }
        // The stored copy keeps the password in memory for the engine.
        if let Some(slot) = self.settings.accounts.iter_mut().find(|a| a.id == id) {
            slot.password = account.password;
        }
        self.mark_settings_dirty();
        id
    }

    pub fn delete_account(&mut self, id: u32) {
        if let Some(account) = self.settings.remove_account(id) {
            if let Err(e) = self.secrets.delete(&account.secret_key()) {
                log::warn!("cannot remove the password for account {id}: {e}");
            }
            self.mark_settings_dirty();
        }
    }

    /// Pushes settings changes to the engine and engine events to the tray.
    fn update_sip(&mut self) {
        for event in self.power.events() {
            match event {
                // pjsua is destroyed before sleep and recreated after.
                fastframe_power::Event::Suspending(ack) => {
                    log::info!("system suspending; stopping the SIP engine");
                    self.sip.send(sip_engine::Command::Suspend);
                    self.save_settings(true);
                    self.files.flush(Duration::from_millis(1500));
                    ack.done();
                }
                fastframe_power::Event::EndingSession(ack) => {
                    self.save_settings(true);
                    self.files.flush(Duration::from_millis(1500));
                    ack.done();
                }
                fastframe_power::Event::Resumed => {
                    log::info!("system resumed; starting the SIP engine");
                    self.sip.send(sip_engine::Command::Resume);
                }
            }
        }
        if self.net.take_changed() && self.settings.sip.network_changes {
            self.sip.send(sip_engine::Command::IpChanged);
        }
        self.sip.sync(&self.settings);
        self.sip.poll();
        let status = self.status();
        if self.tray_status.as_ref() != Some(&status) {
            if let Some(tray) = &mut self.tray {
                tray::show_status(tray, &status, self.settings.active().map(|a| a.title()));
            }
            self.tray_status = Some(status);
        }
    }

    /// Handles tray clicks and requests from other launches. `ctx` is the
    /// window's context, or `None` while running in the tray.
    fn poll(&mut self, ctx: Option<&egui::Context>) {
        self.update_sip();
        let mut show = false;
        let events = self.tray.as_ref().map(Tray::events).unwrap_or_default();
        for event in events {
            match event {
                Event::Show | Event::Toggle | Event::Menu(tray::SHOW) => show = true,
                Event::Menu(tray::QUIT) => self.quit(ctx),
                Event::Menu(_) => {}
            }
        }
        let requests = std::mem::take(&mut *self.requests.lock().unwrap_or_else(|e| e.into_inner()));
        for request in requests {
            match request {
                Request::Show => show = true,
            }
        }
        if show {
            match ctx {
                Some(ctx) => bring_forward(ctx),
                None => self.wants_show = true,
            }
        }
    }

    fn save_settings(&mut self, force: bool) {
        if self.settings_dirty && (force || self.last_save.elapsed() >= SAVE_DEBOUNCE) {
            self.files.write(&self.settings_path, self.settings.to_toml(), fastframe_file::Access::Default);
            self.settings_dirty = false;
            self.last_save = Instant::now();
        }
    }

    /// Runs every frame of the window, including when eframe skips `ui`
    /// (minimised or occluded windows).
    pub fn logic(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.poll(Some(ctx));

        // Decide here, not in `ui`: closing a minimised window from the
        // taskbar must hide it too.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_requested && self.next != Closed::Reopen {
            self.next = if self.can_hide() { Closed::Hide } else { Closed::Quit };
        }

        if !std::mem::replace(&mut self.recovery_checked, true) {
            fastframe_shell::window::recover_offscreen(ctx, frame);
        }

        self.update_theme(ctx);

        let mut geometry = to_shell_geometry(self.settings.ui.window);
        if geometry.remember(ctx) {
            self.settings.ui.window = from_shell_geometry(geometry);
            self.mark_settings_dirty();
        }
        if let Some(zoom) = fastframe_shell::window::keep_zoom(ctx, self.settings.ui.zoom) {
            self.settings.ui.zoom = zoom;
            self.mark_settings_dirty();
        }

        self.save_settings(false);
        if self.settings_dirty {
            ctx.request_repaint_after(SAVE_DEBOUNCE);
        }
    }

    fn update_theme(&mut self, ctx: &egui::Context) {
        let base = theme::wanted_base(self.settings.ui.theme, ctx.system_theme());
        let wanted = Palette::for_base(base);
        if self.applied_palette.as_ref() == Some(&wanted) {
            return;
        }
        // The first frame has nothing to reveal from.
        if self.applied_palette.is_some() {
            self.transition.begin(ctx);
            if self.transition.holding(ctx) {
                return;
            }
        }
        theme::apply(ctx, &wanted, &self.rendering);
        self.applied_palette = Some(wanted);
    }

    /// Draws the old colours over the new ones during a theme change. Last in the frame.
    pub fn paint_transition(&mut self, ctx: &egui::Context) {
        self.transition.paint(ctx);
    }
}

fn bring_forward(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

fn to_shell_geometry(g: WindowGeometry) -> fastframe_shell::window::Geometry {
    fastframe_shell::window::Geometry { size: g.size, position: g.position, maximized: g.maximized }
}

fn from_shell_geometry(g: fastframe_shell::window::Geometry) -> WindowGeometry {
    WindowGeometry { size: g.size, position: g.position, maximized: g.maximized }
}

impl App {
    pub fn window_geometry(&self) -> fastframe_shell::window::Geometry {
        to_shell_geometry(self.settings.ui.window)
    }
}

impl Resident for App {
    fn closed(&self) -> Closed {
        if self.quit_requested { Closed::Quit } else { self.next }
    }

    fn window_gone(&mut self) {
        self.next = Closed::Quit;
        self.wants_show = false;
        self.applied_palette = None;
        self.transition = fastframe_theme::Transition::default();
        self.recovery_checked = false;
        self.save_settings(true);
    }

    fn headless_frame(&mut self, _ctx: &egui::Context) -> Headless {
        self.poll(None);
        self.save_settings(true);
        if self.quit_requested {
            Headless::Quit
        } else if std::mem::take(&mut self.wants_show) {
            Headless::Show
        } else {
            Headless::Wait
        }
    }

    fn start_hidden(&mut self) -> bool {
        // Without a tray there would be no way back to the window.
        match &mut self.tray {
            Some(tray) => {
                tray.create_item();
                true
            }
            None => false,
        }
    }

    fn shutdown(&mut self) {
        self.save_settings(true);
        self.sip.shutdown();
        self.files.flush(fastframe_file::FLUSH_TIMEOUT);
        log::info!("RusTone exiting");
    }
}
