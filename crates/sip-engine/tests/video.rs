//! A real video call: the local account calls itself with video, using
//! pjmedia's colour-bar test camera, and video must flow both ways. It opens
//! SDL video windows, so it only runs when asked:
//! `cargo test -p sip-engine --test video -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{CallState, Command, Direction, Engine, EngineConfig, Event, LocalAccountConfig, VideoConfig};

#[test]
#[ignore = "opens video windows on the desktop"]
fn video_flows_both_ways() {
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
    let feed = engine.debug_feed();
    let deadline = Instant::now() + Duration::from_secs(20);
    let next = |what: &str| rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    while next("Started") != Event::Started {}
    // pjsua's call dump (with the video stream's peer) is logged at debug level.
    engine.send(Command::SetDebug(true));
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));
    engine.send(Command::MakeCall { target: "127.0.0.1:5060".into(), video: true, local: true });
    let incoming = loop {
        if let Event::IncomingCall(c) = next("IncomingCall") {
            break c;
        }
    };
    assert!(incoming.remote_video, "the caller offers video");
    // Ringing first without settings, then the real answer decides on video.
    engine.send(Command::Answer { call: incoming.id, code: 180, video: false });
    engine.send(Command::Answer { call: incoming.id, code: 200, video: true });
    loop {
        if let Event::Call(c) = next("the call to connect")
            && c.direction == Direction::Outgoing
            && c.state == CallState::Confirmed
        {
            break;
        }
    }
    std::thread::sleep(Duration::from_secs(3));
    engine.send(Command::HangupAll);
    std::thread::sleep(Duration::from_secs(1));

    // Both legs' dumps: a video stream with a known peer (RTP arrived) and
    // received packets. Without a format converter (libyuv) nothing is sent.
    let dumps: Vec<String> = feed.logs().iter().filter_map(|l| l.text.find("#1 video").map(|at| l.text[at..].to_owned())).collect();
    assert!(!dumps.is_empty(), "no video stream in the call dumps");
    for dump in &dumps {
        assert!(!dump.contains("peer=-"), "no video arrived:\n{dump}");
        let rx_line = dump.lines().find(|l| l.trim().starts_with("total")).unwrap_or_default();
        assert!(!rx_line.contains(" 0pkt"), "no video packets received:\n{dump}");
    }
    drop(engine);
}
