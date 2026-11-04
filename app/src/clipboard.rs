use image::RgbaImage;

/// Copie une image RGBA dans le presse-papier système.
pub fn copy_image(img: &RgbaImage) -> Result<(), String> {
    let (w, h) = img.dimensions();
    arboard::Clipboard::new()
        .and_then(|mut c| {
            c.set_image(arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: std::borrow::Cow::Borrowed(img.as_raw()),
            })
        })
        .map_err(|e| e.to_string())
}

/// Copie du texte dans le presse-papier système.
pub fn copy_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}
