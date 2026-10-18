//! Minimal pjsua driver for the Phase 0 spike, for a single account. The production wrapper with an
//! engine thread is `sip-engine` (Phase 2).

use std::ffi::{CString, c_char, c_int, c_uint};
use std::mem::MaybeUninit;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use pjsua_sys::*;

use crate::config::{Account, Config, Settings};

/// Declares `$var`, zero-initialises it and runs pjsua's `*_default`
/// function on it **in place**. Some of these structs (e.g.
/// `pjsua_acc_config::reg_hdr_list`) contain self-referential `pj_list`
/// heads, so they must never be moved after initialisation.
macro_rules! pj_default {
    ($var:ident : $ty:ty = $init:path) => {
        let mut $var: $ty = std::mem::zeroed();
        $init(&mut $var);
    };
}

/// Everything pjsua logged during the session (for the Contact check).
static LOG: Mutex<String> = Mutex::new(String::new());
static ECHO_SIP: AtomicBool = AtomicBool::new(false);
static AUTO_ANSWER: AtomicBool = AtomicBool::new(false);
/// Set by `on_reg_state2` when an unregistration has completed.
static UNREGISTERED: AtomicBool = AtomicBool::new(false);

const POLL: Duration = Duration::from_millis(200);
const RING_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Options {
    pub call: Option<String>,
    pub duration: Duration,
    pub listen: Duration,
    pub answer: bool,
    pub reg_timeout: Duration,
    pub echo_sip: bool,
}

/// Runs one session. Returns its outcome and everything pjsua logged,
/// which is returned even when the session fails.
pub fn run(cfg: &Config, opts: &Options, stop: &AtomicBool) -> (Result<()>, String) {
    ECHO_SIP.store(opts.echo_sip, Ordering::Relaxed);
    AUTO_ANSWER.store(opts.answer, Ordering::Relaxed);
    UNREGISTERED.store(false, Ordering::Relaxed);
    LOG.lock().unwrap().clear();

    let mut arena = Arena::default();
    let result = unsafe {
        check(pjsua_create(), "pjsua_create").and_then(|()| {
            let result = session(cfg, opts, stop, &mut arena);
            pjsua_destroy();
            result
        })
    };
    (result, std::mem::take(&mut *LOG.lock().unwrap()))
}

unsafe fn session(cfg: &Config, opts: &Options, stop: &AtomicBool, arena: &mut Arena) -> Result<()> {
    let (a, s) = (&cfg.account, &cfg.settings);
    unsafe {
        init(s, arena)?;
        let udp = create_transports(s)?;
        check(pjsua_start(), "pjsua_start")?;
        set_audio_codecs(&s.audio_codecs, arena);

        let acc_id = add_account(a, s, udp, arena)?;
        wait_registered(acc_id, opts.reg_timeout, stop)?;

        if let Some(target) = &opts.call {
            call(a, acc_id, target, opts.duration, stop, arena)?;
        }
        if !opts.listen.is_zero() {
            println!("[listen] waiting {}s for incoming calls (answer: {})", opts.listen.as_secs(), opts.answer);
            wait_until(opts.listen, stop, || false);
            pjsua_call_hangup_all();
            wait_until(SHUTDOWN_TIMEOUT, stop, || pjsua_call_get_count() == 0);
        }

        println!("[reg] unregistering");
        pjsua_acc_set_registration(acc_id, PJ_FALSE);
        // Wait for the result so pjsua_destroy() doesn't start a second one.
        wait_until(SHUTDOWN_TIMEOUT, &AtomicBool::new(false), || UNREGISTERED.load(Ordering::Relaxed));
    }
    Ok(())
}

unsafe fn init(s: &Settings, arena: &mut Arena) -> Result<()> {
    unsafe {
        pj_default!(ua: pjsua_config = pjsua_config_default);
        let agent = if s.user_agent.is_empty() {
            format!("RusTone/{}", env!("CARGO_PKG_VERSION"))
        } else {
            s.user_agent.clone()
        };
        ua.user_agent = arena.pj(&agent);
        ua.srtp_secure_signaling = 0;
        ua.cb.on_reg_state2 = Some(on_reg_state2);
        ua.cb.on_incoming_call = Some(on_incoming_call);
        ua.cb.on_call_state = Some(on_call_state);
        ua.cb.on_call_media_state = Some(on_call_media_state);

        pj_default!(log: pjsua_logging_config = pjsua_logging_config_default);
        log.level = 5;
        log.console_level = 5;
        log.msg_logging = PJ_TRUE;
        log.cb = Some(on_log);
        if !s.log_file.is_empty() {
            log.log_filename = arena.pj(&s.log_file);
        }

        pj_default!(media: pjsua_media_config = pjsua_media_config_default);
        media.no_vad = (!s.vad) as pj_bool_t;
        media.ec_tail_len = if s.ec { 20 } else { 0 };
        media.enable_ice = PJ_FALSE;
        media.clock_rate = max_clock_rate(&s.audio_codecs);
        media.channel_count = 1;

        (*shim::jf_pjsip_cfg()).endpt.disable_rport = (!s.rport) as pj_bool_t;

        check(pjsua_init(&ua, &log, &media), "pjsua_init")
    }
}

/// UDP on `source_port`, TCP and TLS on ephemeral ports
/// without the local account. Returns the UDP transport id.
unsafe fn create_transports(s: &Settings) -> Result<pjsua_transport_id> {
    let mut udp = PJSUA_INVALID_ID;
    for (kind, port, name) in [
        (PJSIP_TRANSPORT_UDP, s.source_port, "UDP"),
        (PJSIP_TRANSPORT_TCP, 0, "TCP"),
        (PJSIP_TRANSPORT_TLS, 0, "TLS"),
    ] {
        unsafe {
            pj_default!(tc: pjsua_transport_config = pjsua_transport_config_default);
            tc.port = port;
            let mut id = PJSUA_INVALID_ID;
            check(pjsua_transport_create(kind, &tc, &mut id), &format!("{name} transport"))?;
            if kind == PJSIP_TRANSPORT_UDP {
                udp = id;
            }
        }
    }
    Ok(udp)
}

/// Disables every codec, then enables the configured ones with descending
/// priority starting at 128.
unsafe fn set_audio_codecs(list: &str, arena: &mut Arena) {
    unsafe {
        let mut infos: [pjsua_codec_info; 64] = std::mem::zeroed();
        let mut count = infos.len() as c_uint;
        if pjsua_enum_codecs(infos.as_mut_ptr(), &mut count) == PJ_SUCCESS {
            for info in &infos[..count as usize] {
                pjsua_codec_set_priority(&info.codec_id, 0);
            }
        }
        let mut priority: u8 = 128;
        for name in list.split_whitespace() {
            let id = arena.pj(name);
            match pjsua_codec_set_priority(&id, priority) {
                0 => priority -= 1,
                status => eprintln!("[codec] {name}: {}", error_text(status)),
            }
        }
    }
}

unsafe fn add_account(a: &Account, s: &Settings, udp: pjsua_transport_id, arena: &mut Arena) -> Result<pjsua_acc_id> {
    unsafe {
        pj_default!(ac: pjsua_acc_config = pjsua_acc_config_default);
        ac.ka_interval = a.keep_alive;
        ac.reg_timeout = a.register_refresh;
        if a.disable_session_timer {
            ac.use_timer = PJSUA_SIP_TIMER_INACTIVE;
        }
        if s.rtp_port_min > 0 {
            ac.rtp_cfg.port = s.rtp_port_min;
            if s.rtp_port_max > s.rtp_port_min {
                ac.rtp_cfg.port_range = s.rtp_port_max - s.rtp_port_min;
            }
        }

        match a.srtp.as_str() {
            "optional" => ac.use_srtp = PJMEDIA_SRTP_OPTIONAL,
            "mandatory" => ac.use_srtp = PJMEDIA_SRTP_MANDATORY,
            "dtls-sdes" | "dtls" => {
                ac.use_srtp = PJMEDIA_SRTP_MANDATORY;
                ac.enable_rtcp_mux = PJ_TRUE;
                ac.srtp_opt.keying[0] = PJMEDIA_SRTP_KEYING_DTLS_SRTP;
                ac.srtp_opt.keying_count = 1;
                if a.srtp == "dtls-sdes" {
                    ac.srtp_opt.keying[1] = PJMEDIA_SRTP_KEYING_SDES;
                    ac.srtp_opt.keying_count = 2;
                }
            }
            _ => ac.use_srtp = PJMEDIA_SRTP_DISABLED,
        }

        ac.ice_cfg_use = PJSUA_ICE_CONFIG_USE_CUSTOM;
        ac.ice_cfg.enable_ice = a.ice as pj_bool_t;
        ac.allow_via_rewrite = a.allow_rewrite as pj_bool_t;
        ac.allow_sdp_nat_rewrite = a.allow_rewrite as pj_bool_t;
        ac.allow_contact_rewrite = if a.allow_rewrite { 2 } else { 0 };
        ac.contact_rewrite_method =
            (PJSUA_CONTACT_REWRITE_ALWAYS_UPDATE | PJSUA_CONTACT_REWRITE_UNREGISTER) as c_int;
        ac.publish_enabled = a.publish as pj_bool_t;
        if a.transport == "udp" {
            ac.transport_id = udp;
        }

        ac.cred_count = 1;
        let cred = &mut ac.cred_info[0];
        cred.username = arena.pj(a.auth_user());
        cred.realm = arena.pj("*");
        cred.scheme = arena.pj("Digest");
        cred.data_type = PJSIP_CRED_DATA_PLAIN_PASSWD as c_int;
        cred.data = arena.pj(&a.password);

        let proxies = a.proxy_uris();
        if proxies.len() > ac.proxy.len() {
            bail!("at most {} proxies are supported", ac.proxy.len());
        }
        for (slot, uri) in ac.proxy.iter_mut().zip(&proxies) {
            *slot = arena.pj(uri);
        }
        ac.proxy_cnt = proxies.len() as c_uint;

        ac.id = arena.pj(&a.id_uri());
        match a.reg_uri() {
            Some(uri) => ac.reg_uri = arena.pj(&uri),
            None => ac.register_on_acc_add = PJ_FALSE,
        }

        let mut acc_id = PJSUA_INVALID_ID;
        check(pjsua_acc_add(&ac, PJ_TRUE, &mut acc_id), "pjsua_acc_add")?;
        Ok(acc_id)
    }
}

fn wait_registered(acc_id: pjsua_acc_id, timeout: Duration, stop: &AtomicBool) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if stop.load(Ordering::Relaxed) {
            bail!("interrupted");
        }
        let state = acc_info(acc_id, |i| (is_registered(i), i.status, pj_to_string(&i.status_text)));
        if let Some((registered, status, text)) = state {
            if registered {
                return Ok(());
            }
            // 401/407 are retried by pjsua with credentials; other finals are fatal.
            if status >= 300 && status != 401 && status != 407 {
                bail!("registration failed: {status} {text}");
            }
        }
        if Instant::now() >= deadline {
            bail!("registration timed out after {}s", timeout.as_secs());
        }
        sleep(POLL);
    }
}

unsafe fn call(a: &Account, acc_id: pjsua_acc_id, target: &str, duration: Duration, stop: &AtomicBool, arena: &mut Arena) -> Result<()> {
    let uri = a.sip_uri(target);
    println!("[call] dialing {uri}");
    unsafe {
        pj_default!(opt: pjsua_call_setting = pjsua_call_setting_default);
        opt.vid_cnt = 0;
        let dst = arena.pj(&uri);
        let mut call_id = PJSUA_INVALID_ID;
        check(
            pjsua_call_make_call(acc_id, &dst, &opt, ptr::null_mut(), ptr::null(), &mut call_id),
            "pjsua_call_make_call",
        )?;

        let started = Instant::now();
        let mut confirmed_at: Option<Instant> = None;
        while !stop.load(Ordering::Relaxed) {
            sleep(POLL);
            let Some(state) = call_info(call_id, |ci| ci.state) else { break };
            match state {
                PJSIP_INV_STATE_DISCONNECTED => break,
                PJSIP_INV_STATE_CONFIRMED => {
                    let at = *confirmed_at.get_or_insert_with(Instant::now);
                    if at.elapsed() >= duration {
                        println!("[call] {}s elapsed, hanging up", duration.as_secs());
                        break;
                    }
                }
                _ if started.elapsed() >= RING_TIMEOUT => {
                    println!("[call] not answered in {}s, hanging up", RING_TIMEOUT.as_secs());
                    break;
                }
                _ => {}
            }
        }
        pjsua_call_hangup_all();
        wait_until(SHUTDOWN_TIMEOUT, &AtomicBool::new(false), || pjsua_call_get_count() == 0);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Callbacks (run on pjsua worker threads; must never unwind into C)

fn guard(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

unsafe extern "C" fn on_log(level: c_int, data: *const c_char, len: c_int) {
    guard(|| {
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len.max(0) as usize) };
        let text = String::from_utf8_lossy(bytes);
        if level <= 2 || ECHO_SIP.load(Ordering::Relaxed) {
            eprint!("{text}");
        }
        if let Ok(mut log) = LOG.lock() {
            log.push_str(&text);
        }
    });
}

unsafe extern "C" fn on_reg_state2(acc_id: pjsua_acc_id, info: *mut pjsua_reg_info) {
    guard(|| {
        // `renew` is zero when this callback reports an unregistration.
        if !info.is_null() && unsafe { (*info).renew } == 0 {
            UNREGISTERED.store(true, Ordering::Relaxed);
        }
        acc_info(acc_id, |i| {
            println!("[reg] {} {} (expires {})", i.status, pj_to_string(&i.status_text), i.expires as i32);
        });
    });
}

unsafe extern "C" fn on_incoming_call(_acc_id: pjsua_acc_id, call_id: pjsua_call_id, _rdata: *mut pjsip_rx_data) {
    guard(|| unsafe {
        let from = call_info(call_id, |ci| pj_to_string(&ci.remote_info)).unwrap_or_default();
        println!("[call] incoming from {from}");
        if AUTO_ANSWER.load(Ordering::Relaxed) {
            pj_default!(opt: pjsua_call_setting = pjsua_call_setting_default);
            opt.vid_cnt = 0;
            pjsua_call_answer2(call_id, &opt, 200, ptr::null(), ptr::null());
        } else {
            pjsua_call_answer(call_id, 180, ptr::null(), ptr::null());
        }
    });
}

unsafe extern "C" fn on_call_state(call_id: pjsua_call_id, _e: *mut pjsip_event) {
    guard(|| {
        call_info(call_id, |ci| {
            println!(
                "[call] {} (last status {} {})",
                pj_to_string(&ci.state_text),
                ci.last_status,
                pj_to_string(&ci.last_status_text)
            );
        });
    });
}

unsafe extern "C" fn on_call_media_state(call_id: pjsua_call_id) {
    guard(|| unsafe {
        let slot = call_info(call_id, |ci| (ci.media_status == PJSUA_CALL_MEDIA_ACTIVE).then_some(ci.conf_slot));
        if let Some(slot) = slot.flatten() {
            pjsua_conf_connect(slot, 0);
            pjsua_conf_connect(0, slot);
            println!("[call] audio connected");
        }
    });
}

// ---------------------------------------------------------------------------
// Helpers

/// Keeps C strings alive while pjsua holds pointers to them.
#[derive(Default)]
struct Arena(Vec<CString>);

impl Arena {
    fn pj(&mut self, s: &str) -> pj_str_t {
        let c = CString::new(s).expect("string contains NUL");
        let out = pj_str_t { ptr: c.as_ptr().cast_mut(), slen: c.as_bytes().len() as pj_ssize_t };
        self.0.push(c);
        out
    }
}

// pjsua_acc_info / pjsua_call_info keep their strings in an internal `buf_`,
// so they are lent to a closure instead of being moved out (which would
// leave the pj_str_t fields pointing at the old location).

fn acc_info<R>(acc_id: pjsua_acc_id, f: impl FnOnce(&pjsua_acc_info) -> R) -> Option<R> {
    let mut info = MaybeUninit::<pjsua_acc_info>::zeroed();
    unsafe { (pjsua_acc_get_info(acc_id, info.as_mut_ptr()) == 0).then(|| f(info.assume_init_ref())) }
}

fn call_info<R>(call_id: pjsua_call_id, f: impl FnOnce(&pjsua_call_info) -> R) -> Option<R> {
    let mut info = MaybeUninit::<pjsua_call_info>::zeroed();
    unsafe { (pjsua_call_get_info(call_id, info.as_mut_ptr()) == 0).then(|| f(info.assume_init_ref())) }
}

fn is_registered(info: &pjsua_acc_info) -> bool {
    info.status / 100 == 2 && info.expires > 0 && info.expires != u32::MAX
}

fn wait_until(timeout: Duration, stop: &AtomicBool, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() && !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        sleep(POLL);
    }
}

/// Media clock rate: the highest codec clock rate (Opus counts as 24000).
fn max_clock_rate(codecs: &str) -> c_uint {
    codecs
        .split_whitespace()
        .map(|c| {
            let mut parts = c.split('/');
            let name = parts.next().unwrap_or_default();
            if name.eq_ignore_ascii_case("opus") {
                24000
            } else {
                parts.next().and_then(|r| r.parse().ok()).unwrap_or(8000)
            }
        })
        .fold(8000, c_uint::max)
}

fn check(status: pj_status_t, what: &str) -> Result<()> {
    if status == PJ_SUCCESS {
        Ok(())
    } else {
        bail!("{what} failed: {}", error_text(status))
    }
}

fn error_text(status: pj_status_t) -> String {
    let mut buf = [0 as c_char; 256];
    let s = unsafe { pj_strerror(status, buf.as_mut_ptr(), buf.len()) };
    format!("{} ({status})", pj_to_string(&s))
}

fn pj_to_string(s: &pj_str_t) -> String {
    if s.ptr.is_null() || s.slen <= 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.slen as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_rate_is_the_highest_codec_rate() {
        assert_eq!(max_clock_rate("AMR-WB/16000/1 AMR/8000/1"), 16000);
        assert_eq!(max_clock_rate("PCMA/8000/1 PCMU/8000/1"), 8000);
        assert_eq!(max_clock_rate("opus/48000/2 PCMA/8000/1"), 24000);
        assert_eq!(max_clock_rate(""), 8000);
    }
}
