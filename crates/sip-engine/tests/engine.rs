//! Runs the real engine (pjsua is a process-wide singleton, so this is one
//! sequential test). Needs the runtime DLLs on PATH; see CLAUDE.md.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{AccountConfig, Command, Engine, EngineConfig, Event, LocalAccountConfig, RegState, Transport};

fn wait_for(rx: &mpsc::Receiver<Event>, timeout: Duration, what: &str, mut want: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(event) if want(&event) => return event,
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[test]
fn engine_lifecycle() {
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig { local_account: true, ..EngineConfig::default() };
    let engine = Engine::spawn(config.clone(), move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    wait_for(&rx, Duration::from_secs(10), "Started", |e| *e == Event::Started);

    assert!(matches!(Engine::spawn(EngineConfig::default(), |_| {}), Err(sip_engine::SpawnError::AlreadyRunning)));

    // An account without a registrar is usable at once.
    engine.send(Command::SetAccount(Some(AccountConfig { username: "alice".into(), domain: "example.invalid".into(), ..AccountConfig::default() })));
    wait_for(&rx, Duration::from_secs(5), "NoRegistrar", |e| *e == Event::Registration(RegState::NoRegistrar));

    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig::default())));

    // A network change re-creates transports with the accounts in place.
    engine.send(Command::IpChanged);
    engine.send(Command::Reregister);

    // A registrar that refuses the connection fails quickly with a reason.
    engine.send(Command::SetAccount(Some(AccountConfig {
        server: "127.0.0.1:1".into(),
        username: "bob".into(),
        domain: "127.0.0.1".into(),
        password: "x".into(),
        transport: Transport::Tcp,
        ..AccountConfig::default()
    })));
    wait_for(&rx, Duration::from_secs(5), "Registering", |e| *e == Event::Registration(RegState::Registering));
    let failed = wait_for(&rx, Duration::from_secs(20), "Failed", |e| matches!(e, Event::Registration(RegState::Failed { .. })));
    let Event::Registration(RegState::Failed { reason, .. }) = failed else { unreachable!() };
    assert!(!reason.is_empty());

    engine.send(Command::SetAccount(None));
    wait_for(&rx, Duration::from_secs(5), "Unregistered", |e| *e == Event::Registration(RegState::Unregistered));

    // Restart with new settings, then suspend and resume.
    engine.send(Command::Restart(EngineConfig { local_account: false, ..config }));
    wait_for(&rx, Duration::from_secs(10), "Stopped", |e| *e == Event::Stopped);
    wait_for(&rx, Duration::from_secs(10), "Started", |e| *e == Event::Started);
    engine.send(Command::Suspend);
    wait_for(&rx, Duration::from_secs(10), "Stopped", |e| *e == Event::Stopped);
    engine.send(Command::Resume);
    wait_for(&rx, Duration::from_secs(10), "Started", |e| *e == Event::Started);

    drop(engine);
    // A new engine can start once the old one is gone.
    let again = Engine::spawn(EngineConfig::default(), |_| {}).expect("restartable");
    drop(again);
}
