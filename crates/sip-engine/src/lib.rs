//! A safe, threaded wrapper around pjproject's pjsua.
//!
//! One engine thread owns pjsua: it creates and destroys it and makes every
//! pjsua call, so no other thread needs `pj_thread_register`. The
//! application sends [`Command`]s; pjsua's callbacks turn into [`Event`]s
//! delivered to the application's sink.
//!
//! pjsua is a process-wide singleton, so only one [`Engine`] can exist.

mod call;
mod config;
mod event;
mod pj;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

pub use call::{CallId, CallInfo, CallState, Direction, DtmfMethod, MediaState, Sound, uri_user_host};
pub use config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
pub use event::{Event, RegState};
pub use pj::CallOptions;

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
    /// Blind transfer to a number or SIP URI.
    Transfer { call: CallId, target: String },
    /// Attended transfer: join `call`'s party with `with`'s.
    AttendedTransfer { call: CallId, with: CallId },
    PlaySound(Sound),
    StopSound,
}

pub(crate) enum Message {
    Command(Box<Command>),
    /// A non-looping sound finished (from a media thread).
    PlayerEnded(i32),
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
}

impl Engine {
    /// Starts the engine thread and pjsua with `config`. Accounts are added
    /// with [`Command::SetAccount`].
    pub fn spawn(config: EngineConfig, sink: impl Fn(Event) + Send + Sync + 'static) -> Result<Self, SpawnError> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(SpawnError::AlreadyRunning);
        }
        let sink: Sink = Arc::new(sink);
        let (tx, rx) = mpsc::channel();
        let engine_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("sip-engine".into())
            .spawn(move || {
                run(config, sink, rx, engine_tx);
                RUNNING.store(false, Ordering::SeqCst);
            })
            .map_err(|e| {
                RUNNING.store(false, Ordering::SeqCst);
                SpawnError::Thread(e)
            })?;
        Ok(Self { tx, thread: Some(thread) })
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
    }
}

/// The engine thread: owns the pjsua session and the wanted accounts, so a
/// restart or resume brings them back.
fn run(mut config: EngineConfig, sink: Sink, rx: Receiver<Message>, tx: Sender<Message>) {
    pj::install_sink(sink.clone());
    pj::set_engine_tx(Some(tx));
    let mut account: Option<AccountConfig> = None;
    let mut local: Option<LocalAccountConfig> = None;
    let mut session = start(&config, &sink);
    let mut suspended = false;

    loop {
        let command = match rx.recv() {
            Ok(Message::Command(command)) => *command,
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
                add_accounts(&mut session, account.as_ref(), local.as_ref(), &sink);
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
                    add_accounts(&mut session, account.as_ref(), local.as_ref(), &sink);
                }
                suspended = false;
            }
            Command::SetCallOptions(options) => pj::set_options(options),
            other => match &mut session {
                Some(s) => call_command(s, other, &sink),
                None => {
                    if let Command::MakeCall { target, .. } = other {
                        sink(Event::CallFailed { target, reason: "the SIP engine isn't running".into() });
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
        | Command::SetCallOptions(_) => {}
    }
}

fn start(config: &EngineConfig, sink: &Sink) -> Option<pj::Session> {
    match pj::Session::start(config) {
        Ok(session) => {
            sink(Event::Started);
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

fn add_accounts(session: &mut Option<pj::Session>, account: Option<&AccountConfig>, local: Option<&LocalAccountConfig>, sink: &Sink) {
    if let Some(s) = session {
        s.set_local_account(local);
        s.set_account(account, sink);
    }
}
