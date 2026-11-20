//! Fenêtre de réglages (ouverte depuis le tray ; la fermer la cache seulement)
//! et bannière de mise à jour.

use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use egui::{RichText, Ui};
use winit::window::WindowAttributes;

use crate::app::{Action, WinCx, WindowUi};
use crate::settings::Settings;
use crate::ui::style;
use crate::updater::UpdateInfo;

pub fn attributes() -> WindowAttributes {
    winit::window::Window::default_attributes()
        .with_title("ScreenShotPP — Settings")
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 620.0))
        .with_min_inner_size(winit::dpi::LogicalSize::new(560.0, 420.0))
        .with_window_icon(crate::icon::window_icon())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutField {
    Capture,
    Delayed,
    Cancel,
    Record,
}

impl ShortcutField {
    fn get(self, s: &Settings) -> &str {
        match self {
            ShortcutField::Capture => &s.capture_shortcut,
            ShortcutField::Delayed => &s.delayed_capture_shortcut,
            ShortcutField::Cancel => &s.cancel_shortcut,
            ShortcutField::Record => &s.record_shortcut,
        }
    }

    fn set(self, s: &mut Settings, value: String) {
        match self {
            ShortcutField::Capture => s.capture_shortcut = value,
            ShortcutField::Delayed => s.delayed_capture_shortcut = value,
            ShortcutField::Cancel => s.cancel_shortcut = value,
            ShortcutField::Record => s.record_shortcut = value,
        }
    }
}

/// Résultat d'une vérification manuelle (thread réseau → UI).
type CheckResult = Result<Option<UpdateInfo>, String>;

enum InstallState {
    Idle,
    Running(Arc<Mutex<String>>, Receiver<Result<(), String>>),
}

pub struct SettingsUi {
    /// Copie éditée ; appliquée (raccourcis, autostart, fichier) à chaque changement.
    draft: Settings,
    recording_field: Option<ShortcutField>,
    error: Option<String>,
    update_status: String,
    checking: Option<Receiver<CheckResult>>,
    banner: Option<UpdateInfo>,
    banner_status: String,
    install: InstallState,
}

impl SettingsUi {
    pub fn new(settings: &Settings) -> Self {
        SettingsUi {
            draft: settings.clone(),
            recording_field: None,
            error: None,
            update_status: String::new(),
            checking: None,
            banner: None,
            banner_status: String::new(),
            install: InstallState::Idle,
        }
    }

    /// Présente une mise à jour. Auto : ignorée si c'est la version « skippée ».
    /// Renvoie `true` si la bannière est affichée (la fenêtre doit alors s'ouvrir).
    pub fn present_update(&mut self, info: UpdateInfo, skipped: &str, auto: bool) -> bool {
        if auto && info.version == skipped {
            return false;
        }
        self.banner = Some(info);
        self.banner_status.clear();
        true
    }

    fn apply(&mut self, cx: &mut WinCx) {
        match cx.services.apply_settings(self.draft.clone()) {
            Ok(()) => self.error = None,
            Err(e) => {
                // Échec : on revient aux réglages effectifs.
                self.draft = cx.services.settings.clone();
                self.error = Some(format!("Could not save settings: {e}"));
            }
        }
    }

    fn capture_shortcut_key(&mut self, ui: &Ui, cx: &mut WinCx) {
        let Some(field) = self.recording_field else {
            return;
        };
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = pressed else {
            return;
        };
        // Pour le champ d'annulation, Escape est une valeur valide ; ailleurs il annule.
        if key == egui::Key::Escape && field != ShortcutField::Cancel {
            self.recording_field = None;
            return;
        }
        let Some(acc) = crate::hotkey::accelerator_from_key(key, modifiers) else {
            return; // attend une vraie touche
        };
        self.recording_field = None;
        field.set(&mut self.draft, acc);
        self.apply(cx);
    }

    fn shortcut_button(&mut self, ui: &mut Ui, field: ShortcutField) {
        let text = if self.recording_field == Some(field) {
            "Press a combination…".to_string()
        } else {
            field.get(&self.draft).to_string()
        };
        let button = egui::Button::new(RichText::new(text).monospace())
            .min_size(egui::vec2(ui.available_width(), 30.0));
        if ui.add(button).clicked() {
            self.recording_field = Some(field);
        }
    }

    fn poll_workers(&mut self, cx: &mut WinCx) {
        if let Some(rx) = &self.checking {
            if let Ok(result) = rx.try_recv() {
                self.checking = None;
                match result {
                    Ok(Some(info)) => {
                        // Vérification manuelle : ignore la version « skippée ».
                        self.update_status.clear();
                        self.present_update(info, "", false);
                    }
                    Ok(None) => self.update_status = "You're up to date.".into(),
                    Err(e) => self.update_status = format!("Update failed: {e}"),
                }
            }
        }
        if let InstallState::Running(status, rx) = &self.install {
            self.banner_status = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(()) => cx.services.push(Action::Quit),
                    Err(e) => self.banner_status = format!("Update failed: {e}"),
                }
                self.install = InstallState::Idle;
            }
        }
    }

    fn banner_ui(&mut self, ui: &mut Ui, cx: &mut WinCx) {
        let Some(info) = self.banner.clone() else {
            return;
        };
        let installing = matches!(self.install, InstallState::Running(..));
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(0x12, 0x26, 0x3d))
            .stroke(egui::Stroke::new(1.0, style::ACCENT))
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("UPDATE AVAILABLE")
                                .size(11.0)
                                .color(style::HEADING),
                        );
                        ui.label(
                            RichText::new(format!("Version {}", info.version))
                                .size(16.0)
                                .strong(),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.link("Changelog ↗").clicked() {
                            let _ = opener::open(crate::updater::RELEASES_URL);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!installing, |ui| {
                        if ui.add(style::primary_button("Install")).clicked() {
                            self.start_install(info.clone(), ui.ctx().clone());
                        }
                        if ui.button("Later").clicked() {
                            self.banner = None;
                        }
                        if ui.button("Skip this version").clicked() {
                            let v = info.version.clone();
                            cx.services.save_ui_prefs(|s| s.skipped_update_version = v);
                            self.banner = None;
                        }
                    });
                    ui.label(RichText::new(&self.banner_status).color(style::MUTED));
                });
            });
        ui.add_space(6.0);
    }

    fn start_install(&mut self, info: UpdateInfo, ctx: egui::Context) {
        let status = Arc::new(Mutex::new("Downloading…".to_string()));
        let (tx, rx) = channel();
        let status2 = status.clone();
        std::thread::spawn(move || {
            let ctx2 = ctx.clone();
            let result = crate::updater::download_and_install(&info, |done, total| {
                let text = match total {
                    Some(t) if t > 0 => format!("Downloading… {}%", done * 100 / t),
                    _ => "Downloading…".to_string(),
                };
                *status2.lock().unwrap_or_else(|e| e.into_inner()) = text;
                ctx2.request_repaint();
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.install = InstallState::Running(status, rx);
    }
}

fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(12.0).color(style::MUTED));
        add(ui);
    });
}

const OCR_LANGUAGES: [(&str, &str); 5] = [
    ("auto", "Auto-detect"),
    ("en-US", "English"),
    ("fr-FR", "Français"),
    ("es-ES", "Español"),
    ("de-DE", "Deutsch"),
];

impl WindowUi for SettingsUi {
    fn clear_color(&self) -> [f32; 4] {
        style::BG.to_normalized_gamma_f32()
    }

    fn close_requested(&mut self, cx: &mut WinCx) {
        self.recording_field = None;
        cx.services.push(Action::Hide(cx.id));
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut WinCx) {
        self.poll_workers(cx);
        self.capture_shortcut_key(ui, cx);
        if self.checking.is_some() || matches!(self.install, InstallState::Running(..)) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(style::BG).inner_margin(22))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_max_width(760.0);
                    self.banner_ui(ui, cx);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Settings").size(20.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("Version {}", crate::paths::VERSION))
                                    .color(style::MUTED),
                            );
                        });
                    });
                    if let Some(e) = &self.error {
                        ui.colored_label(style::DANGER, e);
                    }

                    // Deux colonnes : espacement de grille (14) + barre de défilement (~16).
                    let col = (ui.available_width() - 14.0 - 16.0) / 2.0;
                    let mut changed = false;

                    ui.separator();
                    style::section_heading(ui, "Capture");
                    egui::Grid::new("capture")
                        .num_columns(2)
                        .spacing([14.0, 12.0])
                        .min_col_width(col)
                        .show(ui, |ui| {
                            field(ui, "Capture shortcut", |ui| {
                                self.shortcut_button(ui, ShortcutField::Capture)
                            });
                            field(ui, "Delayed shortcut", |ui| {
                                self.shortcut_button(ui, ShortcutField::Delayed)
                            });
                            ui.end_row();
                            field(ui, "Capture delay (s)", |ui| {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut self.draft.capture_delay_secs)
                                            .range(1..=60),
                                    )
                                    .changed();
                            });
                            field(ui, "Cancel countdown", |ui| {
                                self.shortcut_button(ui, ShortcutField::Cancel)
                            });
                            ui.end_row();
                        });

                    ui.separator();
                    style::section_heading(ui, "Recording");
                    egui::Grid::new("recording")
                        .num_columns(2)
                        .spacing([14.0, 12.0])
                        .min_col_width(col)
                        .show(ui, |ui| {
                            field(ui, "Record shortcut", |ui| {
                                self.shortcut_button(ui, ShortcutField::Record)
                            });
                            field(ui, "Recording delay (s)", |ui| {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut self.draft.record_delay_secs)
                                            .range(0..=60),
                                    )
                                    .changed();
                            });
                            ui.end_row();
                            field(ui, "Recording FPS", |ui| {
                                egui::ComboBox::from_id_salt("fps")
                                    .selected_text(self.draft.record_fps.to_string())
                                    .show_ui(ui, |ui| {
                                        for fps in [24u32, 30, 60] {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut self.draft.record_fps,
                                                    fps,
                                                    fps.to_string(),
                                                )
                                                .changed();
                                        }
                                    });
                            });
                            field(ui, "Cursor", |ui| {
                                changed |= ui
                                    .checkbox(&mut self.draft.record_cursor, "Show cursor")
                                    .changed();
                            });
                            ui.end_row();
                        });

                    ui.separator();
                    style::section_heading(ui, "Files");
                    field(ui, "Default save folder", |ui| {
                        ui.horizontal(|ui| {
                            let shown = if self.draft.default_save_folder.is_empty() {
                                "Desktop".to_string()
                            } else {
                                self.draft.default_save_folder.clone()
                            };
                            ui.label(RichText::new(shown).monospace());
                            if ui.button("Choose…").clicked() {
                                if let Some(dir) =
                                    rfd::FileDialog::new().set_parent(cx.window).pick_folder()
                                {
                                    self.draft.default_save_folder =
                                        dir.to_string_lossy().to_string();
                                    changed = true;
                                }
                            }
                        });
                    });
                    egui::Grid::new("files")
                        .num_columns(2)
                        .spacing([14.0, 12.0])
                        .min_col_width(col)
                        .show(ui, |ui| {
                            field(ui, "Default format", |ui| {
                                egui::ComboBox::from_id_salt("format")
                                    .selected_text(if self.draft.default_format == "jpeg" {
                                        "JPEG"
                                    } else {
                                        "PNG"
                                    })
                                    .show_ui(ui, |ui| {
                                        changed |= ui
                                            .selectable_value(
                                                &mut self.draft.default_format,
                                                "png".into(),
                                                "PNG",
                                            )
                                            .changed();
                                        changed |= ui
                                            .selectable_value(
                                                &mut self.draft.default_format,
                                                "jpeg".into(),
                                                "JPEG",
                                            )
                                            .changed();
                                    });
                            });
                            field(ui, "OCR language", |ui| {
                                let current = OCR_LANGUAGES
                                    .iter()
                                    .find(|(code, _)| *code == self.draft.ocr_language)
                                    .map(|(_, name)| *name)
                                    .unwrap_or("Auto-detect");
                                egui::ComboBox::from_id_salt("ocr")
                                    .selected_text(current)
                                    .show_ui(ui, |ui| {
                                        for (code, name) in OCR_LANGUAGES {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut self.draft.ocr_language,
                                                    code.to_string(),
                                                    name,
                                                )
                                                .changed();
                                        }
                                    });
                            });
                            ui.end_row();
                        });

                    ui.separator();
                    style::section_heading(ui, "App");
                    egui::Grid::new("app")
                        .num_columns(2)
                        .spacing([14.0, 12.0])
                        .min_col_width(col)
                        .show(ui, |ui| {
                            field(ui, "Startup", |ui| {
                                changed |= ui
                                    .checkbox(&mut self.draft.launch_at_login, "Launch at login")
                                    .changed();
                            });
                            field(ui, "Updates", |ui| {
                                ui.horizontal(|ui| {
                                    let busy = self.checking.is_some();
                                    if ui.add_enabled(!busy, egui::Button::new("Check")).clicked() {
                                        self.update_status = "Checking…".into();
                                        let (tx, rx) = channel();
                                        let ctx = ui.ctx().clone();
                                        std::thread::spawn(move || {
                                            let _ = tx.send(crate::updater::check());
                                            ctx.request_repaint();
                                        });
                                        self.checking = Some(rx);
                                    }
                                    ui.label(
                                        RichText::new(&self.update_status).color(style::MUTED),
                                    );
                                });
                            });
                            ui.end_row();
                        });

                    if changed {
                        self.apply(cx);
                    }
                });
            });
    }
}
