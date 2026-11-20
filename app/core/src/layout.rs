//! Placement letterbox pur : loger une image dans un viewport sans déformation.

/// Rectangle d'affichage (pixels du viewport) et facteur d'échelle appliqué.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitBox {
    pub scale: f64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Plus grande échelle ≤ 1 logeant `content` dans `viewport`, centrée.
/// Si `content` tient déjà, `scale == 1.0` (pas d'agrandissement).
pub fn fit_scale(content: (u32, u32), viewport: (u32, u32)) -> FitBox {
    let (cw, ch) = (content.0.max(1) as f64, content.1.max(1) as f64);
    let (vw, vh) = (viewport.0.max(1) as f64, viewport.1.max(1) as f64);
    let scale = (vw / cw).min(vh / ch).min(1.0);
    let width = (cw * scale).round() as u32;
    let height = (ch * scale).round() as u32;
    let x = ((vw - width as f64) / 2.0).round() as i32;
    let y = ((vh - height as f64) / 2.0).round() as i32;
    FitBox { scale, x, y, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_that_fits_is_centered_at_scale_one() {
        let f = fit_scale((200, 100), (1000, 800));
        assert_eq!(f.scale, 1.0);
        assert_eq!((f.width, f.height), (200, 100));
        assert_eq!((f.x, f.y), (400, 350));
    }

    #[test]
    fn taller_than_viewport_scales_down_to_height() {
        // 500x1600 into 1000x800 → limited by height: scale 0.5.
        let f = fit_scale((500, 1600), (1000, 800));
        assert_eq!(f.scale, 0.5);
        assert_eq!((f.width, f.height), (250, 800));
        assert_eq!(f.x, (1000 - 250) / 2);
        assert_eq!(f.y, 0);
    }
}
