use image::RgbaImage;

// Types géométriques purs (Rect, MonitorRect, monitor_at) déplacés dans
// `screenshotpp-core` ; réexportés ici pour conserver les chemins `crate::capture::*`.
pub use screenshotpp_core::geometry::{MonitorRect, Rect};

/// Recadre l'image source selon le rectangle, en bornant aux dimensions de l'image.
pub fn crop_region(src: &RgbaImage, rect: Rect) -> RgbaImage {
    let max_w = src.width().saturating_sub(rect.x);
    let max_h = src.height().saturating_sub(rect.y);
    let w = rect.width.min(max_w).max(1);
    let h = rect.height.min(max_h).max(1);
    image::imageops::crop_imm(src, rect.x, rect.y, w, h).to_image()
}

/// Rectangle du moniteur contenant (x, y), ou du moniteur principal en repli,
/// accompagné de son facteur d'échelle (pixels physiques / points logiques).
/// Sur macOS, le rectangle est en points logiques (xcap = `CGDisplayBounds`) ;
/// le scale permet de reconvertir une sélection en pixels physiques de l'image.
#[cfg(target_os = "macos")]
pub fn monitor_rect_at(x: i32, y: i32) -> Result<(MonitorRect, f32), String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let rects: Vec<MonitorRect> = monitors
        .iter()
        .map(|m| MonitorRect {
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            width: m.width().unwrap_or(0),
            height: m.height().unwrap_or(0),
        })
        .collect();
    let idx = screenshotpp_core::geometry::monitor_at(&rects, x, y).unwrap_or_else(|| {
        monitors
            .iter()
            .position(|m| m.is_primary().unwrap_or(false))
            .unwrap_or(0)
    });
    let rect = rects
        .get(idx)
        .copied()
        .ok_or("Aucun moniteur".to_string())?;
    let scale = monitors
        .get(idx)
        .and_then(|m| m.scale_factor().ok())
        .unwrap_or(1.0);
    Ok((rect, scale))
}

/// Capture le moniteur contenant (x, y), ou le moniteur principal en repli.
/// Sur Windows : via Windows Graphics Capture (WGC), fiable pour le contenu
/// composé GPU (partage Teams) et les écrans secondaires / DPI mixtes.
/// Capture le bitmap complet de la fenêtre au premier plan (partie hors-écran
/// incluse), avec son rectangle en pixels physiques. `Ok(None)` si non supporté
/// ou si la fenêtre n'est pas capturable — l'appelant retombe alors sur le
/// comportement moniteur habituel. Implémentation réelle : Windows (capture_win),
/// macOS (CGWindowListCreateImage). Défaut neutre pour les autres plateformes.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn capture_foreground_window() -> Result<Option<(RgbaImage, Rect)>, String> {
    Ok(None)
}

#[cfg(target_os = "macos")]
mod mac {
    use swift_rs::{swift, SRData, SRString};
    // Capture la fenêtre au premier plan (partie hors-écran incluse) et renvoie les
    // octets PNG. Renvoie SRData vide sur tout échec.
    swift!(pub(super) fn capture_foreground_window_png() -> SRData);
    // Renvoie les bounds de la fenêtre au premier plan en points logiques, sans
    // filtrage par moniteur. Utilisé pour l'origine globale dans capture_foreground_window.
    swift!(pub(super) fn foreground_window_bounds_unfiltered_json() -> SRString);
}

#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct MacWindowBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// macOS : capture la fenêtre au premier plan dans son intégralité via
/// CGWindowListCreateImage (partie hors-écran incluse).
/// Renvoie `Ok(None)` sur tout échec — l'appelant retombe sur la capture moniteur.
#[cfg(target_os = "macos")]
pub fn capture_foreground_window() -> Result<Option<(RgbaImage, Rect)>, String> {
    // 1. Demande à Swift de capturer la fenêtre en PNG complet.
    let png_data = unsafe { mac::capture_foreground_window_png() };
    if png_data.is_empty() {
        return Ok(None);
    }

    // 2. Décode le PNG → RgbaImage (dimensions = pixels physiques Retina).
    let image = match crate::storage::decode_png_to_rgba(&png_data) {
        Ok(img) => img,
        Err(_) => return Ok(None),
    };

    // 3. Récupère la position de la fenêtre en points logiques (CGWindowList).
    let bounds_json = unsafe { mac::foreground_window_bounds_unfiltered_json() };
    let bounds: Option<MacWindowBounds> = serde_json::from_str(&bounds_json.to_string()).ok();

    // 4. Convertit l'origine en pixels physiques via le scale du moniteur au
    //    centre de la fenêtre. La taille provient des dimensions de l'image capturée
    //    (déjà en pixels physiques) pour éviter tout décalage d'arrondi Retina.
    let rect = if let Some(b) = bounds {
        let cx = b.x.saturating_add((b.width / 2) as i32);
        let cy = b.y.saturating_add((b.height / 2) as i32);
        let scale = monitor_rect_at(cx, cy)
            .map(|(_, s)| s as f64)
            .unwrap_or(1.0);
        Rect {
            x: (b.x.max(0) as f64 * scale).round() as u32,
            y: (b.y.max(0) as f64 * scale).round() as u32,
            width: image.width(),
            height: image.height(),
        }
    } else {
        // Origine inconnue : repli sans positionnement global.
        Rect {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        }
    };

    Ok(Some((image, rect)))
}

#[cfg(windows)]
pub fn capture_foreground_window() -> Result<Option<(RgbaImage, Rect)>, String> {
    crate::capture_win::capture_foreground_window()
}

#[cfg(windows)]
pub fn capture_at(x: i32, y: i32) -> Result<RgbaImage, String> {
    crate::capture_win::capture_at_point(x, y)
}

/// Capture le moniteur contenant (x, y), ou le moniteur principal en repli.
#[cfg(target_os = "macos")]
pub fn capture_at(x: i32, y: i32) -> Result<RgbaImage, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let rects: Vec<MonitorRect> = monitors
        .iter()
        .map(|m| MonitorRect {
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            width: m.width().unwrap_or(0),
            height: m.height().unwrap_or(0),
        })
        .collect();
    let idx = screenshotpp_core::geometry::monitor_at(&rects, x, y).unwrap_or_else(|| {
        monitors
            .iter()
            .position(|m| m.is_primary().unwrap_or(false))
            .unwrap_or(0)
    });
    monitors
        .get(idx)
        .ok_or("Aucun moniteur")?
        .capture_image()
        .map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn capture_at(_x: i32, _y: i32) -> Result<RgbaImage, String> {
    Err("Capture non prise en charge sur cette plateforme".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_image(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]))
    }

    #[test]
    fn crop_returns_requested_dimensions_when_in_bounds() {
        let src = solid_image(100, 100);
        let out = crop_region(
            &src,
            Rect {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            },
        );
        assert_eq!(out.dimensions(), (30, 40));
    }

    #[test]
    fn crop_is_clamped_to_image_bounds() {
        let src = solid_image(100, 100);
        let out = crop_region(
            &src,
            Rect {
                x: 90,
                y: 90,
                width: 50,
                height: 50,
            },
        );
        assert_eq!(out.dimensions(), (10, 10));
    }

    #[test]
    fn crop_never_returns_zero_sized_image() {
        let src = solid_image(100, 100);
        let out = crop_region(
            &src,
            Rect {
                x: 10,
                y: 10,
                width: 0,
                height: 0,
            },
        );
        assert_eq!(out.dimensions(), (1, 1));
    }

    #[test]
    fn crop_extracts_the_correct_pixels() {
        // Image 4x4 : colonne gauche rouge, reste noir.
        let mut src = RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        for y in 0..4 {
            src.put_pixel(0, y, image::Rgba([255, 0, 0, 255]));
        }
        // Recadre une zone 2x2 à partir de (1,1) : doit être entièrement noire.
        let out = crop_region(
            &src,
            Rect {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            },
        );
        assert_eq!(out.dimensions(), (2, 2));
        assert_eq!(*out.get_pixel(0, 0), image::Rgba([0, 0, 0, 255]));
        // Recadre incluant la colonne gauche : pixel (0,0) doit être rouge.
        let out2 = crop_region(
            &src,
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        );
        assert_eq!(*out2.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
    }
}
