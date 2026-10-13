use serde::{Deserialize, Serialize};

/// Rectangle de sélection en pixels physiques de l'image capturée.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Rectangle d'un moniteur en pixels physiques globaux.
#[derive(Debug, Clone, Copy)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Index du premier moniteur contenant le point (x, y), ou None.
pub fn monitor_at(rects: &[MonitorRect], x: i32, y: i32) -> Option<usize> {
    rects
        .iter()
        .position(|m| x >= m.x && x < m.x + m.width as i32 && y >= m.y && y < m.y + m.height as i32)
}

/// Vrai si `inner` déborde d'au moins un bord de `outer`.
pub fn overflows(inner: MonitorRect, outer: MonitorRect) -> bool {
    inner.x < outer.x
        || inner.y < outer.y
        || inner.x + inner.width as i32 > outer.x + outer.width as i32
        || inner.y + inner.height as i32 > outer.y + outer.height as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_at_finds_the_monitor_containing_the_point() {
        let rects = [
            MonitorRect {
                x: 0,
                y: 0,
                width: 1000,
                height: 1000,
            },
            MonitorRect {
                x: 1000,
                y: 0,
                width: 800,
                height: 600,
            },
        ];
        assert_eq!(monitor_at(&rects, 500, 500), Some(0));
        assert_eq!(monitor_at(&rects, 1200, 100), Some(1));
        assert_eq!(monitor_at(&rects, 5000, 5000), None);
        assert_eq!(monitor_at(&rects, 1000, 0), Some(1)); // bord gauche du 2e écran
    }

    #[test]
    fn overflows_detects_any_edge_past_the_monitor() {
        let mon = MonitorRect { x: 0, y: 0, width: 1000, height: 800 };
        // Fully inside → no overflow.
        assert!(!overflows(MonitorRect { x: 10, y: 10, width: 100, height: 100 }, mon));
        // Off the left (negative x).
        assert!(overflows(MonitorRect { x: -20, y: 10, width: 100, height: 100 }, mon));
        // Past the bottom.
        assert!(overflows(MonitorRect { x: 10, y: 700, width: 100, height: 200 }, mon));
        // Past the right.
        assert!(overflows(MonitorRect { x: 950, y: 10, width: 100, height: 100 }, mon));
        // Exactly filling the monitor → no overflow.
        assert!(!overflows(MonitorRect { x: 0, y: 0, width: 1000, height: 800 }, mon));
    }
}
