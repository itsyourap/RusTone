//! Video calls through an operator that first answers
//! with video rejected (reliable 183 with `m=video 0`) and then offers video
//! in an UPDATE before the call is answered. RusTone must accept that video,
//! and "Turn on video" later must re-enable the existing video line rather
//! than add a second one (which the operator rejects with 604).
//! A fake operator on UDP 127.0.0.1:5070 plays the IBCF. Runs only when
//! asked: `cargo test -p sip-engine --test video_operator -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{Peer, sdp, sdp_dir};
use sip_engine::{CallState, Command, Direction, Engine, EngineConfig, Event, LocalAccountConfig, VideoConfig};

const OPERATOR: &str = "127.0.0.1:5070";

/// The port of the `m=video` line(s) in a list of media lines.
fn video_ports(media: &[&str]) -> Vec<u16> {
    media.iter().filter(|m| m.starts_with("m=video")).map(|m| m.split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0)).collect()
}

#[test]
#[ignore = "uses the video stack and UDP port 5070"]
fn video_offered_by_the_operator_after_a_rejecting_183() {
    let op = Peer::bind(OPERATOR, Duration::from_secs(30));
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
    let next = |what: &str| rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    while next("Started") != Event::Started {}
    engine.send(Command::SetLocalAccount(Some(LocalAccountConfig { username: "rustone-test".into(), ..LocalAccountConfig::default() })));
    engine.send(Command::MakeCall { target: OPERATOR.into(), video: true, local: true });

    let invite = op.recv("the INVITE", |m| m.is_request("INVITE"));
    assert_eq!(video_ports(&invite.media()).len(), 1, "a video call offers video");
    op.reply(&invite, "100 Trying", "", "");
    // Early answer with video rejected, reliably (100rel), as the IBCF does.
    op.reply(&invite, "183 Session Progress", "Require: 100rel\r\nRSeq: 1\r\n", &sdp(1, 0));
    let prack = op.recv("the PRACK", |m| m.is_request("PRACK"));
    op.reply(&prack, "200 OK", "", "");

    // The IBCF then offers video in an UPDATE.
    let from = invite.header("From");
    let to = format!("{};tag=op1", invite.header("To"));
    let contact = invite.header("Contact").trim_start_matches('<').split('>').next().unwrap_or_default().to_owned();
    let offer = sdp(2, 49024);
    op.send(
        invite.from,
        &format!(
            "UPDATE {contact} SIP/2.0\r\nVia: SIP/2.0/UDP {OPERATOR};branch=z9hG4bK-op-update1;rport\r\nMax-Forwards: 70\r\nFrom: {to}\r\nTo: {from}\r\nCall-ID: {}\r\nCSeq: 2 UPDATE\r\nContact: <sip:op@{OPERATOR}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{offer}",
            invite.header("Call-ID"),
            offer.len()
        ),
    );
    let answer = op.recv("the UPDATE's answer", |m| m.is_response(200, "UPDATE"));
    let accepted = video_ports(&answer.media());
    assert!(accepted.len() == 1 && accepted[0] != 0, "the operator's video offer is accepted: {:?}", answer.media());

    // Answered: video flows from the start.
    op.reply(&invite, "200 OK", "", "");
    op.recv("the ACK", |m| m.is_request("ACK"));
    let call = loop {
        if let Event::Call(c) = next("the call to connect")
            && c.direction == Direction::Outgoing
            && c.state == CallState::Confirmed
        {
            break c;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut video = call.video;
    while !video && Instant::now() < deadline {
        if let Ok(Event::Call(c)) = rx.recv_timeout(Duration::from_millis(200))
            && c.id == call.id
        {
            video = c.video;
        }
    }
    assert!(video, "the call has video");

    // The peer turns its camera off (re-INVITE with video recvonly): the
    // call keeps its video, we still send ours, and receive nothing.
    let reinvite = |cseq: u32, dir: &str| {
        let offer = sdp_dir(cseq, 49024, dir);
        op.send(
            invite.from,
            &format!(
                "INVITE {contact} SIP/2.0\r\nVia: SIP/2.0/UDP {OPERATOR};branch=z9hG4bK-op-reinvite{cseq};rport\r\nMax-Forwards: 70\r\nFrom: {to}\r\nTo: {from}\r\nCall-ID: {}\r\nCSeq: {cseq} INVITE\r\nContact: <sip:op@{OPERATOR}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{offer}",
                invite.header("Call-ID"),
                offer.len()
            ),
        );
        let ok = op.recv("the re-INVITE's answer", |m| m.is_response(200, "INVITE") && m.header("CSeq").starts_with(&format!("{cseq} ")));
        op.send(
            invite.from,
            &format!(
                "ACK {contact} SIP/2.0\r\nVia: SIP/2.0/UDP {OPERATOR};branch=z9hG4bK-op-ack{cseq};rport\r\nMax-Forwards: 70\r\nFrom: {to}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {cseq} ACK\r\nContent-Length: 0\r\n\r\n",
                ok.header("To"),
                invite.header("Call-ID")
            ),
        );
        ok
    };
    let wait_for = |what: &str, want: &dyn Fn(&sip_engine::CallInfo) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if let Ok(Event::Call(c)) = rx.recv_timeout(Duration::from_millis(200))
                && c.id == call.id
                && want(&c)
            {
                return c;
            }
        }
    };
    let ok = reinvite(3, "recvonly");
    assert!(ok.body.contains("a=sendonly"), "we keep sending:\n{}", ok.body);
    wait_for("the peer's camera off", &|c| c.video && !c.receiving_video && c.sending_video);
    // And on again.
    reinvite(4, "sendrecv");
    wait_for("the peer's camera on", &|c| c.video && c.receiving_video && c.sending_video);

    engine.send(Command::HangupAll);
    let bye = op.recv("the BYE", |m| m.is_request("BYE"));
    op.reply(&bye, "200 OK", "", "");

    // Second call: video stays rejected, then "Turn on video" re-enables
    // the existing video line instead of adding a second one.
    engine.send(Command::MakeCall { target: OPERATOR.into(), video: true, local: true });
    let invite = op.recv("the second INVITE", |m| m.is_request("INVITE") && m.header("Call-ID") != bye.header("Call-ID"));
    op.reply(&invite, "200 OK", "", &sdp(1, 0));
    op.recv("the ACK", |m| m.is_request("ACK"));
    let call = loop {
        if let Event::Call(c) = next("the second call to connect")
            && c.direction == Direction::Outgoing
            && c.state == CallState::Confirmed
        {
            break c;
        }
    };
    assert!(!call.video, "audio only");
    std::thread::sleep(Duration::from_millis(500));
    engine.send(Command::AddVideo(call.id));
    let reinvite = op.recv("the re-INVITE", |m| m.is_request("INVITE") && m.header("Call-ID") == invite.header("Call-ID"));
    let offered = video_ports(&reinvite.media());
    assert!(offered.len() == 1 && offered[0] != 0, "one video line, turned on: {:?}", reinvite.media());
    op.reply(&reinvite, "200 OK", "", &sdp(2, 49024));
    op.recv("the ACK", |m| m.is_request("ACK"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut video = false;
    while !video && Instant::now() < deadline {
        if let Ok(Event::Call(c)) = rx.recv_timeout(Duration::from_millis(200))
            && c.id == call.id
        {
            video = c.video;
        }
    }
    assert!(video, "video after turning it on");
    engine.send(Command::HangupAll);
    let bye = op.recv("the second BYE", |m| m.is_request("BYE"));
    op.reply(&bye, "200 OK", "", "");
    std::thread::sleep(Duration::from_millis(500));
    drop(engine);
}
