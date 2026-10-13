// TEMP stub — remplacé par l'implémentation complète.
use crate::app::{WinCx, WindowUi};
use crate::capture_flow::CaptureSession;
use crate::platform::Monitor;
use crate::settings::Settings;
pub fn attributes(_m: &Monitor) -> winit::window::WindowAttributes { crate::platform::tool_window("Overlay") }
pub struct OverlayUi;
impl OverlayUi { pub fn new(_s: CaptureSession, _st: &Settings) -> Self { OverlayUi } }
impl WindowUi for OverlayUi { fn ui(&mut self, _ui: &mut egui::Ui, cx: &mut WinCx) { cx.close(); } }
