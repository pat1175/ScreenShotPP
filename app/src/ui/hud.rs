//! HUD d'enregistrement : liseré rouge autour de la région (4 fines fenêtres
//! opaques posées À L'EXTÉRIEUR de la région, click-through → absentes de la
//! vidéo) et barre de contrôle (chrono, pause/reprise, stop) hors de la région.

use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};
use winit::window::WindowAttributes;

use crate::app::{Action, WinCx, WindowUi};
use crate::platform::{native_position, native_size, Monitor};
use crate::record::Recording;
use crate::ui::style;
use screenshotpp_core::record::Region;

const BORDER: f64 = 3.0; // épaisseur du liseré, en points
const BAR_W: f64 = 240.0;
const BAR_H: f64 = 48.0;
const BAR_GAP: f64 = 10.0;
const RED: Color32 = Color32::from_rgb(0xff, 0x3b, 0x30);

/// Région en unités natives, absolue (origine du moniteur incluse).
fn region_native(monitor: &Monitor, region: Region) -> (f64, f64, f64, f64) {
    let k = monitor.px_per_unit();
    (
        monitor.rect.x as f64 + region.x as f64 / k,
        monitor.rect.y as f64 + region.y as f64 / k,
        region.width as f64 / k,
        region.height as f64 / k,
    )
}

/// Unités natives par point d'interface (1 sur macOS, le scale sur Windows).
fn native_per_point(monitor: &Monitor) -> f64 {
    monitor.scale / monitor.px_per_unit()
}

/// Les 4 bandes du liseré : haut, bas, gauche, droite.
pub fn border_attributes(monitor: &Monitor, region: Region) -> Vec<WindowAttributes> {
    let (x, y, w, h) = region_native(monitor, region);
    let t = (BORDER * native_per_point(monitor)).round().max(1.0);
    [
        (x - t, y - t, w + 2.0 * t, t),
        (x - t, y + h, w + 2.0 * t, t),
        (x - t, y, t, h),
        (x + w, y, t, h),
    ]
    .into_iter()
    .map(|(bx, by, bw, bh)| {
        crate::platform::tool_window("rec-border")
            .with_position(native_position(bx, by))
            .with_inner_size(native_size(bw, bh))
            .with_active(false)
    })
    .collect()
}

/// Barre de contrôle juste sous la région (au-dessus s'il n'y a pas la place),
/// bornée au moniteur.
pub fn controls_attributes(monitor: &Monitor, region: Region) -> WindowAttributes {
    let k = native_per_point(monitor);
    let (rx, ry, rw, rh) = region_native(monitor, region);
    let (mx, my) = (monitor.rect.x as f64, monitor.rect.y as f64);
    let (mw, mh) = (monitor.rect.width as f64, monitor.rect.height as f64);
    let (bw, bh, gap) = (BAR_W * k, BAR_H * k, BAR_GAP * k);
    let below = ry + rh + gap;
    let above = ry - gap - bh;
    let y = if below + bh <= my + mh {
        below
    } else if above >= my {
        above
    } else {
        my
    };
    let x = (rx + (rw - bw) / 2.0).max(mx).min(mx + (mw - bw).max(0.0));
    let y = y.max(my).min(my + (mh - bh).max(0.0));
    crate::platform::tool_window("Recording")
        .with_position(native_position(x, y))
        .with_inner_size(winit::dpi::LogicalSize::new(BAR_W, BAR_H))
        .with_transparent(true)
        .with_active(false)
}

pub struct BorderUi {
    start: Instant,
}

impl BorderUi {
    pub fn new() -> Self {
        BorderUi {
            start: Instant::now(),
        }
    }
}

impl WindowUi for BorderUi {
    fn ui(&mut self, ui: &mut Ui, _cx: &mut WinCx) {
        // Pulsation (période 1,4 s) entre rouge vif et rouge sombre.
        let t = self.start.elapsed().as_secs_f32() / 1.4 * std::f32::consts::TAU;
        let k = 0.5 + 0.5 * t.cos();
        let dim = Color32::from_rgb(0x70, 0x1a, 0x16);
        let color = dim.lerp_to_gamma(RED, k);
        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
}

pub struct ControlsUi {
    recording: Recording,
    paused: bool,
    busy: Option<Receiver<Result<(), String>>>,
    stopping: bool,
    /// Secondes enregistrées hors pauses.
    elapsed: f64,
    last: Instant,
}

impl ControlsUi {
    pub fn new(recording: Recording) -> Self {
        ControlsUi {
            recording,
            paused: false,
            busy: None,
            stopping: false,
            elapsed: 0.0,
            last: Instant::now(),
        }
    }

    fn toggle_pause(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let recording = self.recording.clone();
        let pause = !self.paused;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = if pause {
                recording.pause()
            } else {
                recording.resume()
            };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.busy = Some(rx);
    }
}

fn fmt(total: f64) -> String {
    let s = total.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl WindowUi for ControlsUi {
    fn ui(&mut self, ui: &mut Ui, cx: &mut WinCx) {
        let now = Instant::now();
        if !self.paused && self.busy.is_none() {
            self.elapsed += now.duration_since(self.last).as_secs_f64();
        }
        self.last = now;
        if let Some(rx) = &self.busy {
            if let Ok(result) = rx.try_recv() {
                self.busy = None;
                match result {
                    Ok(()) => self.paused = !self.paused,
                    Err(e) => eprintln!("pause/resume failed: {e}"),
                }
                // Recale l'horloge : l'aller-retour ffmpeg ne compte pas.
                self.last = Instant::now();
            }
        }

        let rect = ui.max_rect();
        ui.painter().rect(
            rect.shrink(0.5),
            10.0,
            Color32::from_rgba_unmultiplied(28, 28, 30, 235),
            egui::Stroke::new(1.0, Color32::from_white_alpha(30)),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(egui::vec2(12.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        let blink_on = (self.elapsed * 1000.0 / 600.0) as u64 % 2 == 0;
        let dot_color = if self.paused {
            Color32::from_rgb(0xff, 0xcc, 0x00)
        } else if blink_on {
            RED
        } else {
            RED.gamma_multiply(0.25)
        };
        ui.painter().circle_filled(dot.center(), 6.0, dot_color);
        ui.label(RichText::new(fmt(self.elapsed)).size(15.0).monospace());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = self.busy.is_none() && !self.stopping;
            if ui
                .add_enabled(enabled, style::danger_button("Stop"))
                .clicked()
            {
                self.stopping = true;
                cx.services.push(Action::StopRecording);
            }
            let label = if self.paused { "Resume" } else { "Pause" };
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                self.toggle_pause(ui.ctx());
            }
        });
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}
