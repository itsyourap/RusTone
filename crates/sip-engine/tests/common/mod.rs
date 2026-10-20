//! A fake SIP peer on UDP for engine tests: it reads what the engine sends
//! and answers or sends requests by hand, so a test can play an operator
//! or a caller exactly.

#![allow(dead_code)]

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// A SIP message as received.
pub struct Msg {
    pub first: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub from: SocketAddr,
}

impl Msg {
    pub fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map_or("", |(_, v)| v.as_str())
    }
    pub fn is_request(&self, method: &str) -> bool {
        self.first.starts_with(&format!("{method} "))
    }
    pub fn is_response(&self, code: u16, method: &str) -> bool {
        self.first.starts_with(&format!("SIP/2.0 {code} ")) && self.header("CSeq").ends_with(method)
    }
    /// The response's status code, 0 for a request.
    pub fn code(&self) -> u16 {
        self.first.strip_prefix("SIP/2.0 ").and_then(|r| r.get(..3)).and_then(|c| c.parse().ok()).unwrap_or(0)
    }
    /// The SDP's media lines.
    pub fn media(&self) -> Vec<&str> {
        self.body.lines().filter(|l| l.starts_with("m=")).collect()
    }
}

pub struct Peer {
    pub socket: UdpSocket,
    /// Where it listens, e.g. "127.0.0.1:5070".
    pub addr: &'static str,
    pub deadline: Instant,
}

impl Peer {
    pub fn bind(addr: &'static str, timeout: Duration) -> Self {
        Self { socket: UdpSocket::bind(addr).unwrap_or_else(|e| panic!("UDP {addr}: {e}")), addr, deadline: Instant::now() + timeout }
    }

    /// The next message `want` accepts; PRACKs are answered on the way.
    pub fn recv(&self, what: &str, want: impl Fn(&Msg) -> bool) -> Msg {
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
            if msg.is_request("PRACK") && !what.contains("PRACK") {
                self.reply(&msg, "200 OK", "", "");
                continue;
            }
            if want(&msg) {
                return msg;
            }
        }
    }

    pub fn send(&self, to: SocketAddr, text: &str) {
        self.socket.send_to(text.as_bytes(), to).unwrap();
    }

    /// A response to `req`, adding our To tag when it has none.
    pub fn reply(&self, req: &Msg, status: &str, extra: &str, sdp: &str) {
        let to = req.header("To");
        let to = if to.contains("tag=") { to.to_owned() } else { format!("{to};tag=op1") };
        let content = if sdp.is_empty() { String::new() } else { "Content-Type: application/sdp\r\n".to_owned() };
        let text = format!(
            "SIP/2.0 {status}\r\nVia: {}\r\nFrom: {}\r\nTo: {to}\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:op@{}>\r\n{extra}{content}Content-Length: {}\r\n\r\n{sdp}",
            req.header("Via"),
            req.header("From"),
            req.header("Call-ID"),
            req.header("CSeq"),
            self.addr,
            sdp.len()
        );
        self.send(req.from, &text);
    }

    /// Calls `to` (the engine, e.g. "127.0.0.1:5060") with an audio offer
    /// and `extra` header lines. Returns the Call-ID.
    pub fn invite(&self, to: &str, call_id: &str, from_user: &str, extra: &str) -> String {
        let sdp = sdp(1, 0);
        let text = format!(
            "INVITE sip:rustone-test@{to} SIP/2.0\r\nVia: SIP/2.0/UDP {addr};branch=z9hG4bK-{call_id};rport\r\nMax-Forwards: 70\r\nFrom: <sip:{from_user}@{addr}>;tag=caller-{call_id}\r\nTo: <sip:rustone-test@{to}>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:{from_user}@{addr}>\r\n{extra}Content-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
            sdp.len(),
            addr = self.addr,
        );
        self.send(to.parse().unwrap(), &text);
        call_id.to_owned()
    }

    /// Acknowledges a final response to our INVITE (non-2xx in the same
    /// transaction, 2xx as a new request; both fine for pjsua over UDP).
    pub fn ack(&self, to: &str, response: &Msg) {
        // A non-2xx ACK belongs to the INVITE transaction; a 2xx ACK is new.
        let via = if response.code() >= 300 {
            response.header("Via").to_owned()
        } else {
            format!("SIP/2.0/UDP {};branch=z9hG4bK-ack-{};rport", self.addr, response.header("Call-ID"))
        };
        let text = format!(
            "ACK sip:rustone-test@{to} SIP/2.0\r\nVia: {via}\r\nMax-Forwards: 70\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: 1 ACK\r\nContent-Length: 0\r\n\r\n",
            response.header("From"),
            response.header("To"),
            response.header("Call-ID"),
        );
        self.send(to.parse().unwrap(), &text);
    }
}

pub fn sdp(version: u32, video_port: u16) -> String {
    sdp_dir(version, video_port, "sendrecv")
}

/// `dir` is the peer's video direction ("recvonly": its camera is off).
pub fn sdp_dir(version: u32, video_port: u16, dir: &str) -> String {
    let video = if video_port == 0 {
        "m=video 0 RTP/AVP 97\r\n".to_owned()
    } else {
        format!("m=video {video_port} RTP/AVP 97\r\na=rtpmap:97 H264/90000\r\na=fmtp:97 profile-level-id=42C00C; packetization-mode=1;\r\na={dir}\r\n")
    };
    format!(
        "v=0\r\no=IBCF 1929502446 {version} IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49032 RTP/AVP 0 120\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:120 telephone-event/8000\r\na=fmtp:120 0-15\r\na=sendrecv\r\n{video}"
    )
}
