//! A real call through pjsua with no sound device: the local "call by IP"
//! account calls itself on 127.0.0.1:5060, so both legs are in this
//! process. (A separate test binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{CallInfo, CallOptions, CallState, Command, Direction, DtmfMethod, Engine, EngineConfig, Event, LocalAccountConfig, MediaState, Sound};

fn wait_for(rx: &mpsc::Receiver<Event>, what: &str, mut want: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(event) if want(&event) => return event,
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

fn call_where(rx: &mpsc::Receiver<Event>, what: &str, want: impl Fn(&CallInfo) -> bool) -> CallInfo {
    match wait_for(rx, what, |e| matches!(e, Event::Call(c) if want(c))) {
        Event::Call(c) => c,
        _ => unreachable!(),
    }
}

/// Waits until every condition has matched some call event, in any order.
fn calls_all(rx: &mpsc::Receiver<Event>, what: &str, conditions: &[&dyn Fn(&CallInfo) -> bool]) {
    let mut seen = vec![false; conditions.len()];
    while seen.contains(&false) {
        let c = call_where(rx, what, |_| true);
        for (done, cond) in seen.iter_mut().zip(conditions) {
            *done |= cond(&c);
        }
    }
}

/// A 0.2 s 16 kHz mono silent WAV.
fn silent_wav() -> std::path::PathBuf {
    wav("silent", &[0; 3200])
}

/// A 0.2 s 16 kHz mono 1 kHz tone at half scale.
fn tone_wav() -> std::path::PathBuf {
    let samples: Vec<i16> = (0..3200).map(|i| (16000.0 * (std::f32::consts::TAU * 1000.0 * i as f32 / 16000.0).sin()) as i16).collect();
    wav("tone", &samples)
}

fn wav(name: &str, pcm: &[i16]) -> std::path::PathBuf {
    let samples = pcm.len() as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    for v in [1u16, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&16000u32.to_le_bytes());
    b.extend_from_slice(&32000u32.to_le_bytes());
    for v in [2u16, 16] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(samples * 2).to_le_bytes());
    for v in pcm {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let path = std::env::temp_dir().join(format!("sip-engine-test-{name}-{}.wav", std::process::id()));
    std::fs::write(&path, b).unwrap();
    path
}

#[test]
#[cfg_attr(target_os = "macos", ignore = "pjproject's macOS video devices need the main run loop, which the test harness does not run")]
fn call_to_self_over_the_local_account() {
    let (tx, rx) = mpsc::channel();
    let config = EngineConfig { local_account: true, null_audio: true, audio_codecs: vec!["PCMU/8000/1".into()], ..EngineConfig::default() };
    let engine = Engine::spawn(config, move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    wait_for(&rx, "Started", |e| *e == Event::Started);
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));
    // Exercises forceCodec on a real incoming INVITE.
    engine.send(Command::SetCallOptions(CallOptions { dtmf_method: DtmfMethod::Auto, local_dtmf: true, force_codec: true }));

    engine.send(Command::MakeCall { target: "127.0.0.1:5060".into(), video: false, local: true });
    let incoming = match wait_for(&rx, "IncomingCall", |e| matches!(e, Event::IncomingCall(_))) {
        Event::IncomingCall(c) => c,
        _ => unreachable!(),
    };
    assert_eq!(incoming.direction, Direction::Incoming);
    assert!(incoming.remote.contains("rustone-test"), "{}", incoming.remote);

    // Ring, then answer.
    engine.send(Command::Answer { call: incoming.id, code: 180, video: false });
    let ringing = call_where(&rx, "outgoing 180", |c| c.direction == Direction::Outgoing && c.status_code == 180);
    assert!(!ringing.early_media);
    engine.send(Command::Answer { call: incoming.id, code: 200, video: false });
    // Media may become active before or after the call is confirmed.
    let (mut confirmed, mut active) = (None, false);
    while confirmed.is_none() || !active {
        let c = call_where(&rx, "outgoing confirmed with active media", |c| c.direction == Direction::Outgoing);
        if c.state == CallState::Confirmed {
            confirmed = Some(c.clone());
        }
        active |= c.media == MediaState::Active;
    }
    let out = confirmed.unwrap();

    // Hold from the caller's side: local hold here, remote hold there.
    engine.send(Command::Hold(out.id));
    calls_all(&rx, "local and remote hold", &[
        &|c| c.id == out.id && c.media == MediaState::LocalHold,
        &|c| c.id == incoming.id && c.media == MediaState::RemoteHold,
    ]);
    engine.send(Command::Unhold(out.id));
    calls_all(&rx, "resumed", &[
        &|c| c.id == out.id && c.media == MediaState::Active,
        &|c| c.id == incoming.id && c.media == MediaState::Active,
    ]);

    // The debug feed saw the exchange and samples the stream while on.
    let feed = engine.debug_feed();
    let labels: Vec<String> = feed.messages().iter().map(|m| m.label()).collect();
    assert!(labels.iter().any(|l| l == "INVITE rustone-test" || l.starts_with("INVITE")), "{labels:?}");
    assert!(labels.iter().any(|l| l == "180 Ringing · INVITE"), "{labels:?}");
    assert!(labels.iter().any(|l| l == "200 OK · INVITE"), "{labels:?}");
    // Offers carry audio only (no RFC 4103 text stream).
    let invite = feed.messages().into_iter().find(|m| m.is_request() && m.method() == "INVITE").expect("INVITE captured");
    assert!(invite.body().contains("m=audio"), "{}", invite.raw);
    assert!(!invite.body().contains("m=text"), "{}", invite.raw);
    engine.send(Command::SetDebug(true));
    std::thread::sleep(Duration::from_millis(2500));
    let streams = feed.streams();
    assert!(!streams.is_empty(), "stream statistics while debugging");
    assert_eq!(streams[0].latest.codec, "PCMU");
    assert!(!feed.logs().is_empty());
    engine.send(Command::SetDebug(false));

    // RFC 2833 digits arrive on the other leg.
    engine.send(Command::SendDtmf { call: out.id, digits: "5".into(), method: DtmfMethod::Rfc2833 });
    wait_for(&rx, "DTMF 5", |e| matches!(e, Event::DtmfReceived { call, digit: '5' } if *call == incoming.id));

    // A short sound plays to its end and is cleaned up.
    engine.send(Command::PlaySound(Sound { path: silent_wav(), looping: false, level: 0.4, ring: false }));
    engine.send(Command::SetMute { mic: true, speaker: false });
    engine.send(Command::SetMute { mic: false, speaker: false });
    std::thread::sleep(Duration::from_millis(500));

    // The VU meter sees what goes to the speaker (the null device here).
    engine.send(Command::MonitorCalls(true));
    engine.send(Command::PlaySound(Sound { path: tone_wav(), looping: true, level: 1.0, ring: false }));
    let meters = engine.meters();
    let deadline = Instant::now() + Duration::from_secs(3);
    while meters.speaker() == 0.0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(meters.speaker() > 0.0, "the speaker meter moves while a tone plays");
    // Quality comes once a second for the answered calls.
    match wait_for(&rx, "CallQuality", |e| matches!(e, Event::CallQuality(_))) {
        Event::CallQuality(q) => {
            assert!((1.0..=4.5).contains(&q.mos), "{q:?}");
            assert!(q.codec.starts_with("PCMU@8kHz"), "{q:?}");
        }
        _ => unreachable!(),
    }
    engine.send(Command::StopSound);
    engine.send(Command::MonitorCalls(false));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(meters.speaker(), 0.0, "meters off reads zero");

    engine.send(Command::Hangup(out.id));
    calls_all(&rx, "both legs disconnected", &[
        &|c| c.id == out.id && c.state == CallState::Disconnected,
        &|c| c.id == incoming.id && c.state == CallState::Disconnected,
    ]);

    // Calling without an account fails cleanly.
    engine.send(Command::MakeCall { target: "1234".into(), video: false, local: false });
    wait_for(&rx, "CallFailed", |e| matches!(e, Event::CallFailed { .. }));
    drop(engine);
}
