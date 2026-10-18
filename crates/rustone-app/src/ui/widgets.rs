//! Small custom widgets shared by the pages.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType, vec2};
use fastframe_fonts::Weight;

use crate::icons::Icon;
use crate::theme::{CARD_RADIUS, Palette, RADIUS};

const HOVER_TIME: f32 = 0.12;

/// Hover/press feedback, 0..=1, animated.
fn hover_t(ui: &Ui, response: &Response) -> f32 {
    ui.ctx().animate_bool_with_time(response.id, response.hovered() || response.is_pointer_button_down_on(), HOVER_TIME)
}

/// A borderless icon button with a round hover background.
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, tint: Color32, label: &str) -> Response {
    let size = Vec2::splat(34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let t = hover_t(ui, &response);
        if t > 0.0 {
            ui.painter().circle_filled(rect.center(), size.x / 2.0, p.surface_hover.gamma_multiply(t));
        }
        icon.image(tint, 18.0).paint_at(ui, egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0)));
    }
    response.on_hover_text(label)
}

/// A filled circular button with an icon, e.g. the call button.
pub fn round_icon_button(ui: &mut Ui, diameter: f32, fill: Color32, icon: Icon, tint: Color32, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let t = hover_t(ui, &response);
        // Disabled buttons drop their colour so they read as unavailable.
        let (fill, tint) = if ui.is_enabled() {
            (fill.gamma_multiply(1.0 - 0.12 * t), tint)
        } else {
            (ui.visuals().widgets.inactive.bg_fill.gamma_multiply(0.6), ui.visuals().widgets.noninteractive.fg_stroke.color.gamma_multiply(0.45))
        };
        ui.painter().circle_filled(rect.center(), diameter / 2.0, fill);
        let glyph = diameter * 0.4;
        icon.image(tint, glyph).paint_at(ui, egui::Rect::from_center_size(rect.center(), Vec2::splat(glyph)));
    }
    response.on_hover_text(label)
}

/// A keypad key: the digit, with its letters underneath.
pub fn dial_key(ui: &mut Ui, p: &Palette, diameter: f32, digit: &str, letters: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), digit));
    if ui.is_rect_visible(rect) {
        let t = hover_t(ui, &response);
        let fill = lerp_color(p.surface, p.surface_active, t);
        let painter = ui.painter();
        painter.circle_filled(rect.center(), diameter / 2.0, fill);
        let digit_y = if letters.is_empty() { rect.center().y } else { rect.center().y - 6.0 };
        painter.text(
            egui::pos2(rect.center().x, digit_y),
            egui::Align2::CENTER_CENTER,
            digit,
            Weight::Medium.font_id(24.0),
            p.text,
        );
        if !letters.is_empty() {
            painter.text(
                egui::pos2(rect.center().x, rect.center().y + 14.0),
                egui::Align2::CENTER_CENTER,
                letters,
                Weight::SemiBold.font_id(9.5),
                p.dim,
            );
        }
    }
    response
}

/// An on/off switch.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool, label: &str) -> Response {
    let size = vec2(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id, *on, HOVER_TIME);
        let track = lerp_color(p.surface_active, p.accent, t);
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, CornerRadius::same(radius as u8), track);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if *on { p.on_accent } else { p.window };
        ui.painter().circle(egui::pos2(x, rect.center().y), radius - 3.0, knob, Stroke::NONE);
    }
    response
}

/// A row of mutually exclusive choices. Returns true when the value changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, p: &Palette, value: &mut T, options: &[(T, Option<Icon>, &str)]) -> bool {
    let mut changed = false;
    let frame = egui::Frame::new().fill(p.surface).corner_radius(CornerRadius::same(RADIUS)).inner_margin(3);
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Explicitly left-to-right: inside a right-aligned row, `horizontal`
        // would reverse the options.
        let row = vec2(ui.available_width(), 28.0);
        ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            for (option, icon, label) in options {
                let selected = *value == *option;
                let text_color = if selected { p.text } else { p.secondary };
                let galley = ui.painter().layout_no_wrap(label.to_string(), Weight::Medium.font_id(13.0), text_color);
                let icon_room = if icon.is_some() { 22.0 } else { 0.0 };
                let size = vec2(galley.size().x + icon_room + 22.0, 28.0);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, *label));
                let t = hover_t(ui, &response);
                let bg = if selected { p.window } else { p.surface_hover.gamma_multiply(t) };
                ui.painter().rect_filled(rect, CornerRadius::same(RADIUS - 2), bg);
                if let Some(icon) = icon {
                    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(15.0));
                    icon.image(text_color, 15.0).paint_at(ui, icon_rect);
                }
                ui.painter().galley(egui::pos2(rect.left() + 11.0 + icon_room, rect.center().y - galley.size().y / 2.0), galley, text_color);
                if response.clicked() && !selected {
                    *value = *option;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A rounded card for grouping settings.
pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.outline))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A settings row: title and optional description on the left, control on the right.
/// The text wraps in the space the control leaves.
pub fn setting_row(ui: &mut Ui, p: &Palette, title: &str, description: Option<&str>, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            control(ui);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                setting_text(ui, p, title, description);
            });
        });
    });
}

/// A settings block for wide controls: title and description, control underneath.
pub fn setting_stack(ui: &mut Ui, p: &Palette, title: &str, description: Option<&str>, control: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        setting_text(ui, p, title, description);
        ui.add_space(6.0);
        control(ui);
    });
}

fn setting_text(ui: &mut Ui, p: &Palette, title: &str, description: Option<&str>) {
    ui.add(
        egui::Label::new(egui::RichText::new(title).font(Weight::Medium.font_id(14.0)).color(p.text))
            .wrap_mode(egui::TextWrapMode::Extend),
    );
    if let Some(description) = description {
        ui.label(egui::RichText::new(description).size(12.0).color(p.dim));
    }
}

pub fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text.to_uppercase()).font(Weight::SemiBold.font_id(11.0)).color(p.dim));
}

/// Centred icon, title and hint for pages with nothing to show yet.
pub fn empty_state(ui: &mut Ui, p: &Palette, icon: Icon, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.28).max(24.0));
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 36.0, p.surface);
        icon.image(p.dim, 30.0).paint_at(ui, egui::Rect::from_center_size(rect.center(), Vec2::splat(30.0)));
        ui.add_space(12.0);
        ui.label(egui::RichText::new(title).font(Weight::SemiBold.font_id(16.0)).color(p.text));
        ui.label(egui::RichText::new(hint).size(13.0).color(p.dim));
    });
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// A labelled single-line text field filling the width.
pub fn text_field(ui: &mut Ui, p: &Palette, label: &str, hint: &str, value: &mut String) -> Response {
    let label_id = field_label(ui, p, label);
    let edit = egui::TextEdit::singleline(value)
        .hint_text(egui::RichText::new(hint).color(p.dim))
        .desired_width(f32::INFINITY)
        .margin(egui::Margin::symmetric(10, 7));
    ui.add(edit).labelled_by(label_id)
}

/// A password field with a show/hide button.
pub fn password_field(ui: &mut Ui, p: &Palette, label: &str, value: &mut String, shown: &mut bool) -> Response {
    let label_id = field_label(ui, p, label);
    ui.horizontal(|ui| {
        let reveal_width = 34.0 + ui.spacing().item_spacing.x;
        let edit = egui::TextEdit::singleline(value)
            .password(!*shown)
            .desired_width(ui.available_width() - reveal_width)
            .margin(egui::Margin::symmetric(10, 7));
        let response = ui.add(edit).labelled_by(label_id);
        let (icon, tip) = if *shown { (Icon::EyeOff, "Hide password") } else { (Icon::Eye, "Show password") };
        if icon_button(ui, p, icon, p.secondary, tip).clicked() {
            *shown = !*shown;
        }
        response
    })
    .inner
}

/// A labelled whole-number field.
pub fn number_field(ui: &mut Ui, p: &Palette, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>, unit: &str) -> Response {
    let label_id = field_label(ui, p, label);
    ui.add(egui::DragValue::new(value).range(range).suffix(unit).speed(1.0)).labelled_by(label_id)
}

/// Returns the label's id, so the field can be `labelled_by` it.
fn field_label(ui: &mut Ui, p: &Palette, label: &str) -> egui::Id {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(label).size(12.0).color(p.secondary)).id
}

/// A one-line galley that ends in `…` when it doesn't fit.
pub fn elided(ui: &Ui, text: String, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// A full-width filled button.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, fill: Color32) -> Response {
    let button = egui::Button::new(egui::RichText::new(text).color(p.on_accent).font(Weight::SemiBold.font_id(14.0)))
        .fill(fill)
        .corner_radius(CornerRadius::same(RADIUS))
        .min_size(vec2(0.0, 36.0));
    ui.add(button)
}
