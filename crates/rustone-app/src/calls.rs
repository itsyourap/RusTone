//! Call policy: the UI-thread call logic, as a state machine that
//! turns engine events and user actions into engine commands. Pure, so the
//! rules are unit-tested without pjsua.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustone_core::settings::{CallSettings, DtmfMethod};
use sip_engine::{CallId, CallInfo, CallState, Command, Direction, Event, MediaState, Sound, uri_user_host};

/// How long an ended call stays on screen with its reason.
const ENDED_LINGER: Duration = Duration::from_secs(3);
/// Level for in-call sounds (ringback, beeps, hangup).
const IN_CALL_LEVEL: f32 = 0.4;

/// The sound files the policy plays.
#[derive(Debug, Clone, Default)]
pub struct SoundPaths {
    pub ringtone: PathBuf,
    pub ringback: PathBuf,
    pub call_waiting: PathBuf,
    pub hangup: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Call {
    pub info: CallInfo,
    /// Local time it was answered, for the call timer.
    pub answered: Option<Instant>,
    pub ended: Option<Instant>,
    pub end_reason: Option<String>,
    /// Ringing silenced with "Ignore".
    pub silenced: bool,
    /// Rejected automatically; never shown.
    pub hidden: bool,
    /// Latest transfer progress.
    pub transfer: Option<String>,
    /// Hold requested and not yet reflected in the media state.
    hold_requested: bool,
}

impl Call {
    fn new(info: CallInfo) -> Self {
        Self { info, answered: None, ended: None, end_reason: None, silenced: false, hidden: false, transfer: None, hold_requested: false }
    }

    pub fn id(&self) -> CallId {
        self.info.id
    }

    pub fn is_live(&self) -> bool {
        !self.hidden && self.ended.is_none()
    }

    pub fn is_ringing_in(&self) -> bool {
        self.is_live() && self.info.state.is_ringing_in(self.info.direction)
    }

    /// Who's on the other end: the caller ID when the network gave one.
    pub fn party(&self) -> Party {
        Party::parse(self.info.caller_id.as_deref().unwrap_or(&self.info.remote))
    }

    pub fn duration(&self) -> Option<Duration> {
        let answered = self.answered?;
        Some(self.ended.unwrap_or_else(Instant::now).saturating_duration_since(answered))
    }

    /// One line describing where the call is.
    pub fn status_text(&self) -> String {
        if let Some(reason) = &self.end_reason {
            return reason.clone();
        }
        match (self.info.state, self.info.media) {
            (CallState::Calling, _) => "Calling…".into(),
            (CallState::Incoming, _) => "Incoming call".into(),
            (CallState::Early, _) if self.info.direction == Direction::Incoming => "Incoming call".into(),
            (CallState::Early, _) if self.info.status_code == 180 => "Ringing…".into(),
            (CallState::Early, _) => non_empty(&self.info.status_text, "Connecting…"),
            (CallState::Connecting, _) => "Connecting…".into(),
            (CallState::Confirmed, MediaState::LocalHold | MediaState::None) => "On hold".into(),
            (CallState::Confirmed, MediaState::RemoteHold) => "Held by the other side".into(),
            (CallState::Confirmed, _) => self.duration().map_or_else(|| "Connected".into(), format_duration),
            (CallState::Disconnected, _) => "Call ended".into(),
        }
    }
}

fn non_empty(s: &str, fallback: &str) -> String {
    if s.trim().is_empty() { fallback.to_owned() } else { s.to_owned() }
}

pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// A remote party: display name (if any) and user/number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub name: Option<String>,
    /// The user part (number), or the host for user-less URIs.
    pub user: String,
}

impl Party {
    pub fn parse(remote: &str) -> Self {
        let name = remote
            .split_once('<')
            .map(|(before, _)| before.trim().trim_matches('"').trim().to_owned())
            .filter(|n| !n.is_empty());
        let user_host = uri_user_host(remote);
        let user = match user_host.split_once('@') {
            Some((user, _)) if !user.is_empty() => user.to_owned(),
            Some((_, host)) => host.to_owned(),
            None => user_host,
        };
        Self { name, user }
    }

    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.user)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Playing {
    Ringtone,
    Ringback,
    OneShot,
}

/// What the app should do after an event or action.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub commands: Vec<Command>,
    /// An incoming call wants the window brought forward.
    pub show_window: bool,
}

#[derive(Debug, Default)]
pub struct CallBook {
    calls: Vec<Call>,
    pub current: Option<CallId>,
    ringback: HashSet<CallId>,
    playing: Option<Playing>,
    /// SIP Call-IDs already answered or declined; forks get 487.
    handled_sip_ids: Vec<String>,
    pub mic_muted: bool,
    pub speaker_muted: bool,
    /// A message to show briefly (failed call, failed transfer...).
    pub notice: Option<(String, Instant)>,
    pub sounds: SoundPaths,
    /// Ringtone level, 0..=1.
    pub ring_level: f32,
}

impl CallBook {
    pub fn new(sounds: SoundPaths) -> Self {
        Self { sounds, ring_level: 1.0, ..Self::default() }
    }

    /// Calls to show, oldest first.
    pub fn visible(&self) -> impl Iterator<Item = &Call> {
        self.calls.iter().filter(|c| !c.hidden)
    }

    pub fn has_calls(&self) -> bool {
        self.visible().next().is_some()
    }

    pub fn live_count(&self) -> usize {
        self.calls.iter().filter(|c| c.is_live()).count()
    }

    pub fn get(&self, id: CallId) -> Option<&Call> {
        self.calls.iter().find(|c| c.id() == id && !c.hidden)
    }

    pub fn current_call(&self) -> Option<&Call> {
        self.current.and_then(|id| self.get(id))
    }

    fn get_mut(&mut self, id: CallId) -> Option<&mut Call> {
        self.calls.iter_mut().find(|c| c.id() == id && c.ended.is_none())
    }

    // -----------------------------------------------------------------------
    // Engine events

    pub fn on_event(&mut self, event: &Event, cfg: &CallSettings) -> Outcome {
        let mut out = Outcome::default();
        match event {
            Event::IncomingCall(info) => self.incoming(info, cfg, &mut out),
            Event::Call(info) => self.update(info, cfg, &mut out),
            Event::CallFailed { target, reason } => self.notify(format!("Couldn't call {target}: {reason}")),
            Event::CallError { reason, .. } => self.notify(reason.clone()),
            Event::TransferStatus { call, code, text, done } => {
                if let Some(c) = self.get_mut(*call) {
                    c.transfer = Some(if *done && (200..300).contains(code) { "Transferred".into() } else { format!("Transfer: {code} {text}") });
                }
            }
            Event::Stopped => {
                // pjsua went away (restart, suspend): its calls are gone too.
                for c in &mut self.calls {
                    if c.ended.is_none() {
                        c.ended = Some(Instant::now());
                        c.end_reason = Some("Call ended".into());
                    }
                }
                self.ringback.clear();
                self.playing = None;
            }
            _ => {}
        }
        out
    }

    /// An incoming call.
    fn incoming(&mut self, info: &CallInfo, cfg: &CallSettings, out: &mut Outcome) {
        self.calls.retain(|c| c.id() != info.id || c.ended.is_none());
        let mut call = Call::new(info.clone());
        let key = uri_user_host(&info.remote);
        let reject = if self.handled_sip_ids.contains(&info.sip_call_id) {
            Some(487)
        } else if self.calls.iter().any(|c| c.is_live() && uri_user_host(&c.info.remote) == key) {
            Some(486) // "Call already exists"
        } else {
            let count = self.live_count() + 1;
            let over = (!cfg.call_waiting && count > 1) || (cfg.max_concurrent_calls > 0 && count > cfg.max_concurrent_calls as usize);
            over.then_some(486) // "Active calls limit"
        };
        if let Some(code) = reject {
            call.hidden = true;
            out.commands.push(Command::Answer { call: info.id, code, video: false });
            self.calls.push(call);
            return;
        }
        let others = self.live_count() > 0;
        self.calls.push(call);
        out.commands.push(Command::Answer { call: info.id, code: 180, video: false });
        if others {
            // A waiting call beeps only with local sounds on.
            if cfg.local_dtmf {
                out.commands.push(self.play(Playing::OneShot, &self.sounds.call_waiting.clone(), false, IN_CALL_LEVEL));
            }
        } else {
            out.commands.push(self.play(Playing::Ringtone, &self.sounds.ringtone.clone(), true, self.ring_level));
        }
        self.current = Some(info.id);
        out.show_window = cfg.bring_to_front;
    }

    fn update(&mut self, info: &CallInfo, cfg: &CallSettings, out: &mut Outcome) {
        let id = info.id;
        if self.get_mut(id).is_none() {
            if info.state == CallState::Disconnected {
                // A rejected (hidden) call ending: forget it.
                self.calls.retain(|c| !(c.id() == id && c.hidden));
                return;
            }
            self.calls.push(Call::new(info.clone()));
            // A new outgoing call takes the screen, unless an incoming call
            // is ringing there (it needs an answer).
            let ringing_shown = self.current.and_then(|c| self.get(c)).is_some_and(Call::is_ringing_in);
            if info.direction == Direction::Outgoing && !ringing_shown {
                self.current = Some(id);
            }
        }
        let Some(call) = self.get_mut(id) else { return };
        if call.hidden {
            return;
        }
        let was_answered = call.answered.is_some();
        if call.info.media != info.media {
            call.hold_requested = false;
        }
        call.info = info.clone();
        if info.state == CallState::Confirmed && call.answered.is_none() {
            call.answered = Some(Instant::now().checked_sub(info.connected_for).unwrap_or_else(Instant::now));
        }

        // Starting or answering a call holds the others (msip_call_unhold).
        if matches!(info.state, CallState::Calling | CallState::Connecting) {
            out.commands.extend(self.hold_others(id));
            if info.media == MediaState::LocalHold {
                out.commands.push(Command::Unhold(id));
            }
        }

        // Ringback while the far end rings without early media.
        let wants_ringback = info.direction == Direction::Outgoing && info.state == CallState::Early && info.status_code == 180 && !info.early_media;
        if wants_ringback {
            if self.ringback.insert(id) && self.ringback.len() == 1 {
                out.commands.push(self.play(Playing::Ringback, &self.sounds.ringback.clone(), true, IN_CALL_LEVEL));
            }
        } else if self.ringback.remove(&id) && self.ringback.is_empty() && self.playing == Some(Playing::Ringback) {
            out.commands.push(self.stop());
        }

        // Ringtone stops once no incoming call rings audibly.
        if info.direction == Direction::Incoming && !info.state.is_ringing_in(info.direction) {
            let still_ringing = self.calls.iter().any(|c| c.is_ringing_in() && !c.silenced && c.id() != id);
            if !still_ringing && matches!(self.playing, Some(Playing::Ringtone)) {
                out.commands.push(self.stop());
            }
        }

        if info.state == CallState::Disconnected {
            let reason = end_reason(info, was_answered);
            if let Some(call) = self.get_mut(id) {
                call.ended = Some(Instant::now());
                call.end_reason = Some(reason);
            }
            if was_answered && cfg.local_dtmf {
                out.commands.push(self.play(Playing::OneShot, &self.sounds.hangup.clone(), false, IN_CALL_LEVEL));
            }
            if self.current == Some(id) {
                self.current = self.pick_current().or(Some(id));
            }
        }
    }

    /// Hold commands for other confirmed, unheld calls.
    fn hold_others(&mut self, except: CallId) -> Vec<Command> {
        let mut commands = Vec::new();
        for c in self.calls.iter_mut().filter(|c| c.is_live() && c.id() != except) {
            if c.info.state == CallState::Confirmed && matches!(c.info.media, MediaState::Active | MediaState::RemoteHold) && !c.hold_requested {
                c.hold_requested = true;
                commands.push(Command::Hold(c.id()));
            }
        }
        commands
    }

    /// The live call to show next: a ringing one, else an active one, else any.
    fn pick_current(&self) -> Option<CallId> {
        let live = || self.calls.iter().filter(|c| c.is_live());
        live()
            .find(|c| c.is_ringing_in())
            .or_else(|| live().find(|c| c.info.media == MediaState::Active))
            .or_else(|| live().next())
            .map(Call::id)
    }

    fn play(&mut self, kind: Playing, path: &std::path::Path, looping: bool, level: f32) -> Command {
        self.playing = Some(kind);
        // Only the ringtone goes to the ring device; the rest are call sounds.
        let ring = kind == Playing::Ringtone;
        Command::PlaySound(Sound { path: path.to_path_buf(), looping, level, ring })
    }

    fn stop(&mut self) -> Command {
        self.playing = None;
        Command::StopSound
    }

    fn notify(&mut self, text: String) {
        log::warn!("{text}");
        self.notice = Some((text, Instant::now()));
    }

    /// Drops calls that ended a while ago. Returns true if any were removed.
    pub fn tick(&mut self) -> bool {
        let before = self.calls.len();
        self.calls.retain(|c| c.ended.is_none_or(|t| t.elapsed() < ENDED_LINGER));
        if self.current.is_some_and(|id| self.get(id).is_none()) {
            self.current = self.pick_current();
        }
        if self.notice.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(6)) {
            self.notice = None;
        }
        self.calls.len() != before
    }

    // -----------------------------------------------------------------------
    // User actions

    pub fn dial(&self, target: &str, local: bool) -> Vec<Command> {
        vec![Command::MakeCall { target: target.to_owned(), video: false, local }]
    }

    pub fn answer(&mut self, id: CallId) -> Vec<Command> {
        let Some(call) = self.get(id).filter(|c| c.is_ringing_in()) else { return Vec::new() };
        self.handled_sip_ids.push(call.info.sip_call_id.clone());
        self.current = Some(id);
        let mut commands = vec![Command::Answer { call: id, code: 200, video: false }];
        if matches!(self.playing, Some(Playing::Ringtone | Playing::OneShot)) {
            commands.push(self.stop());
        }
        commands
    }

    /// Declines with 486.
    pub fn decline(&mut self, id: CallId) -> Vec<Command> {
        let Some(call) = self.get(id).filter(|c| c.is_ringing_in()) else { return Vec::new() };
        self.handled_sip_ids.push(call.info.sip_call_id.clone());
        let mut commands = vec![Command::Answer { call: id, code: 486, video: false }];
        let others_ringing = self.calls.iter().any(|c| c.is_ringing_in() && !c.silenced && c.id() != id);
        if !others_ringing && self.playing.is_some() {
            commands.push(self.stop());
        }
        commands
    }

    /// Stops ringing without answering (the caller keeps hearing ringback),
    /// or declines when "no ignore" is set.
    pub fn ignore(&mut self, id: CallId, cfg: &CallSettings) -> Vec<Command> {
        if cfg.no_ignore {
            return self.decline(id);
        }
        if let Some(call) = self.calls.iter_mut().find(|c| c.id() == id && c.is_ringing_in()) {
            call.silenced = true;
        }
        let audible = self.calls.iter().any(|c| c.is_ringing_in() && !c.silenced);
        if !audible && self.playing == Some(Playing::Ringtone) { vec![self.stop()] } else { Vec::new() }
    }

    pub fn hangup(&self, id: CallId) -> Vec<Command> {
        vec![Command::Hangup(id)]
    }

    pub fn toggle_hold(&mut self, id: CallId) -> Vec<Command> {
        let Some(call) = self.get_mut(id) else { return Vec::new() };
        match call.info.media {
            MediaState::LocalHold | MediaState::None if call.info.state == CallState::Confirmed => vec![Command::Unhold(id)],
            _ if call.info.state == CallState::Confirmed => {
                call.hold_requested = true;
                vec![Command::Hold(id)]
            }
            _ => Vec::new(),
        }
    }

    /// Makes `id` the shown call; resuming it holds the others.
    pub fn switch_to(&mut self, id: CallId) -> Vec<Command> {
        if self.get(id).is_none() {
            return Vec::new();
        }
        self.current = Some(id);
        let held = self.get(id).is_some_and(|c| c.info.state == CallState::Confirmed && c.info.media == MediaState::LocalHold);
        if held {
            let mut commands = self.hold_others(id);
            commands.push(Command::Unhold(id));
            commands
        } else {
            Vec::new()
        }
    }

    pub fn toggle_mic(&mut self) -> Vec<Command> {
        self.mic_muted = !self.mic_muted;
        vec![Command::SetMute { mic: self.mic_muted, speaker: self.speaker_muted }]
    }

    pub fn toggle_speaker(&mut self) -> Vec<Command> {
        self.speaker_muted = !self.speaker_muted;
        vec![Command::SetMute { mic: self.mic_muted, speaker: self.speaker_muted }]
    }

    pub fn dtmf(&self, id: CallId, digits: &str, method: DtmfMethod) -> Vec<Command> {
        let method = match method {
            DtmfMethod::Auto => sip_engine::DtmfMethod::Auto,
            DtmfMethod::InBand => sip_engine::DtmfMethod::InBand,
            DtmfMethod::Rfc2833 => sip_engine::DtmfMethod::Rfc2833,
            DtmfMethod::SipInfo => sip_engine::DtmfMethod::SipInfo,
        };
        vec![Command::SendDtmf { call: id, digits: digits.to_owned(), method }]
    }

    pub fn transfer(&self, id: CallId, target: &str) -> Vec<Command> {
        vec![Command::Transfer { call: id, target: target.to_owned() }]
    }

    pub fn attended_transfer(&self, id: CallId, with: CallId) -> Vec<Command> {
        vec![Command::AttendedTransfer { call: id, with }]
    }
}

/// The wording for how a call ended.
fn end_reason(info: &CallInfo, answered: bool) -> String {
    let code = info.status_code;
    match code {
        _ if answered => "Call ended".into(),
        0 | 200 => "Call ended".into(),
        404 => "Number not found".into(),
        408 | 480 => "No answer".into(),
        486 | 603 if info.direction == Direction::Incoming => "Declined".into(),
        486 | 600 => "Busy".into(),
        487 if info.direction == Direction::Incoming => "Missed call".into(),
        487 => "Cancelled".into(),
        603 => "Declined".into(),
        500..=599 => format!("Server failure: {code} {}", info.status_text),
        _ => non_empty(&info.status_text, "Call ended"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: CallId, direction: Direction, state: CallState, remote: &str) -> CallInfo {
        CallInfo {
            id,
            direction,
            state,
            status_code: 0,
            status_text: String::new(),
            remote: remote.into(),
            local: "<sip:me@pbx>".into(),
            media: MediaState::None,
            early_media: false,
            connected_for: Duration::ZERO,
            remote_video: false,
            sip_call_id: format!("sip-{id}"),
            caller_id: None,
            user_agent: None,
            diversion: None,
        }
    }

    fn book() -> CallBook {
        CallBook::new(SoundPaths {
            ringtone: "ring.wav".into(),
            ringback: "back.wav".into(),
            call_waiting: "wait.wav".into(),
            hangup: "hang.wav".into(),
        })
    }

    fn played(out: &Outcome) -> Vec<String> {
        out.commands
            .iter()
            .filter_map(|c| match c {
                Command::PlaySound(s) => Some(s.path.display().to_string()),
                Command::StopSound => Some("stop".into()),
                _ => None,
            })
            .collect()
    }

    fn answers(out: &Outcome) -> Vec<u16> {
        out.commands.iter().filter_map(|c| if let Command::Answer { code, .. } = c { Some(*code) } else { None }).collect()
    }

    #[test]
    fn incoming_rings_and_shows_the_window() {
        let mut b = book();
        let cfg = CallSettings::default();
        let out = b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        assert_eq!(answers(&out), [180]);
        assert_eq!(played(&out), ["ring.wav"]);
        assert!(out.show_window);
        assert_eq!(b.current, Some(1));
    }

    #[test]
    fn second_call_beeps_or_is_refused() {
        let cfg = CallSettings::default();
        let mut b = book();
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        let out = b.on_event(&Event::IncomingCall(info(2, Direction::Incoming, CallState::Incoming, "<sip:200@pbx>")), &cfg);
        assert_eq!(answers(&out), [180]);
        assert_eq!(played(&out), ["wait.wav"]);

        let no_waiting = CallSettings { call_waiting: false, ..CallSettings::default() };
        let out = b.on_event(&Event::IncomingCall(info(3, Direction::Incoming, CallState::Incoming, "<sip:300@pbx>")), &no_waiting);
        assert_eq!(answers(&out), [486]);
        assert!(b.get(3).is_none(), "rejected calls are hidden");

        let limited = CallSettings { max_concurrent_calls: 2, ..CallSettings::default() };
        let out = b.on_event(&Event::IncomingCall(info(4, Direction::Incoming, CallState::Incoming, "<sip:400@pbx>")), &limited);
        assert_eq!(answers(&out), [486]);
    }

    #[test]
    fn duplicates_and_forks_are_refused() {
        let cfg = CallSettings::default();
        let mut b = book();
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "\"A\" <sip:100@pbx>")), &cfg);
        let out = b.on_event(&Event::IncomingCall(info(2, Direction::Incoming, CallState::Incoming, "<sip:100@pbx;x=y>")), &cfg);
        assert_eq!(answers(&out), [486]);

        b.answer(1);
        let mut fork = info(5, Direction::Incoming, CallState::Incoming, "<sip:999@pbx>");
        fork.sip_call_id = "sip-1".into();
        let out = b.on_event(&Event::IncomingCall(fork), &cfg);
        assert_eq!(answers(&out), [487]);
    }

    #[test]
    fn ringback_plays_only_without_early_media() {
        let cfg = CallSettings::default();
        let mut b = book();
        let mut c = info(1, Direction::Outgoing, CallState::Calling, "<sip:100@pbx>");
        b.on_event(&Event::Call(c.clone()), &cfg);
        c.state = CallState::Early;
        c.status_code = 183;
        c.early_media = true;
        assert!(played(&b.on_event(&Event::Call(c.clone()), &cfg)).is_empty());
        c.status_code = 180;
        c.early_media = false;
        assert_eq!(played(&b.on_event(&Event::Call(c.clone()), &cfg)), ["back.wav"]);
        c.state = CallState::Confirmed;
        c.status_code = 200;
        c.media = MediaState::Active;
        assert_eq!(played(&b.on_event(&Event::Call(c.clone()), &cfg)), ["stop"]);
        assert_eq!(b.current_call().unwrap().status_text(), "00:00");
    }

    #[test]
    fn a_new_call_holds_the_active_one_and_hangup_beeps() {
        let cfg = CallSettings::default();
        let mut b = book();
        let mut first = info(1, Direction::Outgoing, CallState::Confirmed, "<sip:100@pbx>");
        first.media = MediaState::Active;
        b.on_event(&Event::Call(first.clone()), &cfg);
        let out = b.on_event(&Event::Call(info(2, Direction::Outgoing, CallState::Calling, "<sip:200@pbx>")), &cfg);
        assert!(out.commands.contains(&Command::Hold(1)));
        assert_eq!(b.current, Some(2));
        // Not repeated while the hold is pending.
        let out = b.on_event(&Event::Call(info(2, Direction::Outgoing, CallState::Calling, "<sip:200@pbx>")), &cfg);
        assert!(!out.commands.contains(&Command::Hold(1)));

        first.state = CallState::Disconnected;
        first.status_code = 200;
        let out = b.on_event(&Event::Call(first), &cfg);
        assert_eq!(played(&out), ["hang.wav"]);
        assert_eq!(b.get(1).unwrap().status_text(), "Call ended");
    }

    #[test]
    fn answer_decline_and_ignore() {
        let cfg = CallSettings::default();
        let mut b = book();
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        assert_eq!(b.ignore(1, &cfg), [Command::StopSound]);
        assert!(b.get(1).unwrap().silenced);
        assert_eq!(b.decline(1).first(), Some(&Command::Answer { call: 1, code: 486, video: false }));

        b.on_event(&Event::IncomingCall(info(2, Direction::Incoming, CallState::Incoming, "<sip:200@pbx>")), &cfg);
        let cmds = b.answer(2);
        assert_eq!(cmds, [Command::Answer { call: 2, code: 200, video: false }, Command::StopSound]);
        let strict = CallSettings { no_ignore: true, ..cfg };
        b.on_event(&Event::IncomingCall(info(3, Direction::Incoming, CallState::Incoming, "<sip:300@pbx>")), &strict);
        assert_eq!(b.ignore(3, &strict).first(), Some(&Command::Answer { call: 3, code: 486, video: false }));
    }

    #[test]
    fn a_ringing_call_keeps_the_screen() {
        let cfg = CallSettings::default();
        let mut b = book();
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        b.on_event(&Event::Call(info(0, Direction::Outgoing, CallState::Calling, "<sip:200@pbx>")), &cfg);
        assert_eq!(b.current, Some(1));
    }

    #[test]
    fn parties_and_reasons_read_well() {
        assert_eq!(Party::parse("\"Alice B\" <sip:1001@pbx;transport=tls>"), Party { name: Some("Alice B".into()), user: "1001".into() });
        assert_eq!(Party::parse("<sip:05550100002@192.168.29.1:5068>").title(), "05550100002");
        assert_eq!(Party::parse("sip:pbx.example.com").title(), "pbx.example.com");
        let mut i = info(1, Direction::Outgoing, CallState::Disconnected, "<sip:1@x>");
        i.status_code = 486;
        assert_eq!(end_reason(&i, false), "Busy");
        i.direction = Direction::Incoming;
        assert_eq!(end_reason(&i, false), "Declined");
        i.direction = Direction::Outgoing;
        i.status_code = 503;
        i.status_text = "Service Unavailable".into();
        assert_eq!(end_reason(&i, false), "Server failure: 503 Service Unavailable");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1:02:05");
    }
}
