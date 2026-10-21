//! The eframe window. It borrows the resident `App` for its lifetime and
//! hands it back to the shell when it closes.

use fastframe_shell::{Held, StallWatch};

use crate::app::App;
use crate::ui;

pub struct Window {
    app: Held<App>,
    stalls: StallWatch,
}

impl Window {
    pub fn new(app: Held<App>) -> Self {
        Self { app, stalls: StallWatch::default() }
    }
}

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let app = &mut self.app;
        self.stalls.logic(|| app.logic(ctx, frame));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let app = &mut self.app;
        self.stalls.ui(|| {
            ui::draw(ui, app);
            app.paint_transition(ui.ctx());
        });
    }
}

/// Window options for the next window, read from the app before it opens.
pub fn native_options(app: &App) -> eframe::NativeOptions {
    fastframe_titlebar::set_enabled(app.settings.ui.custom_titlebar);
    let viewport = egui::ViewportBuilder::default()
        .with_title("RusTone")
        // Wayland/X11: must match the installed desktop entry
        // (dev.itsyourap.rustone.desktop) for the dock icon and grouping.
        .with_app_id(crate::DESKTOP_ID)
        .with_icon(crate::icons::window_icon())
        .with_inner_size(rustone_core::settings::DEFAULT_WINDOW_SIZE)
        .with_min_inner_size([340.0, 520.0])
        .with_decorations(!fastframe_titlebar::enabled());
    eframe::NativeOptions {
        viewport: app.window_geometry().apply(viewport),
        // The shell keeps state between windows; egui's own persistence
        // would also restore the zoom behind keep_zoom's back.
        persist_window: false,
        ..Default::default()
    }
}
