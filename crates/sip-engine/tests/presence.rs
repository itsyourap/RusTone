//! A real presence subscription: an account without a registrar (which
//! subscribes at once) watches its own address on
//! 127.0.0.1:5060, and pjsua answers the SUBSCRIBE itself. (A separate test
//! binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{AccountConfig, Command, Engine, EngineConfig, Event, PresenceState};

#[test]
fn watching_our_own_presence() {
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig { local_account: true, null_audio: true, ..EngineConfig::default() };
    let engine = Engine::spawn(config, move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    let feed = engine.debug_feed();
    let deadline = Instant::now() + Duration::from_secs(15);
    let next = |what: &str| {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| {
            // pjsua's log, to see why.
            for line in feed.logs().iter().filter(|l| l.text.contains("pres") || l.text.contains("uddy") || l.text.contains("SUBSCRIBE") || l.level <= 2) {
                eprintln!("{}", line.text);
            }
            for m in feed.messages().iter().filter(|m| m.raw.contains("SUBSCRIBE") || m.raw.contains("NOTIFY")) {
                eprintln!("--- {}", m.raw.lines().next().unwrap_or_default());
            }
            panic!("timed out waiting for {what}")
        })
    };
    while next("Started") != Event::Started {}

    // The number to watch is set first; it's subscribed once the account is ready.
    engine.send(Command::SetBuddies(vec!["rustone-buddy".into(), "rustone-same@127.0.0.1:5060".into()]));
    engine.send(Command::SetAccount(Some(AccountConfig { username: "rustone-buddy".into(), domain: "127.0.0.1:5060".into(), ..AccountConfig::default() })));

    // Both numbers report under their own number. pjsua's own status starts
    // offline (only DND and away are published), so NOTIFY says so.
    let mut seen = std::collections::HashMap::new();
    while seen.len() < 2 || seen.values().any(|s| *s == PresenceState::Unknown) {
        if let Event::Presence(p) = next("Presence") {
            seen.insert(p.number.clone(), p.state);
        }
    }
    assert!(seen.contains_key("rustone-buddy") && seen.contains_key("rustone-same@127.0.0.1:5060"), "{seen:?}");
    assert!(seen.values().all(|s| *s == PresenceState::Offline), "{seen:?}");

    // Dropping a number unsubscribes it; the engine keeps working.
    engine.send(Command::SetBuddies(vec!["rustone-same@127.0.0.1:5060".into()]));
    engine.send(Command::SetBuddies(Vec::new()));
    drop(engine);
}
