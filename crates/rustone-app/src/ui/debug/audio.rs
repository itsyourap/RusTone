//! Audio: levels, waveform and frequency spectrum of the microphone and of
//! what the calls send to the speaker.

use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints};
use sip_engine::debug::AudioTap;

use super::{header, panel_card};
use crate::app::App;
use crate::theme::Palette;

const FFT_SIZE: usize = 2048;
/// Spectrum floor, dBFS.
const FLOOR_DB: f32 = -100.0;

#[derive(Debug, Default)]
pub struct AudioState {
    frozen: bool,
    /// Last captured frames per tap while frozen.
    held: [Option<(u32, Vec<f32>)>; 2],
}

pub fn show(ui: &mut Ui, app: &mut App, p: &Palette) {
    let feed = app.sip.debug_feed();
    let s = &mut app.debug.audio;
    header(ui, p, "Audio", |ui| {
        ui.checkbox(&mut s.frozen, "Freeze");
    });
    if !feed.is_enabled() {
        ui.label(egui::RichText::new("Audio capture starts when the debug window opens.").color(p.dim));
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (i, (tap, title)) in [(AudioTap::Mic, "Microphone"), (AudioTap::Speaker, "Speaker (call audio)")].into_iter().enumerate() {
            let (rate, samples) = if s.frozen && s.held[i].is_some() {
                s.held[i].clone().expect("checked")
            } else {
                let live = feed.audio(tap, FFT_SIZE);
                s.held[i] = Some(live.clone());
                live
            };
            panel_card(ui, p, title, |ui| tap_view(ui, p, title, rate, &samples));
            ui.add_space(10.0);
        }
    });
}

fn tap_view(ui: &mut Ui, p: &Palette, id: &str, rate: u32, samples: &[f32]) {
    if samples.is_empty() || rate == 0 {
        ui.label(egui::RichText::new("No audio — the device is idle (no call, or nothing playing).").color(p.dim));
        return;
    }
    let (rms, peak) = levels(samples);
    ui.horizontal(|ui| {
        meter(ui, p, rms, peak);
        ui.label(egui::RichText::new(format!("RMS {rms:.1} dBFS · peak {peak:.1} dBFS · {rate} Hz")).font(super::mono(12.0)).color(p.text));
    });
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        // The last 50 ms as a waveform.
        let n = (rate as usize / 20).min(samples.len());
        let wave: PlotPoints<'static> = samples[samples.len() - n..]
            .iter()
            .enumerate()
            .map(|(i, &v)| [i as f64 * 1000.0 / f64::from(rate), f64::from(v)])
            .collect::<Vec<_>>()
            .into();
        cols[0].label(egui::RichText::new("Waveform (last 50 ms)").strong().color(p.text));
        Plot::new(format!("{id}-wave"))
            .height(170.0)
            .include_y(-1.0)
            .include_y(1.0)
            .allow_scroll(false)
            .x_axis_label("ms")
            .show(&mut cols[0], |plot| plot.line(Line::new("signal", wave).color(p.accent).width(1.2)));

        let spectrum = spectrum(samples, rate);
        let peak_bin = spectrum.iter().copied().fold((0.0f32, FLOOR_DB), |best, (f, db)| if db > best.1 { (f, db) } else { best });
        let points: PlotPoints<'static> = spectrum.iter().map(|&(f, db)| [f64::from(f), f64::from(db)]).collect::<Vec<_>>().into();
        cols[1].label(egui::RichText::new(format!("Spectrum (peak {:.0} Hz at {:.0} dBFS)", peak_bin.0, peak_bin.1)).strong().color(p.text));
        Plot::new(format!("{id}-spectrum"))
            .height(170.0)
            .include_y(f64::from(FLOOR_DB))
            .include_y(0.0)
            .allow_scroll(false)
            .x_axis_label("Hz")
            .show(&mut cols[1], |plot| plot.line(Line::new("magnitude", points).color(p.success).width(1.2)));
    });
}

/// A horizontal level bar: RMS filled, peak as a tick (-60..0 dBFS).
fn meter(ui: &mut Ui, p: &Palette, rms_db: f32, peak_db: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, 12.0), egui::Sense::hover());
    let frac = |db: f32| ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, egui::CornerRadius::same(6), p.surface);
    let mut fill = rect;
    fill.set_width(rect.width() * frac(rms_db));
    let color = if peak_db > -3.0 { p.danger } else if rms_db > -20.0 { p.warning } else { p.success };
    ui.painter().rect_filled(fill, egui::CornerRadius::same(6), color);
    let x = rect.left() + rect.width() * frac(peak_db);
    ui.painter().line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(2.0, p.text));
}

/// RMS and peak level in dBFS.
pub fn levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (FLOOR_DB, FLOOR_DB);
    }
    let mean_square = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let db = |v: f32| if v <= 0.0 { FLOOR_DB } else { (20.0 * v.log10()).max(FLOOR_DB) };
    (db(mean_square.sqrt()), db(peak))
}

/// Magnitude spectrum (Hz, dBFS) of the last FFT_SIZE samples, Hann-windowed.
pub fn spectrum(samples: &[f32], rate: u32) -> Vec<(f32, f32)> {
    let n = FFT_SIZE.min(samples.len().next_power_of_two() / 2 * 2).max(2);
    if samples.len() < n {
        return Vec::new();
    }
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let start = samples.len() - n;
    let mut input: Vec<f32> = samples[start..]
        .iter()
        .enumerate()
        .map(|(i, &v)| v * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos()))
        .collect();
    let mut output = fft.make_output_vec();
    if fft.process(&mut input, &mut output).is_err() {
        return Vec::new();
    }
    // Hann window has a coherent gain of 0.5; scale so a full-scale sine reads ~0 dBFS.
    let scale = 4.0 / n as f32;
    output
        .iter()
        .enumerate()
        .skip(1)
        .map(|(k, c)| {
            let mag = c.norm() * scale;
            let db = if mag <= 0.0 { FLOOR_DB } else { (20.0 * mag.log10()).max(FLOOR_DB) };
            (k as f32 * rate as f32 / n as f32, db)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, amplitude: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| amplitude * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin()).collect()
    }

    #[test]
    fn spectrum_finds_the_tone() {
        let s = sine(1000.0, 16000, 0.5, 4096);
        let spec = spectrum(&s, 16000);
        let (f, db) = spec.iter().copied().fold((0.0, FLOOR_DB), |b, (f, d)| if d > b.1 { (f, d) } else { b });
        assert!((f - 1000.0).abs() < 16000.0 / FFT_SIZE as f32 * 1.5, "peak at {f}");
        assert!((db - (-6.0)).abs() < 1.5, "0.5 amplitude ≈ -6 dBFS, got {db}");
    }

    #[test]
    fn levels_of_a_sine() {
        let (rms, peak) = levels(&sine(440.0, 8000, 1.0, 8000));
        assert!((peak - 0.0).abs() < 0.1);
        assert!((rms - (-3.0)).abs() < 0.2, "{rms}");
        assert_eq!(levels(&[0.0; 10]).0, FLOOR_DB);
    }
}
