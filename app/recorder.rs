// TEMP stub — remplacé par l'implémentation complète.
use crate::app::{WinCx, WindowUi};
use crate::record::Recording;
pub fn attributes() -> winit::window::WindowAttributes { winit::window::Window::default_attributes() }
pub struct RecorderUi;
impl RecorderUi { pub fn new(_f: std::path::PathBuf, _r: Recording) -> Self { RecorderUi } }
impl WindowUi for RecorderUi { fn ui(&mut self, _ui: &mut egui::Ui, cx: &mut WinCx) { cx.close(); } }
