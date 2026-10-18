//! Overview: engine and registration state, the account, calls, settings.

use egui::Ui;

use super::{header, kv, panel_card};
use crate::app::App;
use crate::sip::EngineState;
use crate::theme::Palette;

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    header(ui, p, "Overview", |_| {});
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.columns(2, |cols| {
            panel_card(&mut cols[0], p, "Engine", |ui| {
                egui::Grid::new("engine").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    let state = match &app.sip.engine_state {
                        EngineState::Starting => "starting".to_owned(),
                        EngineState::Running => "running".to_owned(),
                        EngineState::Stopped => "stopped".to_owned(),
                        EngineState::Failed(e) => format!("failed: {e}"),
                    };
                    kv(ui, p, "pjsua", state);
                    kv(ui, p, "Status", format!("{:?}", app.status()));
                    kv(ui, p, "Registration", format!("{:?}", app.sip.reg));
                    kv(ui, p, "Debug collection", if app.sip.debug_feed().is_enabled() { "on" } else { "off" });
                    kv(ui, p, "RusTone", env!("CARGO_PKG_VERSION"));
                });
            });
            cols[0].add_space(10.0);
            panel_card(&mut cols[0], p, "Account", |ui| match app.settings.active() {
                Some(a) => {
                    egui::Grid::new("account").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                        kv(ui, p, "Name", a.title());
                        kv(ui, p, "Registrar", &a.server);
                        kv(ui, p, "Proxy", &a.proxy);
                        kv(ui, p, "Domain", &a.domain);
                        kv(ui, p, "Username", &a.username);
                        kv(ui, p, "Transport", format!("{:?}", a.transport));
                        kv(ui, p, "SRTP", format!("{:?}", a.srtp));
                        kv(ui, p, "Register refresh", format!("{} s", a.register_refresh));
                        kv(ui, p, "Keep-alive", format!("{} s", a.keep_alive));
                        kv(ui, p, "ICE / rewrite", format!("{} / {}", a.ice, a.allow_rewrite));
                    });
                }
                None => {
                    ui.label(egui::RichText::new("No account").color(p.dim));
                }
            });

            panel_card(&mut cols[1], p, "Calls", |ui| {
                let calls: Vec<_> = app.sip.calls.visible().cloned().collect();
                if calls.is_empty() {
                    ui.label(egui::RichText::new("No calls").color(p.dim));
                }
                for c in calls {
                    egui::Grid::new(("call", c.id())).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                        kv(ui, p, "Call", format!("#{} {:?}", c.id(), c.info.direction));
                        kv(ui, p, "Remote", &c.info.remote);
                        kv(ui, p, "State", format!("{:?} / media {:?}", c.info.state, c.info.media));
                        kv(ui, p, "Last status", format!("{} {}", c.info.status_code, c.info.status_text));
                        kv(ui, p, "Call-ID", &c.info.sip_call_id);
                        if let Some(id) = &c.info.caller_id {
                            kv(ui, p, "Caller ID", id);
                        }
                        if let Some(ua) = &c.info.user_agent {
                            kv(ui, p, "User-Agent", ua);
                        }
                    });
                    ui.separator();
                }
            });
            cols[1].add_space(10.0);
            panel_card(&mut cols[1], p, "Media & network settings", |ui| {
                let s = &app.settings.sip;
                egui::Grid::new("settings").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    kv(ui, p, "Audio codecs", &s.audio_codecs);
                    kv(ui, p, "EC / VAD", format!("{} / {}", s.ec, s.vad));
                    kv(ui, p, "rport", s.rport.to_string());
                    kv(ui, p, "SIP port", if s.source_port == 0 { "any".to_owned() } else { s.source_port.to_string() });
                    kv(ui, p, "RTP ports", format!("{}–{}", s.rtp_port_min, s.rtp_port_max));
                    kv(ui, p, "STUN", if s.enable_stun { s.stun.clone() } else { "off".into() });
                    kv(ui, p, "DTMF", format!("{:?}", app.settings.calls.dtmf_method));
                    kv(ui, p, "Local account", app.settings.local_account.enabled.to_string());
                    kv(ui, p, "SIP log file", s.enable_log.to_string());
                });
            });
            cols[1].add_space(10.0);
            panel_card(&mut cols[1], p, "Codecs in the engine", |ui| {
                if app.sip.codecs.is_empty() {
                    ui.label(egui::RichText::new("Engine not started").color(p.dim));
                }
                egui::Grid::new("codecs").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                    for c in &app.sip.codecs {
                        let state = if c.priority == 0 { "disabled".to_owned() } else { format!("priority {}", c.priority) };
                        kv(ui, p, &c.id, state);
                    }
                });
            });
        });
    });
}
