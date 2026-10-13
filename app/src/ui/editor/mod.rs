//! Descripteurs et logique pure de l'éditeur, indépendants de la fenêtre.
pub mod raster;

use egui::{pos2, vec2, Color32, Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pencil,
    Text,
    Bubble,
    Mosaic,
}
impl Tool {
    pub const ALL: [Self; 9] = [
        Self::Select,
        Self::Rect,
        Self::Ellipse,
        Self::Line,
        Self::Arrow,
        Self::Pencil,
        Self::Text,
        Self::Bubble,
        Self::Mosaic,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select / Move",
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Pencil => "Pencil",
            Self::Text => "Text",
            Self::Bubble => "Numbered bubble",
            Self::Mosaic => "Mosaic",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub tool: Tool,
    pub rect: Rect,
    pub points: Vec<Pos2>,
    pub color: Color32,
    pub width: f32,
    pub font_size: f32,
    pub text: String,
    pub label_offset: Vec2,
}
impl Annotation {
    pub fn new(tool: Tool, at: Pos2, color: Color32, width: f32, font_size: f32) -> Self {
        Self {
            tool,
            rect: Rect::from_min_max(at, at),
            points: vec![at],
            color,
            width,
            font_size,
            text: String::new(),
            label_offset: vec2(0.0, -64.0),
        }
    }
    pub fn update(&mut self, at: Pos2) {
        if self.tool == Tool::Pencil {
            if self.points.last().is_none_or(|p| p.distance(at) >= 0.5) {
                self.points.push(at);
            }
            self.rect = points_bounds(&self.points);
        } else {
            self.rect = Rect::from_two_pos(self.points[0], at);
            if matches!(self.tool, Tool::Line | Tool::Arrow) {
                self.points.truncate(1);
                self.points.push(at);
            }
        }
    }
    pub fn degenerate(&self) -> bool {
        match self.tool {
            Tool::Rect | Tool::Ellipse | Tool::Mosaic => {
                self.rect.width() < 2.0 || self.rect.height() < 2.0
            }
            Tool::Line | Tool::Arrow | Tool::Pencil => {
                self.points
                    .windows(2)
                    .map(|p| p[0].distance(p[1]))
                    .sum::<f32>()
                    < 2.0
            }
            Tool::Text => self.text.trim().is_empty(),
            _ => false,
        }
    }
    pub fn translate(&mut self, delta: Vec2) {
        self.rect = self.rect.translate(delta);
        for p in &mut self.points {
            *p += delta;
        }
    }
    pub fn bounds(&self) -> Rect {
        match self.tool {
            Tool::Bubble => {
                let circle = Rect::from_center_size(self.rect.min, Vec2::splat(30.0));
                if self.text.trim().is_empty() {
                    circle
                } else {
                    circle.union(self.label_rect())
                }
            }
            Tool::Text => {
                Rect::from_min_size(self.rect.min, raster::text_size(&self.text, self.font_size))
            }
            _ => self.rect.expand(self.width / 2.0),
        }
    }
    pub fn label_rect(&self) -> Rect {
        Rect::from_center_size(
            self.rect.min + self.label_offset,
            raster::text_size(&self.text, 14.0) + Vec2::splat(12.0),
        )
    }
    pub fn hit(&self, at: Pos2) -> bool {
        match self.tool {
            Tool::Bubble => at.distance(self.rect.min) <= 19.0,
            Tool::Line | Tool::Arrow | Tool::Pencil => smooth_points(&self.points)
                .windows(2)
                .any(|p| distance_to_segment(at, p[0], p[1]) <= self.width / 2.0 + 5.0),
            Tool::Ellipse => {
                let v = (at - self.rect.center()) / (self.rect.size() / 2.0);
                (v.length() - 1.0).abs() * self.rect.width().min(self.rect.height()) / 2.0
                    <= self.width / 2.0 + 5.0
            }
            Tool::Rect => {
                self.rect.expand(5.0).contains(at)
                    && !self.rect.shrink(self.width / 2.0 + 5.0).contains(at)
            }
            _ => self.bounds().expand(4.0).contains(at),
        }
    }
}
pub fn points_bounds(points: &[Pos2]) -> Rect {
    points
        .iter()
        .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)))
}
pub fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() == 0.0 {
        0.0
    } else {
        ((p - a).dot(d) / d.length_sq()).clamp(0.0, 1.0)
    };
    p.distance(a + d * t)
}
pub fn bubble_number(annotations: &[Annotation], index: usize) -> Option<usize> {
    (annotations.get(index)?.tool == Tool::Bubble).then(|| {
        annotations[..=index]
            .iter()
            .filter(|a| a.tool == Tool::Bubble)
            .count()
    })
}
pub fn connector(offset: Vec2, radius: f32) -> Vec2 {
    if offset.length_sq() == 0.0 {
        Vec2::ZERO
    } else {
        offset.normalized() * radius
    }
}

pub struct History {
    snapshots: Vec<Vec<Annotation>>,
    index: usize,
}
impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}
impl History {
    pub fn new() -> Self {
        Self {
            snapshots: vec![vec![]],
            index: 0,
        }
    }
    pub fn push(&mut self, annotations: &[Annotation]) {
        if self.snapshots[self.index] == annotations {
            return;
        }
        self.snapshots.truncate(self.index + 1);
        self.snapshots.push(annotations.to_vec());
        self.index += 1;
    }
    pub fn can_undo(&self) -> bool {
        self.index > 0
    }
    pub fn can_redo(&self) -> bool {
        self.index + 1 < self.snapshots.len()
    }
    pub fn undo(&mut self) -> Vec<Annotation> {
        if self.can_undo() {
            self.index -= 1;
        }
        self.snapshots[self.index].clone()
    }
    pub fn redo(&mut self) -> Vec<Annotation> {
        if self.can_redo() {
            self.index += 1;
        }
        self.snapshots[self.index].clone()
    }
}
pub fn physical_rect(rect: Rect, background: Rect, scale: f32) -> crate::capture::Rect {
    crate::capture::Rect {
        x: ((rect.left() - background.left()) * scale).round().max(0.0) as u32,
        y: ((rect.top() - background.top()) * scale).round().max(0.0) as u32,
        width: (rect.width() * scale).round().max(1.0) as u32,
        height: (rect.height() * scale).round().max(1.0) as u32,
    }
}
pub fn lock_candidate(start: Pos2, end: Pos2, auto: Option<Rect>, bounds: Rect) -> Option<Rect> {
    let r = if start.distance(end) < 4.0 {
        auto.unwrap_or(Rect::from_two_pos(start, end))
    } else {
        Rect::from_two_pos(start, end)
    }
    .intersect(bounds);
    (r.width() >= 2.0 && r.height() >= 2.0).then_some(r)
}
pub fn resize(mut rect: Rect, anchor: usize, point: Pos2, bounds: Rect) -> Rect {
    // Ancres dans le sens horaire, en partant du coin supérieur gauche.
    if !bounds.contains_rect(rect) {
        return rect;
    }
    if matches!(anchor, 0 | 6 | 7) {
        rect.min.x = point.x.clamp(bounds.left(), rect.right() - 2.0);
    }
    if matches!(anchor, 2 | 3 | 4) {
        rect.max.x = point.x.clamp(rect.left() + 2.0, bounds.right());
    }
    if matches!(anchor, 0 | 1 | 2) {
        rect.min.y = point.y.clamp(bounds.top(), rect.bottom() - 2.0);
    }
    if matches!(anchor, 4 | 5 | 6) {
        rect.max.y = point.y.clamp(rect.top() + 2.0, bounds.bottom());
    }
    rect
}
pub fn anchors(r: Rect) -> [Pos2; 8] {
    [
        r.left_top(),
        pos2(r.center().x, r.top()),
        r.right_top(),
        pos2(r.right(), r.center().y),
        r.right_bottom(),
        pos2(r.center().x, r.bottom()),
        r.left_bottom(),
        pos2(r.left(), r.center().y),
    ]
}
pub fn bounded_delta(rect: Rect, delta: Vec2, bounds: Rect) -> Vec2 {
    if rect.width() > bounds.width() || rect.height() > bounds.height() {
        return Vec2::ZERO;
    }
    vec2(
        delta
            .x
            .clamp(bounds.left() - rect.left(), bounds.right() - rect.right()),
        delta
            .y
            .clamp(bounds.top() - rect.top(), bounds.bottom() - rect.bottom()),
    )
}

/// Interpolation de Konva : contrôles pondérés par les distances, tension 0,4.
pub fn smooth_points(points: &[Pos2]) -> Vec<Pos2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut controls = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[(i + 1).min(points.len() - 1)];
        let d0 = p0.distance(p1);
        let d1 = p1.distance(p2);
        let total = (d0 + d1).max(f32::EPSILON);
        controls.push((
            p1 - (p2 - p0) * (0.4 * d0 / total),
            p1 + (p2 - p0) * (0.4 * d1 / total),
        ));
    }
    let mut out = vec![points[0]];
    for i in 0..points.len() - 1 {
        let count = (points[i].distance(points[i + 1]) / 2.0).ceil().max(4.0) as usize;
        for j in 1..=count {
            let t = j as f32 / count as f32;
            let u = 1.0 - t;
            let v = points[i].to_vec2() * u.powi(3)
                + controls[i].1.to_vec2() * (3.0 * u * u * t)
                + controls[i + 1].0.to_vec2() * (3.0 * u * t * t)
                + points[i + 1].to_vec2() * t.powi(3);
            out.push(v.to_pos2());
        }
    }
    out
}
pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if !s.is_ascii() {
        return None;
    }
    let expanded;
    let s = if s.len() == 3 {
        expanded = s.chars().flat_map(|c| [c, c]).collect::<String>();
        &expanded
    } else {
        s
    };
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
pub fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}
pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color32::from_rgb(
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}
pub fn to_hsv(c: Color32) -> [f32; 3] {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shape(tool: Tool) -> Annotation {
        Annotation::new(tool, pos2(10.0, 10.0), Color32::RED, 4.0, 24.0)
    }
    #[test]
    fn history_snapshots_and_branching() {
        let mut h = History::new();
        assert!(!h.can_undo() && !h.can_redo());
        let mut a = vec![shape(Tool::Rect)];
        h.push(&a);
        a[0].width = 99.0;
        assert_eq!(h.undo(), vec![]);
        assert_eq!(h.redo()[0].width, 4.0);
        h.push(&a);
        assert_eq!(h.undo()[0].width, 4.0);
        h.push(&[]);
        assert!(!h.can_redo());
    }
    #[test]
    fn bubble_ranks_and_connector() {
        let mut a = vec![
            shape(Tool::Rect),
            shape(Tool::Bubble),
            shape(Tool::Arrow),
            shape(Tool::Bubble),
            shape(Tool::Bubble),
        ];
        assert_eq!(bubble_number(&a, 0), None);
        assert_eq!(bubble_number(&a, 1), Some(1));
        assert_eq!(bubble_number(&a, 4), Some(3));
        a.remove(1);
        assert_eq!(bubble_number(&a, 2), Some(1));
        assert_eq!(connector(vec2(0.0, -64.0), 15.0), vec2(0.0, -15.0));
        assert_eq!(connector(vec2(100.0, 0.0), 15.0), vec2(15.0, 0.0));
        assert_eq!(connector(vec2(30.0, 40.0), 15.0), vec2(9.0, 12.0));
        assert_eq!(connector(Vec2::ZERO, 15.0), Vec2::ZERO);
    }
    #[test]
    fn hex_and_hsv_roundtrip() {
        assert_eq!(hex(parse_hex("#abc").unwrap()), "#aabbcc");
        assert_eq!(hex(parse_hex("ABCDEF").unwrap()), "#abcdef");
        for s in ["xyz", "#12", "", "nope"] {
            assert!(parse_hex(s).is_none());
        }
        for s in [
            "#ff0000", "#00ff00", "#0000ff", "#123456", "#abcdef", "#000000", "#ffffff",
        ] {
            let c = parse_hex(s).unwrap();
            let [h, sat, v] = to_hsv(c);
            assert_eq!(hex(hsv(h, sat, v)), s);
        }
        assert_eq!(hsv(60.0, 1.0, 1.0), Color32::YELLOW);
        assert_eq!(hsv(120.0, 1.0, 1.0), Color32::GREEN);
        assert_eq!(hsv(240.0, 1.0, 1.0), Color32::BLUE);
    }
    #[test]
    fn selection_threshold_rounding_and_resize() {
        let b = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 80.0));
        let auto = Rect::from_min_size(pos2(10.0, 12.0), vec2(50.0, 40.0));
        assert_eq!(
            lock_candidate(pos2(20.0, 14.0), pos2(22.0, 15.0), Some(auto), b),
            Some(auto)
        );
        assert_eq!(
            lock_candidate(pos2(20.0, 14.0), pos2(60.0, 50.0), Some(auto), b)
                .unwrap()
                .size(),
            vec2(40.0, 36.0)
        );
        assert!(lock_candidate(pos2(1.0, 1.0), pos2(2.0, 2.0), None, b).is_none());
        let r = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 40.0));
        assert_eq!(resize(r, 7, pos2(99.0, 30.0), b).width(), 2.0);
        assert_eq!(resize(r, 1, pos2(30.0, -20.0), b).top(), 0.0);
        let r = Rect::from_two_pos(pos2(2.2, 3.4), pos2(8.2, 9.8));
        let p = physical_rect(r, b, 2.5);
        assert_eq!((p.x, p.y, p.width, p.height), (6, 9, 15, 16));
    }
    #[test]
    fn smoothing_preserves_endpoints_and_geometry_is_bounded() {
        let p = [pos2(0.0, 0.0), pos2(10.0, 10.0), pos2(20.0, 0.0)];
        let s = smooth_points(&p);
        assert_eq!(s.first(), p.first());
        assert_eq!(s.last(), p.last());
        assert!(s.len() > 3);
        let b = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(20.0, 30.0));
        assert_eq!(bounded_delta(r, vec2(-50.0, 100.0), b), vec2(-10.0, 50.0));
    }
    #[test]
    fn selection_sides_preserve_opposite_edges() {
        let b = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 80.0));
        let r = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 40.0));
        for (anchor, p, expected) in [
            (
                7,
                pos2(5.0, 30.0),
                Rect::from_min_max(pos2(5.0, 10.0), pos2(50.0, 50.0)),
            ),
            (
                3,
                pos2(70.0, 30.0),
                Rect::from_min_max(pos2(10.0, 10.0), pos2(70.0, 50.0)),
            ),
            (
                1,
                pos2(30.0, 5.0),
                Rect::from_min_max(pos2(10.0, 5.0), pos2(50.0, 50.0)),
            ),
            (
                5,
                pos2(30.0, 70.0),
                Rect::from_min_max(pos2(10.0, 10.0), pos2(50.0, 70.0)),
            ),
        ] {
            assert_eq!(resize(r, anchor, p, b), expected);
        }
        assert_eq!(resize(r, 3, pos2(999.0, 30.0), b).right(), 100.0);
        assert_eq!(resize(r, 5, pos2(30.0, 999.0), b).bottom(), 80.0);
    }
    #[test]
    fn degenerate_shapes_are_dropped() {
        for tool in [
            Tool::Rect,
            Tool::Ellipse,
            Tool::Mosaic,
            Tool::Line,
            Tool::Arrow,
            Tool::Pencil,
            Tool::Text,
        ] {
            assert!(shape(tool).degenerate());
        }
        let mut a = shape(Tool::Arrow);
        a.update(pos2(20.0, 20.0));
        assert!(!a.degenerate());
        assert!(a.hit(pos2(15.0, 15.0)));
        assert!(!a.hit(pos2(50.0, 50.0)));
    }
}
