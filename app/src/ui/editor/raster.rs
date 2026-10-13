//! Rendu à la résolution physique : tracés tiny-skia et police proportionnelle egui.
use super::*;
use ab_glyph::{Font, FontArc, ScaleFont};
use image::{Rgba, RgbaImage};
use std::sync::LazyLock;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

static FONT: LazyLock<FontArc> = LazyLock::new(|| {
    let definitions = egui::FontDefinitions::default();
    let name = &definitions.families[&egui::FontFamily::Proportional][0];
    FontArc::try_from_vec(definitions.font_data[name].font.to_vec())
        .expect("Police egui intégrée invalide")
});
fn font() -> &'static FontArc {
    &FONT
}
// egui 0.36 exprime la taille en unités EM ; ab_glyph utilise la hauteur ascent-descent.
fn glyph_scale(size: f32) -> f32 {
    size * (font().ascent_unscaled() - font().descent_unscaled())
        / font()
            .units_per_em()
            .expect("Police intégrée sans unités EM")
}
pub fn text_size(text: &str, size: f32) -> Vec2 {
    let f = font().as_scaled(glyph_scale(size));
    let mut width: f32 = 0.0;
    for line in text.split('\n') {
        let mut x = 0.0;
        let mut prev = None;
        for c in line.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                x += f.kern(p, id);
            }
            x += f.h_advance(id);
            prev = Some(id);
        }
        width = width.max(x);
    }
    vec2(
        width,
        (f.height() + f.line_gap()) * text.split('\n').count() as f32,
    )
}
/// Pixelation par moyenne des blocs ; le dernier bloc partiel est également traité.
pub fn pixelate(image: &RgbaImage, block: u32) -> RgbaImage {
    let mut out = RgbaImage::new(image.width(), image.height());
    let block = block.max(1);
    for y in (0..image.height()).step_by(block as usize) {
        for x in (0..image.width()).step_by(block as usize) {
            let right = (x + block).min(image.width());
            let bottom = (y + block).min(image.height());
            let count = (right - x) as u64 * (bottom - y) as u64;
            let mut sum = [0u64; 4];
            for yy in y..bottom {
                for xx in x..right {
                    let p = image.get_pixel(xx, yy);
                    for i in 0..4 {
                        sum[i] += p[i] as u64;
                    }
                }
            }
            let p = Rgba(sum.map(|n| (n / count) as u8));
            for yy in y..bottom {
                for xx in x..right {
                    out.put_pixel(xx, yy, p);
                }
            }
        }
    }
    out
}
pub fn crop(
    image: &RgbaImage,
    selection: Rect,
    background: Rect,
    scale: f32,
) -> Result<RgbaImage, String> {
    let r = physical_rect(selection, background, scale);
    if r.x >= image.width() || r.y >= image.height() {
        return Err("Selection is outside the captured image".into());
    }
    Ok(crate::capture::crop_region(image, r))
}
pub fn mosaic(
    image: &RgbaImage,
    rect: Rect,
    background: Rect,
    scale: f32,
) -> Result<RgbaImage, String> {
    Ok(pixelate(
        &crop(image, rect, background, scale)?,
        (16.0 * scale).round().max(1.0) as u32,
    ))
}
fn paint(color: Color32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(color.r(), color.g(), color.b(), color.a());
    p.anti_alias = true;
    p
}
fn premultiply(img: &RgbaImage) -> Result<Pixmap, String> {
    let mut map = Pixmap::new(img.width(), img.height()).ok_or("Image too large")?;
    for (dst, src) in map.data_mut().chunks_exact_mut(4).zip(img.pixels()) {
        let a = src[3] as u16;
        dst[0] = ((src[0] as u16 * a + 127) / 255) as u8;
        dst[1] = ((src[1] as u16 * a + 127) / 255) as u8;
        dst[2] = ((src[2] as u16 * a + 127) / 255) as u8;
        dst[3] = src[3];
    }
    Ok(map)
}
fn unpremultiply(map: Pixmap) -> RgbaImage {
    let (w, h) = (map.width(), map.height());
    let mut bytes = map.take();
    for p in bytes.chunks_exact_mut(4) {
        let a = p[3] as u16;
        if a > 0 {
            for c in &mut p[..3] {
                *c = ((*c as u16 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, bytes).expect("Dimensions du pixmap")
}
fn blend(map: &mut Pixmap, x: i32, y: i32, c: Color32, coverage: f32) {
    if x < 0 || y < 0 || x >= map.width() as i32 || y >= map.height() as i32 {
        return;
    }
    let alpha = (coverage * c.a() as f32 / 255.0).clamp(0.0, 1.0);
    let i = (y as usize * map.width() as usize + x as usize) * 4;
    let p = &mut map.data_mut()[i..i + 4];
    for (channel, value) in p[..3].iter_mut().zip([c.r(), c.g(), c.b()]) {
        *channel = (value as f32 * alpha + *channel as f32 * (1.0 - alpha)).round() as u8;
    }
    p[3] = (255.0 * alpha + p[3] as f32 * (1.0 - alpha)).round() as u8;
}
fn text(map: &mut Pixmap, at: Pos2, value: &str, size: f32, c: Color32, bold: bool) {
    let scale = glyph_scale(size);
    let f = font().as_scaled(scale);
    let mut y = at.y + f.ascent();
    for line in value.split('\n') {
        let mut x = at.x;
        let mut prev = None;
        for ch in line.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                x += f.kern(p, id);
            }
            let g = id.with_scale_and_position(scale, ab_glyph::point(x, y));
            if let Some(outline) = font().outline_glyph(g) {
                let bounds = outline.px_bounds();
                outline.draw(|xx, yy, v| {
                    let px = bounds.min.x as i32 + xx as i32;
                    let py = bounds.min.y as i32 + yy as i32;
                    blend(map, px, py, c, v);
                    if bold {
                        blend(map, px + 1, py, c, v * 0.65);
                    }
                });
            }
            x += f.h_advance(id);
            prev = Some(id);
        }
        y += f.height() + f.line_gap();
    }
}
fn rounded_path(rect: Rect, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let mut b = PathBuilder::new();
    b.move_to(rect.left() + r, rect.top());
    b.line_to(rect.right() - r, rect.top());
    b.quad_to(rect.right(), rect.top(), rect.right(), rect.top() + r);
    b.line_to(rect.right(), rect.bottom() - r);
    b.quad_to(rect.right(), rect.bottom(), rect.right() - r, rect.bottom());
    b.line_to(rect.left() + r, rect.bottom());
    b.quad_to(rect.left(), rect.bottom(), rect.left(), rect.bottom() - r);
    b.line_to(rect.left(), rect.top() + r);
    b.quad_to(rect.left(), rect.top(), rect.left() + r, rect.top());
    b.close();
    b.finish()
}
pub fn arrow_head(from: Pos2, to: Pos2, width: f32) -> [Pos2; 3] {
    let direction = (to - from).normalized();
    let normal = vec2(-direction.y, direction.x);
    let length = (width * 3.0).max(10.0);
    [
        to,
        to - direction * length + normal * length * 0.45,
        to - direction * length - normal * length * 0.45,
    ]
}
pub fn composite(
    image: &RgbaImage,
    background: Rect,
    selection: Rect,
    scale: f32,
    annotations: &[Annotation],
) -> Result<RgbaImage, String> {
    let base = crop(image, selection, background, scale)?;
    let mut map = premultiply(&base)?;
    let tr = Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        -selection.left() * scale,
        -selection.top() * scale,
    );
    let point = |p: Pos2| {
        pos2(
            (p.x - selection.left()) * scale,
            (p.y - selection.top()) * scale,
        )
    };
    for (i, a) in annotations.iter().enumerate() {
        let p = paint(a.color);
        let stroke = Stroke {
            width: a.width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        let mut path = PathBuilder::new();
        match a.tool {
            Tool::Rect => {
                if let Some(r) = tiny_skia::Rect::from_xywh(
                    a.rect.left(),
                    a.rect.top(),
                    a.rect.width(),
                    a.rect.height(),
                ) {
                    path.push_rect(r);
                }
            }
            Tool::Ellipse => {
                if let Some(r) = tiny_skia::Rect::from_xywh(
                    a.rect.left(),
                    a.rect.top(),
                    a.rect.width(),
                    a.rect.height(),
                ) {
                    path.push_oval(r);
                }
            }
            Tool::Line | Tool::Arrow | Tool::Pencil => {
                let smoothed;
                let points = if a.tool == Tool::Pencil {
                    smoothed = smooth_points(&a.points);
                    &smoothed
                } else {
                    &a.points
                };
                if let Some(first) = points.first() {
                    path.move_to(first.x, first.y);
                    for at in &points[1..] {
                        path.line_to(at.x, at.y);
                    }
                }
            }
            Tool::Text => {
                text(
                    &mut map,
                    point(a.rect.min),
                    &a.text,
                    a.font_size * scale,
                    a.color,
                    false,
                );
                continue;
            }
            Tool::Mosaic => {
                if let Ok(tile) = mosaic(image, a.rect, background, scale) {
                    let tile = premultiply(&tile)?;
                    let at = point(a.rect.min);
                    map.draw_pixmap(
                        at.x.round() as i32,
                        at.y.round() as i32,
                        tile.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                }
                continue;
            }
            Tool::Bubble => {
                let center = a.rect.min;
                if !a.text.trim().is_empty() {
                    let end = center + connector(a.label_offset, 15.0);
                    let label = center + a.label_offset;
                    let mut b = PathBuilder::new();
                    b.move_to(label.x, label.y);
                    b.line_to(end.x, end.y);
                    if let Some(path) = b.finish() {
                        map.stroke_path(
                            &path,
                            &p,
                            &Stroke {
                                width: 2.0,
                                ..Stroke::default()
                            },
                            tr,
                            None,
                        );
                    }
                    if let Some(path) = rounded_path(a.label_rect(), 5.0) {
                        map.fill_path(
                            &path,
                            &paint(Color32::from_rgb(13, 17, 23)),
                            FillRule::Winding,
                            tr,
                            None,
                        );
                        map.stroke_path(
                            &path,
                            &p,
                            &Stroke {
                                width: 2.0,
                                ..Stroke::default()
                            },
                            tr,
                            None,
                        );
                    }
                    text(
                        &mut map,
                        point(a.label_rect().min + Vec2::splat(6.0)),
                        &a.text,
                        14.0 * scale,
                        Color32::from_rgb(230, 237, 243),
                        false,
                    );
                }
                let mut circle = PathBuilder::new();
                circle.push_circle(center.x, center.y, 15.0);
                if let Some(path) = circle.finish() {
                    map.fill_path(&path, &p, FillRule::Winding, tr, None);
                }
                let number = bubble_number(annotations, i).unwrap_or(1).to_string();
                let size = text_size(&number, 16.0);
                text(
                    &mut map,
                    point(center - size / 2.0),
                    &number,
                    16.0 * scale,
                    Color32::WHITE,
                    true,
                );
                continue;
            }
            Tool::Select => continue,
        }
        if let Some(path) = path.finish() {
            map.stroke_path(&path, &p, &stroke, tr, None);
        }
        if a.tool == Tool::Arrow && a.points.len() >= 2 {
            let head = arrow_head(a.points[0], a.points[1], a.width);
            let mut b = PathBuilder::new();
            b.move_to(head[0].x, head[0].y);
            b.line_to(head[1].x, head[1].y);
            b.line_to(head[2].x, head[2].y);
            b.close();
            if let Some(path) = b.finish() {
                map.fill_path(&path, &p, FillRule::Winding, tr, None);
            }
        }
    }
    Ok(unpremultiply(map))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mosaic_averages_blocks_and_edges() {
        let image = RgbaImage::from_fn(3, 2, |x, _| Rgba([(x * 60) as u8, 0, 0, 255]));
        let out = pixelate(&image, 2);
        assert_eq!(out.get_pixel(0, 0)[0], 30);
        assert_eq!(out.get_pixel(1, 1)[0], 30);
        assert_eq!(out.get_pixel(2, 1)[0], 120);
    }
    #[test]
    fn physical_crop_rounding() {
        let image = RgbaImage::new(100, 100);
        let b = Rect::from_min_size(pos2(5.0, 10.0), vec2(50.0, 50.0));
        let r = Rect::from_min_size(pos2(6.6, 11.9), vec2(10.4, 10.6));
        assert_eq!(crop(&image, r, b, 1.5).unwrap().dimensions(), (16, 16));
    }
    #[test]
    fn exported_pixels_contain_annotation_stroke() {
        let image = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        let b = Rect::from_min_size(Pos2::ZERO, vec2(50.0, 50.0));
        let sel = Rect::from_min_size(pos2(5.0, 5.0), vec2(40.0, 40.0));
        let mut a = Annotation::new(Tool::Rect, pos2(10.0, 10.0), Color32::RED, 2.0, 24.0);
        a.update(pos2(30.0, 30.0));
        let out = composite(&image, b, sel, 2.0, &[a]).unwrap();
        assert_eq!(out.dimensions(), (80, 80));
        assert_eq!(out.get_pixel(10, 25).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(30, 30).0, [255, 255, 255, 255]);
    }
    #[test]
    fn text_rasterizes_default_egui_font() {
        let image = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        let b = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let mut a = Annotation::new(Tool::Text, pos2(10.0, 10.0), Color32::BLACK, 2.0, 24.0);
        a.text = "Hello".into();
        let out = composite(&image, b, b, 1.0, &[a]).unwrap();
        assert!(out.pixels().any(|p| p[0] < 128));
    }
}
