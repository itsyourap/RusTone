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

use crate::idle::IdleWatch;
use crate::netwatch::NetWatch;
use crate::sip::{Sip, Status};
use crate::theme::{self, Palette};
use crate::tray;
use crate::ui::UiState;

/// Where the app keeps its files.
pub struct Paths {
    pub settings: PathBuf,
    /// Where the generated sound files go.
    pub sounds: PathBuf,
    /// The opt-in full SIP log.
    pub sip_log: PathBuf,
    /// `rustone.db`: contacts and call history.
    pub database: PathBuf,
    /// The data folders were just renamed from "RusTone": move the passwords too.
    pub renamed_from_rustone: bool,
}

const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// Credential Manager entries are `<account key>.RusTone`.
const CREDENTIAL_SERVICE: &str = "RusTone";

/// The notification service, under the same ID as the single-instance lock.
fn notifier() -> Option<fastframe_notifications::Client> {
    let app = fastframe_notifications::App::new("dev.itsyourap.rustone", "RusTone").logo(&crate::icons::logo_png(256), crate::icons::LOGO_BLUE);
    fastframe_notifications::Client::new(app).map_err(|e| log::warn!("notifications unavailable: {e}")).ok()
}

/// Moves passwords stored under the app's old name ("RusTone").
fn move_old_passwords(store: &Keyring, settings: &Settings) {
    let old = store.with_service("RusTone");
    for account in &settings.accounts {
        match rustone_core::secrets::move_secret(&old, store, &account.secret_key()) {
            Ok(true) => log::info!("moved the password of account {} to the RusTone credential", account.id),
            Ok(false) => {}
            Err(e) => log::warn!("cannot move the password of account {}: {e}", account.id),
        }
    }
}

/// Requests from other launches (`fastframe-instance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Show,
    /// A missed-call notification was clicked.
    ShowCalls,
    /// A message notification was clicked: open that conversation.
    ShowConversation(String),
}

pub type Requests = Arc<Mutex<Vec<Request>>>;

pub struct App {
    pub settings: Settings,
    settings_path: PathBuf,
    settings_dirty: bool,
    last_save: Instant,
    files: fastframe_file::Writer,
    pub tray: Option<Tray>,
    /// The status (and missed calls) the tray currently shows.
    tray_status: Option<(Status, bool)>,
    tray_toggles: Option<tray::Toggles>,
    requests: Requests,
    pub sip: Sip,
    pub phonebook: crate::phonebook::Phonebook,
    pub directory: crate::directory::UsersDirectory,
    /// Missed calls since the Calls page was last open.
    pub missed: u32,
    notifier: Option<fastframe_notifications::Client>,
    /// The latest missed-call toast, replaced by the next.
    missed_toast: Option<fastframe_notifications::Handle>,
    /// The latest message toast per conversation.
    message_toasts: std::collections::HashMap<String, fastframe_notifications::Handle>,
    waker: Waker,
    /// The missed-call count on the window's taskbar button.
    taskbar: fastframe_dock::Taskbar,
    secrets: Box<dyn SecretStore>,
    power: fastframe_power::Power,
    net: NetWatch,
    idle: IdleWatch,
    rendering: fastframe_text::TextRendering,

    /// What happens when the window goes away.
    next: Closed,
    quit_requested: bool,
    wants_show: bool,
    /// Bring the window forward at the next chance (incoming call).
    show_requested: bool,

    // Per-window state, reset in `window_gone`.
    pub ui: UiState,
    pub debug: crate::ui::debug::DebugState,
    applied_palette: Option<Palette>,
    transition: fastframe_theme::Transition,
    recovery_checked: bool,
}

impl App {
    pub fn new(mut settings: Settings, paths: Paths, requests: Requests, waker: &Waker) -> Self {
        let secrets: Box<dyn SecretStore> = match Keyring::init(CREDENTIAL_SERVICE) {
            Ok(store) => {
                if paths.renamed_from_rustone {
                    move_old_passwords(&store, &settings);
                }
                Box::new(store)
            }
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
        let sounds = match rustone_core::sounds::install(&paths.sounds) {
            Ok(files) => {
                use rustone_core::sounds::Sound;
                crate::calls::SoundPaths {
                    ringtone: files.path(Sound::Ringtone),
                    ringback: files.path(Sound::Ringback),
                    call_waiting: files.path(Sound::CallWaiting),
                    hangup: files.path(Sound::Hangup),
                    message_in: files.path(Sound::MessageIn),
                    message_out: files.path(Sound::MessageOut),
                }
            }
            Err(e) => {
                log::warn!("cannot write sound files: {e}");
                crate::calls::SoundPaths::default()
            }
        };
        let sip = Sip::spawn(&settings, paths.sip_log, sounds, waker);
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
            tray_toggles: None,
            requests,
            sip,
            phonebook: crate::phonebook::Phonebook::open(&paths.database),
            directory: crate::directory::UsersDirectory::default(),
            missed: 0,
            notifier: notifier(),
            missed_toast: None,
            message_toasts: std::collections::HashMap::new(),
            waker: waker.clone(),
            taskbar: fastframe_dock::Taskbar::default(),
            secrets,
            power: fastframe_power::Power::spawn(move || power_wake.wake()),
            net: NetWatch::spawn(move || net_wake.wake()),
            idle: IdleWatch::spawn({
                let wake = waker.clone();
                move || wake.wake()
            }),
            rendering: fastframe_text::detect(),
            next: Closed::Quit,
            quit_requested: false,
            wants_show: false,
            show_requested: false,
            ui: UiState::default(),
            debug: crate::ui::debug::DebugState::default(),
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

    /// Counts a missed call and shows a notification for it, replacing the
    /// previous one; clicking it opens the Calls page.
    fn missed_call(&mut self, who: &str) {
        self.missed += 1;
        let Some(client) = &self.notifier else { return };
        let body = if self.missed > 1 { format!("{who} (and {} more)", self.missed - 1) } else { who.to_owned() };
        let toast = fastframe_notifications::Notification::new("Missed call").body(body).key("missed-call");
        let (requests, waker) = (self.requests.clone(), self.waker.clone());
        match client.show_with(toast, move |event| {
            if event == fastframe_notifications::Event::Activated {
                requests.lock().unwrap_or_else(|e| e.into_inner()).push(Request::ShowCalls);
                waker.wake();
            }
        }) {
            Ok(handle) => self.missed_toast = Some(handle),
            Err(e) => log::warn!("cannot show the missed-call notification: {e}"),
        }
    }

    /// Plays a message sound, unless a call is up (it would cut off the
    /// ringtone or ringback).
    fn message_sound(&self, incoming: bool) {
        if self.sip.calls.has_calls() {
            return;
        }
        let sounds = &self.sip.calls.sounds;
        let path = if incoming { sounds.message_in.clone() } else { sounds.message_out.clone() };
        let level = self.sip.calls.ring_level;
        self.sip.send(sip_engine::Command::PlaySound(sip_engine::Sound { path, looping: false, level, ring: true }));
    }

    /// A message for `peer`: the sound (for every message)
    /// and a notification per conversation, unless it's on screen.
    fn message_arrived(&mut self, peer: &str, who: &str, text: &str) {
        let on_screen = self.ui.page == crate::ui::Page::Messages
            && self.ui.messages.seen.as_ref().is_some_and(|(seen, at)| seen == peer && at.elapsed() < Duration::from_millis(1500));
        if on_screen {
            return;
        }
        self.message_sound(true);
        let Some(client) = &self.notifier else { return };
        let preview: String = text.chars().take(200).collect();
        let toast = fastframe_notifications::Notification::new(format!("Message from {who}")).body(preview).key(format!("message-{peer}"));
        let (requests, waker, peer_owned) = (self.requests.clone(), self.waker.clone(), peer.to_owned());
        match client.show_with(toast, move |event| {
            if event == fastframe_notifications::Event::Activated {
                requests.lock().unwrap_or_else(|e| e.into_inner()).push(Request::ShowConversation(peer_owned.clone()));
                waker.wake();
            }
        }) {
            Ok(handle) => {
                self.message_toasts.insert(peer.to_owned(), handle);
            }
            Err(e) => log::warn!("cannot show the message notification: {e}"),
        }
    }

    /// A conversation is on screen: its notification goes.
    pub fn conversation_seen(&mut self, peer: &str) {
        if self.message_toasts.remove(peer).is_some()
            && let Some(client) = &self.notifier
        {
            let _ = client.withdraw(format!("message-{peer}"));
        }
    }

    /// The Calls page is open: missed calls have been seen.
    pub fn missed_seen(&mut self) {
        if self.missed > 0 {
            self.missed = 0;
            if let Some(client) = &self.notifier {
                let _ = client.withdraw("missed-call");
            }
            self.missed_toast = None;
        }
    }

    /// The active account's registrar, domain and proxies: numbers there are
    /// shown without the host.
    fn own_hosts(&self) -> Vec<String> {
        self.settings
            .active()
            .map(|a| [a.server.trim().to_owned(), a.domain.trim().to_owned()].into_iter().chain(a.proxies()).filter(|h| !h.is_empty()).collect())
            .unwrap_or_default()
    }

    /// Whether to reach `target` through the local account: IP addresses
    /// when it's on, or anything when there's no usable main account.
    pub fn use_local_account(&self, target: &str) -> bool {
        let looks_like_ip = target.split([':', '@']).any(|part| part.parse::<std::net::IpAddr>().is_ok());
        self.settings.local_account.enabled && (looks_like_ip || !self.status().is_online())
    }

    /// Sends a text message to `target`, saving it first; its answer
    /// updates it. Returns the conversation it went into.
    pub fn send_message(&mut self, target: &str, text: &str) -> String {
        let peer = crate::phonebook::conversation_key(target);
        let record = rustone_core::messages::MessageRecord {
            id: 0,
            peer: peer.clone(),
            name: String::new(),
            outgoing: true,
            time: jiff::Timestamp::now().as_second(),
            text: text.to_owned(),
            state: rustone_core::messages::MessageState::Sending,
            reason: String::new(),
            unread: false,
        };
        let id = self.phonebook.add_message(&record);
        self.dispatch_message(id, &peer, text);
        // Played with sound events on.
        if self.settings.calls.local_dtmf {
            self.message_sound(false);
        }
        peer
    }

    /// Sends a failed message again, in place.
    pub fn retry_message(&mut self, id: i64, peer: &str, text: &str) {
        self.phonebook.set_message_state(id, rustone_core::messages::MessageState::Sending, "");
        self.dispatch_message(id, peer, text);
    }

    fn dispatch_message(&mut self, id: i64, peer: &str, text: &str) {
        let local = self.use_local_account(peer);
        self.sip.send(sip_engine::Command::SendMessage { id: id as u64, target: peer.to_owned(), text: text.to_owned(), local });
    }

    /// The users-directory URL's placeholder values, from the active account.
    fn directory_values(&self) -> rustone_core::directory::UrlValues {
        let mut v = rustone_core::directory::UrlValues {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            time: jiff::Timestamp::now().as_second(),
            ..Default::default()
        };
        if let Some(a) = self.settings.active() {
            v.username = a.username.trim().to_owned();
            v.password = a.password.clone();
            v.server = a.server.trim().to_owned();
            v.domain = a.domain.trim().to_owned();
            v.label = a.title().to_owned();
        }
        v
    }

    /// Shows `text` briefly at the bottom of the window.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.sip.calls.notice = Some((text.into(), Instant::now()));
    }

    /// Runs a call action and sends the commands it produces.
    /// `number` with the main account's dialling prefix and dial plan;
    /// the notice to show when the dial plan blocks it.
    pub fn format_number(&self, number: &str) -> Result<String, String> {
        let Some(account) = self.settings.active() else { return Ok(number.to_owned()) };
        rustone_core::dialplan::format_number(number, &account.dialing_prefix, &account.dial_plan).map_err(|_| format!("The dial plan doesn't allow {number}"))
    }

    pub fn act(&mut self, action: impl FnOnce(&mut crate::calls::CallBook) -> Vec<sip_engine::Command>) {
        let commands = action(&mut self.sip.calls);
        self.sip.run(commands);
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
        // Busy for DND, Away after 10 idle minutes.
        self.sip.online_status = if self.settings.calls.dnd {
            sip_engine::OnlineStatus::Busy
        } else if self.idle.is_away() {
            sip_engine::OnlineStatus::Away
        } else {
            sip_engine::OnlineStatus::Online
        };
        self.sip.sync(&self.settings);
        // Without the presence feature nothing is subscribed; contacts keep their choice.
        let buddies = if self.settings.features.presence { self.phonebook.watched_numbers() } else { Vec::new() };
        self.sip.set_buddies(buddies);
        let values = self.directory_values();
        let ready = matches!(self.sip.reg, sip_engine::RegState::Registered { .. } | sip_engine::RegState::NoRegistrar);
        let template = self.settings.sip.users_directory.clone();
        if let Some(message) = self.directory.tick(&template, &values, ready, &mut self.phonebook, &self.waker) {
            self.notify(message);
        }
        let polled = self.sip.poll(&self.settings);
        if !polled.ended.is_empty() {
            let own_hosts = self.own_hosts();
            for call in &polled.ended {
                self.phonebook.record(call, &own_hosts);
                if call.is_missed() {
                    let record = crate::phonebook::record_for(call, &own_hosts);
                    let who = match self.phonebook.lookup(&record.number) {
                        Some(contact) => contact.title(),
                        None if !record.name.is_empty() => record.name.clone(),
                        None => record.number.clone(),
                    };
                    self.missed_call(&who);
                }
            }
        }
        let own_hosts = self.own_hosts();
        for message in polled.messages {
            if !self.settings.features.messages {
                continue;
            }
            let (number, name) = crate::phonebook::peer_for(&message.from, &own_hosts);
            let peer = crate::phonebook::conversation_key(&number);
            let who = match self.phonebook.lookup(&peer) {
                Some(contact) => contact.title(),
                None if !name.is_empty() => name.clone(),
                None => number.clone(),
            };
            self.message_arrived(&peer, &who, &message.text);
            let record = rustone_core::messages::MessageRecord {
                id: 0,
                peer,
                name,
                outgoing: false,
                time: jiff::Timestamp::now().as_second(),
                text: message.text,
                state: rustone_core::messages::MessageState::Received,
                reason: String::new(),
                unread: true,
            };
            self.phonebook.add_message(&record);
        }
        for status in polled.message_statuses {
            use rustone_core::messages::MessageState;
            let (state, reason) = match (status.delivered(), status.code) {
                (true, _) => (MessageState::Sent, String::new()),
                (false, 0) => (MessageState::Failed, status.reason),
                (false, code) => (MessageState::Failed, format!("{code} {}", status.reason).trim().to_owned()),
            };
            self.phonebook.set_message_state(status.id as i64, state, &reason);
        }
        if polled.show_window {
            self.ui.page = crate::ui::Page::Dialer;
            self.ui.adding_call = false;
            self.show_requested = true;
        }
        let status = self.status();
        let shown = (status, self.missed > 0);
        if self.tray_status.as_ref() != Some(&shown) {
            if let Some(tray) = &mut self.tray {
                tray::show_status(tray, &shown.0, self.settings.active().map(|a| a.title()), shown.1);
            }
            self.tray_status = Some(shown);
        }
        let toggles = tray::Toggles::of(&self.settings.calls);
        if self.tray_toggles != Some(toggles) {
            if let Some(tray) = &mut self.tray {
                tray::show_toggles(tray, toggles);
            }
            self.tray_toggles = Some(toggles);
        }
    }

    /// Handles tray clicks and requests from other launches. `ctx` is the
    /// window's context, or `None` while running in the tray.
    fn poll(&mut self, ctx: Option<&egui::Context>) {
        self.update_sip();
        let mut show = std::mem::take(&mut self.show_requested);
        let events = self.tray.as_ref().map(Tray::events).unwrap_or_default();
        for event in events {
            match event {
                Event::Show | Event::Toggle | Event::Menu(tray::SHOW) => {
                    show = true;
                    // Missed calls open the Calls tab.
                    if self.missed > 0 {
                        self.ui.page = crate::ui::Page::Calls;
                    }
                }
                Event::Menu(tray::QUIT) => self.quit(ctx),
                Event::Menu(tray::DND) => {
                    self.settings.calls.dnd = !self.settings.calls.dnd;
                    self.mark_settings_dirty();
                }
                Event::Menu(tray::FORWARD) => {
                    self.settings.calls.forwarding = !self.settings.calls.forwarding;
                    self.mark_settings_dirty();
                }
                Event::Menu(tray::AUTO_ANSWER) => {
                    self.settings.calls.auto_answer = !self.settings.calls.auto_answer;
                    self.mark_settings_dirty();
                }
                Event::Menu(tray::DEBUG) => {
                    show = true;
                    crate::ui::debug::set_open(self, true);
                }
                Event::Menu(_) => {}
            }
        }
        let requests = std::mem::take(&mut *self.requests.lock().unwrap_or_else(|e| e.into_inner()));
        for request in requests {
            match request {
                Request::Show => show = true,
                Request::ShowConversation(peer) => {
                    show = true;
                    self.ui.page = crate::ui::Page::Messages;
                    self.ui.messages.open = Some(peer);
                    self.ui.adding_call = false;
                }
                Request::ShowCalls => {
                    show = true;
                    self.ui.page = crate::ui::Page::Calls;
                    self.ui.adding_call = false;
                }
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
        let label = match self.missed {
            0 => String::new(),
            1 => "1 missed call".to_owned(),
            n => format!("{n} missed calls"),
        };
        if let Some(retry) = self.taskbar.show(frame, self.missed, &label) {
            ctx.request_repaint_after(retry.saturating_duration_since(Instant::now()));
        }
        if let Some(due) = self.directory.next_load() {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }

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
        // Ctrl+Shift+D toggles the debug window.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::D)) {
            let open = !self.debug.open;
            crate::ui::debug::set_open(self, open);
        }
        // Drawn from `logic` so it keeps updating while the main window is minimised.
        crate::ui::debug::show(ctx, self);

        // Call timers and ended-call cleanup.
        if self.sip.calls.has_calls() {
            ctx.request_repaint_after(Duration::from_millis(500));
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
        // Video textures belonged to the old window.
        self.ui.video = crate::ui::video::VideoView::default();
        self.taskbar = fastframe_dock::Taskbar::default();
        // The debug window is a child of the main window.
        crate::ui::debug::set_open(self, false);
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
        } else if self.sip.calls.has_calls() {
            // Keep ended calls expiring while hidden.
            Headless::WaitFor(Duration::from_millis(500))
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
