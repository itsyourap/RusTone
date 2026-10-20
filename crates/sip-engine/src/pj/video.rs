//! Video devices, codecs and the local preview.
//! Engine thread only.

use std::ffi::c_uint;

use pjsua_sys::*;

use super::util::{cstr_to_string, pj_default, pj_to_string};
use crate::video::{VideoConfig, VideoDevice};

/// pjmedia's window flags (videodev.h): a border, resizable.
pub const WND_BORDER: c_uint = 1;
pub const WND_RESIZABLE: c_uint = 2;

/// Every video device pjsua knows.
pub fn devices() -> Vec<VideoDevice> {
    let mut infos: Vec<pjmedia_vid_dev_info> = vec![unsafe { std::mem::zeroed() }; 16];
    let mut count = infos.len() as c_uint;
    // SAFETY: pjsua fills at most `count` entries.
    if unsafe { pjsua_vid_enum_devs(infos.as_mut_ptr(), &mut count) } != PJ_SUCCESS {
        return Vec::new();
    }
    infos[..count as usize]
        .iter()
        .map(|d| VideoDevice {
            index: d.id,
            name: cstr_to_string(&d.name),
            driver: cstr_to_string(&d.driver),
            // Camera filter: formats, and an encoding direction.
            capture: d.fmt_cnt > 0 && (d.dir == PJMEDIA_DIR_ENCODING || d.dir == PJMEDIA_DIR_ENCODING_DECODING),
        })
        .collect()
}

/// Video codecs: all off without video; otherwise H.264 gets the bitrate
/// (found by name since payload types differ
/// between builds).
pub fn apply_codecs(cfg: &VideoConfig) {
    let mut infos: [pjsua_codec_info; 16] = unsafe { std::mem::zeroed() };
    let mut count = infos.len() as c_uint;
    // SAFETY: pjsua fills at most `count` entries; ids point into them.
    if unsafe { pjsua_vid_enum_codecs(infos.as_mut_ptr(), &mut count) } != PJ_SUCCESS {
        return;
    }
    for info in &infos[..count as usize] {
        let id = pj_to_string(&info.codec_id);
        if !cfg.enabled {
            // SAFETY: a codec id from the enumeration.
            unsafe { pjsua_vid_codec_set_priority(&info.codec_id, 0) };
            continue;
        }
        if cfg.bitrate_kbps > 0 && id.to_ascii_uppercase().starts_with("H264") {
            let mut param = std::mem::MaybeUninit::<pjmedia_vid_codec_param>::zeroed();
            // SAFETY: filled on success, then handed back.
            unsafe {
                if pjsua_vid_codec_get_param(&info.codec_id, param.as_mut_ptr()) == PJ_SUCCESS {
                    let param = param.assume_init_mut();
                    let bps = cfg.bitrate_kbps.saturating_mul(1000);
                    param.enc_fmt.det.vid.avg_bps = bps;
                    param.enc_fmt.det.vid.max_bps = bps;
                    if pjsua_vid_codec_set_param(&info.codec_id, param) != PJ_SUCCESS {
                        log::warn!("cannot set the {id} bitrate");
                    }
                }
            }
        }
    }
}

/// Starts the local preview of `camera`, drawn by RusTone (pjsua's window
/// stays hidden). Returns false when the camera can't be opened.
pub fn start_preview(camera: i32) -> bool {
    pj_default!(param: pjsua_vid_preview_param = pjsua_vid_preview_param_default);
    param.show = PJ_FALSE;
    param.wnd_flags = WND_BORDER | WND_RESIZABLE;
    // SAFETY: `param` initialised in place.
    let status = unsafe { pjsua_vid_preview_start(camera, &param) };
    if status != PJ_SUCCESS {
        log::warn!("cannot start the video preview: {}", super::util::error_text(status));
        return false;
    }
    super::vidtap::tap_preview(camera);
    true
}

pub fn stop_preview(camera: i32) {
    super::vidtap::untap_preview();
    // SAFETY: stopping a preview that isn't running just fails.
    unsafe { pjsua_vid_preview_stop(camera) };
}
