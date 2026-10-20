//! USB telephony headsets: the
//! hook switch answers and hangs up, the mute button mutes the microphone,
//! redial calls the last number, and the headset's off-hook, ring and mute
//! lights follow the calls. A worker thread owns the device: it opens the
//! first one on the telephony usage page, reads its buttons, writes its
//! lights, and looks for one again when it's unplugged.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustone_core::hid::{self, Layout};

/// What the headset's buttons did, as absolute states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Off hook (picked up) or on hook (put down).
    Hook(bool),
    /// Its mute button wants the microphone muted or not.
    Mute(bool),
    Redial,
}

/// What the lights show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lights {
    pub off_hook: bool,
    pub ring: bool,
    pub mute: bool,
}

pub struct Headset {
    lights: Sender<Lights>,
    buttons: Arc<Mutex<Vec<Button>>>,
    last: Option<Lights>,
}

/// How often to look for a headset while there's none.
const LOOK_EVERY: Duration = Duration::from_secs(3);

impl Headset {
    /// Starts the worker; `wake` runs when a button is pressed.
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (lights, rx) = mpsc::channel();
        let buttons: Arc<Mutex<Vec<Button>>> = Arc::default();
        let out = buttons.clone();
        let spawned = std::thread::Builder::new().name("headset".into()).spawn(move || run(&rx, &out, &wake));
        if let Err(e) = spawned {
            log::warn!("cannot watch for headsets: {e}");
        }
        Self { lights, buttons, last: None }
    }

    /// Button presses since the last call.
    pub fn buttons(&self) -> Vec<Button> {
        std::mem::take(&mut *self.buttons.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Sets the lights (sent only when they change).
    pub fn show(&mut self, lights: Lights) {
        if self.last != Some(lights) {
            let _ = self.lights.send(lights);
            self.last = Some(lights);
        }
    }
}

struct Open {
    device: hidapi::HidDevice,
    tracker: Tracker,
}

/// A headset's layout and the states tracked to turn toggle
/// buttons into states.
struct Tracker {
    layout: Layout,
    off_hook: bool,
    mute: bool,
    redial: bool,
}

impl Tracker {
    fn new(layout: Layout) -> Self {
        Self { layout, off_hook: false, mute: false, redial: false }
    }
}

/// Ends when the app drops its `Headset` (the lights channel closes).
fn run(rx: &Receiver<Lights>, out: &Mutex<Vec<Button>>, wake: &dyn Fn()) {
    let mut api: Option<hidapi::HidApi> = None;
    let mut open: Option<Open> = None;
    let mut lights = Lights::default();
    let mut next_look = Instant::now();
    loop {
        loop {
            match rx.try_recv() {
                Ok(l) => {
                    lights = l;
                    if let Some(o) = &mut open {
                        o.tracker.follow_app(lights);
                        write_lights(o, lights);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(o) = &mut open else {
            if Instant::now() >= next_look {
                next_look = Instant::now() + LOOK_EVERY;
                open = find(&mut api);
                if let Some(o) = &open {
                    write_lights(o, lights);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        let mut buf = [0u8; 64];
        match o.device.read_timeout(&mut buf, 200) {
            Ok(0) => {}
            Ok(n) => {
                let pressed = o.tracker.decode(&buf[..n]);
                if !pressed.is_empty() {
                    out.lock().unwrap_or_else(|e| e.into_inner()).extend(pressed);
                    wake();
                }
            }
            Err(e) => {
                log::info!("headset gone: {e}");
                open = None;
            }
        }
    }
}

/// The first telephony headset that has a hook switch.
fn find(api: &mut Option<hidapi::HidApi>) -> Option<Open> {
    let api = match api {
        Some(api) => {
            let _ = api.refresh_devices();
            api
        }
        None => api.insert(hidapi::HidApi::new().map_err(|e| log::warn!("no HID access: {e}")).ok()?),
    };
    let candidates: Vec<_> = api.device_list().filter(|d| d.usage_page() == hid::PAGE_TELEPHONY).cloned().collect();
    for info in candidates {
        let Ok(device) = info.open_device(api) else { continue };
        let mut descriptor = [0u8; 4096];
        let Ok(len) = device.get_report_descriptor(&mut descriptor) else { continue };
        let layout = hid::parse(&descriptor[..len]);
        if layout.is_usable() {
            log::info!("headset: {} {}", info.manufacturer_string().unwrap_or_default(), info.product_string().unwrap_or_default());
            return Some(Open { device, tracker: Tracker::new(layout) });
        }
    }
    None
}

impl Tracker {
    /// Buttons in an input report: a relative (toggle) button
    /// flips the state on each press; an absolute one reports it.
    fn decode(&mut self, report: &[u8]) -> Vec<Button> {
        let o = self;
        let mut pressed = Vec::new();
        let follow = |usage: u16, relative: bool, state: &mut bool| -> Option<bool> {
            let on = o.layout.input(usage, report)?;
            let new = if relative { if on { !*state } else { *state } } else { on };
            (new != *state).then(|| {
                *state = new;
                new
            })
        };
        let hook_relative = o.layout.hook_switch.is_some_and(|f| f.relative);
        if let Some(off_hook) = follow(hid::HOOK_SWITCH, hook_relative, &mut o.off_hook) {
            pressed.push(Button::Hook(off_hook));
        }
        let mute_relative = o.layout.mute.is_none_or(|f| f.relative);
        if let Some(mute) = follow(hid::PHONE_MUTE, mute_relative, &mut o.mute) {
            pressed.push(Button::Mute(mute));
        }
        // Redial counts on the press, not the release.
        if let Some(down) = o.layout.input(hid::REDIAL, report) {
            if down && !o.redial {
                pressed.push(Button::Redial);
            }
            o.redial = down;
        }
        pressed
    }
}

fn write_lights(o: &Open, lights: Lights) {
    for report in o.tracker.layout.leds(lights.off_hook, lights.ring, lights.mute) {
        if let Err(e) = o.device.write(&report) {
            log::debug!("headset lights: {e}");
        }
    }
}

impl Tracker {
    /// Toggle buttons flip from the app's state: a call the other side
    /// ended, or mute changed on screen.
    /// An absolute button's state is the device's own and stays.
    fn follow_app(&mut self, lights: Lights) {
        if self.layout.hook_switch.is_some_and(|f| f.relative) {
            self.off_hook = lights.off_hook;
        }
        if self.layout.mute.is_none_or(|f| f.relative) {
            self.mute = lights.mute;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Report 1: hook switch (absolute) bit 0, mute (relative) bit 1,
    /// redial (relative) bit 2.
    fn tracker() -> Tracker {
        let descriptor = [
            0x05, 0x0B, 0x09, 0x05, 0xA1, 0x01, 0x85, 0x01, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01, 0x09, 0x20, 0x81, 0x02, 0x95, 0x02, 0x09, 0x2F, 0x09, 0x24, 0x81, 0x06,
            0x95, 0x05, 0x81, 0x01, 0xC0,
        ];
        Tracker::new(hid::parse(&descriptor))
    }

    #[test]
    fn hook_switch_reports_its_state_and_mute_toggles() {
        let mut t = tracker();
        assert_eq!(t.decode(&[1, 0b001]), [Button::Hook(true)]);
        assert_eq!(t.decode(&[1, 0b001]), [], "no change");
        // Mute pressed (toggle on), released, pressed again (toggle off).
        assert_eq!(t.decode(&[1, 0b011]), [Button::Mute(true)]);
        assert_eq!(t.decode(&[1, 0b001]), []);
        assert_eq!(t.decode(&[1, 0b011]), [Button::Mute(false)]);
        // Redial counts once per press.
        assert_eq!(t.decode(&[1, 0b101]), [Button::Redial]);
        assert_eq!(t.decode(&[1, 0b101]), []);
        assert_eq!(t.decode(&[1, 0b000]), [Button::Hook(false)]);
    }

    #[test]
    fn toggles_follow_the_app() {
        let mut t = tracker();
        // Muted on screen: the headset's next mute press unmutes.
        t.follow_app(Lights { off_hook: true, ring: false, mute: true });
        assert_eq!(t.decode(&[1, 0b010]), [Button::Mute(false)]);
        // The absolute hook switch keeps the device's own state.
        assert!(!t.off_hook);
    }
}
