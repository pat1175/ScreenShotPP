use image::RgbaImage;
#[cfg(target_os = "macos")]
use serde::Deserialize;

#[cfg(target_os = "macos")]
mod mac {
    use swift_rs::{swift, SRData, SRString};
    swift!(pub(crate) fn ocr_recognize(data: SRData, langs: SRString) -> SRString);
}

/// Reconnaît le texte d'une image. macOS : Apple Vision. Ailleurs : pas encore disponible.
#[cfg(target_os = "macos")]
pub fn recognize(img: &RgbaImage, lang: &str) -> Result<String, String> {
    let png = crate::storage::encode_image(img, crate::storage::SaveFormat::Png)?;
    recognize_png(&png, lang)
}

#[cfg(target_os = "macos")]
fn recognize_png(png: &[u8], lang: &str) -> Result<String, String> {
    use swift_rs::{SRData, SRString};
    let data = SRData::from(png);
    let result = unsafe { mac::ocr_recognize(data, SRString::from(lang)) };
    let response: OcrResponse = serde_json::from_str(&result.to_string())
        .map_err(|e| format!("Réponse OCR invalide: {e}"))?;
    match response {
        OcrResponse::Text { text } => Ok(text),
        OcrResponse::Error { error } => Err(error),
    }
}

#[cfg(windows)]
pub fn recognize(img: &RgbaImage, lang: &str) -> Result<String, String> {
    let png = crate::storage::encode_image(img, crate::storage::SaveFormat::Png)?;
    windows_impl::recognize_png(&png, lang)
}

#[cfg(all(not(target_os = "macos"), not(windows)))]
pub fn recognize(_img: &RgbaImage, _lang: &str) -> Result<String, String> {
    Err("OCR pas encore disponible sur cette plateforme".to_string())
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap,
    };
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    pub(super) fn recognize_png(png: &[u8], lang: &str) -> Result<String, String> {
        // Apartment COM (MTA) pour ce thread ; ignore « déjà initialisé ».
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let stream = InMemoryRandomAccessStream::new().map_err(err)?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(err)?;
        writer.WriteBytes(png).map_err(err)?;
        writer.StoreAsync().map_err(err)?.join().map_err(err)?;
        writer.FlushAsync().map_err(err)?.join().map_err(err)?;
        writer.DetachStream().ok();
        stream.Seek(0).map_err(err)?;

        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(err)?
            .join()
            .map_err(err)?;
        let decoded = decoder
            .GetSoftwareBitmapAsync()
            .map_err(err)?
            .join()
            .map_err(err)?;
        let bitmap = SoftwareBitmap::ConvertWithAlpha(
            &decoded,
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(err)?;

        let engine = create_engine(lang)?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(err)?
            .join()
            .map_err(err)?;
        Ok(result.Text().map_err(err)?.to_string())
    }

    fn create_engine(lang: &str) -> Result<OcrEngine, String> {
        if lang == "auto" {
            return OcrEngine::TryCreateFromUserProfileLanguages().map_err(err);
        }
        let language = Language::CreateLanguage(&HSTRING::from(lang)).map_err(err)?;
        match OcrEngine::TryCreateFromLanguage(&language) {
            Ok(engine) => Ok(engine),
            Err(_) => OcrEngine::TryCreateFromUserProfileLanguages().map_err(err),
        }
    }

    fn err<E: std::fmt::Display>(e: E) -> String {
        format!("OCR Windows: {e}")
    }
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
#[serde(untagged)]
enum OcrResponse {
    Text { text: String },
    Error { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn blank_image_has_no_recognized_text() {
        let img = RgbaImage::new(16, 16);

        assert_eq!(recognize(&img, "auto").unwrap(), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn recognizes_english_and_french_text_fixture() {
        let img = image::load_from_memory(include_bytes!("../fixtures/ocr-en-fr.png"))
            .unwrap()
            .to_rgba8();

        let text = recognize(&img, "auto").unwrap();

        assert!(text.contains("HELLO VISION"), "OCR output: {text:?}");
        assert!(text.contains("BONJOUR VISION"), "OCR output: {text:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn invalid_png_data_returns_an_error() {
        assert_eq!(
            recognize_png(b"not a png", "auto").unwrap_err(),
            "Image OCR invalide"
        );
    }
}
