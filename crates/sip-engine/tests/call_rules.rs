//! What the app's incoming-call rules need from the engine: the caller's
//! auto-answer headers, responses with a reason and extra headers (DND's
//! 486, forwarding's 302 with Contact and Diversion), delayed commands that
//! a later answer cancels, and hanging up with a Warning (auto hang-up).
//! A fake caller on UDP 127.0.0.1:5071 calls the engine's local account.
//! Runs only when asked: `cargo test -p sip-engine --test call_rules -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::Peer;
use sip_engine::{CallInfo, Command, Engine, EngineConfig, Event, LocalAccountConfig, RecordingOptions};

const CALLER: &str = "127.0.0.1:5071";
const ENGINE: &str = "127.0.0.1:5060";

#[test]
#[ignore = "uses UDP ports 5060 and 5071"]
fn responses_headers_and_delayed_commands() {
    let peer = Peer::bind(CALLER, Duration::from_secs(40));
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig { local_account: true, null_audio: true, audio_codecs: vec!["PCMU/8000/1".into()], ..EngineConfig::default() };
    let engine = Engine::spawn(config, move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    let next = |what: &str| rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    while next("Started") != Event::Started {}
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));
    std::thread::sleep(Duration::from_millis(300));
    let incoming = |call_id: &str| -> CallInfo {
        loop {
            if let Event::IncomingCall(c) = next("the incoming call")
                && c.sip_call_id == call_id
            {
                return c;
            }
        }
    };

    // 1. The caller asks for auto-answer; we decline with a reason.
    peer.invite(ENGINE, "rules-1", "100", "X-AUTOANSWER: TRUE\r\nCall-Info: <sip:127.0.0.1>;answer-after=3\r\n");
    let call = incoming("rules-1");
    assert_eq!(call.auto_answer_header.as_deref(), Some("TRUE"));
    assert!(call.call_info_header.as_deref().is_some_and(|v| v.contains("answer-after=3")), "{:?}", call.call_info_header);
    engine.send(Command::Respond { call: call.id, code: 486, reason: "Do Not Disturb".into(), headers: Vec::new() });
    let busy = peer.recv("the 486", |m| m.code() == 486);
    assert_eq!(busy.first, "SIP/2.0 486 Do Not Disturb");
    peer.ack(ENGINE, &busy);

    // 2. Forwarded after a delay: a 302 with Contact and Diversion.
    peer.invite(ENGINE, "rules-2", "101", "");
    let call = incoming("rules-2");
    let started = Instant::now();
    let forward = Command::Forward { call: call.id, target: "200@127.0.0.1".into() };
    engine.send(Command::Later { delay: Duration::from_secs(1), command: Box::new(forward) });
    let moved = peer.recv("the 302", |m| m.code() == 302);
    assert!(started.elapsed() >= Duration::from_millis(900), "sent after the delay ({:?})", started.elapsed());
    assert!(moved.header("Contact").contains("sip:200@127.0.0.1"), "Contact: {}", moved.header("Contact"));
    assert!(moved.header("Diversion").contains("sip:rustone-test@"), "Diversion (the called address): {}", moved.header("Diversion"));
    peer.ack(ENGINE, &moved);

    // 3. Answering cancels the delayed forward; ending sends a Warning.
    peer.invite(ENGINE, "rules-3", "102", "");
    let call = incoming("rules-3");
    let forward = Command::Respond { call: call.id, code: 302, reason: String::new(), headers: vec![("Contact".into(), "<sip:200@127.0.0.1>".into())] };
    engine.send(Command::Later { delay: Duration::from_millis(800), command: Box::new(forward) });
    engine.send(Command::Answer { call: call.id, code: 200, video: false });
    let ok = peer.recv("the 200", |m| m.code() >= 200 && m.header("Call-ID") == "rules-3");
    assert_eq!(ok.code(), 200, "answered: {}", ok.first);
    peer.ack(ENGINE, &ok);
    let hangup = Command::End { call: call.id, code: 200, headers: vec![("Warning".into(), "399 rustone \"Call duration exceeded\"".into())] };
    engine.send(Command::Later { delay: Duration::from_millis(1500), command: Box::new(hangup) });
    let bye = peer.recv("the BYE", |m| m.is_request("BYE") || m.code() == 302);
    assert!(bye.is_request("BYE"), "the cancelled forward was sent: {}", bye.first);
    assert!(bye.header("Warning").starts_with("399"), "Warning: {}", bye.header("Warning"));
    peer.reply(&bye, "200 OK", "", "");

    // 4. Hidden caller ID: Privacy: id and a single, anonymous From.
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), hide_cid: true, ..LocalAccountConfig::default() })));
    std::thread::sleep(Duration::from_millis(300));
    engine.send(Command::MakeCall { target: CALLER.into(), video: false, local: true });
    let invite = peer.recv("our INVITE", |m| m.is_request("INVITE"));
    assert_eq!(invite.header("Privacy"), "id");
    let froms: Vec<&str> = invite.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("From") || n == "f").map(|(_, v)| v.as_str()).collect();
    assert_eq!(froms.len(), 1, "one From: {froms:?}");
    assert!(froms[0].contains("anonymous@anonymous.invalid"), "anonymous From: {froms:?}");
    peer.reply(&invite, "486 Busy Here", "", "");

    // 5. Conference: two answered calls merged, then one separated.
    let mut ids = Vec::new();
    for n in ["conf-a", "conf-b"] {
        peer.invite(ENGINE, n, "300", "");
        let call = incoming(n);
        engine.send(Command::Answer { call: call.id, code: 200, video: false });
        let ok = peer.recv("the 200", |m| m.code() == 200 && m.header("Call-ID") == n);
        peer.ack(ENGINE, &ok);
        ids.push(call.id);
    }
    std::thread::sleep(Duration::from_millis(300));
    engine.send(Command::Merge(ids.clone()));
    let wait_conference = |want: bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = std::collections::HashMap::new();
        while ids.iter().any(|id| seen.get(id) != Some(&want)) {
            assert!(Instant::now() < deadline, "conference state {want} not reported: {seen:?}");
            if let Ok(Event::Call(c)) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.insert(c.id, c.in_conference);
            }
        }
    };
    wait_conference(true);
    engine.send(Command::Separate(ids[0]));
    // A conference of one ends.
    wait_conference(false);
    engine.send(Command::HangupAll);
    for _ in 0..2 {
        let bye = peer.recv("a conference BYE", |m| m.is_request("BYE"));
        peer.reply(&bye, "200 OK", "", "");
    }

    // 6. Recording: a WAV with a header, saved when the call ends.
    let dir = std::env::temp_dir().join(format!("rustone-rec-test-{}", std::process::id()));
    engine.send(Command::SetRecording(RecordingOptions { dir: dir.clone(), auto: false, label: "Test".into() }));
    peer.invite(ENGINE, "rec-1", "400", "");
    let call = incoming("rec-1");
    engine.send(Command::Answer { call: call.id, code: 200, video: false });
    let ok = peer.recv("the 200", |m| m.code() == 200 && m.header("Call-ID") == "rec-1");
    peer.ack(ENGINE, &ok);
    std::thread::sleep(Duration::from_millis(300));
    engine.send(Command::Record { call: call.id, on: true });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "not recording");
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Call(c)) if c.id == call.id && c.recording => break,
            Ok(Event::CallError { reason, .. }) => panic!("{reason}"),
            _ => {}
        }
    }
    std::thread::sleep(Duration::from_secs(1));
    engine.send(Command::Hangup(call.id));
    let bye = peer.recv("the BYE", |m| m.is_request("BYE"));
    peer.reply(&bye, "200 OK", "", "");
    let path = loop {
        assert!(Instant::now() < deadline + Duration::from_secs(5), "no RecordingSaved");
        if let Ok(Event::RecordingSaved { path, .. }) = rx.recv_timeout(Duration::from_millis(200)) {
            break path;
        }
    };
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("-400-incoming-Test.wav"), "the recording's name: {name}");
    let wav = std::fs::read(&path).expect("the recording exists");
    assert!(wav.starts_with(b"RIFF") && &wav[8..12] == b"WAVE", "a WAV file");
    assert!(wav.len() > 44 + 8000, "about a second of audio: {} bytes", wav.len());
    let _ = std::fs::remove_dir_all(&dir);

    // 7. Voicemail: an unsolicited MWI NOTIFY (as registrars send).
    let body = "Messages-Waiting: yes\r\nVoice-Message: 3/1 (0/0)\r\n";
    let notify = format!(
        "NOTIFY sip:rustone-test@{ENGINE} SIP/2.0\r\nVia: SIP/2.0/UDP {CALLER};branch=z9hG4bK-mwi1;rport\r\nMax-Forwards: 70\r\nFrom: <sip:vm@{CALLER}>;tag=mwi1\r\nTo: <sip:rustone-test@{ENGINE}>\r\nCall-ID: mwi-1\r\nCSeq: 1 NOTIFY\r\nEvent: message-summary\r\nContact: <sip:vm@{CALLER}>\r\nContent-Type: application/simple-message-summary\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    peer.send(ENGINE.parse().unwrap(), &notify);
    let ok = peer.recv("the NOTIFY's answer", |m| m.header("Call-ID") == "mwi-1" && m.code() > 0);
    assert_eq!(ok.code(), 200, "{}", ok.first);
    let deadline = Instant::now() + Duration::from_secs(5);
    let voicemail = loop {
        assert!(Instant::now() < deadline, "no Voicemail event");
        if let Ok(Event::Voicemail(v)) = rx.recv_timeout(Duration::from_millis(200)) {
            break v;
        }
    };
    assert!(voicemail.waiting && voicemail.new == Some(3) && voicemail.old == Some(1), "{voicemail:?}");
    std::thread::sleep(Duration::from_millis(300));
    drop(engine);
}
