//! One pjsua lifetime: create, configure, run, destroy. Every function here
//! runs on the engine thread.

mod callbacks;
mod util;

use std::ffi::{c_int, c_uint};

use pjsua_sys::*;

pub use callbacks::{install_sink, remove_sink};
use util::{Arena, check, error_text, pj_default, pj_to_string, with_acc_info};

use crate::config::{AccountConfig, EngineConfig, LocalAccountConfig, Srtp, Transport};
use crate::{Event, RegState, Sink};

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
    local_account: pjsua_acc_id,
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
            local_account: PJSUA_INVALID_ID,
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

        pj_default!(log: pjsua_logging_config = pjsua_logging_config_default);
        log.cb = Some(callbacks::on_log);
        log.console_level = 3;
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

        pj_default!(media: pjsua_media_config = pjsua_media_config_default);
        media.no_vad = (!cfg.vad).into();
        media.ec_tail_len = cfg.ec_tail_ms();
        media.enable_ice = PJ_FALSE;
        media.clock_rate = cfg.clock_rate();
        media.channel_count = cfg.channel_count();

        // SAFETY: the configs and the arena's strings outlive the call.
        check(unsafe { pjsua_init(&ua, &log, &media) }, "pjsua_init")?;
        self.create_transports(cfg);
        check(unsafe { pjsua_start() }, "pjsua_start")?;
        self.set_audio_codecs(&cfg.audio_codecs);
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

    /// Replaces the main account.
    pub fn set_account(&mut self, account: Option<&AccountConfig>, sink: &Sink) {
        if self.account != PJSUA_INVALID_ID {
            callbacks::set_main_account(PJSUA_INVALID_ID);
            // SAFETY: a valid account id from this session; deleting unregisters it.
            unsafe { pjsua_acc_del(self.account) };
            self.account = PJSUA_INVALID_ID;
        }
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

    /// Destroys pjsua. Accounts are unregistered first (pjsua waits briefly).
    pub fn stop(mut self) {
        self.destroy();
    }

    fn destroy(&mut self) {
        if std::mem::replace(&mut self.stopped, true) {
            return;
        }
        callbacks::set_main_account(PJSUA_INVALID_ID);
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
