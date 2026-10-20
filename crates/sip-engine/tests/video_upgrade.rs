//! Video added to an audio call: the local account calls itself without
//! video, then the caller turns video on (a re-INVITE) and frames must
//! arrive. Uses pjmedia's colour-bar test camera; runs only when asked:
//! `cargo test -p sip-engine --test video_upgrade -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{CallState, Command, Direction, Engine, EngineConfig, Event, LocalAccountConfig, VideoConfig, VideoSource};

#[test]
#[ignore = "uses the video stack"]
fn video_can_be_added_to_an_audio_call() {
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig {
        local_account: true,
        null_audio: true,
        audio_codecs: vec!["PCMU/8000/1".into()],
        video: VideoConfig { enabled: true, camera: "Colorbar generator".into(), bitrate_kbps: 0 },
        ..EngineConfig::default()
    };
    let engine = Engine::spawn(config, move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    let deadline = Instant::now() + Duration::from_secs(25);
    let next = |what: &str| rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    while next("Started") != Event::Started {}
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));
    engine.send(Command::MakeCall { target: "127.0.0.1:5060".into(), video: false, local: true });
    let incoming = loop {
        if let Event::IncomingCall(c) = next("IncomingCall") {
            break c;
        }
    };
    engine.send(Command::Answer { call: incoming.id, code: 200, video: false });
    let outgoing = loop {
        match next("the audio call to connect") {
            Event::Call(c) if c.direction == Direction::Outgoing && c.state == CallState::Confirmed => break c,
            Event::CallError { reason, .. } => panic!("{reason}"),
            _ => {}
        }
    };
    assert!(!outgoing.video, "starts as audio only");
    std::thread::sleep(Duration::from_millis(500));

    engine.send(Command::AddVideo(outgoing.id));
    // Both legs get video; the callee accepts it with its camera off.
    let (mut caller, mut callee) = (None, None);
    while caller.is_none() || callee.is_none() {
        match next("video on both legs") {
            Event::Call(c) if c.video && c.id == outgoing.id => caller = Some(c),
            Event::Call(c) if c.video && c.id == incoming.id => callee = Some(c),
            Event::CallError { reason, .. } => panic!("{reason}"),
            _ => {}
        }
    }
    assert!(!caller.unwrap().camera_off, "the caller sends its camera");
    assert!(callee.unwrap().camera_off, "the callee's camera starts off");
    std::thread::sleep(Duration::from_secs(3));
    let frames = engine.video();
    // The callee sees the caller's camera; nothing comes back yet.
    let seen = |source| frames.latest(source).map_or(0, |f| f.seq);
    assert!(seen(VideoSource::Call(incoming.id)) > 5, "the caller's camera reaches the callee");
    let before = seen(VideoSource::Call(outgoing.id));
    std::thread::sleep(Duration::from_secs(1));
    assert!(seen(VideoSource::Call(outgoing.id)) <= before + 2, "the callee's camera is off");

    engine.send(Command::SetVideoSending { call: incoming.id, on: true });
    loop {
        match next("the callee's camera on") {
            Event::Call(c) if c.id == incoming.id && c.video && !c.camera_off => break,
            Event::CallError { reason, .. } => panic!("{reason}"),
            _ => {}
        }
    }
    std::thread::sleep(Duration::from_secs(3));
    assert!(seen(VideoSource::Call(outgoing.id)) > before + 5, "the callee's camera reaches the caller");
    engine.send(Command::HangupAll);
    std::thread::sleep(Duration::from_secs(1));
    drop(engine);
}
