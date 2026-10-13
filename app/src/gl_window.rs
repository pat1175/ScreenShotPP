//! Une fenêtre native = une fenêtre winit + un contexte OpenGL (glutin) + un
//! contexte egui (egui_glow). Chaque fenêtre de l'app est indépendante : pas de
//! partage de textures, pas d'état egui commun — fermer une fenêtre libère tout.

use std::num::NonZeroU32;
use std::sync::Arc;

use egui_glow::glow;
use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext,
    PossiblyCurrentGlContext,
};
use glutin::display::{Display, GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::HasWindowHandle;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

/// Display + config OpenGL partagés par toutes les fenêtres. Créés une seule fois,
/// avec la première fenêtre (la fenêtre de réglages, jamais détruite : sous WGL le
/// display reste ainsi adossé à une fenêtre vivante).
pub struct GlDisplayState {
    display: Display,
    config: Config,
}

pub struct GlWindow {
    // Ordre de drop : egui (painter) avant la surface/le contexte GL, puis la fenêtre.
    pub egui: egui_glow::EguiGlow,
    gl_surface: Surface<WindowSurface>,
    gl_context: PossiblyCurrentContext,
    gl: Arc<glow::Context>,
    pub window: Window,
    /// Fenêtre créée invisible, affichée après la première image peinte (pas de flash).
    pending_show: Option<ShowMode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShowMode {
    /// Afficher sans prendre le focus (compteur, HUD).
    NoFocus,
    /// Afficher et donner le focus clavier (overlay, éditeurs).
    Focus,
}

impl GlWindow {
    /// Crée la fenêtre (toujours invisible au départ) et son contexte GL.
    /// `show` : comment l'afficher après la première image ; `None` = rester cachée.
    pub fn new(
        event_loop: &ActiveEventLoop,
        gl_state: &mut Option<GlDisplayState>,
        attributes: WindowAttributes,
        show: Option<ShowMode>,
    ) -> Result<Self, String> {
        let attributes = attributes.with_visible(false);
        let window = match gl_state {
            Some(state) => glutin_winit::finalize_window(event_loop, attributes, &state.config)
                .map_err(|e| format!("Création de fenêtre échouée: {e}"))?,
            None => {
                let template = ConfigTemplateBuilder::new()
                    .prefer_hardware_accelerated(None)
                    .with_depth_size(0)
                    .with_stencil_size(0)
                    .with_transparency(true);
                let (window, config) = glutin_winit::DisplayBuilder::new()
                    .with_preference(glutin_winit::ApiPreference::FallbackEgl)
                    .with_window_attributes(Some(attributes.clone()))
                    .build(event_loop, template, |configs| {
                        // Préfère une config avec canal alpha (fenêtres transparentes :
                        // compteur, barre du HUD) ; sinon la première venue.
                        let configs: Vec<Config> = configs.collect();
                        configs
                            .iter()
                            .find(|c| c.supports_transparency() == Some(true))
                            .cloned()
                            .unwrap_or_else(|| configs[0].clone())
                    })
                    .map_err(|e| format!("Initialisation OpenGL échouée: {e}"))?;
                let window = match window {
                    Some(w) => w,
                    None => glutin_winit::finalize_window(event_loop, attributes, &config)
                        .map_err(|e| format!("Création de fenêtre échouée: {e}"))?,
                };
                *gl_state = Some(GlDisplayState {
                    display: config.display(),
                    config,
                });
                window
            }
        };
        let state = gl_state.as_ref().expect("display GL initialisé ci-dessus");

        let raw = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        let context_attributes = ContextAttributesBuilder::new().build(Some(raw));
        let fallback_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(None))
            .build(Some(raw));
        // SAFETY: `raw` provient d'une fenêtre vivante, possédée par ce GlWindow.
        let not_current = unsafe {
            state
                .display
                .create_context(&state.config, &context_attributes)
                .or_else(|_| {
                    state
                        .display
                        .create_context(&state.config, &fallback_attributes)
                })
                .map_err(|e| format!("Contexte OpenGL indisponible: {e}"))?
        };

        let (w, h): (u32, u32) = window.inner_size().into();
        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw,
            NonZeroU32::new(w).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(h).unwrap_or(NonZeroU32::MIN),
        );
        // SAFETY: idem, la surface vit moins longtemps que la fenêtre (ordre des champs).
        let gl_surface = unsafe {
            state
                .display
                .create_window_surface(&state.config, &surface_attributes)
                .map_err(|e| format!("Surface OpenGL indisponible: {e}"))?
        };
        let gl_context = not_current
            .make_current(&gl_surface)
            .map_err(|e| e.to_string())?;
        let _ = gl_surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::MIN));

        // SAFETY: le contexte est courant ; les pointeurs de fonctions sont résolus
        // par le display qui l'a créé.
        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function_cstr(|s| state.display.get_proc_address(s))
        });
        let egui = egui_glow::EguiGlow::new(
            event_loop,
            gl.clone(),
            None,
            Some(window.scale_factor() as f32),
            true,
        );
        crate::ui::style::apply(&egui.egui_ctx);

        Ok(GlWindow {
            egui,
            gl_surface,
            gl_context,
            gl,
            window,
            pending_show: show,
        })
    }

    fn make_current(&self) {
        if !self.gl_context.is_current() {
            let _ = self.gl_context.make_current(&self.gl_surface);
        }
    }

    pub fn resize(&self, size: winit::dpi::PhysicalSize<u32>) {
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            self.make_current();
            self.gl_surface.resize(&self.gl_context, w, h);
        }
    }

    /// Une passe egui + peinture + swap. Affiche la fenêtre après la première image.
    pub fn redraw(&mut self, clear: [f32; 4], mut run_ui: impl FnMut(&mut egui::Ui, &Window)) {
        self.make_current();
        let window = &self.window;
        self.egui.run(window, |ui| run_ui(ui, window));
        // SAFETY: contexte courant (make_current ci-dessus).
        unsafe {
            use glow::HasContext as _;
            self.gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.egui.paint(&self.window);
        let _ = self.gl_surface.swap_buffers(&self.gl_context);
        if let Some(mode) = self.pending_show.take() {
            self.window.set_visible(true);
            if mode == ShowMode::Focus {
                self.window.focus_window();
            }
        }
    }

    /// Affiche (et focalise) immédiatement, puis demande une image : une fenêtre
    /// cachée ne reçoit pas toujours `RedrawRequested` (macOS), on ne peut donc pas
    /// attendre la première image pour l'afficher.
    pub fn show(&mut self, mode: ShowMode) {
        self.window.set_visible(true);
        if mode == ShowMode::Focus {
            self.window.focus_window();
        }
        self.window.request_redraw();
    }
}

impl Drop for GlWindow {
    fn drop(&mut self) {
        self.make_current();
        self.egui.destroy();
    }
}
