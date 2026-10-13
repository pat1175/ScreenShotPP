//! Curseur, moniteurs et placement de fenêtres dans UN seul espace de
//! coordonnées « natif » par OS, celui de xcap / de l'OS :
//! - Windows : pixels physiques (le process est DPI-aware par moniteur, winit
//!   l'active au démarrage) ;
//! - macOS : points logiques (CoreGraphics), origine en haut à gauche.
//!
//! Les captures, elles, sont toujours en pixels physiques : `Monitor::px_per_unit`
//! convertit de l'un à l'autre.

pub use screenshotpp_core::geometry::MonitorRect;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Position, Size};

#[derive(Debug, Clone, Copy)]
pub struct Monitor {
    /// Index dans l'énumération OS (= sortie DXGI / écran avfoundation pour ffmpeg).
    pub index: usize,
    /// Rectangle en unités natives.
    pub rect: MonitorRect,
    /// Facteur d'échelle de l'écran (pixels physiques / points).
    pub scale: f64,
}

impl Monitor {
    /// Pixels physiques par unité native : le scale sur macOS (unités = points),
    /// 1 sur Windows (unités = pixels physiques).
    pub fn px_per_unit(&self) -> f64 {
        if cfg!(target_os = "macos") {
            self.scale
        } else {
            1.0
        }
    }
}

/// Tous les moniteurs, dans l'ordre d'énumération de l'OS.
#[cfg(any(windows, target_os = "macos"))]
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let all = xcap::Monitor::all().map_err(|e| e.to_string())?;
    if all.is_empty() {
        return Err("Aucun moniteur disponible".into());
    }
    Ok(all
        .iter()
        .enumerate()
        .map(|(index, m)| Monitor {
            index,
            rect: MonitorRect {
                x: m.x().unwrap_or(0),
                y: m.y().unwrap_or(0),
                width: m.width().unwrap_or(0),
                height: m.height().unwrap_or(0),
            },
            scale: m.scale_factor().map(|s| s as f64).unwrap_or(1.0),
        })
        .collect())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn monitors() -> Result<Vec<Monitor>, String> {
    Err("Capture non prise en charge sur cette plateforme".into())
}

/// Index du moniteur principal (repli quand le curseur n'est sur aucun écran).
#[cfg(any(windows, target_os = "macos"))]
fn primary_index() -> usize {
    xcap::Monitor::all()
        .ok()
        .and_then(|all| all.iter().position(|m| m.is_primary().unwrap_or(false)))
        .unwrap_or(0)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn primary_index() -> usize {
    0
}

/// Position globale du curseur, en unités natives.
#[cfg(windows)]
pub fn cursor_position() -> Result<(i32, i32), String> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    // SAFETY: `p` est un POINT valide, écrit par l'API.
    unsafe { GetCursorPos(&mut p) }.map_err(|e| e.to_string())?;
    Ok((p.x, p.y))
}

#[cfg(target_os = "macos")]
pub fn cursor_position() -> Result<(i32, i32), String> {
    use objc2_core_graphics::CGEvent;
    // Un CGEvent vide porte la position courante du curseur, en points globaux,
    // origine en haut à gauche — le même espace que CGDisplayBounds / xcap.
    let event = CGEvent::new(None).ok_or("Position du curseur indisponible")?;
    let p = CGEvent::location(Some(&event));
    Ok((p.x.floor() as i32, p.y.floor() as i32))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn cursor_position() -> Result<(i32, i32), String> {
    Err("Position du curseur non prise en charge sur cette plateforme".into())
}

/// Moniteur sous le curseur (ou principal en repli) et position du curseur.
pub fn monitor_at_cursor() -> Result<(Monitor, (i32, i32)), String> {
    let cursor = cursor_position()?;
    let all = monitors()?;
    let rects: Vec<MonitorRect> = all.iter().map(|m| m.rect).collect();
    let idx = screenshotpp_core::geometry::monitor_at(&rects, cursor.0, cursor.1)
        .unwrap_or_else(primary_index);
    let monitor = all
        .get(idx)
        .or(all.first())
        .copied()
        .ok_or("Aucun moniteur")?;
    Ok((monitor, cursor))
}

/// Position d'une fenêtre exprimée en unités natives.
pub fn native_position(x: f64, y: f64) -> Position {
    if cfg!(target_os = "macos") {
        LogicalPosition::new(x, y).into()
    } else {
        PhysicalPosition::new(x.round() as i32, y.round() as i32).into()
    }
}

/// Taille d'une fenêtre exprimée en unités natives.
pub fn native_size(w: f64, h: f64) -> Size {
    if cfg!(target_os = "macos") {
        LogicalSize::new(w, h).into()
    } else {
        PhysicalSize::new(w.round().max(1.0) as u32, h.round().max(1.0) as u32).into()
    }
}

/// Attributs communs aux fenêtres « outil » : sans bordure, au premier plan,
/// absentes de la barre des tâches.
pub fn tool_window(title: &str) -> winit::window::WindowAttributes {
    let attrs = winit::window::Window::default_attributes()
        .with_title(title)
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(winit::window::WindowLevel::AlwaysOnTop);
    #[cfg(windows)]
    let attrs = {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs.with_skip_taskbar(true)
    };
    attrs
}
