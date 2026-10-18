//! Data for RusTone's debug window: every SIP message, pjsua's log, per-call
//! stream statistics and audio samples. Kept in memory only, bounded.
//!
//! SIP messages and log lines are always collected (they're cheap and let a
//! window opened later show what already happened). Stream statistics and
//! audio taps run only while debugging is on (`Command::SetDebug`).

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::call::CallId;

const MAX_MESSAGES: usize = 2000;
const MAX_LOGS: usize = 5000;
const MAX_SAMPLES: usize = 600;
const MAX_STREAMS: usize = 16;
/// Seconds of audio kept per tap.
const AUDIO_SECONDS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgDirection {
    Sent,
    Received,
}

/// A SIP message as it went over the wire.
#[derive(Debug, Clone)]
pub struct SipMessage {
    /// 1-based, in capture order.
    pub seq: u64,
    pub time: SystemTime,
    pub direction: MsgDirection,
    /// "UDP", "TCP", "TLS"...
    pub transport: String,
    /// Remote "address:port".
    pub remote: String,
    pub raw: String,
}

impl SipMessage {
    pub fn start_line(&self) -> &str {
        self.raw.lines().next().unwrap_or_default().trim_end()
    }

    pub fn is_request(&self) -> bool {
        !self.start_line().starts_with("SIP/")
    }

    /// The request method, or the CSeq method for responses.
    pub fn method(&self) -> String {
        if self.is_request() {
            self.start_line().split_whitespace().next().unwrap_or_default().to_owned()
        } else {
            self.header("CSeq").and_then(|c| c.split_whitespace().nth(1).map(str::to_owned)).unwrap_or_default()
        }
    }

    /// Status code of a response.
    pub fn status(&self) -> Option<u16> {
        let line = self.start_line();
        if !line.starts_with("SIP/") {
            return None;
        }
        line.split_whitespace().nth(1)?.parse().ok()
    }

    /// Reason phrase of a response ("OK", "Ringing").
    pub fn reason(&self) -> String {
        self.start_line().splitn(3, ' ').nth(2).unwrap_or_default().to_owned()
    }

    pub fn request_uri(&self) -> Option<&str> {
        self.is_request().then(|| self.start_line().split_whitespace().nth(1)).flatten()
    }

    /// Header lines up to the blank line (folded lines joined).
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for line in self.raw.lines().skip(1) {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                break;
            }
            if line.starts_with([' ', '\t']) {
                if let Some(last) = out.last_mut() {
                    last.1.push(' ');
                    last.1.push_str(line.trim());
                }
            } else if let Some((name, value)) = line.split_once(':') {
                out.push((name.trim().to_owned(), value.trim().to_owned()));
            }
        }
        out
    }

    /// First header named `name` (case-insensitive, compact forms too).
    pub fn header(&self, name: &str) -> Option<String> {
        let wanted = canonical(name);
        self.headers().into_iter().find(|(n, _)| canonical(n) == wanted).map(|(_, v)| v)
    }

    pub fn body(&self) -> &str {
        self.raw.split_once("\r\n\r\n").or_else(|| self.raw.split_once("\n\n")).map_or("", |(_, b)| b)
    }

    pub fn call_id(&self) -> Option<String> {
        self.header("Call-ID")
    }

    /// A short label: "REGISTER", "INVITE 05550100002", "200 OK · INVITE".
    pub fn label(&self) -> String {
        if let Some(status) = self.status() {
            return format!("{status} {} · {}", self.reason(), self.method());
        }
        let method = self.method();
        let user = self.request_uri().and_then(uri_user);
        match (method.as_str(), user) {
            ("REGISTER", _) | (_, None) => method,
            (_, Some(user)) => format!("{method} {user}"),
        }
    }
}

/// Lowercase long form of a header name (compact forms expanded).
fn canonical(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "i" => "call-id",
        "f" => "from",
        "t" => "to",
        "m" => "contact",
        "v" => "via",
        "l" => "content-length",
        "c" => "content-type",
        "s" => "subject",
        "k" => "supported",
        other => other,
    }
    .to_owned()
}

/// The user part of a SIP URI, if it has one.
fn uri_user(uri: &str) -> Option<String> {
    let rest = uri.split_once(':').map_or(uri, |(_, r)| r);
    let (user, _) = rest.split_once('@')?;
    Some(user.split(';').next().unwrap_or(user).to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    /// pjsua's own log.
    Pjsua,
    /// RusTone's log.
    App,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: SystemTime,
    /// 1 error, 2 warning, 3 info, 4 debug, 5 trace.
    pub level: u8,
    pub source: LogSource,
    pub text: String,
}

/// One audio stream's state at a poll.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    pub codec: String,
    pub clock_rate: u32,
    pub channels: u32,
    pub remote: String,
    pub duration_ms: u32,
    pub tx_packets: u32,
    pub tx_bytes: u32,
    pub rx_packets: u32,
    pub rx_bytes: u32,
    pub rx_lost: u32,
    pub rx_discarded: u32,
    pub rx_duplicates: u32,
    pub rx_reordered: u32,
    pub rx_jitter_ms: f32,
    pub rx_jitter_mean_ms: f32,
    pub rx_jitter_max_ms: f32,
    /// Loss and jitter of what we send, as the remote reports in RTCP.
    pub tx_lost: u32,
    pub tx_jitter_ms: f32,
    pub rtt_ms: f32,
    pub rtt_mean_ms: f32,
    /// Size of one jitter-buffer frame, bytes.
    pub jb_frame_bytes: u32,
    pub jb_frames: u32,
    pub jb_prefetch: u32,
    pub jb_delay_ms: u32,
    pub jb_lost: u32,
    pub jb_discarded: u32,
    pub jb_empty: u32,
}

/// Graph points, one per poll.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamSample {
    /// Seconds since the stream started.
    pub t: f64,
    pub rx_kbps: f64,
    pub tx_kbps: f64,
    /// Packets lost in this interval, as a percentage of expected.
    pub rx_loss_pct: f64,
    pub jitter_ms: f64,
    pub rtt_ms: f64,
    pub jb_delay_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct StreamHistory {
    pub call: CallId,
    pub latest: StreamStats,
    pub samples: VecDeque<StreamSample>,
    pub ended: bool,
}

impl StreamHistory {
    fn add(&mut self, stats: StreamStats) {
        let prev = &self.latest;
        let dt = f64::from(stats.duration_ms.saturating_sub(prev.duration_ms)) / 1000.0;
        if dt > 0.0 && prev.duration_ms > 0 {
            let kbps = |now: u32, before: u32| f64::from(now.saturating_sub(before)) * 8.0 / 1000.0 / dt;
            let received = stats.rx_packets.saturating_sub(prev.rx_packets);
            let lost = stats.rx_lost.saturating_sub(prev.rx_lost);
            let expected = received + lost;
            self.samples.push_back(StreamSample {
                t: f64::from(stats.duration_ms) / 1000.0,
                rx_kbps: kbps(stats.rx_bytes, prev.rx_bytes),
                tx_kbps: kbps(stats.tx_bytes, prev.tx_bytes),
                rx_loss_pct: if expected == 0 { 0.0 } else { f64::from(lost) * 100.0 / f64::from(expected) },
                jitter_ms: f64::from(stats.rx_jitter_ms),
                rtt_ms: f64::from(stats.rtt_ms),
                jb_delay_ms: f64::from(stats.jb_delay_ms),
            });
            while self.samples.len() > MAX_SAMPLES {
                self.samples.pop_front();
            }
        }
        self.latest = stats;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTap {
    /// What the microphone captures.
    Mic,
    /// What the calls send to the speaker.
    Speaker,
}

#[derive(Debug, Default)]
struct AudioRing {
    rate: u32,
    samples: VecDeque<f32>,
}

#[derive(Debug, Default)]
struct Inner {
    next_seq: u64,
    messages: VecDeque<Arc<SipMessage>>,
    logs: VecDeque<Arc<LogLine>>,
    streams: BTreeMap<CallId, StreamHistory>,
    audio: [AudioRing; 2],
}

/// Shared between the engine (writer) and the debug window (reader).
#[derive(Debug, Default)]
pub struct DebugFeed {
    enabled: AtomicBool,
    inner: Mutex<Inner>,
}

impl DebugFeed {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether stream statistics and audio taps are running.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub(crate) fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            let mut inner = self.lock();
            for ring in &mut inner.audio {
                ring.samples.clear();
            }
        }
    }

    pub(crate) fn push_message(&self, direction: MsgDirection, transport: String, remote: String, raw: String) {
        let mut inner = self.lock();
        inner.next_seq += 1;
        let seq = inner.next_seq;
        inner.messages.push_back(Arc::new(SipMessage { seq, time: SystemTime::now(), direction, transport, remote, raw }));
        while inner.messages.len() > MAX_MESSAGES {
            inner.messages.pop_front();
        }
    }

    /// Adds a log line (the engine adds pjsua's; the app adds its own).
    pub fn push_log(&self, source: LogSource, level: u8, text: String) {
        let mut inner = self.lock();
        inner.logs.push_back(Arc::new(LogLine { time: SystemTime::now(), level, source, text }));
        while inner.logs.len() > MAX_LOGS {
            inner.logs.pop_front();
        }
    }

    pub(crate) fn update_stream(&self, call: CallId, stats: StreamStats) {
        let mut inner = self.lock();
        // A reused call id starts a new history.
        if inner.streams.get(&call).is_some_and(|h| h.ended || stats.duration_ms < h.latest.duration_ms) {
            inner.streams.remove(&call);
        }
        let history = inner.streams.entry(call).or_insert_with(|| StreamHistory { call, ..StreamHistory::default() });
        history.add(stats);
        while inner.streams.len() > MAX_STREAMS {
            let oldest_ended = inner.streams.iter().find(|(_, h)| h.ended).map(|(id, _)| *id);
            match oldest_ended.or_else(|| inner.streams.keys().next().copied()) {
                Some(id) => inner.streams.remove(&id),
                None => break,
            };
        }
    }

    pub(crate) fn end_stream(&self, call: CallId) {
        if let Some(h) = self.lock().streams.get_mut(&call) {
            h.ended = true;
        }
    }

    pub(crate) fn push_audio(&self, tap: AudioTap, rate: u32, samples: &[i16]) {
        let mut inner = self.lock();
        let ring = &mut inner.audio[tap as usize];
        if ring.rate != rate {
            ring.rate = rate;
            ring.samples.clear();
        }
        ring.samples.extend(samples.iter().map(|&s| f32::from(s) / 32768.0));
        let cap = rate as usize * AUDIO_SECONDS;
        while ring.samples.len() > cap {
            ring.samples.pop_front();
        }
    }

    pub fn messages(&self) -> Vec<Arc<SipMessage>> {
        self.lock().messages.iter().cloned().collect()
    }

    pub fn logs(&self) -> Vec<Arc<LogLine>> {
        self.lock().logs.iter().cloned().collect()
    }

    pub fn streams(&self) -> Vec<StreamHistory> {
        self.lock().streams.values().cloned().collect()
    }

    /// The last `count` samples of a tap (fewer if not captured yet), and
    /// the sample rate.
    pub fn audio(&self, tap: AudioTap, count: usize) -> (u32, Vec<f32>) {
        let inner = self.lock();
        let ring = &inner.audio[tap as usize];
        let skip = ring.samples.len().saturating_sub(count);
        (ring.rate, ring.samples.iter().skip(skip).copied().collect())
    }

    pub fn clear_messages(&self) {
        self.lock().messages.clear();
    }

    pub fn clear_logs(&self) {
        self.lock().logs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(raw: &str) -> SipMessage {
        SipMessage {
            seq: 1,
            time: SystemTime::now(),
            direction: MsgDirection::Sent,
            transport: "TLS".into(),
            remote: "192.168.29.1:5068".into(),
            raw: raw.replace('\n', "\r\n"),
        }
    }

    #[test]
    fn requests_and_responses_get_labels() {
        let invite = msg("INVITE sip:05550100002@192.168.29.1:5068;transport=tls SIP/2.0\ni: abc@host\nCSeq: 1 INVITE\nContent-Type: application/sdp\nContent-Length: 4\n\nv=0\n");
        assert_eq!(invite.label(), "INVITE 05550100002");
        assert_eq!(invite.call_id().as_deref(), Some("abc@host"));
        assert_eq!(invite.body().trim(), "v=0");
        let register = msg("REGISTER sip:sip.example.com;transport=tls SIP/2.0\nCSeq: 2 REGISTER\n\n");
        assert_eq!(register.label(), "REGISTER");
        let ok = msg("SIP/2.0 200 OK\nCSeq: 1 INVITE\nVia: SIP/2.0/TLS a;\n branch=z9\n\n");
        assert_eq!(ok.status(), Some(200));
        assert_eq!(ok.label(), "200 OK · INVITE");
        assert_eq!(ok.header("v").as_deref(), Some("SIP/2.0/TLS a; branch=z9"), "folded and compact");
        let ack = msg("ACK sip:sip.example.com SIP/2.0\nCSeq: 1 ACK\n\n");
        assert_eq!(ack.label(), "ACK");
    }

    #[test]
    fn feed_is_bounded_and_numbered() {
        let feed = DebugFeed::default();
        for i in 0..(MAX_MESSAGES + 5) {
            feed.push_message(MsgDirection::Received, "UDP".into(), "x".into(), format!("OPTIONS sip:{i}@h SIP/2.0\r\n\r\n"));
        }
        let messages = feed.messages();
        assert_eq!(messages.len(), MAX_MESSAGES);
        assert_eq!(messages.last().unwrap().seq, (MAX_MESSAGES + 5) as u64);
        feed.push_audio(AudioTap::Mic, 8000, &[16384; 10]);
        let (rate, samples) = feed.audio(AudioTap::Mic, 4);
        assert_eq!((rate, samples.len()), (8000, 4));
        assert!((samples[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stream_samples_come_from_deltas() {
        let feed = DebugFeed::default();
        let s = |ms, rx_bytes, rx_packets, rx_lost| StreamStats { duration_ms: ms, rx_bytes, rx_packets, rx_lost, ..StreamStats::default() };
        feed.update_stream(1, s(1000, 0, 0, 0));
        feed.update_stream(1, s(2000, 8000, 50, 0));
        feed.update_stream(1, s(3000, 16000, 98, 2));
        let h = &feed.streams()[0];
        assert_eq!(h.samples.len(), 2);
        assert!((h.samples[0].rx_kbps - 64.0).abs() < 1e-9);
        assert!((h.samples[1].rx_loss_pct - 4.0).abs() < 1e-9);
    }
}
