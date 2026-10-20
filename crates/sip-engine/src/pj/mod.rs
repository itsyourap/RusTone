//! One pjsua lifetime: create, configure, run, destroy. Every function here
//! runs on the engine thread.

mod audio;
mod callbacks;
mod calls;
pub mod debug;
mod messages;
mod presence;
mod quality;
mod util;
mod vidtap;
mod video;

use std::ffi::{c_int, c_uint};

use pjsua_sys::*;

pub use callbacks::{CallOptions, install_sink, remove_sink, set_engine_tx, set_options};
pub use audio::signal_levels;
pub use calls::set_levels;
pub use quality::QualityMonitor;
pub use vidtap::set_frames as set_video_frames;
use util::{Arena, check, error_text, pj_default, pj_to_string, with_acc_info};

use crate::audio::{AudioDevice, AudioDevices, DeviceIds};
use crate::call::{CallId, CodecInfo, DtmfMethod, Sound};
use crate::config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
use crate::{Event, RegState, Sink};
use calls::ToneKey;

const MAX_STUN: usize = 8;
const MAX_NAMESERVERS: usize = 4;

pub struct Session {
    /// Strings handed to pjsua during this session.
    arena: Arena,
    udp: pjsua_transport_id,
    /// UDP transport the local account uses (5060 when free).
    udp_local: pjsua_transport_id,
    stun: bool,
    rtp_port_min: u16,
    rtp_port_max: u16,
    account: pjsua_acc_id,
    /// The main account's settings, for building call URIs.
    account_config: Option<AccountConfig>,
    local_account: pjsua_acc_id,
    player: Option<calls::Player>,
    /// No sound device at all (tests, headless use).
    null_audio: bool,
    /// Sound devices pjsua sees, the ones wanted (by name), and their indexes.
    devices: Vec<AudioDevice>,
    wanted_devices: AudioDevices,
    device_ids: DeviceIds,
    /// Numbers whose presence is wanted, and the live subscriptions; they
    /// start once the account is registered (or has no registrar).
    wanted_buddies: Vec<String>,
    buddies: presence::Buddies,
    presence_ready: bool,
    /// Video settings, the cameras pjsua sees, and the running preview's camera.
    video: crate::video::VideoConfig,
    video_devices: Vec<crate::video::VideoDevice>,
    preview: Option<i32>,
    stopped: bool,
}

impl Session {
    /// Creates and starts pjsua.
    pub fn start(cfg: &EngineConfig) -> Result<Self, String> {
        // SAFETY: called on the engine thread, which owns pjsua.
        check(unsafe { pjsua_create() }, "pjsua_create")?;
        let mut session = Self {
            arena: Arena::default(),
            udp: PJSUA_INVALID_ID,
            udp_local: PJSUA_INVALID_ID,
            stun: !cfg.stun_servers.is_empty(),
            rtp_port_min: cfg.rtp_port_min,
            rtp_port_max: cfg.rtp_port_max,
            account: PJSUA_INVALID_ID,
            account_config: None,
            local_account: PJSUA_INVALID_ID,
            player: None,
            null_audio: cfg.null_audio,
            devices: Vec::new(),
            wanted_devices: AudioDevices::default(),
            device_ids: DeviceIds::default(),
            wanted_buddies: Vec::new(),
            buddies: presence::Buddies::default(),
            presence_ready: false,
            video: cfg.video.clone(),
            video_devices: Vec::new(),
            preview: None,
            stopped: false,
        };
        match session.init(cfg) {
            Ok(()) => Ok(session),
            Err(e) => {
                session.stop();
                Err(e)
            }
        }
    }

    fn init(&mut self, cfg: &EngineConfig) -> Result<(), String> {
        let arena = &mut self.arena;

        // SAFETY: pjsua is created; the global pjsip config may be changed before init.
        unsafe { (*shim::jf_pjsip_cfg()).endpt.disable_rport = (!cfg.rport).into() };

        pj_default!(ua: pjsua_config = pjsua_config_default);
        let agent = if cfg.user_agent.is_empty() {
            format!("RusTone/{}", env!("CARGO_PKG_VERSION"))
        } else {
            cfg.user_agent.clone()
        };
        ua.user_agent = arena.pj(&agent);
        ua.srtp_secure_signaling = 0;
        for (slot, server) in ua.stun_srv.iter_mut().zip(cfg.stun_servers.iter().take(MAX_STUN)) {
            *slot = arena.pj(server);
        }
        ua.stun_srv_cnt = cfg.stun_servers.len().min(MAX_STUN) as c_uint;
        for (slot, ns) in ua.nameserver.iter_mut().zip(cfg.nameservers.iter().take(MAX_NAMESERVERS)) {
            *slot = arena.pj(ns);
        }
        ua.nameserver_count = cfg.nameservers.len().min(MAX_NAMESERVERS) as c_uint;
        ua.cb.on_reg_started2 = Some(callbacks::on_reg_started2);
        ua.cb.on_reg_state2 = Some(callbacks::on_reg_state2);
        ua.cb.on_incoming_call = Some(callbacks::on_incoming_call);
        ua.cb.on_call_state = Some(callbacks::on_call_state);
        ua.cb.on_call_media_state = Some(callbacks::on_call_media_state);
        ua.cb.on_call_rx_offer = Some(callbacks::on_call_rx_offer);
        ua.cb.on_dtmf_digit = Some(callbacks::on_dtmf_digit);
        ua.cb.on_call_transfer_request2 = Some(callbacks::on_call_transfer_request2);
        ua.cb.on_call_transfer_status = Some(callbacks::on_call_transfer_status);
        ua.cb.on_call_redirected = Some(callbacks::on_call_redirected);
        ua.cb.on_buddy_state = Some(presence::on_buddy_state);
        ua.cb.on_pager2 = Some(messages::on_pager2);
        ua.cb.on_pager_status2 = Some(messages::on_pager_status2);

        pj_default!(log: pjsua_logging_config = pjsua_logging_config_default);
        log.cb = Some(callbacks::on_log);
        // Everything pjsua logs reaches the callback (for the debug window).
        log.console_level = 5;
        log.msg_logging = PJ_TRUE;
        match &cfg.log_file {
            Some(path) => {
                // The full log, SIP messages included, only when asked for
                // (it holds numbers and addresses).
                log.level = 5;
                log.log_filename = arena.pj(&path.to_string_lossy());
            }
            None => log.level = 3,
        }
        debug::set_base_log_level(log.level as c_int);

        pj_default!(media: pjsua_media_config = pjsua_media_config_default);
        media.no_vad = (!cfg.vad).into();
        media.ec_tail_len = cfg.ec_tail_ms();
        media.enable_ice = PJ_FALSE;
        media.clock_rate = cfg.clock_rate();
        media.channel_count = cfg.channel_count();

        // SAFETY: the configs and the arena's strings outlive the call.
        check(unsafe { pjsua_init(&ua, &log, &media) }, "pjsua_init")?;
        debug::install_message_tap();
        self.create_transports(cfg);
        check(unsafe { pjsua_start() }, "pjsua_start")?;
        if cfg.null_audio {
            // SAFETY: pjsua is running.
            check(unsafe { pjsua_set_null_snd_dev() }, "selecting no sound device")?;
        }
        self.set_audio_codecs(&cfg.audio_codecs);
        video::apply_codecs(&cfg.video);
        calls::set_accept_video(cfg.video.enabled);
        self.video_devices = video::devices();
        for d in &self.video_devices {
            log::info!("video device {}: {} ({}{})", d.index, d.name, d.driver, if d.capture { ", camera" } else { "" });
        }
        self.devices = audio::devices();
        for d in &self.devices {
            log::info!("sound device {}: {} ({}, {} in, {} out)", d.index, d.name, d.driver, d.inputs, d.outputs);
        }
        log::info!("SIP engine started");
        Ok(())
    }

    /// Transports: UDP on `source_port` (falling back to any
    /// port), and with the local account also UDP/TCP on 5060 and TLS on
    /// 5061; otherwise TCP and TLS on any port.
    fn create_transports(&mut self, cfg: &EngineConfig) {
        let udp_any = || create_transport(PJSIP_TRANSPORT_UDP, 0);
        if cfg.source_port != 0 {
            self.udp = create_transport(PJSIP_TRANSPORT_UDP, cfg.source_port).or_else(|_| udp_any()).unwrap_or(PJSUA_INVALID_ID);
            if cfg.local_account {
                self.udp_local = if cfg.source_port == 5060 {
                    self.udp
                } else {
                    create_transport(PJSIP_TRANSPORT_UDP, 5060).unwrap_or(self.udp)
                };
            }
        } else {
            if cfg.local_account {
                self.udp_local = create_transport(PJSIP_TRANSPORT_UDP, 5060).unwrap_or(PJSUA_INVALID_ID);
            }
            self.udp = udp_any().unwrap_or(PJSUA_INVALID_ID);
            if self.udp_local == PJSUA_INVALID_ID {
                self.udp_local = self.udp;
            }
        }
        for (kind, local_port) in [(PJSIP_TRANSPORT_TCP, 5060), (PJSIP_TRANSPORT_TLS, 5061)] {
            let port = if cfg.local_account { local_port } else { 0 };
            let created = create_transport(kind, port).or_else(|e| if port != 0 { create_transport(kind, 0) } else { Err(e) });
            if let Err(e) = created {
                log::warn!("{e}");
            }
        }
        if self.udp == PJSUA_INVALID_ID {
            log::warn!("no UDP transport");
        }
    }

    /// Disables every codec, then enables the configured ones with
    /// descending priority from 128.
    fn set_audio_codecs(&mut self, codecs: &[String]) {
        let mut infos: [pjsua_codec_info; 64] = unsafe { std::mem::zeroed() };
        let mut count = infos.len() as c_uint;
        // SAFETY: pjsua fills at most `count` entries.
        if unsafe { pjsua_enum_codecs(infos.as_mut_ptr(), &mut count) } == PJ_SUCCESS {
            for info in &infos[..count as usize] {
                unsafe { pjsua_codec_set_priority(&info.codec_id, 0) };
            }
        }
        let mut priority: u8 = 128;
        for name in codecs {
            let id = self.arena.pj(name);
            match unsafe { pjsua_codec_set_priority(&id, priority) } {
                PJ_SUCCESS => priority = priority.saturating_sub(1),
                status => log::warn!("codec {name} unavailable: {}", error_text(status)),
            }
        }
    }

    /// Every audio codec pjsua has, preferred first, with its priority.
    pub fn codecs(&self) -> Vec<CodecInfo> {
        let mut infos: [pjsua_codec_info; 64] = unsafe { std::mem::zeroed() };
        let mut count = infos.len() as c_uint;
        // SAFETY: pjsua fills at most `count` entries; ids point into them.
        if unsafe { pjsua_enum_codecs(infos.as_mut_ptr(), &mut count) } != PJ_SUCCESS {
            return Vec::new();
        }
        let mut codecs: Vec<CodecInfo> =
            infos[..count as usize].iter().map(|i| CodecInfo { id: pj_to_string(&i.codec_id), priority: i.priority }).collect();
        codecs.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        codecs
    }

    /// Replaces the main account.
    pub fn set_account(&mut self, account: Option<&AccountConfig>, sink: &Sink) {
        // Subscriptions belong to the old account.
        self.buddies.clear();
        self.presence_ready = false;
        if self.account != PJSUA_INVALID_ID {
            callbacks::set_main_account(PJSUA_INVALID_ID);
            // SAFETY: a valid account id from this session; deleting unregisters it.
            unsafe { pjsua_acc_del(self.account) };
            self.account = PJSUA_INVALID_ID;
        }
        self.account_config = account.cloned();
        let Some(account) = account else {
            sink(Event::Registration(RegState::Unregistered));
            return;
        };
        let registers = account.reg_uri().is_some();
        if registers {
            sink(Event::Registration(RegState::Registering));
        }
        match self.add_account(account) {
            Ok(id) => {
                self.account = id;
                if !registers {
                    sink(Event::Registration(RegState::NoRegistrar));
                    self.start_presence();
                }
            }
            Err(e) => {
                log::warn!("{e}");
                sink(Event::Registration(RegState::Failed { code: 0, reason: e, detail: None }));
            }
        }
    }

    fn add_account(&mut self, a: &AccountConfig) -> Result<pjsua_acc_id, String> {
        pj_default!(ac: pjsua_acc_config = pjsua_acc_config_default);
        self.apply_account_config(&mut ac, a);
        let arena = &mut self.arena;

        ac.cred_count = 1;
        let cred = &mut ac.cred_info[0];
        cred.username = arena.pj(a.auth_user());
        cred.realm = arena.pj("*");
        cred.scheme = arena.pj("Digest");
        cred.data_type = PJSIP_CRED_DATA_PLAIN_PASSWD as c_int;
        cred.data = arena.pj(&a.password);

        let proxies = a.proxy_uris();
        for (slot, uri) in ac.proxy.iter_mut().zip(&proxies) {
            *slot = arena.pj(uri);
        }
        ac.proxy_cnt = proxies.len().min(ac.proxy.len()) as c_uint;

        ac.id = arena.pj(&a.id_uri());
        match a.reg_uri() {
            Some(uri) => ac.reg_uri = arena.pj(&uri),
            None => ac.register_on_acc_add = PJ_FALSE,
        }

        let mut id = PJSUA_INVALID_ID;
        // Set before adding: the first registration callback can arrive
        // before pjsua_acc_add returns.
        callbacks::set_main_account(PJSUA_INVALID_ID);
        // SAFETY: `ac` was initialised in place and its strings live in the arena.
        let status = unsafe { pjsua_acc_add(&ac, PJ_TRUE, &mut id) };
        check(status, "adding the account")?;
        callbacks::set_main_account(id);
        // A registration that already finished during pjsua_acc_add was
        // ignored above; report it now.
        if let Some(state @ (RegState::Registered { .. } | RegState::Failed { .. })) = with_acc_info(id, current_reg_state) {
            callbacks::emit(Event::Registration(state));
        }
        Ok(id)
    }

    /// The account settings shared by the main and local accounts.
    fn apply_account_config(&mut self, ac: &mut pjsua_acc_config, a: &AccountConfig) {
        ac.ka_interval = a.keep_alive;
        ac.reg_timeout = a.register_refresh;
        if a.disable_session_timer {
            ac.use_timer = PJSUA_SIP_TIMER_INACTIVE;
        }
        if self.rtp_port_min > 0 {
            ac.rtp_cfg.port = c_uint::from(self.rtp_port_min);
            if self.rtp_port_max > self.rtp_port_min {
                ac.rtp_cfg.port_range = c_uint::from(self.rtp_port_max - self.rtp_port_min);
            }
        }
        match a.srtp {
            Srtp::Disabled => ac.use_srtp = PJMEDIA_SRTP_DISABLED,
            Srtp::Optional => ac.use_srtp = PJMEDIA_SRTP_OPTIONAL,
            Srtp::Mandatory => ac.use_srtp = PJMEDIA_SRTP_MANDATORY,
            Srtp::Dtls | Srtp::DtlsSdes => {
                ac.use_srtp = PJMEDIA_SRTP_MANDATORY;
                ac.enable_rtcp_mux = PJ_TRUE;
                ac.srtp_opt.keying[0] = PJMEDIA_SRTP_KEYING_DTLS_SRTP;
                ac.srtp_opt.keying_count = 1;
                if a.srtp == Srtp::DtlsSdes {
                    ac.srtp_opt.keying[1] = PJMEDIA_SRTP_KEYING_SDES;
                    ac.srtp_opt.keying_count = 2;
                }
            }
        }
        if !self.stun && !a.public_addr.is_empty() {
            ac.rtp_cfg.public_addr = self.arena.pj(&a.public_addr);
        }
        ac.ice_cfg_use = PJSUA_ICE_CONFIG_USE_CUSTOM;
        ac.ice_cfg.enable_ice = a.ice.into();
        ac.allow_via_rewrite = a.allow_rewrite.into();
        ac.allow_sdp_nat_rewrite = a.allow_rewrite.into();
        ac.allow_contact_rewrite = if a.allow_rewrite { 2 } else { 0 };
        ac.contact_rewrite_method = (PJSUA_CONTACT_REWRITE_ALWAYS_UPDATE | PJSUA_CONTACT_REWRITE_UNREGISTER) as c_int;
        ac.publish_enabled = a.publish.into();
        // Video: show and send automatically, with the chosen camera.
        if self.video.enabled {
            // RusTone draws remote video itself; pjsua's window stays hidden.
            ac.vid_in_auto_show = PJ_FALSE;
            ac.vid_out_auto_transmit = PJ_TRUE;
            ac.vid_cap_dev = self.camera();
            ac.vid_wnd_flags = video::WND_BORDER | video::WND_RESIZABLE;
        }
        ac.mwi_enabled = a.mwi.into();
        if a.transport == Transport::Udp && self.udp != PJSUA_INVALID_ID {
            ac.transport_id = self.udp;
        }
    }

    /// Replaces the local "call by IP" account.
    pub fn set_local_account(&mut self, local: Option<&LocalAccountConfig>) {
        if self.local_account != PJSUA_INVALID_ID {
            // SAFETY: valid id from this session.
            unsafe { pjsua_acc_del(self.local_account) };
            self.local_account = PJSUA_INVALID_ID;
        }
        let Some(local) = local else { return };
        if self.udp_local == PJSUA_INVALID_ID {
            log::warn!("local account needs the local-account transports; enable it in the engine config");
            return;
        }
        let host = if local.domain.is_empty() { self.transport_host(self.udp_local) } else { local.domain.clone() };
        let mut uri = if local.username.is_empty() { format!("<sip:{host}>") } else { format!("<sip:{}@{host}>", local.username) };
        if !local.display_name.is_empty() {
            uri = format!("\"{}\" {uri}", local.display_name);
        }

        pj_default!(ac: pjsua_acc_config = pjsua_acc_config_default);
        // The local account is configured like any other, without
        // STUN, on the local UDP transport, at a lower priority.
        let base = AccountConfig::default();
        self.apply_account_config(&mut ac, &base);
        ac.sip_stun_use = PJSUA_STUN_USE_DISABLED;
        ac.media_stun_use = PJSUA_STUN_USE_DISABLED;
        ac.transport_id = self.udp_local;
        ac.priority -= 1;
        ac.register_on_acc_add = PJ_FALSE;
        ac.id = self.arena.pj(&uri);

        let mut id = PJSUA_INVALID_ID;
        // SAFETY: as in `add_account`.
        match check(unsafe { pjsua_acc_add(&ac, PJ_TRUE, &mut id) }, "adding the local account") {
            Ok(()) => self.local_account = id,
            Err(e) => log::warn!("{e}"),
        }
    }

    fn transport_host(&self, id: pjsua_transport_id) -> String {
        let mut info = std::mem::MaybeUninit::<pjsua_transport_info>::zeroed();
        // SAFETY: filled on success and read in place.
        unsafe {
            if pjsua_transport_get_info(id, info.as_mut_ptr()) == PJ_SUCCESS {
                return pj_to_string(&info.assume_init_ref().local_name.host);
            }
        }
        String::from("127.0.0.1")
    }

    pub fn reregister(&self) {
        if self.account != PJSUA_INVALID_ID {
            // SAFETY: valid id from this session.
            unsafe { pjsua_acc_set_registration(self.account, PJ_TRUE) };
        }
    }

    /// Re-creates transports and re-registers after an address change.
    pub fn ip_changed(&self) {
        pj_default!(param: pjsua_ip_change_param = pjsua_ip_change_param_default);
        // SAFETY: on the engine thread with pjsua running.
        if let Err(e) = check(unsafe { pjsua_handle_ip_change(&param) }, "handling the network change") {
            log::warn!("{e}");
        }
    }

    // -----------------------------------------------------------------------
    // Video

    pub fn video_devices(&self) -> &[crate::video::VideoDevice] {
        &self.video_devices
    }

    fn camera(&self) -> i32 {
        crate::video::camera_index(&self.video.camera, &self.video_devices)
    }

    /// Pauses or resumes sending our camera during a call.
    pub fn set_video_sending(&mut self, call: CallId, on: bool) -> Result<(), String> {
        if on {
            // The picture-in-picture shows what we send.
            self.set_preview(true);
        }
        calls::set_camera(call, on)?;
        if let Some(info) = calls::call_info(call) {
            callbacks::emit(crate::Event::Call(info));
        }
        Ok(())
    }

    /// Adds video to a call that has none (a re-INVITE). A video line the
    /// answer rejected is turned on in place: a second `m=video` line makes
    /// some networks drop the call (answering 604).
    pub fn add_video(&mut self, call: CallId) -> Result<(), String> {
        if !self.video.enabled {
            return Err("video calls are off".into());
        }
        pj_default!(param: pjsua_call_vid_strm_op_param = pjsua_call_vid_strm_op_param_default);
        param.dir = PJMEDIA_DIR_ENCODING_DECODING;
        param.cap_dev = self.camera();
        let op = match calls::unused_video_index(call) {
            Some(index) => {
                param.med_idx = index as c_int;
                PJSUA_CALL_VID_STRM_CHANGE_DIR
            }
            None => PJSUA_CALL_VID_STRM_ADD,
        };
        self.set_preview(true);
        calls::adding_video(call);
        // SAFETY: `param` initialised in place.
        check(unsafe { pjsua_call_set_vid_strm(call, op, &param) }, "adding video")
    }

    /// Shows or hides the local preview of the chosen camera.
    pub fn set_preview(&mut self, on: bool) {
        match (on, self.preview) {
            (true, None) if self.video.enabled => {
                let camera = self.camera();
                if video::start_preview(camera) {
                    self.preview = Some(camera);
                }
            }
            (false, Some(camera)) => {
                video::stop_preview(camera);
                self.preview = None;
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Presence

    /// The numbers to watch (contacts with presence on).
    pub fn set_buddies(&mut self, numbers: &[String]) {
        self.wanted_buddies = numbers.to_vec();
        if self.presence_ready {
            self.sync_buddies();
        }
    }

    /// The account is registered: subscribe (once until the account changes).
    pub fn start_presence(&mut self) {
        if !std::mem::replace(&mut self.presence_ready, true) {
            self.sync_buddies();
        }
    }

    fn sync_buddies(&mut self) {
        let wanted: Vec<(String, String)> = self
            .wanted_buddies
            .iter()
            .filter_map(|number| {
                // Formatted without the dial plan.
                // pjsua_buddy_add wants a bare URI, not `<name-addr>`.
                let uri = self.target_uri(number, false).ok()?;
                let uri = uri.trim().trim_start_matches('<').trim_end_matches('>').to_owned();
                Some((number.clone(), uri))
            })
            .collect();
        self.buddies.sync(&wanted, self.account, &mut self.arena);
    }

    // -----------------------------------------------------------------------
    // Sound devices

    /// The sound devices pjsua sees.
    pub fn audio_devices(&self) -> &[AudioDevice] {
        &self.devices
    }

    /// Picks the devices to use. Applies at once when a device is open:
    /// the ring device stays in use if it was, otherwise the speaker.
    pub fn set_audio_devices(&mut self, wanted: &AudioDevices) {
        let old = self.device_ids;
        self.wanted_devices = wanted.clone();
        self.device_ids = DeviceIds::resolve(wanted, &self.devices);
        log::info!("sound devices wanted {wanted:?} -> {:?}", self.device_ids);
        if self.null_audio || self.device_ids == old {
            return;
        }
        let ringing = audio::current().is_some_and(|(_, playback)| ringing(old, playback));
        self.reapply_devices(audio::current().is_some(), ringing);
    }

    /// Re-reads the device list after Windows reported a change, keeping
    /// the open device in use:
    /// close it, refresh, then open the wanted devices again, falling back
    /// to the default for any that went away.
    pub fn refresh_devices(&mut self) {
        if self.null_audio {
            return;
        }
        let active = audio::current();
        let was_ringing = active.is_some_and(|(_, playback)| ringing(self.device_ids, playback));
        // SAFETY: on the engine thread with pjsua running.
        unsafe {
            if active.is_some() {
                pjsua_set_null_snd_dev();
            }
            pjmedia_aud_dev_refresh();
        }
        self.devices = audio::devices();
        self.device_ids = DeviceIds::resolve(&self.wanted_devices, &self.devices);
        log::info!("sound devices changed: {}", self.devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", "));
        self.reapply_devices(active.is_some(), was_ringing);
    }

    /// Opens the resolved devices again if one was open (the ring device if
    /// that was playing); otherwise only selects them, since indexes may
    /// have changed.
    fn reapply_devices(&self, was_open: bool, was_ringing: bool) {
        let ids = self.device_ids;
        if was_open {
            audio::open(ids.input, if was_ringing { ids.ring } else { ids.output }, true);
        } else {
            // Whatever opens the device next (early media, the debug
            // window's taps) gets these.
            audio::select(ids.input, ids.output);
        }
    }

    /// Switches to the call devices (before calling,
    /// answering and local tones).
    fn use_call_devices(&self) {
        if !self.null_audio {
            audio::open(self.device_ids.input, self.device_ids.output, false);
        }
    }

    // -----------------------------------------------------------------------
    // Calls

    /// The URI to call for `target`: SIP URIs as given; numbers through the
    /// main account's rules, or straight to the host
    /// for the local account.
    fn target_uri(&self, target: &str, local: bool) -> Result<String, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("nothing to call".into());
        }
        let lower = target.to_ascii_lowercase();
        if lower.starts_with("sip:") || lower.starts_with("sips:") || target.starts_with('<') {
            return Ok(target.to_owned());
        }
        if local {
            return Ok(format!("<sip:{target}>"));
        }
        match &self.account_config {
            Some(account) => Ok(account.sip_uri(target)),
            None => Err("no account to call from".into()),
        }
    }

    pub fn make_call(&mut self, target: &str, video: bool, local: bool) -> Result<CallId, String> {
        let acc = if local { self.local_account } else { self.account };
        if acc == PJSUA_INVALID_ID {
            return Err(if local { "the local account is off".into() } else { "no account to call from".into() });
        }
        let uri = self.target_uri(target, local)?;
        self.use_call_devices();
        let video = video && self.video.enabled;
        if video {
            // The preview shows before the INVITE goes out.
            self.set_preview(true);
        }
        pj_default!(opt: pjsua_call_setting = pjsua_call_setting_default);
        opt.vid_cnt = c_uint::from(video);
        // pjsua 2.17 offers RFC 4103 real-time text by default; RusTone's
        // offers have none.
        opt.txt_cnt = 0;
        let dst = self.arena.pj(&uri);
        let mut id = PJSUA_INVALID_ID;
        // SAFETY: `opt` initialised in place; `dst` lives in the arena.
        check(unsafe { pjsua_call_make_call(acc, &dst, &opt, std::ptr::null_mut(), std::ptr::null(), &mut id) }, "calling")?;
        Ok(id)
    }

    /// Sends a provisional or final response to an incoming call (180, 200,
    /// 486...). 200 answers with audio, plus video when asked.
    pub fn answer(&mut self, call: CallId, code: u16, video: bool) -> Result<(), String> {
        let video = video && self.video.enabled;
        if code == 200 {
            self.use_call_devices();
            if video {
                self.set_preview(true);
            }
        }
        pj_default!(opt: pjsua_call_setting = pjsua_call_setting_default);
        opt.vid_cnt = c_uint::from(video);
        // pjsua 2.17 offers RFC 4103 real-time text by default; RusTone's
        // offers have none.
        opt.txt_cnt = 0;
        // pjsua takes the call settings from the first answer that carries
        // them and ignores later changes, so ringing (1xx) carries none and
        // the real answer decides audio or video.
        let opt_ptr: *const pjsua_call_setting = if code >= 200 { &opt } else { std::ptr::null() };
        // SAFETY: `opt` initialised in place and outlives the call.
        check(unsafe { pjsua_call_answer2(call, opt_ptr, c_uint::from(code), std::ptr::null(), std::ptr::null()) }, "answering")
    }

    /// A final response to an incoming call with a reason phrase and extra
    /// headers.
    pub fn respond(&mut self, call: CallId, code: u16, reason: &str, headers: &[(String, String)]) -> Result<(), String> {
        let msg = util::MsgData::new(headers);
        let reason_c = std::ffi::CString::new(reason).unwrap_or_default();
        let reason_pj = pj_str_t { ptr: reason_c.as_ptr().cast_mut(), slen: reason_c.as_bytes().len() as pj_ssize_t };
        let reason_ptr: *const pj_str_t = if reason.is_empty() { std::ptr::null() } else { &reason_pj };
        // SAFETY: `msg` and the reason live until the call returns.
        check(unsafe { pjsua_call_answer2(call, std::ptr::null(), c_uint::from(code), reason_ptr, msg.as_ptr()) }, "responding to the call")
    }

    /// Ends a call with `code` and extra headers.
    pub fn end(&self, call: CallId, code: u16, headers: &[(String, String)]) {
        let msg = util::MsgData::new(headers);
        // SAFETY: `msg` lives until the call returns; an unknown id just fails.
        unsafe { pjsua_call_hangup(call, c_uint::from(code), std::ptr::null(), msg.as_ptr()) };
    }

    /// Ends a call; with code 0 pjsua picks CANCEL, BYE or 603.
    pub fn hangup(&self, call: CallId, code: u16) {
        // SAFETY: plain pjsua call; an unknown id just fails.
        unsafe { pjsua_call_hangup(call, c_uint::from(code), std::ptr::null(), std::ptr::null()) };
    }

    pub fn hangup_all(&self) {
        // SAFETY: plain pjsua call.
        unsafe { pjsua_call_hangup_all() };
    }

    pub fn hold(&self, call: CallId) -> Result<(), String> {
        // SAFETY: plain pjsua call.
        check(unsafe { pjsua_call_set_hold(call, std::ptr::null()) }, "holding the call")
    }

    pub fn unhold(&self, call: CallId) -> Result<(), String> {
        // SAFETY: plain pjsua call.
        check(unsafe { pjsua_call_reinvite(call, PJSUA_CALL_UNHOLD as c_uint, std::ptr::null()) }, "resuming the call")
    }

    /// Sends DTMF; plays the
    /// digits locally when they couldn't be sent and local DTMF is on.
    pub fn send_dtmf(&mut self, call: CallId, digits: &str, method: DtmfMethod) -> Result<(), String> {
        let active = calls::call_info(call).is_some_and(|i| i.media == crate::MediaState::Active);
        let sent = active
            && match method {
                DtmfMethod::InBand => calls::play_digits(ToneKey::Call(call), digits),
                DtmfMethod::SipInfo => calls::send_dtmf_info(call, digits),
                DtmfMethod::Rfc2833 => self.dial_rfc2833(call, digits).is_ok(),
                DtmfMethod::Auto => self.dial_rfc2833(call, digits).is_ok() || calls::play_digits(ToneKey::Call(call), digits),
            };
        if !sent && callbacks::options().local_dtmf {
            calls::play_digits(ToneKey::Local, digits);
        }
        if sent { Ok(()) } else { Err("the digits couldn't be sent".into()) }
    }

    fn dial_rfc2833(&mut self, call: CallId, digits: &str) -> Result<(), String> {
        let d = self.arena.pj(digits);
        // SAFETY: `d` lives in the arena.
        check(unsafe { pjsua_call_dial_dtmf(call, &d) }, "sending DTMF")
    }

    /// Key-press feedback on the local speaker (dialer keypad).
    pub fn play_local_digits(&self, digits: &str) {
        if callbacks::options().local_dtmf {
            self.use_call_devices();
            calls::play_digits(ToneKey::Local, digits);
        }
    }

    pub fn set_mute(&self, mic: bool, speaker: bool) {
        calls::set_mute(mic, speaker);
    }

    /// Sends a text message from the main account, or from the local account
    /// when `local`.
    pub fn send_message(&mut self, id: crate::MessageId, target: &str, text: &str, local: bool) -> Result<(), String> {
        let acc = if local { self.local_account } else { self.account };
        if acc == PJSUA_INVALID_ID {
            return Err(if local { "the local account is off".into() } else { "no account to send from".into() });
        }
        let uri = self.target_uri(target, local)?;
        let uri = uri.trim().trim_start_matches('<').trim_end_matches('>');
        messages::send(acc, uri, text, id)
    }

    /// Blind transfer (REFER) to `target`.
    pub fn transfer(&mut self, call: CallId, target: &str) -> Result<(), String> {
        let uri = self.target_uri(target, false)?;
        let dest = self.arena.pj(&uri);
        // SAFETY: `dest` lives in the arena.
        check(unsafe { pjsua_call_xfer(call, &dest, std::ptr::null()) }, "transferring the call")
    }

    /// Attended transfer: connects `call`'s party with `other`'s.
    pub fn attended_transfer(&self, call: CallId, other: CallId) -> Result<(), String> {
        // SAFETY: plain pjsua call (options 0).
        check(unsafe { pjsua_call_xfer_replaces(call, other, 0, std::ptr::null()) }, "transferring the call")
    }

    // -----------------------------------------------------------------------
    // Sounds

    pub fn play_sound(&mut self, sound: &Sound) -> Result<(), String> {
        self.stop_sound();
        // The ring device only while no call has audio.
        if !self.null_audio && !calls::any_call_audio() {
            let output = if sound.ring { self.device_ids.ring } else { self.device_ids.output };
            audio::open(self.device_ids.input, output, false);
        }
        self.player = Some(calls::Player::start(&sound.path, sound.looping, sound.level, callbacks::on_player_eof)?);
        Ok(())
    }

    pub fn stop_sound(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }

    /// A non-looping sound reached its end.
    pub fn sound_ended(&mut self, player_id: pjsua_player_id) {
        if self.player.as_ref().is_some_and(|p| p.id() == player_id) {
            self.stop_sound();
        }
    }

    /// Destroys pjsua. Accounts are unregistered first (pjsua waits briefly).
    pub fn stop(mut self) {
        self.destroy();
    }

    fn destroy(&mut self) {
        if std::mem::replace(&mut self.stopped, true) {
            return;
        }
        callbacks::set_main_account(PJSUA_INVALID_ID);
        self.set_preview(false);
        vidtap::clear();
        self.buddies.clear();
        self.stop_sound();
        calls::drop_all_tonegens();
        debug::remove_taps();
        debug::remove_message_tap();
        // SAFETY: on the engine thread; pjsua was created by `start`.
        unsafe { pjsua_destroy() };
        log::info!("SIP engine stopped");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Whether `playback` is the ring device in use for ringing (no call audio).
fn ringing(ids: DeviceIds, playback: i32) -> bool {
    !calls::any_call_audio() && playback == ids.ring && ids.ring != ids.output
}

fn create_transport(kind: pjsip_transport_type_e, port: u16) -> Result<pjsua_transport_id, String> {
    pj_default!(tc: pjsua_transport_config = pjsua_transport_config_default);
    tc.port = c_uint::from(port);
    let mut id = PJSUA_INVALID_ID;
    // SAFETY: `tc` initialised in place.
    check(unsafe { pjsua_transport_create(kind, &tc, &mut id) }, &format!("transport {kind} on port {port}"))?;
    Ok(id)
}

/// The account's registration as pjsua currently sees it.
fn current_reg_state(info: &pjsua_acc_info) -> RegState {
    let code = u16::try_from(info.status).unwrap_or(0);
    if info.has_registration == 0 {
        RegState::NoRegistrar
    } else if (200..300).contains(&code) && info.expires > 0 && info.expires != u32::MAX {
        RegState::Registered { expires: info.expires }
    } else if code >= 300 && code != 401 && code != 407 {
        RegState::Failed { code, reason: pj_to_string(&info.status_text), detail: None }
    } else if code == 0 {
        RegState::Unregistered
    } else {
        RegState::Registering
    }
}
