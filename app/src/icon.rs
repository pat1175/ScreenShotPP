//! Icône de l'app embarquée dans le binaire (tray, icône des fenêtres).

use image::RgbaImage;

static ICON_PNG: &[u8] = include_bytes!("../icons/128x128.png");

pub fn app_icon_rgba() -> RgbaImage {
    image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .map(|i| i.to_rgba8())
        .unwrap_or_else(|_| RgbaImage::new(1, 1))
}

pub fn window_icon() -> Option<winit::window::Icon> {
    let img = app_icon_rgba();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}
