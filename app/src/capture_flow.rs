//! Construction d'une session de capture : moniteur sous le curseur, image gelée,
//! fenêtre au premier plan (auto-sélection), bitmap complet si elle déborde.

use image::RgbaImage;

use crate::capture::{self, Rect};
use crate::platform::{self, Monitor};
use crate::window_pick;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureMode {
    Image,
    Video,
}

pub struct WindowCapture {
    pub image: RgbaImage,
}

/// Fenêtre auto-sélectionnable : rectangle de sélection et bande d'activation
/// (zone de survol/clic), en pixels de l'image capturée.
#[derive(Clone, Copy, Debug)]
pub struct WindowSelection {
    pub selection: Rect,
    pub activation: Rect,
}

pub struct CaptureSession {
    pub image: RgbaImage,
    pub window_selections: Vec<WindowSelection>,
    pub window_capture: Option<WindowCapture>,
    pub mode: CaptureMode,
    pub monitor: Monitor,
}

fn rect_from_global(rect: window_pick::GlobalRect) -> Rect {
    Rect {
        x: rect.x.max(0) as u32,
        y: rect.y.max(0) as u32,
        width: rect.width,
        height: rect.height,
    }
}

/// Capture le moniteur sous le curseur. Appelé hors du thread principal.
pub fn capture_session(mode: CaptureMode) -> Result<CaptureSession, String> {
    let (monitor, (cx, cy)) = platform::monitor_at_cursor()?;
    let monitor_global_rect = window_pick::GlobalRect {
        x: monitor.rect.x,
        y: monitor.rect.y,
        width: monitor.rect.width,
        height: monitor.rect.height,
    };
    let candidate = window_pick::foreground_window_selection(monitor_global_rect, monitor.scale);
    // La fenêtre déborde si son rect GLOBAL non-rogné sort du moniteur (le rect
    // relatif, lui, est borné au moniteur par construction). Même espace de
    // coordonnées des deux côtés (unités natives).
    let window_overflows = candidate
        .map(|c| {
            screenshotpp_core::geometry::overflows(
                capture::MonitorRect {
                    x: c.global_rect.x,
                    y: c.global_rect.y,
                    width: c.global_rect.width,
                    height: c.global_rect.height,
                },
                monitor.rect,
            )
        })
        .unwrap_or(false);
    let window_selections = candidate
        .into_iter()
        .map(|c| WindowSelection {
            selection: rect_from_global(c.monitor_relative_rect),
            activation: rect_from_global(c.monitor_relative_activation_rect),
        })
        .collect();
    let image = capture::capture_at(cx, cy)?;
    // Fenêtre débordante : bitmap complet (partie hors écran incluse) pour le rendu
    // « fenêtre entière ». En vidéo, l'enregistrement cible toujours le moniteur.
    let window_capture = if window_overflows && mode == CaptureMode::Image {
        capture::capture_foreground_window()
            .ok()
            .flatten()
            .map(|(image, _rect)| WindowCapture { image })
    } else {
        None
    };
    Ok(CaptureSession {
        image,
        window_selections,
        window_capture,
        mode,
        monitor,
    })
}
