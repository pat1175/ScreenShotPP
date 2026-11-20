//! Windows screen capture via Windows Graphics Capture (WGC).
//! Replaces xcap's GDI BitBlt path, which returns black for GPU-composited
//! content (Teams screen-share) and is unreliable on secondary / mixed-DPI
//! monitors. The WGC plumbing is OS integration (verified manually on Windows);
//! `frame_to_rgba` below is the pure, unit-tested part.

use image::RgbaImage;
use std::sync::mpsc::{sync_channel, SyncSender};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};

/// Convertit un buffer de frame WGC (RGBA8, lignes rembourrées à `stride` octets)
/// en `RgbaImage` compact. `stride` est le nombre d'octets par ligne dans
/// `buffer` et peut dépasser `width * 4`.
pub fn frame_to_rgba(
    buffer: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<RgbaImage, String> {
    let row_bytes = (width as usize).checked_mul(4).ok_or("Largeur invalide")?;
    if stride < row_bytes {
        return Err(format!("Stride {stride} < largeur*4 {row_bytes}"));
    }
    let needed = stride
        .checked_mul(height as usize)
        .ok_or("Dimensions invalides")?;
    if buffer.len() < needed {
        return Err(format!(
            "Buffer trop court: {} < {} attendu",
            buffer.len(),
            needed
        ));
    }
    let mut tight = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        tight.extend_from_slice(&buffer[start..start + row_bytes]);
    }
    RgbaImage::from_raw(width, height, tight).ok_or_else(|| "from_raw a échoué".to_string())
}

/// Canal de sortie de la frame depuis le thread de rappel WGC.
type FrameSink = SyncSender<Result<RgbaImage, String>>;

struct OneShot {
    sink: FrameSink,
}

impl GraphicsCaptureApiHandler for OneShot {
    type Flags = FrameSink;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { sink: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // `on_frame_arrived` est appelé sous la boucle de messages WGC (DispatchMessage,
        // code C). Un panic qui en sortirait traverserait le FFI → abort du processus
        // (0xc0000409). On l'isole donc dans un catch_unwind et on le convertit en Err.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let width = frame.width();
            let height = frame.height();
            // Le buffer brut conserve le rembourrage de ligne ; `stride()` donne le
            // nombre d'octets par ligne. `frame_to_rgba` enlève le rembourrage.
            match frame.buffer() {
                Ok(mut buf) => {
                    let bytes = buf.as_raw_buffer();
                    // windows-capture n'expose pas le stride : on le déduit de la
                    // longueur du buffer brut (lignes rembourrées) divisée par la hauteur.
                    let stride = if height > 0 {
                        bytes.len() / height as usize
                    } else {
                        0
                    };
                    frame_to_rgba(bytes, width, height, stride)
                }
                Err(e) => Err(format!("frame.buffer() a échoué: {e}")),
            }
        }))
        .unwrap_or_else(|_| Err("Panique pendant la conversion de la frame WGC".to_string()));
        // Best-effort : si le récepteur a disparu, on s'arrête quand même.
        let _ = self.sink.try_send(result);
        capture_control.stop();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Exécute `f` sur un thread dédié, nommé `thread_name`, avec un panic
/// capturé et converti en `Err` plutôt que de traverser la frontière FFI
/// (voir le commentaire de `capture_at_point` : c'est ce qui causait le
/// 0xc0000409). Utilisé par toutes les captures WGC (moniteur et fenêtre).
fn run_isolated<T: Send + 'static>(
    thread_name: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let thread_name_owned = thread_name.to_string();
    let handle = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                .unwrap_or_else(|_| Err(format!("{thread_name_owned}: panique interne")))
        })
        .map_err(|e| format!("Échec du lancement du thread {thread_name}: {e}"))?;
    handle
        .join()
        .map_err(|_| format!("Le thread {thread_name} a paniqué"))?
}

/// Capture le moniteur contenant le point physique (x, y) via WGC et renvoie son
/// image en RGBA. Repli sur le moniteur principal si le point n'est sur aucun
/// moniteur.
///
/// La capture est exécutée sur un thread dédié, neuf : `windows-capture` initialise
/// COM et crée un `DispatcherQueue` + une boucle de messages sur le thread appelant.
/// Lancée directement depuis le callback du raccourci global (thread GUI principal,
/// qui possède déjà un DispatcherQueue / un apartment COM), la crate paniquait en
/// interne, et ce panic traversait la frontière FFI → abort du processus (0xc0000409).
/// Un thread frais résout le conflit ; `catch_unwind` garantit qu'un panic résiduel
/// devient une `Err` au lieu de fermer l'application.
pub fn capture_at_point(x: i32, y: i32) -> Result<RgbaImage, String> {
    run_isolated("wgc-capture", move || capture_at_point_inner(x, y))
}

/// Corps de la capture WGC, exécuté sur le thread dédié de `capture_at_point`.
fn capture_at_point_inner(x: i32, y: i32) -> Result<RgbaImage, String> {
    // windows-capture n'a pas de sélection par point : on récupère le HMONITOR
    // sous le point via Win32 (MONITOR_DEFAULTTONEAREST gère hors-écran et le
    // multi-écran / DPI), puis on l'enveloppe.
    let hmonitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let monitor = Monitor::from_raw_hmonitor(hmonitor.0);

    let (tx, rx) = sync_channel::<Result<RgbaImage, String>>(1);

    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        tx,
    );

    // `start` bloque ce thread et pompe la boucle de messages WGC jusqu'à ce que
    // le handler appelle `stop()` après la première frame.
    OneShot::start(settings).map_err(|e| format!("Capture WGC échouée: {e}"))?;

    rx.recv()
        .map_err(|_| "Aucune frame WGC reçue".to_string())?
}

/// Capture le bitmap complet de la fenêtre au premier plan via WGC (même pipeline
/// que la capture moniteur, mais avec un item "fenêtre"). Renvoie l'image + son
/// rectangle physique. `Ok(None)` si aucune fenêtre au premier plan exploitable.
/// Exécuté sur un thread dédié (comme capture_at_point) pour isoler WGC.
pub fn capture_foreground_window() -> Result<Option<(RgbaImage, crate::capture::Rect)>, String> {
    run_isolated("wgc-window-capture", capture_foreground_window_inner)
}

fn capture_foreground_window_inner() -> Result<Option<(RgbaImage, crate::capture::Rect)>, String> {
    let window = match Window::foreground() {
        Ok(w) => w,
        Err(_) => return Ok(None),
    };
    let rect = window
        .rect()
        .map_err(|e| format!("window.rect a échoué: {e}"))?;
    let (rx, ry) = (rect.left, rect.top);
    let (rw, rh) = (
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    );

    let (tx, rx_chan) = sync_channel::<Result<RgbaImage, String>>(1);
    let settings = Settings::new(
        window,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        tx,
    );
    OneShot::start(settings).map_err(|e| format!("Capture fenêtre WGC échouée: {e}"))?;
    let image = rx_chan
        .recv()
        .map_err(|_| "Aucune frame fenêtre WGC reçue".to_string())??;
    let rect = crate::capture::Rect {
        x: rx.max(0) as u32,
        y: ry.max(0) as u32,
        width: rw,
        height: rh,
    };
    Ok(Some((image, rect)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_row_padding_into_tight_rgba() {
        // Image 2x2, RGBA, avec un stride rembourré de 12 octets (2px*4=8, +4).
        let width = 2u32;
        let height = 2u32;
        let stride = 12usize;
        // Ligne 0 : rouge, vert, puis 4 octets de rembourrage.
        // Ligne 1 : bleu, blanc, puis 4 octets de rembourrage.
        let buffer = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 9, 9, 9, 9, //
            0, 0, 255, 255, 255, 255, 255, 255, 9, 9, 9, 9,
        ];
        let img = frame_to_rgba(&buffer, width, height, stride).unwrap();
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
        assert_eq!(*img.get_pixel(1, 0), image::Rgba([0, 255, 0, 255]));
        assert_eq!(*img.get_pixel(0, 1), image::Rgba([0, 0, 255, 255]));
        assert_eq!(*img.get_pixel(1, 1), image::Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn handles_zero_padding_stride() {
        // stride == width*4 (pas de rembourrage).
        let buffer = vec![
            1, 2, 3, 4, 5, 6, 7, 8, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let img = frame_to_rgba(&buffer, 2, 2, 8).unwrap();
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([1, 2, 3, 4]));
        assert_eq!(*img.get_pixel(1, 1), image::Rgba([13, 14, 15, 16]));
    }

    #[test]
    fn rejects_buffer_too_small() {
        let buffer = vec![0u8; 4]; // bien trop petit pour 2x2
        assert!(frame_to_rgba(&buffer, 2, 2, 8).is_err());
    }

    #[test]
    fn run_isolated_converts_string_panic_to_err() {
        let result: Result<i32, String> =
            run_isolated("test-panic-string", || -> Result<i32, String> {
                panic!("boom");
            });
        assert!(result.is_err());
    }

    #[test]
    fn run_isolated_converts_non_string_panic_to_err() {
        let result: Result<i32, String> =
            run_isolated("test-panic-nonstring", || -> Result<i32, String> {
                std::panic::panic_any(42i32);
            });
        assert!(result.is_err());
    }

    #[test]
    fn run_isolated_passes_through_ok() {
        let result = run_isolated("test-ok", || -> Result<i32, String> { Ok(7) });
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn run_isolated_passes_through_err() {
        let result: Result<i32, String> =
            run_isolated("test-err", || Err("échec attendu".to_string()));
        assert_eq!(result, Err("échec attendu".to_string()));
    }
}
