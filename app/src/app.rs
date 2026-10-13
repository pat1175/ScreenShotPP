//! Boucle d'événements de l'app : une seule boucle winit sur le thread principal,
//! des fenêtres créées/détruites à la demande (réglages, overlay, compteur, HUD,
//! mini-éditeur vidéo), raccourcis globaux et tray branchés sur la même boucle.
//!
//! Les travaux lourds (capture, ffmpeg, OCR, réseau) tournent sur des threads et
//! reviennent ici via `UserEvent::Command`, ou directement à la fenêtre concernée
//! par un canal + `egui::Context::request_repaint`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{WindowAttributes, WindowId};

use crate::capture_flow::{CaptureMode, CaptureSession};
use crate::gl_window::{GlDisplayState, GlWindow, ShowMode};
use crate::hotkey::{HotkeyAction, Hotkeys};
use crate::platform::Monitor;
use crate::settings::Settings;
use crate::ui;
use screenshotpp_core::record::Region;

pub enum UserEvent {
    /// Un contexte egui demande une nouvelle image (éventuellement différée).
    Repaint {
        window: WindowId,
        delay: Duration,
    },
    Hotkey(global_hotkey::GlobalHotKeyEvent),
    #[cfg(any(windows, target_os = "macos"))]
    Menu(tray_icon::menu::MenuEvent),
    Command(Command),
}

/// Résultats des threads de travail, à traiter sur le thread principal.
pub enum Command {
    OpenOverlay(Box<CaptureSession>),
    RecordingStarted { monitor: Monitor, region: Region },
    RecordingStopped,
    UpdateAvailable(crate::updater::UpdateInfo),
    Error(String),
}

/// Demandes émises par les fenêtres pendant une image, appliquées juste après.
pub enum Action {
    Close(WindowId),
    Hide(WindowId),
    /// Fin du compte à rebours : fermer le compteur puis capturer.
    CountdownFinished(CaptureMode),
    /// Sélection vidéo validée dans l'overlay.
    StartRecording {
        monitor: Monitor,
        region: Region,
    },
    StopRecording,
    Quit,
}

/// État et services partagés, accessibles aux fenêtres via `WinCx`.
pub struct Services {
    pub settings: Settings,
    pub hotkeys: Option<Hotkeys>,
    pub recording: crate::record::Recording,
    pub proxy: EventLoopProxy<UserEvent>,
    actions: Vec<Action>,
}

impl Services {
    pub fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// Applique et persiste de nouveaux réglages : raccourcis globaux, lancement
    /// à l'ouverture de session, fichier. En cas d'échec, rien n'est persisté et
    /// les anciens raccourcis sont réenregistrés.
    pub fn apply_settings(&mut self, new: Settings) -> Result<(), String> {
        if let Some(hk) = self.hotkeys.as_mut() {
            if let Err(e) = hk.register_all(&new) {
                let _ = hk.register_all(&self.settings);
                return Err(e);
            }
        }
        crate::autostart::set_enabled(new.launch_at_login)?;
        crate::settings::save(&new)?;
        self.settings = new;
        Ok(())
    }

    /// Persiste une préférence d'interface (couleur, taille de sortie, version
    /// ignorée) sans toucher aux raccourcis.
    pub fn save_ui_prefs(&mut self, update: impl FnOnce(&mut Settings)) {
        update(&mut self.settings);
        if let Err(e) = crate::settings::save(&self.settings) {
            eprintln!("Sauvegarde des préférences échouée: {e}");
        }
    }
}

/// Contexte passé à une fenêtre pendant son image.
pub struct WinCx<'a> {
    pub id: WindowId,
    pub window: &'a winit::window::Window,
    pub services: &'a mut Services,
}

impl WinCx<'_> {
    pub fn close(&mut self) {
        self.services.push(Action::Close(self.id));
    }
}

pub trait WindowUi: std::any::Any {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut WinCx);
    /// Couleur de fond GL (prémultipliée). Transparente par défaut : chaque UI
    /// peint son propre fond.
    fn clear_color(&self) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    /// Croix de fermeture / Alt+F4 : ferme la fenêtre par défaut.
    fn close_requested(&mut self, cx: &mut WinCx) {
        cx.close();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Settings,
    Overlay,
    Countdown,
    HudBorder(u8),
    HudControls,
    Recorder,
}

struct Managed {
    kind: Kind,
    gl: GlWindow,
    ui: Box<dyn WindowUi>,
}

enum Timer {
    Capture(CaptureMode),
    StartRecording { monitor: Monitor, region: Region },
}

pub struct App {
    services: Services,
    gl_state: Option<GlDisplayState>,
    windows: HashMap<WindowId, Managed>,
    repaint_at: HashMap<WindowId, Instant>,
    timers: Vec<(Instant, Timer)>,
    #[cfg(any(windows, target_os = "macos"))]
    tray: Option<crate::tray::Tray>,
    initialized: bool,
}

pub fn run() {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let event_loop = builder.build().expect("boucle d'événements");
    let proxy = event_loop.create_proxy();

    {
        let proxy = proxy.clone();
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Hotkey(e));
        }));
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let proxy = proxy.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Menu(e));
        }));
    }

    let mut app = App {
        services: Services {
            settings: crate::settings::load(),
            hotkeys: None,
            recording: crate::record::Recording::default(),
            proxy,
            actions: Vec::new(),
        },
        gl_state: None,
        windows: HashMap::new(),
        repaint_at: HashMap::new(),
        timers: Vec::new(),
        #[cfg(any(windows, target_os = "macos"))]
        tray: None,
        initialized: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("Boucle d'événements terminée en erreur: {e}");
    }
}

impl App {
    fn find(&self, kind: Kind) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, m)| m.kind == kind)
            .map(|(id, _)| *id)
    }

    fn has(&self, kind: Kind) -> bool {
        self.find(kind).is_some()
    }

    fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        kind: Kind,
        attrs: WindowAttributes,
        show: Option<ShowMode>,
        ui: Box<dyn WindowUi>,
    ) -> Option<WindowId> {
        let gl = match GlWindow::new(event_loop, &mut self.gl_state, attrs, show) {
            Ok(gl) => gl,
            Err(e) => {
                eprintln!("{e}");
                crate::dialogs::error(None, &e);
                return None;
            }
        };
        let id = gl.window.id();
        let proxy = self.services.proxy.clone();
        gl.egui.egui_ctx.set_request_repaint_callback(move |info| {
            let _ = proxy.send_event(UserEvent::Repaint {
                window: id,
                delay: info.delay,
            });
        });
        gl.window.request_redraw();
        self.windows.insert(id, Managed { kind, gl, ui });
        Some(id)
    }

    fn close(&mut self, id: WindowId) {
        self.repaint_at.remove(&id);
        self.windows.remove(&id);
    }

    fn close_kind(&mut self, kind: Kind) {
        let ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, m)| m.kind == kind)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.close(id);
        }
    }

    fn close_hud(&mut self) {
        let ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, m)| matches!(m.kind, Kind::HudBorder(_) | Kind::HudControls))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.close(id);
        }
    }

    fn show_settings(&mut self) {
        if let Some(id) = self.find(Kind::Settings) {
            if let Some(m) = self.windows.get_mut(&id) {
                m.gl.show(ShowMode::Focus);
            }
        }
    }

    fn settings_ui(&mut self) -> Option<&mut ui::settings::SettingsUi> {
        let id = self.find(Kind::Settings)?;
        let m = self.windows.get_mut(&id)?;
        (m.ui.as_mut() as &mut dyn std::any::Any).downcast_mut()
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) {
        // La fenêtre de réglages est créée d'emblée (cachée) : elle porte aussi le
        // display OpenGL partagé, et reste en vie jusqu'à la sortie de l'app.
        self.open(
            event_loop,
            Kind::Settings,
            ui::settings::attributes(),
            None,
            Box::new(ui::settings::SettingsUi::new(&self.services.settings)),
        );

        #[cfg(any(windows, target_os = "macos"))]
        {
            match crate::tray::Tray::new() {
                Ok(t) => self.tray = Some(t),
                Err(e) => eprintln!("Tray indisponible: {e}"),
            }
        }

        match Hotkeys::new() {
            Ok(mut hk) => {
                if let Err(e) = hk.register_all(&self.services.settings) {
                    crate::dialogs::error(None, &e);
                }
                self.services.hotkeys = Some(hk);
            }
            Err(e) => eprintln!("Raccourcis globaux indisponibles: {e}"),
        }
        let _ = crate::autostart::set_enabled(self.services.settings.launch_at_login);
        crate::updater::check_in_background(self.services.proxy.clone());
    }

    // ---- Flux de capture ------------------------------------------------------

    fn on_hotkey(&mut self, event_loop: &ActiveEventLoop, action: HotkeyAction) {
        match action {
            HotkeyAction::Capture => self.request_capture(CaptureMode::Image),
            HotkeyAction::DelayedCapture => {
                let secs = self.services.settings.capture_delay_secs.max(1);
                self.start_countdown(event_loop, CaptureMode::Image, secs);
            }
            HotkeyAction::Record => {
                let recording = self.services.recording.is_active();
                let busy = self.has(Kind::Overlay) || self.has(Kind::Countdown);
                match screenshotpp_core::record::on_record_shortcut(recording, busy) {
                    screenshotpp_core::record::ShortcutAction::Stop => self.stop_recording(),
                    screenshotpp_core::record::ShortcutAction::StartSelection => {
                        let delay = self.services.settings.record_delay_secs.min(60);
                        if delay == 0 {
                            self.request_capture(CaptureMode::Video);
                        } else {
                            self.start_countdown(event_loop, CaptureMode::Video, delay);
                        }
                    }
                    screenshotpp_core::record::ShortcutAction::Ignore => {}
                }
            }
            HotkeyAction::CancelCountdown => self.end_countdown(),
        }
    }

    /// Capture sur un thread (la capture WGC/xcap prend des dizaines de ms), puis
    /// `Command::OpenOverlay`. Un overlay déjà ouvert est fermé d'abord et la
    /// capture légèrement différée, pour qu'il n'apparaisse pas dans l'image.
    fn request_capture(&mut self, mode: CaptureMode) {
        if self.has(Kind::Overlay) {
            self.close_kind(Kind::Overlay);
            self.timers.push((
                Instant::now() + Duration::from_millis(80),
                Timer::Capture(mode),
            ));
            return;
        }
        let proxy = self.services.proxy.clone();
        std::thread::spawn(move || {
            let command = match crate::capture_flow::capture_session(mode) {
                Ok(session) => Command::OpenOverlay(Box::new(session)),
                Err(e) => Command::Error(format!("Capture échouée: {e}")),
            };
            let _ = proxy.send_event(UserEvent::Command(command));
        });
    }

    fn start_countdown(&mut self, event_loop: &ActiveEventLoop, mode: CaptureMode, secs: u32) {
        if self.has(Kind::Countdown) {
            return; // double appui : un seul décompte à la fois
        }
        let monitor = match crate::platform::monitor_at_cursor() {
            Ok((m, _)) => m,
            Err(e) => {
                crate::dialogs::error(None, &format!("Capture différée échouée: {e}"));
                return;
            }
        };
        let cancel = self.services.settings.cancel_shortcut.clone();
        if let Some(hk) = self.services.hotkeys.as_mut() {
            if let Err(e) = hk.register_cancel(&cancel) {
                crate::dialogs::error(
                    None,
                    &format!("Échec d'enregistrement du raccourci d'annulation: {e}"),
                );
                return;
            }
        }
        self.open(
            event_loop,
            Kind::Countdown,
            ui::countdown::attributes(),
            Some(ShowMode::NoFocus),
            Box::new(ui::countdown::CountdownUi::new(mode, secs, monitor)),
        );
        if let Some(id) = self.find(Kind::Countdown) {
            if let Some(m) = self.windows.get(&id) {
                let _ = m.gl.window.set_cursor_hittest(false);
            }
        }
    }

    fn end_countdown(&mut self) {
        self.close_kind(Kind::Countdown);
        if let Some(hk) = self.services.hotkeys.as_mut() {
            hk.unregister_cancel();
        }
    }

    fn open_overlay(&mut self, event_loop: &ActiveEventLoop, session: CaptureSession) {
        self.close_kind(Kind::Overlay);
        let attrs = ui::overlay::attributes(&session.monitor);
        let overlay = ui::overlay::OverlayUi::new(session, &self.services.settings);
        self.open(
            event_loop,
            Kind::Overlay,
            attrs,
            Some(ShowMode::Focus),
            Box::new(overlay),
        );
    }

    // ---- Flux d'enregistrement -----------------------------------------------

    fn spawn_recording(&mut self, monitor: Monitor, region: Region) {
        let recording = self.services.recording.clone();
        let settings = self.services.settings.clone();
        let proxy = self.services.proxy.clone();
        std::thread::spawn(move || {
            let command = match recording.start(monitor.index, region, &settings) {
                Ok(()) => Command::RecordingStarted { monitor, region },
                Err(e) => Command::Error(format!("Démarrage d'enregistrement échoué: {e}")),
            };
            let _ = proxy.send_event(UserEvent::Command(command));
        });
    }

    fn open_hud(&mut self, event_loop: &ActiveEventLoop, monitor: Monitor, region: Region) {
        self.close_hud();
        for (i, attrs) in ui::hud::border_attributes(&monitor, region)
            .into_iter()
            .enumerate()
        {
            if let Some(id) = self.open(
                event_loop,
                Kind::HudBorder(i as u8),
                attrs,
                Some(ShowMode::NoFocus),
                Box::new(ui::hud::BorderUi::new()),
            ) {
                if let Some(m) = self.windows.get(&id) {
                    let _ = m.gl.window.set_cursor_hittest(false);
                }
            }
        }
        self.open(
            event_loop,
            Kind::HudControls,
            ui::hud::controls_attributes(&monitor, region),
            Some(ShowMode::NoFocus),
            Box::new(ui::hud::ControlsUi::new(self.services.recording.clone())),
        );
        #[cfg(any(windows, target_os = "macos"))]
        if let Some(t) = self.tray.as_mut() {
            t.set_recording(true);
        }
    }

    fn stop_recording(&mut self) {
        let recording = self.services.recording.clone();
        let proxy = self.services.proxy.clone();
        std::thread::spawn(move || {
            let command = match recording.stop() {
                Ok(()) => Command::RecordingStopped,
                Err(e) => Command::Error(format!("Arrêt d'enregistrement échoué: {e}")),
            };
            let _ = proxy.send_event(UserEvent::Command(command));
        });
    }

    fn on_command(&mut self, event_loop: &ActiveEventLoop, command: Command) {
        match command {
            Command::OpenOverlay(session) => self.open_overlay(event_loop, *session),
            Command::RecordingStarted { monitor, region } => {
                self.open_hud(event_loop, monitor, region)
            }
            Command::RecordingStopped => {
                self.close_hud();
                #[cfg(any(windows, target_os = "macos"))]
                if let Some(t) = self.tray.as_mut() {
                    t.set_recording(false);
                }
                self.close_kind(Kind::Recorder);
                match self.services.recording.last_file() {
                    Some(file) => {
                        self.open(
                            event_loop,
                            Kind::Recorder,
                            ui::recorder::attributes(),
                            Some(ShowMode::Focus),
                            Box::new(ui::recorder::RecorderUi::new(
                                file,
                                self.services.recording.clone(),
                            )),
                        );
                    }
                    None => crate::dialogs::error(None, "Aucun enregistrement disponible"),
                }
            }
            Command::UpdateAvailable(info) => {
                let skipped = self.services.settings.skipped_update_version.clone();
                if let Some(s) = self.settings_ui() {
                    if s.present_update(info, &skipped, true) {
                        self.show_settings();
                    }
                }
            }
            Command::Error(message) => {
                eprintln!("{message}");
                // Échec pendant l'enregistrement : HUD et tray reviennent à l'état normal.
                if !self.services.recording.is_active() {
                    self.close_hud();
                    #[cfg(any(windows, target_os = "macos"))]
                    if let Some(t) = self.tray.as_mut() {
                        t.set_recording(false);
                    }
                }
                crate::dialogs::error(None, &message);
            }
        }
    }

    fn apply_actions(&mut self, event_loop: &ActiveEventLoop) {
        let actions = std::mem::take(&mut self.services.actions);
        for action in actions {
            match action {
                Action::Close(id) => self.close(id),
                Action::Hide(id) => {
                    if let Some(m) = self.windows.get(&id) {
                        m.gl.window.set_visible(false);
                    }
                }
                Action::CountdownFinished(mode) => {
                    self.end_countdown();
                    // Petit répit pour que le compositeur retire le compteur.
                    self.timers.push((
                        Instant::now() + Duration::from_millis(60),
                        Timer::Capture(mode),
                    ));
                }
                Action::StartRecording { monitor, region } => {
                    self.close_kind(Kind::Overlay);
                    // Laisse le compositeur retirer l'overlay avant que ffmpeg capture.
                    self.timers.push((
                        Instant::now() + Duration::from_millis(120),
                        Timer::StartRecording { monitor, region },
                    ));
                }
                Action::StopRecording => self.stop_recording(),
                Action::Quit => event_loop.exit(),
            }
        }
    }

    fn redraw(&mut self, id: WindowId) {
        let Some(m) = self.windows.get_mut(&id) else {
            return;
        };
        let clear = m.ui.clear_color();
        let ui = &mut m.ui;
        let services = &mut self.services;
        m.gl.redraw(clear, |egui_ui, window| {
            let mut cx = WinCx {
                id,
                window,
                services,
            };
            ui.ui(egui_ui, &mut cx);
        });
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init && !self.initialized {
            self.initialized = true;
            self.init(event_loop);
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Repaint { window, delay } => {
                if let Some(m) = self.windows.get(&window) {
                    if delay.is_zero() {
                        m.gl.window.request_redraw();
                    } else if let Some(at) = Instant::now().checked_add(delay) {
                        let entry = self.repaint_at.entry(window).or_insert(at);
                        *entry = (*entry).min(at);
                    }
                }
            }
            UserEvent::Hotkey(e) => {
                if e.state() != global_hotkey::HotKeyState::Pressed {
                    return;
                }
                let action = self
                    .services
                    .hotkeys
                    .as_ref()
                    .and_then(|h| h.action(e.id()));
                if let Some(action) = action {
                    self.on_hotkey(event_loop, action);
                }
            }
            #[cfg(any(windows, target_os = "macos"))]
            UserEvent::Menu(e) => match crate::tray::menu_action(&e) {
                Some(crate::tray::MenuAction::Settings) => self.show_settings(),
                Some(crate::tray::MenuAction::StopRecording) => self.stop_recording(),
                Some(crate::tray::MenuAction::Quit) => event_loop.exit(),
                None => {}
            },
            UserEvent::Command(c) => self.on_command(event_loop, c),
        }
        self.apply_actions(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::RedrawRequested => {
                self.repaint_at.remove(&id);
                self.redraw(id);
            }
            WindowEvent::CloseRequested => {
                if let Some(m) = self.windows.get_mut(&id) {
                    let window = &m.gl.window;
                    let mut cx = WinCx {
                        id,
                        window,
                        services: &mut self.services,
                    };
                    m.ui.close_requested(&mut cx);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(m) = self.windows.get(&id) {
                    m.gl.resize(*size);
                    m.gl.window.request_redraw();
                }
            }
            _ => {}
        }
        if let Some(m) = self.windows.get_mut(&id) {
            let response = m.gl.egui.on_window_event(&m.gl.window, &event);
            if response.repaint {
                m.gl.window.request_redraw();
            }
        }
        self.apply_actions(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        let due: Vec<Timer> = {
            let (due, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.timers)
                .into_iter()
                .partition(|(at, _)| *at <= now);
            self.timers = pending;
            due.into_iter().map(|(_, t)| t).collect()
        };
        for timer in due {
            match timer {
                Timer::Capture(mode) => self.request_capture(mode),
                Timer::StartRecording { monitor, region } => self.spawn_recording(monitor, region),
            }
        }

        let due_windows: Vec<WindowId> = self
            .repaint_at
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due_windows {
            self.repaint_at.remove(&id);
            if let Some(m) = self.windows.get(&id) {
                m.gl.window.request_redraw();
            }
        }

        let next = self
            .repaint_at
            .values()
            .chain(self.timers.iter().map(|(at, _)| at))
            .min()
            .copied();
        event_loop.set_control_flow(match next {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
