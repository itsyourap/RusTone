//! RTP streams: per-call statistics and live graphs.

use egui::Ui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use sip_engine::debug::{StreamHistory, StreamSample};

use super::{header, kv, panel_card};
use crate::app::App;
use crate::theme::Palette;

#[derive(Debug, Default)]
pub struct MediaState {
    selected: Option<i32>,
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    header(ui, p, "RTP streams", |_| {});
    let streams = app.sip.debug_feed().streams();
    if streams.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("No audio streams yet").size(16.0).color(p.text));
            ui.label(egui::RichText::new("Statistics appear here once a call has audio, sampled every second while this window is open.").color(p.dim));
        });
        return;
    }
    let s = &mut app.debug.media;
    if s.selected.is_none_or(|id| streams.iter().all(|h| h.call != id)) {
        s.selected = streams.iter().rev().find(|h| !h.ended).or(streams.last()).map(|h| h.call);
    }
    ui.horizontal_wrapped(|ui| {
        for h in &streams {
            let label = format!("Call #{} · {}{}", h.call, h.latest.codec, if h.ended { " (ended)" } else { "" });
            ui.selectable_value(&mut s.selected, Some(h.call), label);
        }
    });
    ui.add_space(6.0);
    let Some(h) = streams.iter().find(|h| Some(h.call) == s.selected) else { return };

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        stats(ui, p, h);
        ui.add_space(10.0);
        graphs(ui, p, h);
    });
}

fn stats(ui: &mut Ui, p: &Palette, h: &StreamHistory) {
    let st = &h.latest;
    ui.columns(3, |cols| {
        panel_card(&mut cols[0], p, "Stream", |ui| {
            egui::Grid::new("stream").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                kv(ui, p, "Codec", format!("{} {} Hz ×{}", st.codec, st.clock_rate, st.channels));
                kv(ui, p, "Remote", &st.remote);
                kv(ui, p, "Duration", format_secs(st.duration_ms));
                kv(ui, p, "RTT", format!("{:.1} ms (avg {:.1})", st.rtt_ms, st.rtt_mean_ms));
            });
        });
        panel_card(&mut cols[1], p, "Receive", |ui| {
            egui::Grid::new("rx").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                kv(ui, p, "Packets", format!("{} · {} KB", st.rx_packets, st.rx_bytes / 1024));
                kv(ui, p, "Lost", format!("{} ({:.2}%)", st.rx_lost, pct(st.rx_lost, st.rx_packets + st.rx_lost)));
                kv(ui, p, "Disc/dup/reord", format!("{} / {} / {}", st.rx_discarded, st.rx_duplicates, st.rx_reordered));
                kv(ui, p, "Jitter", format!("{:.1} ms ({:.1}–{:.1})", st.rx_jitter_ms, st.rx_jitter_mean_ms, st.rx_jitter_max_ms));
            });
        });
        panel_card(&mut cols[2], p, "Send · jitter buffer", |ui| {
            egui::Grid::new("tx").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                kv(ui, p, "Packets", format!("{} · {} KB", st.tx_packets, st.tx_bytes / 1024));
                kv(ui, p, "Remote saw", format!("{} lost · {:.1} ms", st.tx_lost, st.tx_jitter_ms));
                kv(ui, p, "JB", format!("{} frames · {} ms", st.jb_frames, st.jb_delay_ms));
                kv(ui, p, "JB lost/disc/empty", format!("{} / {} / {}", st.jb_lost, st.jb_discarded, st.jb_empty));
            });
        });
    });
}

fn format_secs(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn pct(part: u32, whole: u32) -> f64 {
    if whole == 0 { 0.0 } else { f64::from(part) * 100.0 / f64::from(whole) }
}

fn graphs(ui: &mut Ui, p: &Palette, h: &StreamHistory) {
    let series = |f: fn(&StreamSample) -> f64| -> PlotPoints<'static> { h.samples.iter().map(|s| [s.t, f(s)]).collect::<Vec<_>>().into() };
    let plot = |ui: &mut Ui, id: &str, title: &str, unit: &str, min_top: f64, lines: Vec<(&str, PlotPoints<'static>, egui::Color32)>| {
        ui.label(egui::RichText::new(title).strong().color(p.text));
        let unit = unit.to_owned();
        Plot::new(id.to_owned())
            .height(150.0)
            .legend(Legend::default())
            .allow_scroll(false)
            .include_y(0.0)
            .include_y(min_top)
            .width(ui.available_width())
            .x_axis_label("seconds")
            .y_axis_formatter(move |mark, _| format!("{:.0} {unit}", mark.value))
            .show(ui, |plot| {
                for (name, points, color) in lines {
                    plot.line(Line::new(name.to_owned(), points).color(color).width(1.5));
                }
            });
        ui.add_space(8.0);
    };
    ui.columns(2, |cols| {
        plot(&mut cols[0], "bitrate", "Bitrate", "kbps", 100.0, vec![("receive", series(|s| s.rx_kbps), p.success), ("send", series(|s| s.tx_kbps), p.accent)]);
        plot(&mut cols[1], "loss", "Packet loss", "%", 5.0, vec![("receive loss", series(|s| s.rx_loss_pct), p.danger)]);
        plot(&mut cols[0], "jitter", "Jitter & RTT", "ms", 10.0, vec![("jitter", series(|s| s.jitter_ms), p.warning), ("RTT", series(|s| s.rtt_ms), p.accent)]);
        plot(&mut cols[1], "jbuf", "Jitter buffer delay", "ms", 40.0, vec![("delay", series(|s| s.jb_delay_ms), p.secondary)]);
    });
}
