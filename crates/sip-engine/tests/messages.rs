//! A real SIP MESSAGE: the local "call by IP" account messages itself on
//! 127.0.0.1:5060. (A separate test binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{Command, Engine, EngineConfig, Event, LocalAccountConfig};

#[test]
#[cfg_attr(target_os = "macos", ignore = "pjproject's macOS video devices need the main run loop, which the test harness does not run")]
fn messaging_ourselves() {
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig { local_account: true, null_audio: true, ..EngineConfig::default() };
    let engine = Engine::spawn(config, move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    let deadline = Instant::now() + Duration::from_secs(15);
    let next = |what: &str| rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    while next("Started") != Event::Started {}
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));

    let text = "Hello ✓\nsecond line";
    engine.send(Command::SendMessage { id: 7, target: "127.0.0.1:5060".into(), text: text.into(), local: true });
    let (mut received, mut status) = (None, None);
    while received.is_none() || status.is_none() {
        match next("the message and its status") {
            Event::MessageReceived(m) => received = Some(m),
            Event::MessageStatus(s) => status = Some(s),
            _ => {}
        }
    }
    let (received, status) = (received.unwrap(), status.unwrap());
    assert_eq!(received.text, text, "UTF-8 and newlines arrive intact");
    assert!(received.from.contains("rustone-test"), "{}", received.from);
    assert_eq!(status.id, 7);
    assert!(status.delivered(), "{status:?}");

    // Without a main account the send fails at once, with the same id.
    engine.send(Command::SendMessage { id: 8, target: "1234".into(), text: "x".into(), local: false });
    loop {
        if let Event::MessageStatus(s) = next("the failure") {
            assert_eq!((s.id, s.code), (8, 0));
            assert!(!s.reason.is_empty());
            break;
        }
    }
    drop(engine);
}
