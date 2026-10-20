//! Call policy: the UI-thread call logic, as a state machine that
//! turns engine events and user actions into engine commands. Pure, so the
//! rules are unit-tested without pjsua.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use rustone_core::hooks::{HookCall, HookKind};
use rustone_core::settings::{AutoAnswerWhen, CallSettings, DtmfMethod};
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
    pub message_in: PathBuf,
    pub message_out: PathBuf,
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
    /// Latest quality sample while media flows.
    pub quality: Option<sip_engine::CallQuality>,
    /// Wall-clock start, for the history.
    pub started: SystemTime,
    /// When the engine answers it by itself (auto-answer with a delay).
    pub auto_answer_at: Option<Instant>,
    /// Where it is forwarded, and when (now for an immediate forward).
    pub forward: Option<(String, Instant)>,
}

impl Call {
    fn new(info: CallInfo) -> Self {
        Self { info, answered: None, ended: None, end_reason: None, silenced: false, hidden: false, transfer: None, hold_requested: false, quality: None, started: SystemTime::now(), auto_answer_at: None, forward: None }
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

    /// What a command hook gets to know about the call.
    pub fn hook_call(&self) -> HookCall {
        let party = self.party();
        HookCall {
            number: party.user.clone(),
            name: party.name.clone().unwrap_or_default(),
            call_id: self.info.sip_call_id.clone(),
            incoming: self.info.direction == Direction::Incoming,
            duration_secs: self.duration().map_or(0, |d| d.as_secs()),
        }
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
    /// Calls that just ended, for the history (rejected duplicates excluded).
    pub ended: Vec<EndedCall>,
    /// Command hooks to run.
    pub hooks: Vec<(HookKind, HookCall)>,
}

/// A finished call, as the history needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct EndedCall {
    pub info: CallInfo,
    pub started: SystemTime,
    pub answered: bool,
    /// How it ended.
    pub reason: String,
}

impl EndedCall {
    pub fn kind(&self) -> rustone_core::history::CallKind {
        use rustone_core::history::CallKind;
        match (self.info.direction, self.answered) {
            (Direction::Outgoing, _) => CallKind::Outgoing,
            (Direction::Incoming, true) => CallKind::Incoming,
            (Direction::Incoming, false) => CallKind::Missed,
        }
    }

    /// Whether it counts as a missed call for the badge: an incoming call
    /// that wasn't answered and that the user didn't decline.
    pub fn is_missed(&self) -> bool {
        self.info.direction == Direction::Incoming && !self.answered && !matches!(self.info.status_code, 302 | 486)
    }
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
    /// The registering account's user and domain, for the deny rules.
    pub account: Option<rustone_core::rules::Identity>,
    /// A call to join with the next outgoing call once that's answered
    /// (a conference speed-dial button).
    pub merge_on_answer: Option<CallId>,
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
            Event::RecordingSaved { path, .. } => {
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                self.notice = Some((format!("Recording saved: {name}"), Instant::now()));
            }
            Event::CallQuality(q) => {
                if let Some(c) = self.get_mut(q.call) {
                    c.quality = Some(q.clone());
                }
            }
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
                        if !c.hidden {
                            out.ended.push(EndedCall { info: c.info.clone(), started: c.started, answered: c.answered.is_some(), reason: "Call ended".into() });
                        }
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
        // The incoming-call hook runs before the rules.
        out.hooks.push((HookKind::IncomingCall, call.hook_call()));
        let key = uri_user_host(&info.remote);
        let local = uri_user_host(&info.local);
        let addresses = rustone_core::rules::Addresses { local: &local, remote: &key };
        let reject = if self.handled_sip_ids.contains(&info.sip_call_id) {
            Some((487, ""))
        } else if self.calls.iter().any(|c| c.is_live() && uri_user_host(&c.info.remote) == key)
            || (!cfg.call_waiting && self.live_count() > 0)
            || (cfg.max_concurrent_calls > 0 && self.live_count() >= cfg.max_concurrent_calls as usize)
        {
            Some((486, "")) // "Call already exists" / "Active calls limit"
        } else if cfg.dnd {
            Some((486, "Do Not Disturb"))
        } else if rustone_core::rules::denied(cfg.deny_incoming, self.account.as_ref(), addresses) {
            Some((486, "Denied"))
        } else {
            None
        };
        if let Some((code, reason)) = reject {
            call.hidden = true;
            out.commands.push(if reason.is_empty() {
                Command::Answer { call: info.id, code, video: false }
            } else {
                Command::Respond { call: info.id, code, reason: reason.into(), headers: Vec::new() }
            });
            self.calls.push(call);
            return;
        }
        let others = self.live_count() > 0;
        let forward = Some(cfg.forwarding_number.trim()).filter(|n| cfg.forwarding && !n.is_empty()).map(|n| (n.to_owned(), Duration::from_secs(u64::from(cfg.forwarding_delay))));
        if let Some((target, delay)) = &forward
            && delay.is_zero()
        {
            // Without ringing; it shows only in the history.
            call.hidden = true;
            call.forward = Some((target.clone(), Instant::now()));
            self.handled_sip_ids.push(info.sip_call_id.clone());
            self.calls.push(call);
            out.commands.push(Command::Forward { call: info.id, target: target.clone() });
            return;
        }
        if let Some((target, delay)) = forward {
            call.forward = Some((target.clone(), Instant::now() + delay));
            out.commands.push(Command::Later { delay, command: Box::new(Command::Forward { call: info.id, target }) });
        }
        if let Some(delay) = self.auto_answer_delay(info, cfg) {
            if delay.is_zero() {
                // At once, with a beep instead of the ringtone.
                self.handled_sip_ids.push(info.sip_call_id.clone());
                self.calls.push(call);
                self.current = Some(info.id);
                out.commands.push(Command::Answer { call: info.id, code: 200, video: info.remote_video });
                if cfg.local_dtmf {
                    out.commands.push(self.play(Playing::OneShot, &self.sounds.call_waiting.clone(), false, IN_CALL_LEVEL));
                }
                out.show_window = cfg.bring_to_front;
                return;
            }
            // Rings meanwhile; answering or declining first cancels it.
            call.auto_answer_at = Some(Instant::now() + delay);
            out.commands.push(Command::Later { delay, command: Box::new(Command::Answer { call: info.id, code: 200, video: info.remote_video }) });
        }
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
        if let Some(call) = self.get(info.id) {
            out.hooks.push((HookKind::CallRing, call.hook_call()));
        }
        self.current = Some(info.id);
        out.show_window = cfg.bring_to_front;
    }

    /// Whether auto-answer takes the call, and after how long.
    fn auto_answer_delay(&self, info: &CallInfo, cfg: &CallSettings) -> Option<Duration> {
        let requested = rustone_core::rules::auto_answer_requested(info.auto_answer_header.as_deref(), info.call_info_header.as_deref());
        let on = cfg.auto_answer || (cfg.auto_answer_on_request && requested.is_some());
        let party = Party::parse(info.caller_id.as_deref().unwrap_or(&info.remote));
        if !on || !rustone_core::rules::number_allowed(&cfg.auto_answer_numbers, &party.user) {
            return None;
        }
        let others_allow = match cfg.auto_answer_when {
            AutoAnswerWhen::NoOtherCalls => self.live_count() == 0,
            AutoAnswerWhen::OthersOnHold => !self.calls.iter().any(|c| c.is_live() && c.info.media == MediaState::Active),
            AutoAnswerWhen::Always => true,
        };
        let seconds = requested.flatten().filter(|_| cfg.auto_answer_on_request).unwrap_or(cfg.auto_answer_delay);
        others_allow.then(|| Duration::from_secs(u64::from(seconds)))
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
            if info.direction == Direction::Outgoing {
                out.hooks.push((HookKind::OutgoingCall, Call::new(info.clone()).hook_call()));
            }
            // A new outgoing call takes the screen, unless an incoming call
            // is ringing there (it needs an answer).
            let ringing_shown = self.current.and_then(|c| self.get(c)).is_some_and(Call::is_ringing_in);
            if info.direction == Direction::Outgoing && !ringing_shown {
                self.current = Some(id);
            }
        }
        let Some(call) = self.get_mut(id) else { return };
        if call.hidden {
            if info.state == CallState::Disconnected {
                // Ended: `tick` drops it.
                call.ended = Some(Instant::now());
                // A call forwarded without ringing still goes to the history.
                if let Some((target, _)) = call.forward.clone() {
                    out.ended.push(EndedCall { info: info.clone(), started: call.started, answered: false, reason: format!("Forwarded to {target}") });
                }
            }
            return;
        }
        let was_answered = call.answered.is_some();
        if call.info.media != info.media {
            call.hold_requested = false;
        }
        call.info = info.clone();
        if info.state == CallState::Confirmed && call.answered.is_none() {
            call.answered = Some(Instant::now().checked_sub(info.connected_for).unwrap_or_else(Instant::now));
            let hook = call.hook_call();
            if info.direction == Direction::Incoming {
                out.hooks.push((HookKind::CallAnswer, hook.clone()));
                if info.remote_video {
                    out.hooks.push((HookKind::CallAnswerVideo, hook.clone()));
                }
            }
            out.hooks.push((HookKind::CallStart, hook));
            if info.direction == Direction::Outgoing
                && let Some(existing) = self.merge_on_answer.take()
                && existing != id
                && self.get(existing).is_some_and(Call::is_live)
            {
                out.commands.push(Command::Merge(vec![existing, id]));
            }
            if cfg.auto_hangup > 0 {
                // Auto hang-up: BYE with a Warning.
                let end = Command::End { call: id, code: 200, headers: vec![("Warning".into(), "399 localhost \"Call duration exceeded\"".into())] };
                out.commands.push(Command::Later { delay: Duration::from_secs(u64::from(cfg.auto_hangup)), command: Box::new(end) });
            }
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
                let kind = if matches!(info.status_code, 486 | 600 | 603) { HookKind::CallBusy } else { HookKind::CallEnd };
                out.hooks.push((kind, call.hook_call()));
                call.ended = Some(Instant::now());
                call.end_reason = Some(reason.clone());
                out.ended.push(EndedCall { info: info.clone(), started: call.started, answered: was_answered, reason });
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

    pub fn dial(&self, target: &str, local: bool, video: bool) -> Vec<Command> {
        vec![Command::MakeCall { target: target.to_owned(), video, local }]
    }

    /// Answers a ringing call, with video when `video`.
    pub fn answer(&mut self, id: CallId, video: bool) -> Vec<Command> {
        let Some(call) = self.get(id).filter(|c| c.is_ringing_in()) else { return Vec::new() };
        self.handled_sip_ids.push(call.info.sip_call_id.clone());
        self.current = Some(id);
        let mut commands = vec![Command::Answer { call: id, code: 200, video }];
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

    /// Hangs up a call, or the whole conference it's in.
    pub fn hangup(&self, id: CallId) -> Vec<Command> {
        let members = self.conference_of(id);
        if members.is_empty() { vec![Command::Hangup(id)] } else { members.into_iter().map(Command::Hangup).collect() }
    }

    /// Holds or resumes a call; in a conference, every member together.
    pub fn toggle_hold(&mut self, id: CallId) -> Vec<Command> {
        let Some(call) = self.get(id) else { return Vec::new() };
        if call.info.state != CallState::Confirmed {
            return Vec::new();
        }
        let resume = matches!(call.info.media, MediaState::LocalHold | MediaState::None);
        let members = self.conference_of(id);
        let ids = if members.is_empty() { vec![id] } else { members };
        let mut commands = Vec::new();
        for id in ids {
            let Some(call) = self.get_mut(id) else { continue };
            if resume {
                commands.push(Command::Unhold(id));
            } else {
                call.hold_requested = true;
                commands.push(Command::Hold(id));
            }
        }
        commands
    }

    /// The live members of the conference `id` is in (empty if none).
    pub fn conference_of(&self, id: CallId) -> Vec<CallId> {
        if !self.get(id).is_some_and(|c| c.info.in_conference) {
            return Vec::new();
        }
        self.calls.iter().filter(|c| c.is_live() && c.info.in_conference).map(Call::id).collect()
    }

    /// Answered calls that could join `id` in a conference.
    pub fn mergeable(&self, id: CallId) -> Vec<CallId> {
        let members = self.conference_of(id);
        self.calls
            .iter()
            .filter(|c| c.is_live() && c.id() != id && c.info.state == CallState::Confirmed && !members.contains(&c.id()))
            .map(Call::id)
            .collect()
    }

    /// Merges `id` with the other answered calls into one conference.
    pub fn merge(&mut self, id: CallId) -> Vec<Command> {
        let others = self.mergeable(id);
        if others.is_empty() {
            return Vec::new();
        }
        let mut ids = vec![id];
        ids.extend(self.conference_of(id).into_iter().filter(|&c| c != id));
        ids.extend(others);
        for c in self.calls.iter_mut().filter(|c| ids.contains(&c.id())) {
            c.hold_requested = false;
        }
        vec![Command::Merge(ids)]
    }

    /// Starts or stops recording a call (and its conference).
    pub fn toggle_recording(&self, id: CallId) -> Vec<Command> {
        let Some(call) = self.get(id).filter(|c| c.info.state == CallState::Confirmed) else { return Vec::new() };
        vec![Command::Record { call: id, on: !call.info.recording }]
    }

    /// Takes a call out of its conference; it stays connected.
    pub fn separate(&self, id: CallId) -> Vec<Command> {
        if self.conference_of(id).is_empty() { Vec::new() } else { vec![Command::Separate(id)] }
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

    /// Turns our camera off or on during a video call.
    pub fn set_camera(&mut self, id: CallId, on: bool) -> Vec<Command> {
        if self.get(id).is_none() {
            return Vec::new();
        }
        vec![Command::SetVideoSending { call: id, on }]
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

    /// Transfers to `target`: a REFER, or the PBX's transfer code and the
    /// number as DTMF when that's set.
    pub fn transfer(&self, id: CallId, target: &str, cfg: &CallSettings) -> Vec<Command> {
        match feature_code(cfg.use_transfer_code, &cfg.transfer_code) {
            Some(code) => self.dtmf(id, &format!("{code}{}", dial_user(target)), cfg.dtmf_method),
            None => vec![Command::Transfer { call: id, target: target.to_owned() }],
        }
    }

    /// Attended transfer by the PBX's code: the
    /// PBX calls `target` and connects us when we hang up.
    pub fn attended_transfer_by_code(&self, id: CallId, target: &str, cfg: &CallSettings) -> Vec<Command> {
        match feature_code(cfg.use_attended_code, &cfg.attended_code) {
            Some(code) => self.dtmf(id, &format!("{code}{}", dial_user(target)), cfg.dtmf_method),
            None => Vec::new(),
        }
    }

    pub fn attended_transfer(&self, id: CallId, with: CallId) -> Vec<Command> {
        vec![Command::AttendedTransfer { call: id, with }]
    }
}

/// A feature code when it's on and set.
fn feature_code(on: bool, code: &str) -> Option<&str> {
    Some(code.trim()).filter(|c| on && !c.is_empty())
}

/// The number part of a dialled target, for feature codes: the user of a
/// SIP address, or the target as typed.
fn dial_user(target: &str) -> String {
    let target = target.trim();
    if !target.contains(':') && !target.contains('<') {
        return target.split('@').next().unwrap_or(target).to_owned();
    }
    let user_host = uri_user_host(target);
    user_host.split('@').next().unwrap_or(&user_host).to_owned()
}

/// The wording for how a call ended.
fn end_reason(info: &CallInfo, answered: bool) -> String {
    let code = info.status_code;
    match code {
        _ if answered => "Call ended".into(),
        0 | 200 => "Call ended".into(),
        302 if info.direction == Direction::Incoming => "Forwarded".into(),
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
            video: false,
            receiving_video: false,
            sending_video: false,
            camera_off: false,
            in_conference: false,
            recording: false,
            sip_call_id: format!("sip-{id}"),
            caller_id: None,
            user_agent: None,
            diversion: None,
            auto_answer_header: None,
            call_info_header: None,
        }
    }

    fn book() -> CallBook {
        CallBook::new(SoundPaths {
            ringtone: "ring.wav".into(),
            ringback: "back.wav".into(),
            call_waiting: "wait.wav".into(),
            hangup: "hang.wav".into(),
            message_in: "in.wav".into(),
            message_out: "out.wav".into(),
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
    fn dnd_and_deny_reject_with_a_reason() {
        let rejected = |b: &mut CallBook, cfg: &CallSettings, id, remote: &str| {
            let out = b.on_event(&Event::IncomingCall(info(id, Direction::Incoming, CallState::Incoming, remote)), cfg);
            assert!(played(&out).is_empty() && !out.show_window, "a rejected call doesn't ring");
            out.commands.iter().find_map(|c| if let Command::Respond { code, reason, .. } = c { Some((*code, reason.clone())) } else { None })
        };
        let mut b = book();
        let dnd = CallSettings { dnd: true, ..CallSettings::default() };
        assert_eq!(rejected(&mut b, &dnd, 1, "<sip:100@pbx>"), Some((486, "Do Not Disturb".into())));
        assert!(!b.has_calls(), "not shown");

        // Calls to another user than the account's (local is me@pbx).
        let deny = CallSettings { deny_incoming: rustone_core::settings::DenyIncoming::OtherUser, ..CallSettings::default() };
        b.account = Some(rustone_core::rules::Identity { user: "someone".into(), domain: "pbx".into() });
        assert_eq!(rejected(&mut b, &deny, 2, "<sip:200@pbx>"), Some((486, "Denied".into())));
        b.account = Some(rustone_core::rules::Identity { user: "me".into(), domain: "pbx".into() });
        let out = b.on_event(&Event::IncomingCall(info(3, Direction::Incoming, CallState::Incoming, "<sip:300@pbx>")), &deny);
        assert_eq!(answers(&out), [180], "addressed to us: rings");
    }

    #[test]
    fn auto_answer_at_once_or_after_ringing() {
        let ring = |b: &mut CallBook, cfg: &CallSettings, i: CallInfo| b.on_event(&Event::IncomingCall(i), cfg);
        let later = |out: &Outcome| {
            out.commands.iter().find_map(|c| match c {
                Command::Later { delay, command } if matches!(**command, Command::Answer { code: 200, .. }) => Some(*delay),
                _ => None,
            })
        };
        // On, no delay: answered at once (with video when offered), beep, no ringtone.
        let mut b = book();
        let cfg = CallSettings { auto_answer: true, ..CallSettings::default() };
        let mut i = info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>");
        i.remote_video = true;
        let out = ring(&mut b, &cfg, i);
        assert!(out.commands.contains(&Command::Answer { call: 1, code: 200, video: true }));
        assert_eq!(played(&out), ["wait.wav"]);

        // With a delay: it rings and the engine answers later.
        let mut b = book();
        let cfg = CallSettings { auto_answer: true, auto_answer_delay: 5, ..CallSettings::default() };
        let out = ring(&mut b, &cfg, info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>"));
        assert_eq!(answers(&out), [180]);
        assert_eq!(later(&out), Some(Duration::from_secs(5)));
        assert!(b.get(1).is_some_and(|c| c.auto_answer_at.is_some()));

        // On request only: the caller's answer-after sets the delay.
        let mut b = book();
        let cfg = CallSettings { auto_answer_on_request: true, auto_answer_delay: 9, ..CallSettings::default() };
        assert_eq!(later(&ring(&mut b, &cfg, info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>"))), None, "not asked");
        let mut b = book();
        let mut i = info(2, Direction::Incoming, CallState::Incoming, "<sip:200@pbx>");
        i.call_info_header = Some("<sip:pbx>;answer-after=2".into());
        assert_eq!(later(&ring(&mut b, &cfg, i)), Some(Duration::from_secs(2)));

        // Other numbers, or another call in progress: rings normally.
        let mut b = book();
        let cfg = CallSettings { auto_answer: true, auto_answer_numbers: "1*".into(), ..CallSettings::default() };
        let out = ring(&mut b, &cfg, info(1, Direction::Incoming, CallState::Incoming, "<sip:200@pbx>"));
        assert_eq!(answers(&out), [180]);
        let out = ring(&mut b, &cfg, info(2, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>"));
        assert_eq!(answers(&out), [180], "busy with another call");
    }

    #[test]
    fn forwarding_at_once_or_after_ringing() {
        // At once: no ringing, no screen, but a history entry.
        let mut b = book();
        let cfg = CallSettings { forwarding: true, forwarding_number: "200".into(), ..CallSettings::default() };
        let out = b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        assert_eq!(out.commands, [Command::Forward { call: 1, target: "200".into() }]);
        assert!(!b.has_calls() && !out.show_window);
        let mut ended = info(1, Direction::Incoming, CallState::Disconnected, "<sip:100@pbx>");
        ended.status_code = 302;
        let out = b.on_event(&Event::Call(ended), &cfg);
        assert_eq!(out.ended.len(), 1);
        assert_eq!((out.ended[0].reason.as_str(), out.ended[0].is_missed()), ("Forwarded to 200", false));

        // After ringing: scheduled, and the screen counts down.
        let mut b = book();
        let cfg = CallSettings { forwarding_delay: 10, ..cfg };
        let out = b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        assert!(out.commands.contains(&Command::Later { delay: Duration::from_secs(10), command: Box::new(Command::Forward { call: 1, target: "200".into() }) }));
        assert_eq!(answers(&out), [180]);
        assert!(b.get(1).is_some_and(|c| c.forward.is_some()));

        // On without a number: nothing.
        let mut b = book();
        let cfg = CallSettings { forwarding: true, ..CallSettings::default() };
        let out = b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        assert_eq!(answers(&out), [180]);
    }

    #[test]
    fn auto_hangup_is_scheduled_once_answered() {
        let cfg = CallSettings { auto_hangup: 60, ..CallSettings::default() };
        let mut b = book();
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        let mut answered = info(1, Direction::Incoming, CallState::Confirmed, "<sip:100@pbx>");
        answered.media = MediaState::Active;
        let out = b.on_event(&Event::Call(answered.clone()), &cfg);
        let scheduled: Vec<_> = out.commands.iter().filter_map(|c| if let Command::Later { delay, command } = c { Some((*delay, command.as_ref())) } else { None }).collect();
        assert!(matches!(scheduled.as_slice(), [(d, Command::End { call: 1, code: 200, headers })] if *d == Duration::from_secs(60) && headers[0].1.starts_with("399 ")));
        // Only once, not on every later update.
        assert!(!b.on_event(&Event::Call(answered), &cfg).commands.iter().any(|c| matches!(c, Command::Later { .. })));
    }

    #[test]
    fn transfers_use_feature_codes_when_set() {
        let b = book();
        let dtmf = |cmds: Vec<Command>| cmds.into_iter().find_map(|c| if let Command::SendDtmf { digits, .. } = c { Some(digits) } else { None });
        let refer = CallSettings::default();
        assert_eq!(b.transfer(1, "200", &refer), [Command::Transfer { call: 1, target: "200".into() }]);
        assert!(b.attended_transfer_by_code(1, "200", &refer).is_empty(), "off by default");

        let codes = CallSettings { use_transfer_code: true, use_attended_code: true, ..CallSettings::default() };
        assert_eq!(dtmf(b.transfer(1, "200", &codes)).as_deref(), Some("##200"));
        assert_eq!(dtmf(b.transfer(1, "sip:300@pbx", &codes)).as_deref(), Some("##300"));
        assert_eq!(dtmf(b.attended_transfer_by_code(1, "400@pbx", &codes)).as_deref(), Some("*2400"));
        // On but empty: a normal transfer.
        let empty = CallSettings { transfer_code: " ".into(), ..codes };
        assert_eq!(b.transfer(1, "200", &empty), [Command::Transfer { call: 1, target: "200".into() }]);
    }

    #[test]
    fn conference_merges_holds_and_ends_together() {
        let cfg = CallSettings::default();
        let mut b = book();
        let answered = |id, media, conf| {
            let mut i = info(id, Direction::Outgoing, CallState::Confirmed, &format!("<sip:{id}00@pbx>"));
            i.media = media;
            i.in_conference = conf;
            i
        };
        b.on_event(&Event::Call(answered(1, MediaState::LocalHold, false)), &cfg);
        b.on_event(&Event::Call(answered(2, MediaState::Active, false)), &cfg);
        assert_eq!(b.mergeable(2), [1]);
        assert_eq!(b.merge(2), [Command::Merge(vec![2, 1])]);
        b.on_event(&Event::Call(answered(1, MediaState::Active, true)), &cfg);
        b.on_event(&Event::Call(answered(2, MediaState::Active, true)), &cfg);
        assert!(b.mergeable(2).is_empty(), "nothing left to merge");
        assert_eq!(b.toggle_hold(2), [Command::Hold(1), Command::Hold(2)], "held together");
        assert_eq!(b.hangup(2), [Command::Hangup(1), Command::Hangup(2)], "ended together");
        assert_eq!(b.separate(1), [Command::Separate(1)]);

        // A third call can join the running conference.
        b.on_event(&Event::Call(answered(3, MediaState::LocalHold, false)), &cfg);
        assert_eq!(b.merge(2), [Command::Merge(vec![2, 1, 3])]);
    }

    #[test]
    fn hooks_fire_on_call_events() {
        let cfg = CallSettings::default();
        let kinds = |out: &Outcome| out.hooks.iter().map(|(k, _)| *k).collect::<Vec<_>>();
        let mut b = book();
        let mut i = info(1, Direction::Incoming, CallState::Incoming, "\"Alice\" <sip:100@pbx>");
        i.remote_video = true;
        let out = b.on_event(&Event::IncomingCall(i.clone()), &cfg);
        assert_eq!(kinds(&out), [HookKind::IncomingCall, HookKind::CallRing]);
        assert_eq!((out.hooks[0].1.number.as_str(), out.hooks[0].1.name.as_str()), ("100", "Alice"));
        i.state = CallState::Confirmed;
        i.media = MediaState::Active;
        assert_eq!(kinds(&b.on_event(&Event::Call(i.clone()), &cfg)), [HookKind::CallAnswer, HookKind::CallAnswerVideo, HookKind::CallStart]);
        i.state = CallState::Disconnected;
        i.status_code = 200;
        assert_eq!(kinds(&b.on_event(&Event::Call(i), &cfg)), [HookKind::CallEnd]);

        let mut o = info(2, Direction::Outgoing, CallState::Calling, "<sip:200@pbx>");
        assert_eq!(kinds(&b.on_event(&Event::Call(o.clone()), &cfg)), [HookKind::OutgoingCall]);
        o.state = CallState::Disconnected;
        o.status_code = 486;
        assert_eq!(kinds(&b.on_event(&Event::Call(o), &cfg)), [HookKind::CallBusy]);
    }

    #[test]
    fn a_conference_button_merges_once_answered() {
        let cfg = CallSettings::default();
        let mut b = book();
        let mut first = info(1, Direction::Outgoing, CallState::Confirmed, "<sip:100@pbx>");
        first.media = MediaState::Active;
        b.on_event(&Event::Call(first), &cfg);
        b.merge_on_answer = Some(1);
        let mut second = info(2, Direction::Outgoing, CallState::Calling, "<sip:200@pbx>");
        assert!(!b.on_event(&Event::Call(second.clone()), &cfg).commands.iter().any(|c| matches!(c, Command::Merge(_))), "not before it's answered");
        second.state = CallState::Confirmed;
        second.media = MediaState::Active;
        assert!(b.on_event(&Event::Call(second), &cfg).commands.contains(&Command::Merge(vec![1, 2])));
        assert_eq!(b.merge_on_answer, None, "once");
    }

    #[test]
    fn ended_calls_go_to_the_history() {
        use rustone_core::history::CallKind;
        let cfg = CallSettings::default();
        let mut b = book();
        let ended = |b: &mut CallBook, id, code| {
            let mut i = info(id, Direction::Incoming, CallState::Disconnected, "<sip:100@pbx>");
            i.status_code = code;
            b.on_event(&Event::Call(i), &cfg).ended
        };
        b.on_event(&Event::IncomingCall(info(1, Direction::Incoming, CallState::Incoming, "<sip:100@pbx>")), &cfg);
        let missed = ended(&mut b, 1, 487);
        assert_eq!((missed.len(), missed[0].kind(), missed[0].is_missed()), (1, CallKind::Missed, true));
        assert_eq!(missed[0].reason, "Missed call");

        b.on_event(&Event::IncomingCall(info(2, Direction::Incoming, CallState::Incoming, "<sip:200@pbx>")), &cfg);
        let declined = ended(&mut b, 2, 486);
        assert_eq!((declined[0].kind(), declined[0].is_missed(), declined[0].reason.as_str()), (CallKind::Missed, false, "Declined"));

        // A refused duplicate leaves no trace.
        b.on_event(&Event::IncomingCall(info(3, Direction::Incoming, CallState::Incoming, "<sip:300@pbx>")), &cfg);
        b.on_event(&Event::IncomingCall(info(4, Direction::Incoming, CallState::Incoming, "<sip:300@pbx>")), &cfg);
        assert!(ended(&mut b, 4, 486).is_empty());
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

        b.answer(1, false);
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
        let cmds = b.answer(2, false);
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
