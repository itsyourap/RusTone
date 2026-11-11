//! DNS lookup: the active account's server found the way RFC 3263
//! describes (NAPTR, SRV, then AAAA and A records), with the DNS servers
//! in Settings or the system's, to see what pjsua will connect to.

use egui::Ui;
use rustone_core::account::{IpVersion, Transport};

use super::{header, kv, mono, panel_card};
use crate::app::App;
use crate::dns_discovery::Discovery;
use crate::theme::Palette;

/// The last lookup, and the one running.
#[derive(Debug, Default)]
pub struct DnsState {
    running: Option<std::sync::mpsc::Receiver<Discovery>>,
    /// The host looked up and what was found.
    result: Option<(String, Discovery)>,
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    let account = app.settings.active().map(|a| {
        let server = if a.server.trim().is_empty() { &a.domain } else { &a.server };
        (crate::dns_discovery::host_of(server).to_owned(), a.transport, a.ip_version)
    });
    let state = &mut app.debug.dns;
    if let Some(rx) = &state.running
        && let Ok(found) = rx.try_recv()
    {
        state.running = None;
        let host = account.as_ref().map(|a| a.0.clone()).unwrap_or_default();
        state.result = Some((host, found));
    }
    let running = state.running.is_some();
    let configured = app.settings.sip.dns_servers();
    let servers = if configured.is_empty() { crate::dns::system_dns_servers() } else { configured.clone() };

    let mut start = false;
    header(ui, p, "DNS lookup", |ui| {
        let enabled = !running && account.as_ref().is_some_and(|a| !a.0.is_empty());
        start = ui.add_enabled(enabled, egui::Button::new(if running { "Looking…" } else { "Look up" })).clicked();
    });
    if start && let Some((host, transport, _)) = account.clone() {
        let ips: Vec<std::net::IpAddr> = servers.iter().filter_map(|s| s.parse().ok()).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::dns_discovery::discover(&host, transport, &ips));
            ctx.request_repaint();
        });
        app.debug.dns.running = Some(rx);
    }

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        panel_card(ui, p, "What is looked up", |ui| {
            egui::Grid::new("dns-what").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| match &account {
                Some((host, transport, version)) => {
                    kv(ui, p, "Server", if host.is_empty() { "none" } else { host });
                    kv(ui, p, "Transport", transport_label(*transport));
                    kv(ui, p, "IP version", format!("{version:?}"));
                    let from = if configured.is_empty() { "the system's" } else { "Settings" };
                    kv(ui, p, "DNS servers", if servers.is_empty() { format!("none ({from})") } else { format!("{} ({from})", servers.join(", ")) });
                    kv(ui, p, "DNS SRV", if app.settings.sip.dns_srv { "on" } else { "off" });
                }
                None => kv(ui, p, "Server", "no account"),
            });
            ui.label(egui::RichText::new("NAPTR records name the transports the domain offers, SRV records the servers for the account's transport, AAAA and A records their addresses. pjsua looks up SRV, AAAA and A when it connects, but never NAPTR.").size(12.0).color(p.dim));
        });
        ui.add_space(10.0);
        // pjsua's Contact takes the preferred IP version, not the server's.
        let prefers_ipv4 = account.as_ref().is_some_and(|a| matches!(a.2, IpVersion::Ipv4 | IpVersion::PreferIpv4));
        match &app.debug.dns.result {
            Some((host, found)) => results(ui, p, host, found, prefers_ipv4),
            None if running => {
                ui.label(egui::RichText::new("Looking up…").color(p.dim));
            }
            None => {
                ui.label(egui::RichText::new("Press Look up to find the account's servers.").color(p.dim));
            }
        }
    });
    if running {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

fn transport_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Udp => "UDP",
        Transport::Tcp => "TCP",
        Transport::Tls => "TLS",
    }
}

/// What a lookup found, query by query; `prefers_ipv4` is whether the
/// account's IP version puts IPv4 first (or uses it only).
fn results(ui: &mut Ui, p: &Palette, host: &str, found: &Discovery, prefers_ipv4: bool) {
    let line = |ui: &mut Ui, text: String, color| {
        ui.label(egui::RichText::new(text).font(mono(12.5)).color(color));
    };
    let mut notes = found.notes.clone();
    if prefers_ipv4 && !found.endpoints.is_empty() && found.endpoints.iter().all(|e| e.starts_with('[')) {
        notes.push("Only IPv6 addresses: set the account's IP version to IPv6 (or IPv6, then IPv4), so its Contact and media use IPv6 too.".into());
    }
    for note in notes {
        ui.label(egui::RichText::new(note).size(12.5).color(p.warning));
    }
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        let asked = if found.servers.is_empty() { "nobody".to_owned() } else { found.servers.join(", ") };
        panel_card(&mut cols[0], p, &format!("Records for {host}, from {asked}"), |ui| {
            for step in &found.steps {
                line(ui, step.query.clone(), p.text);
                for answer in &step.answers {
                    line(ui, format!("  {answer}"), if step.failed { p.danger } else { p.secondary });
                }
                ui.add_space(4.0);
            }
        });
        panel_card(&mut cols[1], p, "Where the account connects, best first", |ui| {
            if found.endpoints.is_empty() {
                ui.label(egui::RichText::new("No addresses").color(p.dim));
            }
            for endpoint in &found.endpoints {
                line(ui, endpoint.clone(), p.success);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns_discovery::Step;

    #[test]
    fn results_show_the_chain() {
        use egui_kittest::kittest::Queryable;
        let step = |query: &str, answers: &[&str]| Step { query: query.into(), answers: answers.iter().map(|a| (*a).to_owned()).collect(), failed: false };
        let aaaa: Vec<String> = (1..=6).map(|i| format!("2001:db8::{i}")).collect();
        let aaaa: Vec<&str> = aaaa.iter().map(String::as_str).collect();
        let found = Discovery {
            servers: vec!["192.0.2.53".into()],
            steps: vec![
                step("NAPTR ims.example", &["10 0 \"s\" \"SIPS+D2T\" _sips._tcp.ims.example.  (TLS)"]),
                step("SRV _sips._tcp.ims.example", &["10 50 5061 pcscf.ims.example."]),
                step("AAAA pcscf.ims.example", &aaaa),
            ],
            endpoints: vec!["[2001:db8::1]:5061".into()],
            notes: Vec::new(),
        };
        let harness = crate::ui::test_support::harness([900.0, 600.0], (), move |ui, _| {
            results(ui, &Palette::dark(), "ims.example", &found, true);
        });
        harness.get_by_label("SRV _sips._tcp.ims.example");
        harness.get_by_label("  10 50 5061 pcscf.ims.example.");
        harness.get_by_label("  2001:db8::6");
        harness.get_by_label("[2001:db8::1]:5061");
        harness.get_by_label("Only IPv6 addresses: set the account's IP version to IPv6 (or IPv6, then IPv4), so its Contact and media use IPv6 too.");
    }
}
