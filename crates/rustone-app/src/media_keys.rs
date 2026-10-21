//! Media keys and headset buttons for calls:
//! while a call rings or is up, RusTone shows it in the
//! desktop's media controls, so play/pause answers or holds and stop
//! declines or hangs up. Between calls the controls show "No call",
//! stopped, so the keys go to music players. They start with the first
//! call (the platform's controls can't be removed once made).

use fastframe_now_playing::{App as MediaApp, Command, Controls, NowPlaying, Playback, State, Track};

/// What a key press asks of the calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKey {
    /// Answer the ringing call, or hold/resume the current one.
    AnswerOrHold,
    /// Decline the ringing call, or hang up the current one.
    DeclineOrHangUp,
    /// Bring the window forward.
    Raise,
}

/// The call the controls show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub id: String,
    pub caller: String,
    pub status: String,
    /// Ringing or talking: shown as playing, which is what Windows sends
    /// the keys to. A held call shows as paused.
    pub playing: bool,
}

pub struct MediaKeys {
    controls: NowPlaying,
    shown: Option<Option<Shown>>,
}

impl MediaKeys {
    /// Starts the controls; call on the main thread.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let mut app = MediaApp::new("rustone", "RusTone");
        app.desktop_entry = crate::DESKTOP_ENTRY.to_owned();
        app.can_quit = false;
        Self { controls: NowPlaying::start(app, wake), shown: None }
    }

    /// Shows the current call, or nothing.
    pub fn show(&mut self, call: Option<Shown>) {
        if self.shown.as_ref() == Some(&call) {
            return;
        }
        let state = match &call {
            Some(c) => State {
                playback: if c.playing { Playback::Playing } else { Playback::Paused },
                track: Some(Track { id: c.id.clone(), title: c.caller.clone(), artists: vec![c.status.clone()], ..Track::default() }),
                controls: Controls { play: true, pause: true, next: false, previous: false, seek: false },
                ..State::default()
            },
            // Stopped, so Windows hands the keys back to music players.
            None => State {
                playback: Playback::Stopped,
                track: Some(Track { id: "idle".into(), title: "RusTone".into(), artists: vec!["No call".into()], ..Track::default() }),
                controls: Controls { play: false, pause: false, next: false, previous: false, seek: false },
                ..State::default()
            },
        };
        self.controls.update(state);
        self.shown = Some(call);
    }

    /// Key presses since the last call.
    pub fn keys(&self) -> Vec<CallKey> {
        self.controls
            .commands()
            .into_iter()
            .filter_map(|command| match command {
                // Windows sends a play/pause key as Pause to a playing
                // session (ringing, talking) and Play to a paused one (held).
                Command::Play | Command::Pause | Command::PlayPause => Some(CallKey::AnswerOrHold),
                Command::Stop => Some(CallKey::DeclineOrHangUp),
                Command::Raise => Some(CallKey::Raise),
                _ => None,
            })
            .collect()
    }
}
