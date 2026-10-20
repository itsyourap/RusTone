//! A safe, threaded wrapper around pjproject's pjsua.
//!
//! One engine thread owns pjsua: it creates and destroys it and makes every
//! pjsua call, so no other thread needs `pj_thread_register`. The
//! application sends [`Command`]s; pjsua's callbacks turn into [`Event`]s
//! delivered to the application's sink.
//!
//! pjsua is a process-wide singleton, so only one [`Engine`] can exist.

mod audio;
mod call;
mod config;
pub mod debug;
mod event;
mod message;
#[cfg(windows)]
mod hotplug;
mod pj;
mod presence;
mod quality;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use audio::{AudioDevice, AudioDevices, Levels, Meters};
pub use call::{CallId, CallInfo, CallState, CodecInfo, Direction, DtmfMethod, MediaState, Sound, uri_user_host};
pub use config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
pub use event::{Event, RegState};
pub use pj::CallOptions;
pub use message::{IncomingMessage, MessageId, MessageStatus};
pub use presence::{Presence, PresenceState};
pub use quality::CallQuality;

/// Receives engine events. Called from pjsua's threads: keep it short.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Restart pjsua with new global settings, keeping the accounts.
    Restart(EngineConfig),
    /// Set or remove the main (registering) account.
    SetAccount(Option<AccountConfig>),
    /// Set or remove the local "call by IP" account.
    SetLocalAccount(Option<LocalAccountConfig>),
    /// Register the main account again now.
    Reregister,
    /// The machine's network addresses changed.
    IpChanged,
    /// The machine is going to sleep: shut pjsua down.
    Suspend,
    /// The machine woke up: start pjsua again with the same settings.
    Resume,

    /// Options that apply without a restart (DTMF method, local DTMF).
    SetCallOptions(CallOptions),
    /// Call `target` (a number or SIP URI) from the main account, or from
    /// the local account when `local` is set.
    MakeCall { target: String, video: bool, local: bool },
    /// Respond to an incoming call: 180 to ring, 200 to answer, 486 to
    /// decline, 487 to ignore a duplicate.
    Answer { call: CallId, code: u16, video: bool },
    /// End a call (CANCEL, BYE or 603 as fits its state).
    Hangup(CallId),
    HangupAll,
    Hold(CallId),
    Unhold(CallId),
    /// Mute the microphone and/or the speaker (levels, all calls).
    SetMute { mic: bool, speaker: bool },
    SendDtmf { call: CallId, digits: String, method: DtmfMethod },
    /// Play digits on the local speaker only (keypad feedback).
    PlayLocalDigits(String),
    /// Send a text message (SIP MESSAGE, text/plain); its outcome comes back
    /// as [`Event::MessageStatus`] with the same `id`. `local` sends from the
    /// local account.
    SendMessage { id: MessageId, target: String, text: String, local: bool },
    /// Blind transfer to a number or SIP URI.
    Transfer { call: CallId, target: String },
    /// Attended transfer: join `call`'s party with `with`'s.
    AttendedTransfer { call: CallId, with: CallId },
    /// While a call is live: sample the sound device's signal levels into
    /// [`Engine::meters`] every 100 ms and report [`Event::CallQuality`]
    /// every second.
    MonitorCalls(bool),
    /// Speaker and microphone levels; applies without a restart.
    SetLevels(Levels),
    /// Watch these numbers' presence (contacts with presence on).
    SetBuddies(Vec<String>),
    /// Use these sound devices (by name); applies without a restart.
    SetAudioDevices(AudioDevices),
    PlaySound(Sound),
    StopSound,
    /// Collect stream statistics and audio for the debug window, and raise
    /// pjsua's log level (SIP messages and logs are always collected).
    SetDebug(bool),
}

pub(crate) enum Message {
    Command(Box<Command>),
    /// A non-looping sound finished (from a media thread).
    PlayerEnded(i32),
    /// Windows reported a sound device change (from a COM thread).
    DevicesChanged,
    /// The main account registered (from pjsua's callback).
    Registered,
    Shutdown,
}

#[derive(Debug)]
pub enum SpawnError {
    AlreadyRunning,
    Thread(std::io::Error),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "a SIP engine is already running"),
            Self::Thread(e) => write!(f, "cannot start the SIP engine thread: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Handle to the engine thread. Dropping it shuts pjsua down and waits for
/// the thread (pjsua unregisters accounts first, which can take seconds).
pub struct Engine {
    tx: Sender<Message>,
    thread: Option<JoinHandle<()>>,
    debug: Arc<debug::DebugFeed>,
    meters: Arc<Meters>,
}

impl Engine {
    /// Starts the engine thread and pjsua with `config`. Accounts are added
    /// with [`Command::SetAccount`].
    pub fn spawn(config: EngineConfig, sink: impl Fn(Event) + Send + Sync + 'static) -> Result<Self, SpawnError> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(SpawnError::AlreadyRunning);
        }
        let sink: Sink = Arc::new(sink);
        let feed = Arc::new(debug::DebugFeed::default());
        let meters = Arc::new(Meters::default());
        let run_meters = meters.clone();
        pj::debug::set_feed(Some(feed.clone()));
        let (tx, rx) = mpsc::channel();
        let engine_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("sip-engine".into())
            .spawn(move || {
                run(config, sink, rx, engine_tx, run_meters);
                RUNNING.store(false, Ordering::SeqCst);
            })
            .map_err(|e| {
                RUNNING.store(false, Ordering::SeqCst);
                SpawnError::Thread(e)
            })?;
        Ok(Self { tx, thread: Some(thread), debug: feed, meters })
    }

    /// The debug window's data (SIP messages, logs, streams, audio).
    pub fn debug_feed(&self) -> Arc<debug::DebugFeed> {
        self.debug.clone()
    }

    /// Live microphone and speaker levels (see [`Command::MonitorCalls`]).
    pub fn meters(&self) -> Arc<Meters> {
        self.meters.clone()
    }

    pub fn send(&self, command: Command) {
        // The thread only stops on Shutdown, so sending can't fail before Drop.
        let _ = self.tx.send(Message::Command(Box::new(command)));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        pj::debug::set_feed(None);
    }
}

/// The engine thread: owns the pjsua session and the wanted accounts, so a
/// restart or resume brings them back.
/// Sound device changes come in bursts; wait this long after the last one.
const DEVICE_SETTLE: Duration = Duration::from_millis(1500);
/// How often stream statistics are sampled while debugging.
const STATS_INTERVAL: Duration = Duration::from_secs(1);
/// The VU meter timer.
const METER_INTERVAL: Duration = Duration::from_millis(100);
/// The call timer, which updates the quality icon.
const QUALITY_INTERVAL: Duration = Duration::from_secs(1);

fn run(mut config: EngineConfig, sink: Sink, rx: Receiver<Message>, tx: Sender<Message>, meters: Arc<Meters>) {
    pj::install_sink(sink.clone());
    #[cfg(windows)]
    let _watcher = {
        let tx = tx.clone();
        hotplug::DeviceWatcher::start(move || {
            let _ = tx.send(Message::DevicesChanged);
        })
        .map_err(|e| log::warn!("no sound device notifications: {e}"))
        .ok()
    };
    pj::set_engine_tx(Some(tx));
    let mut account: Option<AccountConfig> = None;
    let mut local: Option<LocalAccountConfig> = None;
    let mut devices = AudioDevices::default();
    let mut buddies: Vec<String> = Vec::new();
    let mut session = start(&config, &sink);
    let mut suspended = false;
    let mut debug_on = false;
    let mut next_stats = Instant::now();
    let mut devices_due: Option<Instant> = None;
    let mut meters_on = false;
    let mut next_meter = Instant::now();
    let mut next_quality = Instant::now();
    let mut quality = pj::QualityMonitor::default();

    loop {
        // Wake for the next timer: device refresh, or (while debugging)
        // sampling stream statistics.
        let deadline = [devices_due, debug_on.then_some(next_stats), meters_on.then_some(next_meter), meters_on.then_some(next_quality)].into_iter().flatten().min();
        let received = match deadline {
            None => rx.recv(),
            Some(deadline) => match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(message) => Ok(message),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    if devices_due.is_some_and(|due| due <= now) {
                        devices_due = None;
                        if let Some(s) = &mut session {
                            s.refresh_devices();
                            sink(Event::AudioDevices(s.audio_devices().to_vec()));
                        }
                    }
                    if meters_on && next_meter <= now {
                        next_meter = now + METER_INTERVAL;
                        let (mic, speaker) = if session.is_some() { pj::signal_levels() } else { (0, 0) };
                        meters.set(mic, speaker);
                    }
                    if meters_on && next_quality <= now {
                        next_quality = now + QUALITY_INTERVAL;
                        if session.is_some() {
                            for q in quality.sample() {
                                sink(Event::CallQuality(q));
                            }
                        }
                    }
                    if debug_on && next_stats <= now {
                        next_stats = now + STATS_INTERVAL;
                        if session.is_some() {
                            pj::debug::poll_streams();
                        }
                    }
                    continue;
                }
            },
        };
        let command = match received {
            Ok(Message::Command(command)) => *command,
            Ok(Message::Registered) => {
                if let Some(s) = &mut session {
                    s.start_presence();
                }
                continue;
            }
            Ok(Message::DevicesChanged) => {
                devices_due = Some(Instant::now() + DEVICE_SETTLE);
                continue;
            }
            Ok(Message::PlayerEnded(id)) => {
                if let Some(s) = &mut session {
                    s.sound_ended(id);
                }
                continue;
            }
            Ok(Message::Shutdown) | Err(_) => break,
        };
        match command {
            Command::Restart(new_config) => {
                config = new_config;
                stop(&mut session, &sink);
                session = start(&config, &sink);
                set_devices(&mut session, &devices);
                set_buddies(&mut session, &buddies);
                add_accounts(&mut session, account.as_ref(), local.as_ref(), &sink);
                if debug_on && session.is_some() {
                    pj::debug::apply(true);
                }
                suspended = false;
            }
            Command::SetAccount(new) => {
                account = new;
                if let Some(s) = &mut session {
                    s.set_account(account.as_ref(), &sink);
                } else if account.is_none() {
                    sink(Event::Registration(RegState::Unregistered));
                }
            }
            Command::SetLocalAccount(new) => {
                local = new;
                if let Some(s) = &mut session {
                    s.set_local_account(local.as_ref());
                }
            }
            Command::Reregister => {
                if let Some(s) = &session {
                    s.reregister();
                }
            }
            Command::IpChanged => {
                if let Some(s) = &session {
                    s.ip_changed();
                }
            }
            Command::Suspend => {
                stop(&mut session, &sink);
                suspended = true;
            }
            Command::Resume => {
                if suspended && session.is_none() {
                    session = start(&config, &sink);
                    set_devices(&mut session, &devices);
                    set_buddies(&mut session, &buddies);
                    add_accounts(&mut session, account.as_ref(), local.as_ref(), &sink);
                }
                suspended = false;
            }
            Command::SetCallOptions(options) => pj::set_options(options),
            Command::MonitorCalls(on) => {
                meters_on = on;
                next_meter = Instant::now();
                next_quality = Instant::now() + QUALITY_INTERVAL;
                if !on {
                    meters.set(0, 0);
                }
            }
            Command::SetLevels(levels) => pj::set_levels(levels, session.is_some()),
            Command::SetBuddies(new) => {
                buddies = new;
                if let Some(s) = &mut session {
                    s.set_buddies(&buddies);
                }
            }
            Command::SetAudioDevices(new) => {
                devices = new;
                set_devices(&mut session, &devices);
            }
            Command::SetDebug(on) => {
                debug_on = on;
                next_stats = Instant::now() + STATS_INTERVAL;
                pj::debug::feed_enabled(on);
                if session.is_some() {
                    pj::debug::apply(on);
                }
            }
            other => match &mut session {
                Some(s) => call_command(s, other, &sink),
                None => {
                    match other {
                        Command::MakeCall { target, .. } => sink(Event::CallFailed { target, reason: "the SIP engine isn't running".into() }),
                        Command::SendMessage { id, .. } => sink(Event::MessageStatus(MessageStatus { id, code: 0, reason: "the SIP engine isn't running".into() })),
                        _ => {}
                    }
                }
            },
        }
    }
    stop(&mut session, &sink);
    pj::set_engine_tx(None);
    pj::remove_sink();
}

/// Commands that need a running session.
fn call_command(s: &mut pj::Session, command: Command, sink: &Sink) {
    let call_error = |call: Option<CallId>, result: Result<(), String>| {
        if let Err(reason) = result {
            log::warn!("{reason}");
            sink(Event::CallError { call, reason });
        }
    };
    match command {
        Command::MakeCall { target, video, local } => {
            if let Err(reason) = s.make_call(&target, video, local) {
                log::warn!("call failed: {reason}");
                sink(Event::CallFailed { target, reason });
            }
        }
        Command::Answer { call, code, video } => call_error(Some(call), s.answer(call, code, video)),
        Command::Hangup(call) => s.hangup(call, 0),
        Command::HangupAll => s.hangup_all(),
        Command::Hold(call) => call_error(Some(call), s.hold(call)),
        Command::Unhold(call) => call_error(Some(call), s.unhold(call)),
        Command::SetMute { mic, speaker } => s.set_mute(mic, speaker),
        Command::SendDtmf { call, digits, method } => call_error(Some(call), s.send_dtmf(call, &digits, method)),
        Command::PlayLocalDigits(digits) => s.play_local_digits(&digits),
        Command::Transfer { call, target } => call_error(Some(call), s.transfer(call, &target)),
        Command::SendMessage { id, target, text, local } => {
            if let Err(reason) = s.send_message(id, &target, &text, local) {
                log::warn!("message not sent: {reason}");
                sink(Event::MessageStatus(MessageStatus { id, code: 0, reason }));
            }
        }
        Command::AttendedTransfer { call, with } => call_error(Some(call), s.attended_transfer(call, with)),
        Command::PlaySound(sound) => call_error(None, s.play_sound(&sound)),
        Command::StopSound => s.stop_sound(),
        // Handled by the engine loop.
        Command::Restart(_)
        | Command::SetAccount(_)
        | Command::SetLocalAccount(_)
        | Command::Reregister
        | Command::IpChanged
        | Command::Suspend
        | Command::Resume
        | Command::SetCallOptions(_)
        | Command::SetAudioDevices(_)
        | Command::SetBuddies(_)
        | Command::SetLevels(_)
        | Command::MonitorCalls(_)
        | Command::SetDebug(_) => {}
    }
}

fn start(config: &EngineConfig, sink: &Sink) -> Option<pj::Session> {
    match pj::Session::start(config) {
        Ok(session) => {
            sink(Event::Started);
            sink(Event::Codecs(session.codecs()));
            sink(Event::AudioDevices(session.audio_devices().to_vec()));
            Some(session)
        }
        Err(e) => {
            log::error!("SIP engine failed to start: {e}");
            sink(Event::Failed(e));
            None
        }
    }
}

fn stop(session: &mut Option<pj::Session>, sink: &Sink) {
    if let Some(s) = session.take() {
        s.stop();
        sink(Event::Stopped);
        sink(Event::Registration(RegState::Unregistered));
    }
}

fn set_devices(session: &mut Option<pj::Session>, devices: &AudioDevices) {
    if let Some(s) = session {
        s.set_audio_devices(devices);
    }
}

fn set_buddies(session: &mut Option<pj::Session>, numbers: &[String]) {
    if let Some(s) = session {
        s.set_buddies(numbers);
    }
}

fn add_accounts(session: &mut Option<pj::Session>, account: Option<&AccountConfig>, local: Option<&LocalAccountConfig>, sink: &Sink) {
    if let Some(s) = session {
        s.set_local_account(local);
        s.set_account(account, sink);
    }
}
