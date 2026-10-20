//! Test harnesses that draw real widgets with the app's fonts and icons.

use crate::icons::Icon;

/// A harness running `draw` with `state` in a window of `size`. The
/// fastframe fonts and icons only exist from the frame after they're
/// installed, so the first frame draws nothing.
pub fn harness<S: 'static>(size: [f32; 2], state: S, mut draw: impl FnMut(&mut egui::Ui, &mut S) + 'static) -> egui_kittest::Harness<'static, S> {
    let mut ready = false;
    let mut harness = egui_kittest::Harness::builder().with_size(size).build_ui_state(
        move |ui, state: &mut S| {
            if !ready {
                ui.ctx().set_fonts(fastframe_fonts::FontSetup::default().definitions());
                fastframe_icons::install::<Icon>(ui.ctx());
                ui.ctx().request_repaint();
                ready = true;
                return;
            }
            draw(ui, state);
        },
        state,
    );
    // Fixed frames rather than `run`: animated widgets (a ringing contact)
    // never settle.
    harness.run_steps(2);
    harness
}
