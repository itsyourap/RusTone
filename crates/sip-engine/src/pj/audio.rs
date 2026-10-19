//! Sound device enumeration and switching.
//! Engine thread only.

use std::ffi::c_uint;

use pjsua_sys::*;

use super::util::{cstr_to_string, pj_default};
use crate::audio::{AudioDevice, DEFAULT_CAPTURE};

/// Every sound device pjsua knows, in pjmedia's index order.
pub fn devices() -> Vec<AudioDevice> {
    let mut infos: Vec<pjmedia_aud_dev_info> = vec![unsafe { std::mem::zeroed() }; 64];
    let mut count = infos.len() as c_uint;
    // SAFETY: pjsua fills at most `count` entries.
    if unsafe { pjsua_enum_aud_devs(infos.as_mut_ptr(), &mut count) } != PJ_SUCCESS {
        return Vec::new();
    }
    infos[..count as usize]
        .iter()
        .enumerate()
        .map(|(i, d)| AudioDevice {
            index: i as i32,
            name: cstr_to_string(&d.name),
            driver: cstr_to_string(&d.driver),
            inputs: d.input_count,
            outputs: d.output_count,
        })
        .collect()
}

/// The devices pjsua has open, if any: `(capture, playback)`.
pub fn current() -> Option<(i32, i32)> {
    let (mut capture, mut playback) = (0, 0);
    // SAFETY: plain queries.
    unsafe { (pjsua_snd_is_active() != 0 && pjsua_get_snd_dev(&mut capture, &mut playback) == PJ_SUCCESS).then_some((capture, playback)) }
}

/// Opens `input` and `output` unless they're already in use (or `force`).
/// When the microphone can't be opened, plays without one (the
/// `PJSUA_SND_DEV_SPEAKER_ONLY` fallback).
pub fn open(input: i32, output: i32, force: bool) {
    let (mut capture, mut playback) = (0, 0);
    // SAFETY: plain query.
    let same = unsafe { pjsua_get_snd_dev(&mut capture, &mut playback) } == PJ_SUCCESS && capture == input && playback == output;
    // Always re-open for the default microphone: its index
    // doesn't say which device Windows currently uses as the default.
    if same && !force && input != DEFAULT_CAPTURE {
        return;
    }
    pj_default!(params: pjsua_snd_dev_param = pjsua_snd_dev_param_default);
    params.capture_dev = input;
    params.playback_dev = output;
    // SAFETY: `params` initialised in place.
    if unsafe { pjsua_set_snd_dev2(&params) } != PJ_SUCCESS {
        params.mode |= PJSUA_SND_DEV_SPEAKER_ONLY as c_uint;
        // SAFETY: as above.
        let status = unsafe { pjsua_set_snd_dev2(&params) };
        if status == PJ_SUCCESS {
            log::warn!("cannot open the microphone (device {input}); playing without it");
        } else {
            log::warn!("cannot open sound device {output}: {}", super::util::error_text(status));
        }
    }
}

/// Tells pjsua which devices to open next time it needs one, without
/// opening them now.
pub fn select(input: i32, output: i32) {
    pj_default!(params: pjsua_snd_dev_param = pjsua_snd_dev_param_default);
    params.capture_dev = input;
    params.playback_dev = output;
    params.mode = PJSUA_SND_DEV_NO_IMMEDIATE_OPEN as c_uint;
    // SAFETY: `params` initialised in place.
    let status = unsafe { pjsua_set_snd_dev2(&params) };
    if status != PJ_SUCCESS {
        log::warn!("cannot select sound devices {input}/{output}: {}", super::util::error_text(status));
    }
}

/// The sound device port's levels: what the microphone sends into the
/// bridge and what the bridge sends to the speaker (0..=255).
pub fn signal_levels() -> (u8, u8) {
    let (mut to_speaker, mut from_mic): (c_uint, c_uint) = (0, 0);
    // SAFETY: slot 0 is the sound device port; plain query.
    if unsafe { pjsua_conf_get_signal_level(0, &mut to_speaker, &mut from_mic) } != PJ_SUCCESS {
        return (0, 0);
    }
    (from_mic.min(255) as u8, to_speaker.min(255) as u8)
}
