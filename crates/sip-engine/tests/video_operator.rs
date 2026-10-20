//! Video calls through an operator that first answers
//! with video rejected (reliable 183 with `m=video 0`) and then offers video
//! in an UPDATE before the call is answered. RusTone must accept that video,
//! and "Turn on video" later must re-enable the existing video line rather
//! than add a second one (which the operator rejects with 604).
//! A fake operator on UDP 127.0.0.1:5070 plays the IBCF. Runs only when
//! asked: `cargo test -p sip-engine --test video_operator -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{CallState, Command, Direction, Engine, EngineConfig, Event, LocalAccountConfig, VideoConfig};

const OPERATOR: &str = "127.0.0.1:5070";

/// A SIP message as received.
struct Msg {
    first: String,
    headers: Vec<(String, String)>,
    body: String,
    from: SocketAddr,
}

impl Msg {
    fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map_or("", |(_, v)| v.as_str())
    }
    fn is_request(&self, method: &str) -> bool {
        self.first.starts_with(&format!("{method} "))
    }
    fn is_response(&self, code: u16, method: &str) -> bool {
        self.first.starts_with(&format!("SIP/2.0 {code} ")) && self.header("CSeq").ends_with(method)
    }
    /// The SDP's media lines.
    fn media(&self) -> Vec<&str> {
        self.body.lines().filter(|l| l.starts_with("m=")).collect()
    }
}

struct Operator {
    socket: UdpSocket,
    deadline: Instant,
}

impl Operator {
    fn recv(&self, what: &str, want: impl Fn(&Msg) -> bool) -> Msg {
        let mut buf = vec![0u8; 65536];
        loop {
            let left = self.deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}");
            self.socket.set_read_timeout(Some(left)).unwrap();
            let Ok((n, from)) = self.socket.recv_from(&mut buf) else { continue };
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
            let mut lines = head.lines();
            let first = lines.next().unwrap_or_default().to_owned();
            let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned())).collect();
            let msg = Msg { first, headers, body: body.to_owned(), from };
            // Requests other than what we wait for still get a 200 (PRACK).
            if msg.is_request("PRACK") && !what.contains("PRACK") {
                self.reply(&msg, "200 OK", "", "");
                continue;
            }
            if want(&msg) {
                return msg;
            }
        }
    }

    fn send(&self, to: SocketAddr, text: &str) {
        self.socket.send_to(text.as_bytes(), to).unwrap();
    }

    /// A response to `req`; `to_tag` is added to To when it has none.
    fn reply(&self, req: &Msg, status: &str, extra: &str, sdp: &str) {
        let to = req.header("To");
        let to = if to.contains("tag=") { to.to_owned() } else { format!("{to};tag=op1") };
        let content = if sdp.is_empty() { String::new() } else { "Content-Type: application/sdp\r\n".to_owned() };
        let text = format!(
            "SIP/2.0 {status}\r\nVia: {}\r\nFrom: {}\r\nTo: {to}\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:op@{OPERATOR}>\r\n{extra}{content}Content-Length: {}\r\n\r\n{sdp}",
            req.header("Via"),
            req.header("From"),
            req.header("Call-ID"),
            req.header("CSeq"),
            sdp.len()
        );
        self.send(req.from, &text);
    }
}

fn sdp(version: u32, video_port: u16) -> String {
    let video = if video_port == 0 {
        "m=video 0 RTP/AVP 97\r\n".to_owned()
    } else {
        format!("m=video {video_port} RTP/AVP 97\r\na=rtpmap:97 H264/90000\r\na=fmtp:97 profile-level-id=42C00C; packetization-mode=1;\r\na=sendrecv\r\n")
    };
    format!(
        "v=0\r\no=IBCF 1929502446 {version} IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49032 RTP/AVP 0 120\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:120 telephone-event/8000\r\na=fmtp:120 0-15\r\na=sendrecv\r\n{video}"
    )
}

/// The port of the `m=video` line(s) in a list of media lines.
fn video_ports(media: &[&str]) -> Vec<u16> {
    media.iter().filter(|m| m.starts_with("m=video")).map(|m| m.split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0)).collect()
}

#[test]
#[ignore = "uses the video stack and UDP port 5070"]
fn video_offered_by_the_operator_after_a_rejecting_183() {
    let op = Operator { socket: UdpSocket::bind(OPERATOR).expect("UDP 5070 free"), deadline: Instant::now() + Duration::from_secs(30) };
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
