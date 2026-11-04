use chrono::{Local, NaiveDateTime};
use image::{ExtendedColorType, ImageFormat, RgbaImage};
use std::io::Cursor;

/// Format d'image supporté au Palier 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaveFormat {
    Png,
    Jpeg,
}

impl SaveFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            SaveFormat::Png => "png",
            SaveFormat::Jpeg => "jpg",
        }
    }
}

/// Nom de fichier par défaut : "Screenshot_2026-05-30_143205.png".
pub fn default_filename(now: NaiveDateTime, format: SaveFormat) -> String {
    format!(
        "Screenshot_{}_{}.{}",
        now.format("%Y-%m-%d"),
        now.format("%H%M%S"),
        format.extension()
    )
}

/// Nom basé sur l'heure locale courante.
pub fn current_filename(format: SaveFormat) -> String {
    default_filename(Local::now().naive_local(), format)
}

/// Encode une image RGBA dans le format demandé et renvoie les octets.
pub fn encode_image(img: &RgbaImage, format: SaveFormat) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::new());
    match format {
        SaveFormat::Png => img
            .write_to(&mut buf, ImageFormat::Png)
            .map_err(|e| e.to_string())?,
        SaveFormat::Jpeg => {
            // JPEG ne gère pas l'alpha : on convertit en RGB.
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            rgb.write_to(&mut buf, ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(buf.into_inner())
}

/// Décode des octets PNG en image RGBA (capture de fenêtre macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn decode_png_to_rgba(png: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory_with_format(png, ImageFormat::Png)
        .map_err(|e| e.to_string())
        .map(|img| img.to_rgba8())
}

/// Écrit les octets encodés sur le disque.
pub fn write_to_disk(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Cible textuelle de taille → octets max (None = pleine qualité).
pub fn target_max_bytes(target: &str) -> Option<usize> {
    match target {
        "1mb" => Some(1_000_000),
        "2mb" => Some(2_000_000),
        "5mb" => Some(5_000_000),
        _ => None,
    }
}

/// Encode en JPEG à une qualité donnée (0-100).
pub fn encode_jpeg_quality(img: &RgbaImage, quality: u8) -> Result<Vec<u8>, String> {
    use image::codecs::jpeg::JpegEncoder;
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, quality)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Réduit la résolution jusqu'à ce que le PNG tienne sous `max_bytes` (pour le presse-papier,
/// qui transporte du sans-perte : le levier fiable est la réduction de pixels).
pub fn fit_by_downscale(img: &RgbaImage, max_bytes: usize) -> Result<RgbaImage, String> {
    let mut current = img.clone();
    loop {
        let png = encode_image(&current, SaveFormat::Png)?;
        if png.len() <= max_bytes || current.width() <= 32 || current.height() <= 32 {
            return Ok(current);
        }
        let nw = ((current.width() as f32) * 0.85) as u32;
        let nh = ((current.height() as f32) * 0.85) as u32;
        current = image::imageops::resize(
            &current,
            nw.max(1),
            nh.max(1),
            image::imageops::FilterType::Lanczos3,
        );
    }
}

/// JPEG à la meilleure qualité qui tient sous `max_bytes` ; downscale en dernier recours.
pub fn fit_by_jpeg_quality(img: &RgbaImage, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut work = img.clone();
    loop {
        for q in [92u8, 85, 78, 70, 62, 54, 46, 38, 30, 22] {
            let bytes = encode_jpeg_quality(&work, q)?;
            if bytes.len() <= max_bytes {
                return Ok(bytes);
            }
        }
        if work.width() <= 32 || work.height() <= 32 {
            return encode_jpeg_quality(&work, 22);
        }
        let nw = ((work.width() as f32) * 0.8) as u32;
        let nh = ((work.height() as f32) * 0.8) as u32;
        work = image::imageops::resize(
            &work,
            nw.max(1),
            nh.max(1),
            image::imageops::FilterType::Lanczos3,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn filename_is_formatted_with_date_and_extension() {
        let dt = NaiveDate::from_ymd_opt(2026, 5, 30)
            .unwrap()
            .and_hms_opt(14, 32, 5)
            .unwrap();
        assert_eq!(
            default_filename(dt, SaveFormat::Png),
            "Screenshot_2026-05-30_143205.png"
        );
        assert_eq!(
            default_filename(dt, SaveFormat::Jpeg),
            "Screenshot_2026-05-30_143205.jpg"
        );
    }

    #[test]
    fn png_encoding_starts_with_png_magic_bytes() {
        let img = RgbaImage::new(4, 4);
        let bytes = encode_image(&img, SaveFormat::Png).unwrap();
        assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]); // .PNG
    }

    #[test]
    fn jpeg_encoding_starts_with_jpeg_magic_bytes() {
        let img = RgbaImage::new(4, 4);
        let bytes = encode_image(&img, SaveFormat::Jpeg).unwrap();
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]); // JPEG SOI
    }

    #[test]
    fn png_round_trip_preserves_dimensions_and_pixels() {
        let mut img = RgbaImage::from_pixel(6, 4, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, image::Rgba([12, 34, 56, 255]));
        let png = encode_image(&img, SaveFormat::Png).unwrap();
        let back = decode_png_to_rgba(&png).unwrap();
        assert_eq!(back.dimensions(), (6, 4));
        assert_eq!(*back.get_pixel(0, 0), image::Rgba([12, 34, 56, 255]));
    }

    fn busy_image(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                ((x * 7) % 256) as u8,
                ((y * 13) % 256) as u8,
                (((x + y) * 17) % 256) as u8,
                255,
            ])
        })
    }

    #[test]
    fn target_max_bytes_mapping() {
        assert_eq!(target_max_bytes("full"), None);
        assert_eq!(target_max_bytes("1mb"), Some(1_000_000));
        assert_eq!(target_max_bytes("2mb"), Some(2_000_000));
        assert_eq!(target_max_bytes("5mb"), Some(5_000_000));
    }

    #[test]
    fn jpeg_quality_encodes_jpeg() {
        let bytes = encode_jpeg_quality(&busy_image(32, 32), 60).unwrap();
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn downscale_fits_under_target() {
        let out = fit_by_downscale(&busy_image(1200, 1200), 200_000).unwrap();
        let png = encode_image(&out, SaveFormat::Png).unwrap();
        assert!(png.len() <= 200_000, "png={} > cible", png.len());
        assert!(out.width() < 1200);
    }

    #[test]
    fn jpeg_fit_under_target() {
        let bytes = fit_by_jpeg_quality(&busy_image(1200, 1200), 150_000).unwrap();
        assert!(bytes.len() <= 150_000, "jpeg={} > cible", bytes.len());
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
    }
}
