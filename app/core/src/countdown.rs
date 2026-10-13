//! Aides pures pour la capture différée (compte à rebours).
//! La partie intégration OS (fenêtre, boucle) vit dans `commands.rs`.

use crate::geometry::MonitorRect;

/// Secondes entières restantes à afficher pour un décompte de `total_secs`
/// après `elapsed_ms` écoulées. Vaut `total_secs` à 0 ms et atteint 0 à
/// `total_secs * 1000` ms (le décompte est alors terminé).
pub fn remaining_seconds(total_secs: u32, elapsed_ms: u128) -> u32 {
    let elapsed_secs = (elapsed_ms / 1000) as u32;
    total_secs.saturating_sub(elapsed_secs)
}

/// Coin haut-gauche (pixels physiques) de la fenêtre du compteur, placée en bas
/// à droite du curseur, puis bornée pour rester entièrement sur le moniteur.
pub fn window_origin(
    cursor: (i32, i32),
    win_size: (u32, u32),
    monitor: MonitorRect,
) -> (i32, i32) {
    const OFFSET: i32 = 16; // décalage par rapport au curseur
    let (cx, cy) = cursor;
    let (w, h) = (win_size.0 as i32, win_size.1 as i32);
    let min_x = monitor.x;
    let min_y = monitor.y;
    let max_x = monitor.x + monitor.width as i32 - w;
    let max_y = monitor.y + monitor.height as i32 - h;
    let x = (cx + OFFSET).clamp(min_x, max_x.max(min_x));
    let y = (cy + OFFSET).clamp(min_y, max_y.max(min_y));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon() -> MonitorRect {
        MonitorRect { x: 0, y: 0, width: 1000, height: 800 }
    }

    #[test]
    fn remaining_counts_down_and_reaches_zero_at_end() {
        assert_eq!(remaining_seconds(3, 0), 3);
        assert_eq!(remaining_seconds(3, 999), 3);
        assert_eq!(remaining_seconds(3, 1000), 2);
        assert_eq!(remaining_seconds(3, 2999), 1);
        assert_eq!(remaining_seconds(3, 3000), 0);
        assert_eq!(remaining_seconds(3, 9999), 0); // jamais négatif
    }

    #[test]
    fn origin_sits_below_right_of_cursor_when_room() {
        assert_eq!(window_origin((100, 100), (64, 64), mon()), (116, 116));
    }

    #[test]
    fn origin_is_clamped_to_the_monitor_bottom_right() {
        // Curseur tout en bas à droite : la fenêtre est ramenée dans l'écran.
        let (x, y) = window_origin((995, 795), (64, 64), mon());
        assert_eq!(x, 1000 - 64);
        assert_eq!(y, 800 - 64);
    }

    #[test]
    fn origin_respects_monitor_offset() {
        let m = MonitorRect { x: 1000, y: -200, width: 800, height: 600 };
        // Curseur près du coin haut-gauche de ce moniteur décalé.
        assert_eq!(window_origin((1000, -200), (64, 64), m), (1016, -184));
    }
}
