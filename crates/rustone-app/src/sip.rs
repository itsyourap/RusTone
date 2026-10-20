//! Connects the settings to the SIP engine and keeps the engine's state for
//! the UI.

use std::path::PathBuf;
use std::borrow::Cow;
use std::sync::{Arc, Mutex, OnceLock};

use fastframe_shell::Waker;
use rustone_core::account::{Account, LocalAccount, SrtpMode, Transport};
use rustone_core::settings::{AudioSettings, Settings, split_list};
use sip_engine::debug::{DebugFeed, LogSource};
use sip_engine::{AccountConfig, AudioDevice, AudioDevices, CallOptions, Levels, Meters, Command, Engine, EngineConfig, Event, LocalAccountConfig, RegState, Srtp};

use crate::calls::{CallBook, SoundPaths};

/// What the engine was last told, so only changes are sent.
#[derive(Default)]
struct Applied {
    config: Option<EngineConfig>,
    account: Option<Option<AccountConfig>>,
    local: Option<Option<LocalAccountConfig>>,
    options: Option<CallOptions>,
    devices: Option<AudioDevices>,
    levels: Option<Levels>,
    meters: Option<bool>,
    buddies: Option<Vec<String>>,
    recording: Option<sip_engine::RecordingOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    Starting,
    Running,
    Failed(String),
    Stopped,
}

/// The registration status shown in the header and the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NoAccount,
    Starting,
    Connecting,
    Online,
    /// An account without a registrar (calls only).
    Ready,
    Offline,
    Error(String),
}

impl Status {
    pub fn label(&self) -> &str {
        match self {
            Self::NoAccount => "No account",
            Self::Starting => "Starting…",
            Self::Connecting => "Connecting…",
            Self::Online => "Online",
            Self::Ready => "Ready",
            Self::Offline => "Not registered",
            Self::Error(_) => "Error",
        }
    }

    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online | Self::Ready)
    }
}

pub struct Sip {
    engine: Option<Engine>,
    events: Arc<Mutex<Vec<Event>>>,
    applied: Applied,
    log_file: PathBuf,
    pub engine_state: EngineState,
    pub reg: RegState,
    /// Audio codecs the engine has (ids, preferred first), with priorities.
    pub codecs: Vec<sip_engine::CodecInfo>,
    /// Sound devices the engine sees.
    pub audio_devices: Vec<AudioDevice>,
    /// Video devices (cameras and renderers) the engine sees.
    pub video_devices: Vec<sip_engine::VideoDevice>,
    /// The camera preview is on from Settings.
    pub preview: bool,
    /// Calls were live last time: their end stops the preview.
    had_calls: bool,
    /// Watched contacts' presence, by the number they were subscribed with.
    pub presence: std::collections::HashMap<String, sip_engine::Presence>,
    /// The voicemail box's latest summary (MWI).
    pub voicemail: Option<sip_engine::Voicemail>,
    pub calls: CallBook,
    feed: Arc<DebugFeed>,
    meters: Arc<Meters>,
    video: Arc<sip_engine::VideoFrames>,
}

/// What a poll produced that the window should react to.
#[derive(Debug, Default)]
pub struct Polled {
    /// An incoming call wants the window shown.
    pub show_window: bool,
    /// Calls that ended, for the history.
    pub ended: Vec<crate::calls::EndedCall>,
    /// Text messages that arrived, and answers to ones we sent.
    pub messages: Vec<sip_engine::IncomingMessage>,
    pub message_statuses: Vec<sip_engine::MessageStatus>,
}

impl Sip {
    pub fn spawn(settings: &Settings, log_file: PathBuf, sounds: SoundPaths, waker: &Waker) -> Self {
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink_events = events.clone();
        let sink_waker = waker.clone();
        let config = engine_config(settings, &log_file);
        let (engine, engine_state) = match Engine::spawn(config.clone(), move |event| {
            sink_events.lock().unwrap_or_else(|e| e.into_inner()).push(event);
            sink_waker.wake();
        }) {
            Ok(engine) => (Some(engine), EngineState::Starting),
            Err(e) => {
                log::error!("{e}");
                (None, EngineState::Failed(e.to_string()))
            }
        };
        let feed = engine.as_ref().map_or_else(|| Arc::new(DebugFeed::default()), Engine::debug_feed);
        let meters = engine.as_ref().map_or_else(|| Arc::new(Meters::default()), Engine::meters);
        let video = engine.as_ref().map_or_else(|| Arc::new(sip_engine::VideoFrames::default()), Engine::video);
        let _ = APP_LOG_FEED.set(feed.clone());
        let mut sip = Self {
            feed,
            meters,
            video,
            engine,
            events,
            applied: Applied { config: Some(config), ..Applied::default() },
            log_file,
            engine_state,
            reg: RegState::Unregistered,
            codecs: Vec::new(),
            audio_devices: Vec::new(),
            video_devices: Vec::new(),
            preview: false,
            had_calls: false,
            presence: std::collections::HashMap::new(),
            voicemail: None,
            calls: CallBook::new(sounds),
        };
        sip.sync(settings);
        sip
    }

    /// Sends the engine whatever changed in the settings since last time.
    pub fn sync(&mut self, settings: &Settings) {
        let Some(engine) = &self.engine else { return };
        let config = engine_config(settings, &self.log_file);
        // A restart drops calls: wait until none are live.
        if self.applied.config.as_ref() != Some(&config) && self.calls.live_count() == 0 {
            log::info!("SIP settings changed; restarting the engine");
            engine.send(Command::Restart(config.clone()));
            self.applied.config = Some(config.clone());
        }
        let account = settings.active().filter(|a| a.is_usable()).map(|a| account_config(a, &settings.sip));
        if self.applied.account.as_ref() != Some(&account) {
            self.calls.account = settings.active().filter(|a| a.is_usable()).map(|a| rustone_core::rules::Identity { user: a.username.clone(), domain: a.domain.clone() });
            // The engine drops the old account's subscriptions.
            self.presence.clear();
            self.voicemail = None;
            engine.send(Command::SetAccount(account.clone()));
            self.applied.account = Some(account);
        }
        let options = call_options(settings);
        if self.applied.options != Some(options) {
            engine.send(Command::SetCallOptions(options));
            self.applied.options = Some(options);
        }
        let recording = sip_engine::RecordingOptions {
            dir: settings.recording.dir(),
            auto: settings.recording.auto,
            label: settings.active().map(|a| a.label.trim().to_owned()).unwrap_or_default(),
        };
        if self.applied.recording.as_ref() != Some(&recording) {
            engine.send(Command::SetRecording(recording.clone()));
            self.applied.recording = Some(recording);
        }
        let devices = audio_devices(&settings.audio);
        if self.applied.devices.as_ref() != Some(&devices) {
            engine.send(Command::SetAudioDevices(devices.clone()));
            self.applied.devices = Some(devices);
        }
        let levels = levels(&settings.audio);
        if self.applied.levels != Some(levels) {
            engine.send(Command::SetLevels(levels));
            self.applied.levels = Some(levels);
        }
        // The preview closes when the last call ends.
        let live = self.calls.live_count() > 0;
        if self.had_calls && !live && !self.preview {
            engine.send(Command::SetPreview(false));
        }
        self.had_calls = live;
        // The VU meter only matters while a call is shown.
        let meters = self.calls.live_count() > 0;
        if self.applied.meters != Some(meters) {
            engine.send(Command::MonitorCalls(meters));
            self.applied.meters = Some(meters);
        }
        self.calls.ring_level = f32::from(settings.audio.volume_ring.min(100)) / 100.0;
        let local = local_config(&settings.local_account);
        if self.applied.local.as_ref() != Some(&local) {
            engine.send(Command::SetLocalAccount(local.clone()));
            self.applied.local = Some(local);
        }
    }

    /// Watches these numbers' presence (sent only when the list changes).
    pub fn set_buddies(&mut self, numbers: Vec<String>) {
        if self.applied.buddies.as_ref() != Some(&numbers) {
            self.presence.retain(|number, _| numbers.contains(number));
            self.send(Command::SetBuddies(numbers.clone()));
            self.applied.buddies = Some(numbers);
        }
    }

    pub fn send(&self, command: Command) {
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    /// The latest video frames (calls' remote video and the preview).
    pub fn video(&self) -> Arc<sip_engine::VideoFrames> {
        self.video.clone()
    }

    /// Live microphone and speaker levels.
    pub fn meters(&self) -> &Meters {
        &self.meters
    }

    /// The debug window's data.
    pub fn debug_feed(&self) -> Arc<DebugFeed> {
        self.feed.clone()
    }

    /// Sends commands produced by the call policy or the UI.
    pub fn run(&self, commands: Vec<Command>) {
        for command in commands {
            self.send(command);
        }
    }

    /// Applies queued engine events.
    pub fn poll(&mut self, settings: &Settings) -> Polled {
        let mut polled = Polled::default();
        let events = std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()));
        for event in events {
            let outcome = self.calls.on_event(&event, &settings.calls);
            polled.show_window |= outcome.show_window;
            polled.ended.extend(outcome.ended);
            self.run(outcome.commands);
            match event {
                Event::Started => self.engine_state = EngineState::Running,
                Event::Codecs(codecs) => self.codecs = codecs,
                Event::AudioDevices(devices) => self.audio_devices = devices,
                Event::VideoDevices(devices) => self.video_devices = devices,
                Event::Voicemail(v) => self.voicemail = Some(v),
                Event::Presence(p) => {
                    self.presence.insert(p.number.clone(), p);
                }
                Event::MessageReceived(m) => polled.messages.push(m),
                Event::MessageStatus(s) => polled.message_statuses.push(s),
                Event::Stopped => {
                    self.engine_state = EngineState::Stopped;
                    self.presence.clear();
                }
                Event::Failed(e) => self.engine_state = EngineState::Failed(e),
                Event::Registration(state) => {
                    match &state {
                        RegState::Registered { expires } => log::info!("registered (expires in {expires}s)"),
                        RegState::Failed { code, reason, .. } => log::warn!("registration failed: {code} {reason}"),
                        _ => {}
                    }
                    self.reg = state;
                }
                _ => {}
            }
        }
        self.calls.tick();
        polled
    }

    pub fn status(&self, settings: &Settings) -> Status {
        match &self.engine_state {
            EngineState::Failed(e) => return Status::Error(e.clone()),
            EngineState::Starting => return Status::Starting,
            EngineState::Stopped => return Status::Offline,
            EngineState::Running => {}
        }
        if settings.active().is_none_or(|a| !a.is_usable()) {
            return Status::NoAccount;
        }
        match &self.reg {
            RegState::Unregistered => Status::Offline,
            RegState::NoRegistrar => Status::Ready,
            RegState::Registering => Status::Connecting,
            RegState::Registered { .. } => Status::Online,
            RegState::Failed { code, reason, detail } => Status::Error(failure_text(*code, reason, detail.as_deref())),
        }
    }

    /// Shuts pjsua down (unregistering). Blocks for up to a few seconds.
    pub fn shutdown(&mut self) {
        self.engine = None;
    }
}

/// Where RusTone's own log lines go for the debug window.
static APP_LOG_FEED: OnceLock<Arc<DebugFeed>> = OnceLock::new();

/// fastframe-log's redactor hook, used as a tap: copies RusTone's log lines
/// into the debug feed and leaves them unchanged. pjsua's lines reach the
/// feed directly, so they're skipped here.
pub fn app_log_tap(record: &log::Record<'_>, message: &str) -> Option<Cow<'static, str>> {
    if let Some(feed) = APP_LOG_FEED.get()
        && !record.target().starts_with("sip_engine::pjsua")
    {
        let level = match record.level() {
            log::Level::Error => 1,
            log::Level::Warn => 2,
            log::Level::Info => 3,
            log::Level::Debug => 4,
            log::Level::Trace => 5,
        };
        feed.push_log(LogSource::App, level, format!("{}: {message}", record.target()));
    }
    None
}

fn failure_text(code: u16, reason: &str, detail: Option<&str>) -> String {
    let base = match (code, reason.is_empty()) {
        (0, _) => reason.to_owned(),
        (_, true) => format!("Error {code}"),
        (_, false) => format!("{code} {reason}"),
    };
    match detail {
        Some(detail) => format!("{base}: {detail}"),
        None => base,
    }
}

pub fn engine_config(settings: &Settings, log_file: &std::path::Path) -> EngineConfig {
    let (sip, local) = (&settings.sip, &settings.local_account);
    EngineConfig {
        user_agent: sip.user_agent.trim().to_owned(),
        rport: sip.rport,
        source_port: sip.source_port,
        rtp_port_min: sip.rtp_port_min,
        rtp_port_max: sip.rtp_port_max,
        stun_servers: if sip.enable_stun { split_list(&sip.stun) } else { Vec::new() },
        nameservers: if sip.dns_srv { split_list(&sip.dns_srv_ns) } else { Vec::new() },
        audio_codecs: sip.audio_codecs.split_whitespace().map(str::to_owned).collect(),
        vad: sip.vad,
        ec: sip.ec,
        opus_stereo: sip.opus_stereo,
        local_account: local.enabled,
        log_file: sip.enable_log.then(|| log_file.to_path_buf()),
        null_audio: false,
        video: sip_engine::VideoConfig {
            enabled: settings.features.video,
            camera: settings.video.camera.clone(),
            bitrate_kbps: settings.video.bitrate_kbps,
        },
    }
}

pub fn account_config(a: &Account, _sip: &rustone_core::settings::SipSettings) -> AccountConfig {
    AccountConfig {
        server: a.server.trim().to_owned(),
        proxies: a.proxies(),
        domain: a.domain.trim().to_owned(),
        username: a.username.trim().to_owned(),
        password: a.password.clone(),
        auth_id: a.auth_id.trim().to_owned(),
        display_name: a.display_name.trim().to_owned(),
        transport: match a.transport {
            Transport::Udp => sip_engine::Transport::Udp,
            Transport::Tcp => sip_engine::Transport::Tcp,
            Transport::Tls => sip_engine::Transport::Tls,
        },
        srtp: match a.srtp {
            SrtpMode::Disabled => Srtp::Disabled,
            SrtpMode::Optional => Srtp::Optional,
            SrtpMode::Mandatory => Srtp::Mandatory,
            SrtpMode::Dtls => Srtp::Dtls,
            SrtpMode::DtlsSdes => Srtp::DtlsSdes,
        },
        public_addr: a.public_addr.trim().to_owned(),
        register_refresh: a.register_refresh,
        keep_alive: a.keep_alive,
        publish: a.publish,
        ice: a.ice,
        allow_rewrite: a.allow_rewrite,
        disable_session_timer: a.disable_session_timer,
        mwi: !a.voicemail_number.trim().is_empty(),
        hide_cid: a.hide_cid,
    }
}

fn call_options(settings: &Settings) -> CallOptions {
    use rustone_core::settings::DtmfMethod as M;
    CallOptions {
        dtmf_method: match settings.calls.dtmf_method {
            M::Auto => sip_engine::DtmfMethod::Auto,
            M::InBand => sip_engine::DtmfMethod::InBand,
            M::Rfc2833 => sip_engine::DtmfMethod::Rfc2833,
            M::SipInfo => sip_engine::DtmfMethod::SipInfo,
        },
        local_dtmf: settings.calls.local_dtmf,
        force_codec: settings.sip.force_codec,
    }
}

fn audio_devices(audio: &AudioSettings) -> AudioDevices {
    AudioDevices { input: audio.input_device.clone(), output: audio.output_device.clone(), ring: audio.ring_device.clone() }
}

fn levels(audio: &AudioSettings) -> Levels {
    Levels {
        speaker: audio.volume_output,
        mic: audio.volume_input,
        mic_amplification: audio.mic_amplification,
        software_mic: audio.sw_level_adjustment,
    }
}

fn local_config(local: &LocalAccount) -> Option<LocalAccountConfig> {
    local.enabled.then(|| LocalAccountConfig {
        display_name: local.display_name.trim().to_owned(),
        username: local.username.trim().to_owned(),
        domain: local.domain.trim().to_owned(),
        hide_cid: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_follows_switches() {
        let mut settings = Settings::default();
        settings.sip = rustone_core::settings::SipSettings { stun: "a; b".into(), dns_srv_ns: "8.8.8.8".into(), ..Default::default() };
        let log = std::path::Path::new("sip.log");
        let c = engine_config(&settings, log);
        assert!(c.stun_servers.is_empty() && c.nameservers.is_empty() && c.log_file.is_none());
        settings.sip.enable_stun = true;
        settings.sip.dns_srv = true;
        settings.sip.enable_log = true;
        settings.local_account = LocalAccount { enabled: true, ..LocalAccount::default() };
        settings.features.video = false;
        let c = engine_config(&settings, log);
        assert!(!c.video.enabled);
        assert_eq!(c.stun_servers, ["a", "b"]);
        assert_eq!(c.nameservers, ["8.8.8.8"]);
        assert!(c.local_account);
        assert_eq!(c.log_file.as_deref(), Some(log));
    }

    #[test]
    fn account_maps_and_trims() {
        let a = Account {
            server: " sip.example.com ".into(),
            proxy: "192.168.29.1:5068".into(),
            username: "+910000000000@sip.example.com".into(),
            password: "pw".into(),
            transport: Transport::Tls,
            voicemail_number: "*86".into(),
            ..Account::default()
        };
        let c = account_config(&a, &rustone_core::settings::SipSettings::default());
        assert_eq!(c.server, "sip.example.com");
        assert_eq!(c.proxies, ["192.168.29.1:5068"]);
        assert_eq!(c.transport, sip_engine::Transport::Tls);
        assert!(c.mwi);
        assert_eq!(c.password, "pw");
    }

    #[test]
    fn failures_read_well() {
        assert_eq!(failure_text(403, "Forbidden", None), "403 Forbidden");
        assert_eq!(failure_text(403, "Forbidden", Some("Wrong password")), "403 Forbidden: Wrong password");
        assert_eq!(failure_text(0, "Connection refused", None), "Connection refused");
    }
}
