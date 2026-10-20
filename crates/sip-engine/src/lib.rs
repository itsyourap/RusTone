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
mod video;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use audio::{AudioDevice, AudioDevices, Levels, Meters};
pub use call::{CallId, CallInfo, CallState, CodecInfo, Direction, DtmfMethod, MediaState, Sound, uri_user_host};
pub use config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
pub use event::{Event, RegState, Voicemail};
pub use pj::{CallOptions, RecordingOptions};
pub use message::{IncomingMessage, MessageId, MessageStatus};
pub use presence::{Presence, PresenceState};
pub use quality::CallQuality;
pub use video::{Frame, VideoConfig, VideoDevice, VideoFrames, VideoSource};

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
    /// Respond to an incoming call with a reason phrase and extra headers
    /// (486 "Do Not Disturb", 302 with Contact and Diversion).
    Respond { call: CallId, code: u16, reason: String, headers: Vec<(String, String)> },
    /// Forward a ringing call to `target` (a number or SIP URI): 302 with
    /// it as Contact and our address as Diversion.
    Forward { call: CallId, target: String },
    /// End a call (CANCEL, BYE or 603 as fits its state).
    Hangup(CallId),
    /// End a call with a status code and extra headers (e.g.
    /// auto hang-up sends a `Warning`).
    End { call: CallId, code: u16, headers: Vec<(String, String)> },
    /// Run `command` after `delay` (auto-answer, delayed forwarding, auto
    /// hang-up). Dropped when its call ends, or gets another final answer
    /// or hang-up first.
    Later { delay: Duration, command: Box<Command> },
    HangupAll,
    Hold(CallId),
    Unhold(CallId),
    /// Join calls in a conference (resuming held ones), or add calls to
    /// the running one.
    Merge(Vec<CallId>),
    /// Take a call out of the conference; it stays connected to us.
    Separate(CallId),
    /// Start or stop recording a call (with its conference) to WAV.
    Record { call: CallId, on: bool },
    /// Where recordings go, and whether answered calls are recorded.
    SetRecording(RecordingOptions),
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
    /// Show or hide the camera preview (Settings' preview button).
    SetPreview(bool),
    /// Turn our camera on or off during a call.
    SetVideoSending { call: CallId, on: bool },
    /// Add video to a call that has none.
    AddVideo(CallId),
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

impl Command {
    /// The call a command is about, if any.
    pub fn call(&self) -> Option<CallId> {
        match self {
            Self::Answer { call, .. } | Self::Respond { call, .. } | Self::End { call, .. } | Self::Forward { call, .. } => Some(*call),
            Self::Hangup(call) | Self::Hold(call) | Self::Unhold(call) | Self::AddVideo(call) => Some(*call),
            Self::Later { command, .. } => command.call(),
            _ => None,
        }
    }

    /// Whether it answers (finally) or ends its call.
    fn settles_call(&self) -> bool {
        match self {
            Self::Answer { code, .. } | Self::Respond { code, .. } => *code >= 200,
            Self::Hangup(_) | Self::End { .. } | Self::Forward { .. } => true,
            _ => false,
        }
    }
}

pub(crate) enum Message {
    Command(Box<Command>),
    /// A non-looping sound finished (from a media thread).
    PlayerEnded(i32),
    /// Windows reported a sound device change (from a COM thread).
    DevicesChanged,
    /// The main account registered (from pjsua's callback).
    Registered,
    /// A call ended (from pjsua's callback): its delayed commands go.
    CallEnded(CallId),
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
    video: Arc<VideoFrames>,
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
        let video = Arc::new(VideoFrames::default());
        pj::set_video_frames(Some(video.clone()));
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
        Ok(Self { tx, thread: Some(thread), debug: feed, meters, video })
    }

    /// The debug window's data (SIP messages, logs, streams, audio).
    pub fn debug_feed(&self) -> Arc<debug::DebugFeed> {
        self.debug.clone()
    }

    /// The latest video frames (calls' remote video and the preview).
    pub fn video(&self) -> Arc<VideoFrames> {
        self.video.clone()
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
        pj::set_video_frames(None);
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
    // Commands waiting for their time (Command::Later).
    let mut scheduled: Vec<(Instant, Command)> = Vec::new();

    loop {
        // Wake for the next timer: device refresh, or (while debugging)
        // sampling stream statistics.
        let first_scheduled = scheduled.iter().map(|(at, _)| *at).min();
        let deadline = [devices_due, debug_on.then_some(next_stats), meters_on.then_some(next_meter), meters_on.then_some(next_quality), first_scheduled]
            .into_iter()
            .flatten()
            .min();
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
                    match scheduled.iter().position(|(at, _)| *at <= now) {
                        Some(due) => Ok(Message::Command(Box::new(scheduled.remove(due).1))),
                        None => continue,
                    }
                }
            },
        };
        let command = match received {
            Ok(Message::Command(command)) => *command,
            Ok(Message::CallEnded(call)) => {
                scheduled.retain(|(_, c)| c.call() != Some(call));
                continue;
            }
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
        // A final answer or a hang-up settles the call: what was scheduled
        // for it (say, forwarding after auto-answer) no longer applies.
        if command.settles_call()
            && let Some(call) = command.call()
        {
            scheduled.retain(|(_, c)| c.call() != Some(call));
        }
        match command {
            Command::Later { delay, command } => scheduled.push((Instant::now() + delay, *command)),
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
            Command::SetRecording(options) => pj::set_recording(options),
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
        Command::Respond { call, code, reason, headers } => call_error(Some(call), s.respond(call, code, &reason, &headers)),
        Command::Forward { call, target } => call_error(Some(call), s.forward(call, &target)),
        Command::Hangup(call) => s.hangup(call, 0),
        Command::End { call, code, headers } => s.end(call, code, &headers),
        Command::HangupAll => s.hangup_all(),
        Command::Hold(call) => call_error(Some(call), s.hold(call)),
        Command::Unhold(call) => call_error(Some(call), s.unhold(call)),
        Command::Merge(calls) => {
            call_error(calls.first().copied(), s.merge(&calls));
            pj::report_calls();
        }
        Command::Record { call, on } => s.record(call, on),
        Command::Separate(call) => {
            pj::leave_conference(call);
            pj::report_calls();
        }
        Command::SetMute { mic, speaker } => s.set_mute(mic, speaker),
        Command::SendDtmf { call, digits, method } => call_error(Some(call), s.send_dtmf(call, &digits, method)),
        Command::PlayLocalDigits(digits) => s.play_local_digits(&digits),
        Command::Transfer { call, target } => call_error(Some(call), s.transfer(call, &target)),
        Command::SetPreview(on) => s.set_preview(on),
        Command::SetVideoSending { call, on } => call_error(Some(call), s.set_video_sending(call, on)),
        Command::AddVideo(call) => call_error(Some(call), s.add_video(call)),
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
        | Command::Later { .. }
        | Command::SetAccount(_)
        | Command::SetLocalAccount(_)
        | Command::Reregister
        | Command::IpChanged
        | Command::Suspend
        | Command::Resume
        | Command::SetCallOptions(_)
        | Command::SetRecording(_)
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
            sink(Event::VideoDevices(session.video_devices().to_vec()));
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
