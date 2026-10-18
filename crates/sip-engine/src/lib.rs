//! A safe, threaded wrapper around pjproject's pjsua.
//!
//! One engine thread owns pjsua: it creates and destroys it and makes every
//! pjsua call, so no other thread needs `pj_thread_register`. The
//! application sends [`Command`]s; pjsua's callbacks turn into [`Event`]s
//! delivered to the application's sink.
//!
//! pjsua is a process-wide singleton, so only one [`Engine`] can exist.

mod config;
mod event;
mod pj;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

pub use config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
pub use event::{Event, RegState};

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
}

enum Message {
    Command(Box<Command>),
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
        let thread = std::thread::Builder::new()
            .name("sip-engine".into())
            .spawn(move || {
                run(config, sink, rx);
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
fn run(mut config: EngineConfig, sink: Sink, rx: Receiver<Message>) {
    pj::install_sink(sink.clone());
    let mut account: Option<AccountConfig> = None;
    let mut local: Option<LocalAccountConfig> = None;
    let mut session = start(&config, &sink);
    let mut suspended = false;

    while let Ok(Message::Command(command)) = rx.recv() {
        match *command {
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
        }
    }
    stop(&mut session, &sink);
    pj::remove_sink();
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
