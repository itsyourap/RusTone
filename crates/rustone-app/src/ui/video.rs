//! Video inside the window: a call's remote video with the camera as a
//! picture-in-picture, its camera and full-screen buttons, and the camera
//! preview in Settings. Frames come from the engine's video taps.

use std::collections::HashMap;
use std::time::Duration;

use egui::{Rect, Sense, Ui, Vec2, pos2, vec2};
use sip_engine::{CallId, VideoFrames, VideoSource};

use crate::icons::Icon;
use crate::theme::Palette;

/// Per-window video state: textures belong to the window's context.
#[derive(Default)]
pub struct VideoView {
    textures: HashMap<VideoSource, (u64, egui::TextureHandle)>,
    /// The call's video fills the window.
    pub fullscreen: bool,
}

impl std::fmt::Debug for VideoView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoView").field("sources", &self.textures.len()).field("fullscreen", &self.fullscreen).finish()
    }
}

/// What the video's buttons asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoAction {
    Camera(bool),
    Fullscreen(bool),
}

impl VideoView {
    /// The latest frame of `source` as a texture (uploaded only when new).
    fn texture(&mut self, ctx: &egui::Context, frames: &VideoFrames, source: VideoSource) -> Option<(egui::TextureId, Vec2)> {
        let Some(frame) = frames.latest(source) else {
            self.textures.remove(&source);
            return None;
        };
        let size = [frame.width as usize, frame.height as usize];
        match self.textures.get_mut(&source) {
            Some((seq, _)) if *seq == frame.seq => {}
            Some((seq, texture)) => {
                texture.set(egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba), egui::TextureOptions::LINEAR);
                *seq = frame.seq;
            }
            None => {
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
                let texture = ctx.load_texture(format!("video-{source:?}"), image, egui::TextureOptions::LINEAR);
                self.textures.insert(source, (frame.seq, texture));
            }
        }
        self.textures.get(&source).map(|(_, t)| (t.id(), vec2(frame.width as f32, frame.height as f32)))
    }
}

/// Keeps video moving (~30 fps); pjsua delivers frames at its own pace.
fn keep_drawing(ui: &Ui) {
    // egui shortens delayed repaints by the predicted frame time; add it back.
    let predicted = Duration::try_from_secs_f32(ui.input(|i| i.predicted_dt)).unwrap_or_default();
    ui.ctx().request_repaint_after(Duration::from_millis(33) + predicted);
}

/// `size` fitted inside `area`, centred.
fn fit(area: Rect, size: Vec2) -> Rect {
    let scale = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), size * scale)
}

/// What a call's video shows.
#[derive(Debug, Clone, Copy)]
pub struct CallVideo {
    pub call: CallId,
    /// The peer's video reaches us (its camera is on).
    pub receiving: bool,
    /// Our camera is on (the button's state).
    pub camera_on: bool,
    /// Our camera is on and the peer accepts it.
    pub sending: bool,
    /// The call is on hold (either side).
    pub held: bool,
}

/// A call's video in `rect`: remote video (or a placeholder while there's
/// none), the camera in a corner while it's sent, and buttons for the
/// camera and full screen.
pub fn call_video(ui: &mut Ui, view: &mut VideoView, frames: &VideoFrames, p: &Palette, rect: Rect, video: CallVideo) -> Option<VideoAction> {
    let CallVideo { call, receiving, camera_on, sending, held } = video;
    keep_drawing(ui);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, egui::CornerRadius::same(14), egui::Color32::BLACK);
    let remote = if receiving { view.texture(ui.ctx(), frames, VideoSource::Call(call)) } else { None };
    match remote {
        Some((texture, size)) => {
            let shown = fit(rect, size);
            painter.image(texture, shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            let text = if held {
                "On hold"
            } else if receiving {
                "Waiting for video…"
            } else {
                "Their camera is off"
            };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), p.dim);
        }
    }
    // The camera in the bottom-right corner, a quarter of the width.
    if sending && let Some((texture, size)) = view.texture(ui.ctx(), frames, VideoSource::Preview) {
        let w = (rect.width() * 0.28).clamp(80.0, 360.0);
        let h = w * size.y / size.x;
        let pip = Rect::from_min_size(pos2(rect.right() - w - 10.0, rect.bottom() - h - 10.0), vec2(w, h));
        painter.rect_filled(pip.expand(2.0), egui::CornerRadius::same(8), egui::Color32::from_black_alpha(160));
        painter.image(texture, pip, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    // Buttons over the top-right corner.
    let mut action = None;
    // Painted over the video without taking layout space.
    let button = |ui: &mut Ui, center: egui::Pos2, icon: Icon, label: &str| {
        let r = Rect::from_center_size(center, Vec2::splat(36.0));
        let response = ui.interact(r, ui.id().with(("video-button", call, label)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let alpha = if response.hovered() { 200 } else { 140 };
        ui.painter().circle_filled(center, 18.0, egui::Color32::from_black_alpha(alpha));
        icon.image(egui::Color32::WHITE, 18.0).paint_at(ui, Rect::from_center_size(center, Vec2::splat(18.0)));
        response.on_hover_text(label).clicked()
    };
    let top = rect.top() + 26.0;
    let (camera_icon, camera_label) = if camera_on { (Icon::Video, "Turn camera off") } else { (Icon::VideoOff, "Turn camera on") };
    if button(ui, pos2(rect.right() - 26.0, top), camera_icon, camera_label) {
        action = Some(VideoAction::Camera(!camera_on));
    }
    let (fs_icon, fs_label) = if view.fullscreen { (Icon::Minimize, "Exit full screen") } else { (Icon::Maximize, "Full screen") };
    if button(ui, pos2(rect.right() - 70.0, top), fs_icon, fs_label) {
        action = Some(VideoAction::Fullscreen(!view.fullscreen));
    }
    let response = ui.interact(rect, ui.id().with(("video", call)), Sense::click());
    if response.double_clicked() {
        action = Some(VideoAction::Fullscreen(!view.fullscreen));
    }
    action
}

/// The camera preview in Settings, while it's on.
pub fn preview(ui: &mut Ui, view: &mut VideoView, frames: &VideoFrames, p: &Palette) {
    keep_drawing(ui);
    let width = ui.available_width().min(320.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 0.75), Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(10), egui::Color32::BLACK);
    match view.texture(ui.ctx(), frames, VideoSource::Preview) {
        Some((texture, size)) => {
            ui.painter().image(texture, fit(rect, size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Starting the camera…", egui::FontId::proportional(13.0), p.dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_fits_without_stretching() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let shown = fit(area, vec2(960.0, 540.0));
        assert_eq!(shown.width(), 400.0);
        assert!((shown.height() - 225.0).abs() < 0.01 && (shown.center().y - 150.0).abs() < 0.01);
    }
}
