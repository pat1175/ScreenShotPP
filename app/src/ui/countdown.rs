//! Compte à rebours de la capture différée : pastille qui suit le curseur,
//! click-through, sans focus. À zéro, demande la capture à la boucle principale.

use std::time::{Duration, Instant};

use egui::{Color32, Ui};
use winit::window::WindowAttributes;

use crate::app::{Action, WinCx, WindowUi};
use crate::capture_flow::CaptureMode;
use crate::platform::Monitor;
use screenshotpp_core::countdown;

const SIZE: f64 = 64.0;

pub fn attributes() -> WindowAttributes {
    crate::platform::tool_window("Countdown")
        .with_inner_size(winit::dpi::LogicalSize::new(SIZE, SIZE))
        .with_transparent(true)
        .with_active(false)
}

pub struct CountdownUi {
    mode: CaptureMode,
    total_secs: u32,
    monitor: Monitor,
    start: Instant,
    finished: bool,
}

impl CountdownUi {
    pub fn new(mode: CaptureMode, total_secs: u32, monitor: Monitor) -> Self {
        CountdownUi {
            mode,
            total_secs,
            monitor,
            start: Instant::now(),
            finished: false,
        }
    }

    /// Place la pastille en bas à droite du curseur, bornée au moniteur.
    fn follow_cursor(&self, cx: &WinCx) {
        let Ok(cursor) = crate::platform::cursor_position() else {
            return;
        };
        // Taille de la fenêtre en unités natives (points sur macOS, pixels sur Windows).
        let side = (SIZE * self.monitor.scale / self.monitor.px_per_unit()).round() as u32;
        let (x, y) = countdown::window_origin(cursor, (side, side), self.monitor.rect);
        cx.window
            .set_outer_position(crate::platform::native_position(x as f64, y as f64));
    }
}

impl WindowUi for CountdownUi {
    fn ui(&mut self, ui: &mut Ui, cx: &mut WinCx) {
        if self.finished {
            return;
        }
        let elapsed = self.start.elapsed().as_millis();
        let remaining = countdown::remaining_seconds(self.total_secs, elapsed);
        if remaining == 0 {
            self.finished = true;
            cx.services.push(Action::CountdownFinished(self.mode));
            return;
        }
        self.follow_cursor(cx);

        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.circle_filled(
            rect.center(),
            rect.width().min(rect.height()) / 2.0,
            Color32::from_black_alpha(184),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            remaining.to_string(),
            egui::FontId::proportional(30.0),
            Color32::WHITE,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
