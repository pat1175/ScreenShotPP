//! Capture gelée, sélection et éditeur d'annotations natif.
use super::editor::{self as ed, raster, Annotation, History, Tool};
use crate::app::{Action, WinCx, WindowUi};
use crate::capture_flow::{CaptureMode, CaptureSession};
use crate::platform::{self, Monitor};
use crate::settings::Settings;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use image::RgbaImage;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

pub fn attributes(monitor: &Monitor) -> winit::window::WindowAttributes {
    platform::tool_window("ScreenShotPP Overlay")
        .with_position(platform::native_position(
            monitor.rect.x as f64,
            monitor.rect.y as f64,
        ))
        .with_inner_size(platform::native_size(
            monitor.rect.width as f64,
            monitor.rect.height as f64,
        ))
        .with_transparent(false)
}
#[derive(Clone)]
enum Gesture {
    Selection { start: Pos2, auto: Option<Rect> },
    SelectionResize(usize),
    Draw,
    Move { index: usize, previous: Pos2 },
    Resize { index: usize, anchor: usize },
    LabelMove { index: usize, previous: Pos2 },
}
struct TextEntry {
    at: Pos2,
    value: String,
    target: Option<usize>,
    color: Color32,
    size: f32,
    focus: bool,
}
struct MosaicTexture {
    rect: Rect,
    texture: egui::TextureHandle,
}
pub struct OverlayUi {
    session: CaptureSession,
    settings: Settings,
    texture: Option<egui::TextureHandle>,
    background: Rect,
    scale: f32,
    selection: Option<Rect>,
    preview: Option<Rect>,
    gesture: Option<Gesture>,
    draft: Option<Annotation>,
    annotations: Vec<Annotation>,
    history: History,
    selected: Option<usize>,
    last_click: Option<(usize, f64, Pos2)>,
    tool: Tool,
    color: Color32,
    custom: Color32,
    hsv: [f32; 3],
    hex: String,
    picker: bool,
    picker_rect: Rect,
    width: f32,
    font_size: f32,
    output: String,
    toolbar_pos: Option<Pos2>,
    toolbar_rect: Rect,
    text: Option<TextEntry>,
    text_rect: Rect,
    ocr: Option<String>,
    ocr_rx: Option<Receiver<Result<String, String>>>,
    busy: bool,
    mosaics: HashMap<usize, MosaicTexture>,
    overflow: bool,
}
impl OverlayUi {
    pub fn new(session: CaptureSession, settings: &Settings) -> Self {
        let custom =
            ed::parse_hex(&settings.custom_color).unwrap_or(Color32::from_rgb(255, 136, 0));
        Self {
            session,
            settings: settings.clone(),
            texture: None,
            background: Rect::NOTHING,
            scale: 1.0,
            selection: None,
            preview: None,
            gesture: None,
            draft: None,
            annotations: vec![],
            history: History::new(),
            selected: None,
            last_click: None,
            tool: Tool::Select,
            color: Color32::from_rgb(229, 72, 77),
            custom,
            hsv: ed::to_hsv(custom),
            hex: ed::hex(custom),
            picker: false,
            picker_rect: Rect::NOTHING,
            width: 4.0,
            font_size: 24.0,
            output: settings.output_size.clone(),
            toolbar_pos: None,
            toolbar_rect: Rect::NOTHING,
            text: None,
            text_rect: Rect::NOTHING,
            ocr: None,
            ocr_rx: None,
            busy: false,
            mosaics: HashMap::new(),
            overflow: false,
        }
    }
    fn initialize(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.texture.is_none() {
            self.background = screen;
            self.scale = self.session.image.width() as f32 / screen.width();
            self.texture = Some(image_texture(ctx, "capture", &self.session.image));
        }
    }
    fn auto_at(&self, at: Pos2) -> Option<Rect> {
        if self.overflow {
            return None;
        }
        self.session.window_selections.iter().find_map(|candidate| {
            let activation = self.image_rect(candidate.activation);
            let r = self
                .image_rect(candidate.selection)
                .intersect(self.background);
            (activation.contains(at) && r.width() >= 2.0 && r.height() >= 2.0).then_some(r)
        })
    }
    fn image_rect(&self, r: crate::capture::Rect) -> Rect {
        Rect::from_min_size(
            self.background.min + vec2(r.x as f32, r.y as f32) / self.scale,
            vec2(r.width as f32, r.height as f32) / self.scale,
        )
    }
    fn locked(&mut self, selection: Rect, ctx: &egui::Context, screen: Rect, cx: &mut WinCx) {
        self.selection = Some(selection);
        self.preview = None;
        if self.session.mode == CaptureMode::Video {
            let r = ed::physical_rect(selection, self.background, self.scale);
            self.busy = true;
            cx.services.push(Action::StartRecording {
                monitor: self.session.monitor,
                region: screenshotpp_core::record::Region {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                },
            });
            return;
        }
        let matches = self
            .session
            .window_selections
            .first()
            .map(|c| {
                let a = self.image_rect(c.selection);
                (a.min - selection.min).abs().max_elem() <= 1.0
                    && (a.size() - selection.size()).abs().max_elem() <= 1.0
            })
            .unwrap_or(false);
        if !self.overflow && matches {
            let larger = self.session.window_capture.as_ref().is_some_and(|w| {
                w.image.width() as f32 > selection.width() * self.scale
                    || w.image.height() as f32 > selection.height() * self.scale
            });
            if larger {
                let window = self
                    .session
                    .window_capture
                    .take()
                    .expect("Bitmap de fenêtre présent");
                let fit = screenshotpp_core::layout::fit_scale(
                    window.image.dimensions(),
                    (
                        (screen.width() * 0.9).round() as u32,
                        (screen.height() * 0.9).round() as u32,
                    ),
                );
                self.background = Rect::from_center_size(
                    screen.center(),
                    vec2(fit.width as f32, fit.height as f32),
                );
                self.scale = window.image.width() as f32 / self.background.width();
                self.session.image = window.image;
                self.texture = Some(image_texture(ctx, "window-capture", &self.session.image));
                self.selection = Some(self.background);
                self.overflow = true;
            }
        }
        self.position_toolbar(screen);
    }
    fn position_toolbar(&mut self, screen: Rect) {
        let Some(r) = self.selection else {
            return;
        };
        let size = if self.toolbar_rect.is_positive() {
            self.toolbar_rect.size()
        } else {
            vec2(screen.width().min(940.0), 94.0)
        };
        let y = if r.bottom() + size.y + 16.0 > screen.bottom() {
            r.bottom() - size.y - 8.0
        } else {
            r.bottom() + 8.0
        };
        self.toolbar_pos = Some(clamp_position(pos2(r.left(), y), size, screen.shrink(8.0)));
    }
    fn snapshot(&mut self) {
        self.history.push(&self.annotations);
    }
    fn undo(&mut self) {
        self.annotations = self.history.undo();
        self.selected = None;
        self.draft = None;
        self.gesture = None;
        self.mosaics.clear();
    }
    fn redo(&mut self) {
        self.annotations = self.history.redo();
        self.selected = None;
        self.draft = None;
        self.gesture = None;
        self.mosaics.clear();
    }
    fn commit_text(&mut self) {
        let Some(entry) = self.text.take() else {
            return;
        };
        let value = entry.value.trim_end().to_string();
        if let Some(index) = entry.target {
            if let Some(a) = self.annotations.get_mut(index) {
                a.text = value;
                self.snapshot();
            }
        } else if !value.trim().is_empty() {
            let mut a = Annotation::new(Tool::Text, entry.at, entry.color, self.width, entry.size);
            a.text = value;
            self.annotations.push(a);
            self.snapshot();
        }
        self.text_rect = Rect::NOTHING;
    }
    fn start_text(&mut self, at: Pos2, target: Option<usize>) {
        self.commit_text();
        let value = target
            .map(|i| self.annotations[i].text.clone())
            .unwrap_or_default();
        self.text = Some(TextEntry {
            at,
            value,
            target,
            color: self.color,
            size: if target.is_some() {
                14.0
            } else {
                self.font_size
            },
            focus: true,
        });
    }
    fn export(&mut self) -> Result<RgbaImage, String> {
        self.commit_text();
        self.draft = None;
        raster::composite(
            &self.session.image,
            self.background,
            self.selection.ok_or("No selection")?,
            self.scale,
            &self.annotations,
        )
    }
    fn copy(&mut self, cx: &mut WinCx) {
        if self.busy {
            return;
        }
        self.busy = true;
        let result = self.export().and_then(|image| {
            if let Some(max) = crate::storage::target_max_bytes(&self.output) {
                crate::clipboard::copy_image(&crate::storage::fit_by_downscale(&image, max)?)
            } else {
                crate::clipboard::copy_image(&image)
            }
        });
        if let Err(error) = result {
            crate::dialogs::error(Some(cx.window), &error);
        }
        cx.close();
    }
    fn save(&mut self, cx: &mut WinCx) {
        if self.busy {
            return;
        }
        self.commit_text();
        self.busy = true;
        let limited = crate::storage::target_max_bytes(&self.output);
        let format = if limited.is_some() {
            crate::storage::SaveFormat::Jpeg
        } else {
            crate::storage::SaveFormat::Png
        };
        let folder = if self.settings.default_save_folder.is_empty() {
            crate::paths::desktop_dir().ok_or_else(|| "Desktop folder is unavailable".to_string())
        } else {
            Ok(std::path::PathBuf::from(&self.settings.default_save_folder))
        };
        let folder = match folder {
            Ok(folder) => folder,
            Err(error) => {
                crate::dialogs::error(Some(cx.window), &error);
                cx.close();
                return;
            }
        };
        let path = rfd::FileDialog::new()
            .set_parent(cx.window)
            .set_directory(folder)
            .set_file_name(crate::storage::current_filename(format))
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .save_file();
        let Some(mut path) = path else {
            self.busy = false;
            return;
        };
        let jpeg = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        if limited.is_some() && !jpeg {
            path = std::path::PathBuf::from(format!("{}.jpg", path.to_string_lossy()));
        }
        let result = self.export().and_then(|image| {
            let bytes = if let Some(max) = limited {
                crate::storage::fit_by_jpeg_quality(&image, max)?
            } else {
                crate::storage::encode_image(
                    &image,
                    if jpeg {
                        crate::storage::SaveFormat::Jpeg
                    } else {
                        crate::storage::SaveFormat::Png
                    },
                )?
            };
            crate::storage::write_to_disk(&path.to_string_lossy(), &bytes)
        });
        if let Err(error) = result {
            crate::dialogs::error(Some(cx.window), &error);
        }
        cx.close();
    }
    fn start_ocr(&mut self, ctx: &egui::Context, cx: &mut WinCx) {
        if self.busy {
            return;
        }
        let Some(selection) = self.selection else {
            return;
        };
        match raster::crop(&self.session.image, selection, self.background, self.scale) {
            Ok(image) => {
                let (tx, rx) = mpsc::channel();
                self.ocr_rx = Some(rx);
                self.busy = true;
                let ctx = ctx.clone();
                let lang = self.settings.ocr_language.clone();
                std::thread::spawn(move || {
                    let result = crate::ocr::recognize(&image, &lang);
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
            }
            Err(error) => crate::dialogs::error(Some(cx.window), &error),
        }
    }
    fn toolbar(&mut self, ctx: &egui::Context, screen: Rect, cx: &mut WinCx) {
        if self.selection.is_none() || self.session.mode == CaptureMode::Video {
            return;
        }
        let at = self.toolbar_pos.unwrap_or(screen.min);
        let mut command = None;
        let response = egui::Area::new(egui::Id::new("annotation-toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 27, 34))
                    .stroke(Stroke::new(1.0, Color32::from_gray(65)))
                    .corner_radius(8)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_max_width((screen.width() - 32.0).min(940.0));
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let (r, drag) =
                                    ui.allocate_exact_size(vec2(20.0, 28.0), Sense::drag());
                                for x in [r.center().x - 3.0, r.center().x + 3.0] {
                                    for y in [r.center().y - 6.0, r.center().y, r.center().y + 6.0]
                                    {
                                        ui.painter().circle_filled(pos2(x, y), 1.4, Color32::GRAY);
                                    }
                                }
                                if drag.dragged() {
                                    self.toolbar_pos = Some(clamp_position(
                                        at + drag.drag_delta(),
                                        self.toolbar_rect.size(),
                                        screen,
                                    ));
                                }
                                drag.on_hover_text("Move toolbar");
                                for tool in Tool::ALL {
                                    let (r, response) =
                                        ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                                    if response.hovered() || tool == self.tool {
                                        ui.painter().rect_filled(
                                            r,
                                            4,
                                            if tool == self.tool {
                                                Color32::from_rgb(22, 90, 150)
                                            } else {
                                                Color32::from_gray(55)
                                            },
                                        );
                                    }
                                    tool_icon(ui.painter(), r.shrink(6.0), tool, Color32::WHITE);
                                    if response.clicked() {
                                        self.commit_text();
                                        self.tool = tool;
                                        self.selected = None;
                                    }
                                    response.on_hover_text(tool.name());
                                }
                                ui.separator();
                                for c in [
                                    Color32::from_rgb(229, 72, 77),
                                    Color32::from_rgb(77, 163, 255),
                                    Color32::from_rgb(63, 185, 80),
                                    Color32::from_rgb(242, 204, 96),
                                    Color32::WHITE,
                                ] {
                                    let (r, response) =
                                        ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                                    ui.painter().circle_filled(r.center(), 8.0, c);
                                    if self.color == c {
                                        ui.painter().circle_stroke(
                                            r.center(),
                                            10.0,
                                            Stroke::new(1.5, Color32::WHITE),
                                        );
                                    }
                                    if response.clicked() {
                                        self.color = c;
                                    }
                                }
                                let (r, response) =
                                    ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                                ui.painter().rect_filled(r.shrink(3.0), 3, self.custom);
                                ui.painter().rect_stroke(
                                    r.shrink(2.0),
                                    3,
                                    Stroke::new(1.0, Color32::WHITE),
                                    StrokeKind::Outside,
                                );
                                if response.clicked() {
                                    self.picker = !self.picker;
                                    self.color = self.custom;
                                }
                                response.on_hover_text("Custom color");
                                egui::ComboBox::from_id_salt("stroke-width")
                                    .width(42.0)
                                    .selected_text(match self.width as u32 {
                                        2 => "S",
                                        8 => "L",
                                        14 => "XL",
                                        _ => "M",
                                    })
                                    .show_ui(ui, |ui| {
                                        for (label, value) in
                                            [("S", 2.0), ("M", 4.0), ("L", 8.0), ("XL", 14.0)]
                                        {
                                            ui.selectable_value(&mut self.width, value, label);
                                        }
                                    });
                                ui.add(
                                    egui::DragValue::new(&mut self.font_size)
                                        .range(6.0..=300.0)
                                        .speed(1.0),
                                )
                                .on_hover_text("Text size");
                            });
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .add_enabled(self.history.can_undo(), egui::Button::new("Undo"))
                                    .clicked()
                                {
                                    self.undo();
                                }
                                if ui
                                    .add_enabled(self.history.can_redo(), egui::Button::new("Redo"))
                                    .clicked()
                                {
                                    self.redo();
                                }
                                let old = self.output.clone();
                                egui::ComboBox::from_id_salt("output-size")
                                    .width(68.0)
                                    .selected_text(output_label(&self.output))
                                    .show_ui(ui, |ui| {
                                        for target in ["full", "5mb", "2mb", "1mb"] {
                                            ui.selectable_value(
                                                &mut self.output,
                                                target.to_owned(),
                                                output_label(target),
                                            );
                                        }
                                    });
                                if old != self.output {
                                    let output = self.output.clone();
                                    cx.services.save_ui_prefs(|s| s.output_size = output);
                                }
                                for (label, value) in
                                    [("OCR", 0), ("Copy", 1), ("Save", 2), ("Cancel", 3)]
                                {
                                    if ui.button(label).clicked() {
                                        command = Some(value);
                                    }
                                }
                            });
                        });
                        if self.busy {
                            ui.label("Working…");
                        }
                    });
            });
        self.toolbar_rect = response.response.rect;
        // La première mesure corrige le placement sans empêcher le déplacement manuel.
        let clamped = clamp_position(
            self.toolbar_pos.unwrap_or(at),
            self.toolbar_rect.size(),
            screen.shrink(8.0),
        );
        self.toolbar_pos = Some(clamped);
        match command {
            Some(0) => self.start_ocr(ctx, cx),
            Some(1) => self.copy(cx),
            Some(2) => self.save(cx),
            Some(3) => cx.close(),
            _ => {}
        }
    }
    fn color_picker(&mut self, ctx: &egui::Context, screen: Rect, cx: &mut WinCx) {
        self.picker_rect = Rect::NOTHING;
        if !self.picker {
            return;
        }
        let at = clamp_position(
            pos2(self.toolbar_rect.left(), self.toolbar_rect.bottom() + 8.0),
            vec2(212.0, 254.0),
            screen.shrink(8.0),
        );
        let response = egui::Area::new("color-picker".into())
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 27, 34))
                    .stroke(Stroke::new(1.0, Color32::GRAY))
                    .corner_radius(6)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        let mut changed = false;
                        let (r, response) =
                            ui.allocate_exact_size(vec2(192.0, 160.0), Sense::click_and_drag());
                        for y in 0..32 {
                            for x in 0..32 {
                                let cell = Rect::from_min_size(
                                    r.min + vec2(x as f32 * 6.0, y as f32 * 5.0),
                                    vec2(6.0, 5.0),
                                );
                                ui.painter().rect_filled(
                                    cell,
                                    0,
                                    ed::hsv(self.hsv[0], x as f32 / 31.0, 1.0 - y as f32 / 31.0),
                                );
                            }
                        }
                        let cursor =
                            r.min + vec2(self.hsv[1] * r.width(), (1.0 - self.hsv[2]) * r.height());
                        ui.painter()
                            .circle_stroke(cursor, 4.0, Stroke::new(1.5, Color32::WHITE));
                        if response.dragged() || response.clicked() {
                            if let Some(p) = response.interact_pointer_pos() {
                                self.hsv[1] = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0);
                                self.hsv[2] = (1.0 - (p.y - r.top()) / r.height()).clamp(0.0, 1.0);
                                changed = true;
                            }
                        }
                        let (r, response) =
                            ui.allocate_exact_size(vec2(192.0, 18.0), Sense::click_and_drag());
                        for x in 0..48 {
                            ui.painter().rect_filled(
                                Rect::from_min_size(
                                    r.min + vec2(x as f32 * 4.0, 0.0),
                                    vec2(4.0, 18.0),
                                ),
                                0,
                                ed::hsv(x as f32 * 360.0 / 48.0, 1.0, 1.0),
                            );
                        }
                        ui.painter().line_segment(
                            [
                                pos2(r.left() + self.hsv[0] / 360.0 * r.width(), r.top()),
                                pos2(r.left() + self.hsv[0] / 360.0 * r.width(), r.bottom()),
                            ],
                            Stroke::new(2.0, Color32::WHITE),
                        );
                        if response.dragged() || response.clicked() {
                            if let Some(p) = response.interact_pointer_pos() {
                                self.hsv[0] =
                                    ((p.x - r.left()) / r.width()).clamp(0.0, 1.0) * 360.0;
                                changed = true;
                            }
                        }
                        if changed {
                            self.custom = ed::hsv(self.hsv[0], self.hsv[1], self.hsv[2]);
                            self.hex = ed::hex(self.custom);
                        }
                        if ui
                            .add(egui::TextEdit::singleline(&mut self.hex).desired_width(192.0))
                            .changed()
                        {
                            if let Some(c) = ed::parse_hex(&self.hex) {
                                self.custom = c;
                                self.hsv = ed::to_hsv(c);
                                changed = true;
                            }
                        }
                        if changed {
                            self.color = self.custom;
                            let hex = ed::hex(self.custom);
                            cx.services.save_ui_prefs(|s| s.custom_color = hex);
                        }
                        if ui.button("Close").clicked() {
                            self.picker = false;
                        }
                    });
            });
        self.picker_rect = response.response.rect;
    }
    fn text_entry(&mut self, ctx: &egui::Context) {
        self.text_rect = Rect::NOTHING;
        let Some(entry) = self.text.as_mut() else {
            return;
        };
        let mut commit = false;
        let response = egui::Area::new("inline-text".into())
            .order(egui::Order::Foreground)
            .fixed_pos(entry.at)
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(entry.color);
                let width = raster::text_size(&entry.value, entry.size).x.max(100.0) + 12.0;
                let response = ui.add(
                    egui::TextEdit::multiline(&mut entry.value)
                        .font(FontId::proportional(entry.size))
                        .text_color(entry.color)
                        .frame(egui::Frame::NONE)
                        .desired_width(width)
                        .desired_rows(1),
                );
                if entry.focus {
                    response.request_focus();
                    entry.focus = false;
                }
                let key = ui.input(|i| {
                    i.key_pressed(egui::Key::Escape)
                        || (i.modifiers.command && i.key_pressed(egui::Key::Enter))
                });
                commit = key || response.lost_focus();
            });
        self.text_rect = response.response.rect;
        if commit {
            self.commit_text();
        }
    }
    fn ocr_panel(&mut self, ctx: &egui::Context, screen: Rect, cx: &mut WinCx) {
        if let Some(rx) = &self.ocr_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.busy = false;
                    self.ocr_rx = None;
                    match result {
                        Ok(text) => self.ocr = Some(text),
                        Err(error) => crate::dialogs::error(Some(cx.window), &error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.busy = false;
                    self.ocr_rx = None;
                    crate::dialogs::error(Some(cx.window), "OCR worker stopped unexpectedly");
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(text) = self.ocr.as_mut() else {
            return;
        };
        let mut close = false;
        let mut copy = false;
        egui::Area::new("ocr-modal".into())
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(13, 17, 23))
                    .stroke(Stroke::new(1.0, Color32::GRAY))
                    .inner_margin(16)
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.heading("Recognized text");
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .desired_width((screen.width() - 64.0).min(640.0))
                                .desired_rows(16),
                        );
                        ui.horizontal(|ui| {
                            copy = ui.button("Copy text").clicked();
                            close = ui.button("Close").clicked();
                        });
                    });
            });
        if copy {
            if let Err(error) = crate::clipboard::copy_text(self.ocr.as_deref().unwrap_or_default())
            {
                crate::dialogs::error(Some(cx.window), &error);
            }
            cx.close();
        }
        if close {
            self.ocr = None;
        }
    }
    fn pointer(&mut self, _ui: &mut egui::Ui, ctx: &egui::Context, screen: Rect, cx: &mut WinCx) {
        if self.busy || self.ocr.is_some() {
            return;
        }
        let (at, pressed, released, down) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.pointer.primary_down(),
            )
        });
        let Some(at) = at else {
            return;
        };
        let over_controls = self.toolbar_rect.contains(at)
            || self.picker_rect.contains(at)
            || self.text_rect.contains(at);
        if self.selection.is_none() {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if pressed && !over_controls {
            if self.picker {
                self.picker = false;
            }
            if self.text.is_some() {
                self.commit_text();
                return;
            }
            if self.selection.is_none() {
                self.gesture = Some(Gesture::Selection {
                    start: screen.clamp(at),
                    auto: self.auto_at(at),
                });
            } else {
                let selection = self.selection.unwrap();
                let mut handled = false;
                if let Some(i) = self
                    .selected
                    .filter(|i| matches!(self.annotations[*i].tool, Tool::Rect | Tool::Ellipse))
                {
                    for (anchor, p) in ed::anchors(self.annotations[i].rect)
                        .into_iter()
                        .enumerate()
                    {
                        if p.distance(at) < 9.0 {
                            self.gesture = Some(Gesture::Resize { index: i, anchor });
                            handled = true;
                            break;
                        }
                    }
                }
                if !handled {
                    for anchor in [1, 3, 5, 7] {
                        if ed::anchors(selection)[anchor].distance(at) < 13.0 {
                            self.gesture = Some(Gesture::SelectionResize(anchor));
                            handled = true;
                            break;
                        }
                    }
                }
                if !handled && selection.contains(at) {
                    if self.tool == Tool::Select {
                        let hit = self
                            .annotations
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(i, a)| {
                                if a.tool == Tool::Bubble
                                    && !a.text.is_empty()
                                    && a.label_rect().contains(at)
                                {
                                    Some((i, true))
                                } else if a.hit(at) {
                                    Some((i, false))
                                } else {
                                    None
                                }
                            });
                        self.selected = hit.map(|h| h.0);
                        if let Some((index, label)) = hit {
                            let now = ctx.input(|i| i.time);
                            let double = self.last_click.is_some_and(|(i, time, p)| {
                                i == index && now - time < 0.35 && p.distance(at) < 5.0
                            });
                            self.last_click = Some((index, now, at));
                            if double && self.annotations[index].tool == Tool::Bubble {
                                let a = &self.annotations[index];
                                self.start_text(a.rect.min + a.label_offset, Some(index));
                            } else {
                                self.gesture = Some(if label {
                                    Gesture::LabelMove {
                                        index,
                                        previous: at,
                                    }
                                } else {
                                    Gesture::Move {
                                        index,
                                        previous: at,
                                    }
                                });
                            }
                        }
                    } else if self.tool == Tool::Text {
                        self.start_text(at, None);
                    } else if self.tool == Tool::Bubble {
                        let now = ctx.input(|i| i.time);
                        let double = self.last_click.and_then(|(i, time, p)| {
                            self.annotations
                                .get(i)
                                .filter(|a| {
                                    a.tool == Tool::Bubble
                                        && a.hit(at)
                                        && now - time < 0.35
                                        && p.distance(at) < 5.0
                                })
                                .map(|_| i)
                        });
                        if let Some(index) = double {
                            let a = &self.annotations[index];
                            self.start_text(a.rect.min + a.label_offset, Some(index));
                        } else {
                            let index = self.annotations.len();
                            self.annotations.push(Annotation::new(
                                Tool::Bubble,
                                at,
                                self.color,
                                self.width,
                                self.font_size,
                            ));
                            self.last_click = Some((index, now, at));
                            self.snapshot();
                        }
                    } else {
                        self.draft = Some(Annotation::new(
                            self.tool,
                            at,
                            self.color,
                            self.width,
                            self.font_size,
                        ));
                        self.gesture = Some(Gesture::Draw);
                    }
                } else if !handled {
                    self.selected = None;
                }
            }
        }
        if self.selection.is_none() && self.gesture.is_none() {
            self.preview = self.auto_at(at);
        }
        if let Some(gesture) = self.gesture.clone().filter(|_| down || released) {
            match gesture {
                Gesture::Selection { start, auto } => {
                    let at = screen.clamp(at);
                    self.preview = if start.distance(at) < 4.0 {
                        auto.or(Some(Rect::from_two_pos(start, at)))
                    } else {
                        Some(Rect::from_two_pos(start, at))
                    };
                    if released {
                        if let Some(r) = ed::lock_candidate(start, at, auto, screen) {
                            self.locked(r, ctx, screen, cx);
                        }
                    }
                }
                Gesture::SelectionResize(anchor) => {
                    self.selection = Some(ed::resize(
                        self.selection.unwrap(),
                        anchor,
                        at,
                        self.background,
                    ));
                    if released {
                        self.position_toolbar(screen);
                    }
                }
                Gesture::Draw => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.update(self.selection.unwrap().clamp(at));
                    }
                    if released {
                        if let Some(draft) = self.draft.take().filter(|a| !a.degenerate()) {
                            self.annotations.push(draft);
                            self.snapshot();
                        }
                    }
                }
                Gesture::Move { index, previous } => {
                    let delta = ed::bounded_delta(
                        self.annotations[index].bounds(),
                        at - previous,
                        self.selection.unwrap(),
                    );
                    self.annotations[index].translate(delta);
                    self.gesture = Some(Gesture::Move {
                        index,
                        previous: at,
                    });
                    if released {
                        self.snapshot();
                    }
                }
                Gesture::Resize { index, anchor } => {
                    let r = ed::resize(
                        self.annotations[index].rect,
                        anchor,
                        at,
                        self.selection.unwrap(),
                    );
                    self.annotations[index].rect = r;
                    if released {
                        self.snapshot();
                    }
                }
                Gesture::LabelMove { index, previous } => {
                    let delta = ed::bounded_delta(
                        self.annotations[index].label_rect(),
                        at - previous,
                        self.selection.unwrap(),
                    );
                    self.annotations[index].label_offset += delta;
                    self.gesture = Some(Gesture::LabelMove {
                        index,
                        previous: at,
                    });
                    if released {
                        self.snapshot();
                    }
                }
            }
        }
        if released {
            self.gesture = None;
            self.preview = if self.selection.is_none() {
                self.auto_at(at)
            } else {
                None
            };
        }
        if pressed || released {
            ctx.request_repaint();
        }
    }
    fn paint(&mut self, ctx: &egui::Context, screen: Rect) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(screen, 0, Color32::from_rgb(13, 17, 23));
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                self.background,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some(selection) = self.selection {
            let clipped = painter.with_clip_rect(selection);
            for (i, a) in self.annotations.iter().enumerate() {
                if a.tool == Tool::Mosaic {
                    if self
                        .mosaics
                        .get(&i)
                        .is_none_or(|cache| cache.rect != a.rect)
                    {
                        if let Ok(tile) =
                            raster::mosaic(&self.session.image, a.rect, self.background, self.scale)
                        {
                            self.mosaics.insert(
                                i,
                                MosaicTexture {
                                    rect: a.rect,
                                    texture: image_texture(ctx, &format!("mosaic-{i}"), &tile),
                                },
                            );
                        }
                    }
                    if let Some(cache) = self.mosaics.get(&i) {
                        clipped.image(
                            cache.texture.id(),
                            a.rect,
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                } else {
                    paint_annotation(&clipped, a, ed::bubble_number(&self.annotations, i));
                }
            }
            if let Some(a) = &self.draft {
                paint_annotation(&clipped, a, None);
            }
        }
        let rect = self.selection.or(self.preview);
        let shade = Color32::from_black_alpha(115);
        if let Some(r) = rect {
            for outside in [
                Rect::from_min_max(screen.min, pos2(screen.right(), r.top())),
                Rect::from_min_max(pos2(screen.left(), r.top()), pos2(r.left(), r.bottom())),
                Rect::from_min_max(pos2(r.right(), r.top()), pos2(screen.right(), r.bottom())),
                Rect::from_min_max(pos2(screen.left(), r.bottom()), screen.max),
            ] {
                if outside.is_positive() {
                    painter.rect_filled(outside, 0, shade);
                }
            }
            let stroke = Stroke::new(2.0, Color32::from_rgb(22, 140, 255));
            if self.selection.is_none() {
                dashed_rect(&painter, r, stroke);
            } else {
                painter.rect_stroke(r, 0, stroke, StrokeKind::Middle);
                for anchor in [1, 3, 5, 7] {
                    let p = ed::anchors(r)[anchor];
                    painter.circle_filled(p, 5.0, Color32::WHITE);
                    painter.circle_stroke(p, 5.0, Stroke::new(1.0, Color32::from_black_alpha(115)));
                }
            }
        } else {
            painter.rect_filled(screen, 0, shade);
        }
        if let Some(a) = self.selected.and_then(|i| self.annotations.get(i)) {
            painter.rect_stroke(
                a.bounds(),
                0,
                Stroke::new(1.0, Color32::from_rgb(77, 163, 255)),
                StrokeKind::Outside,
            );
            if matches!(a.tool, Tool::Rect | Tool::Ellipse) {
                for p in ed::anchors(a.rect) {
                    painter.rect_filled(
                        Rect::from_center_size(p, Vec2::splat(7.0)),
                        1,
                        Color32::WHITE,
                    );
                }
            }
        }
        if self.selection.is_none() {
            if let Some(at) = ctx.input(|i| i.pointer.hover_pos()) {
                self.loupe(&painter, at, screen);
            }
        }
    }
    fn loupe(&self, painter: &egui::Painter, at: Pos2, screen: Rect) {
        let size = vec2(142.0, 166.0);
        let mut origin = at + vec2(24.0, 24.0);
        if origin.x + size.x + 8.0 > screen.right() {
            origin.x = at.x - size.x - 24.0;
        }
        if origin.y + size.y + 8.0 > screen.bottom() {
            origin.y = at.y - size.y - 24.0;
        }
        origin = clamp_position(origin, size, screen.shrink(8.0));
        let r = Rect::from_min_size(origin, size);
        painter.rect_filled(r, 3, Color32::from_rgb(13, 17, 23));
        let pixel = (at - self.background.min) * self.scale;
        for y in 0..18 {
            for x in 0..18 {
                let sx = (pixel.x - 8.75 + x as f32).floor() as i32;
                let sy = (pixel.y - 8.75 + y as f32).floor() as i32;
                if sx >= 0
                    && sy >= 0
                    && sx < self.session.image.width() as i32
                    && sy < self.session.image.height() as i32
                {
                    let c = self.session.image.get_pixel(sx as u32, sy as u32);
                    let cell = Rect::from_min_size(
                        origin + vec2(1.0 + x as f32 * 8.0, 1.0 + y as f32 * 8.0),
                        Vec2::splat(8.0),
                    )
                    .intersect(Rect::from_min_size(
                        origin + Vec2::splat(1.0),
                        Vec2::splat(140.0),
                    ));
                    painter.rect_filled(cell, 0, Color32::from_rgb(c[0], c[1], c[2]));
                }
            }
        }
        let c = origin + vec2(71.0, 71.0);
        let stroke = Stroke::new(1.0, Color32::from_rgb(77, 163, 255));
        painter.line_segment(
            [pos2(c.x, origin.y + 1.0), pos2(c.x, origin.y + 141.0)],
            stroke,
        );
        painter.line_segment(
            [pos2(origin.x + 1.0, c.y), pos2(origin.x + 141.0, c.y)],
            stroke,
        );
        let dimensions = self
            .preview
            .map(|r| {
                format!(
                    "  {} × {}",
                    (r.width() * self.scale).round() as u32,
                    (r.height() * self.scale).round() as u32
                )
            })
            .unwrap_or_default();
        painter.text(
            origin + vec2(5.0, 145.0),
            Align2::LEFT_TOP,
            format!(
                "{}, {}{}",
                pixel.x.round() as i32,
                pixel.y.round() as i32,
                dimensions
            ),
            FontId::proportional(11.0),
            Color32::WHITE,
        );
    }
}
impl WindowUi for OverlayUi {
    fn clear_color(&self) -> [f32; 4] {
        [0.05, 0.067, 0.09, 1.0]
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut WinCx) {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        self.initialize(&ctx, screen);
        let was_editing = self.text.is_some();
        let had_ocr = self.ocr.is_some();
        let (escape, command, z, y, c, shift, delete) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.modifiers.command,
                i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::Y),
                i.key_pressed(egui::Key::C)
                    || i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.modifiers.shift,
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        if escape && !was_editing {
            if had_ocr {
                self.ocr = None;
            } else if self.picker {
                self.picker = false;
            } else {
                cx.close();
                return;
            }
        }
        if !was_editing && !had_ocr && !ctx.text_edit_focused() && !self.busy {
            if command && z && !shift {
                self.undo();
            } else if command && (y || (z && shift)) {
                self.redo();
            }
            if c && self.selection.is_some()
                && (command
                    || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))))
            {
                self.copy(cx);
                return;
            }
            if delete {
                if let Some(index) = self.selected.take() {
                    self.annotations.remove(index);
                    self.snapshot();
                }
            }
        }
        self.toolbar(&ctx, screen, cx);
        self.color_picker(&ctx, screen, cx);
        self.text_entry(&ctx);
        if was_editing
            && self.text.is_some()
            && ctx.input(|i| {
                i.pointer.button_pressed(egui::PointerButton::Primary)
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !self.text_rect.contains(p))
            })
        {
            self.commit_text();
        }
        if !was_editing {
            self.pointer(ui, &ctx, screen, cx);
        }
        self.paint(&ctx, screen);
        self.ocr_panel(&ctx, screen, cx);
    }
}
fn image_texture(ctx: &egui::Context, name: &str, image: &RgbaImage) -> egui::TextureHandle {
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::NEAREST,
    )
}
fn output_label(target: &str) -> &'static str {
    match target {
        "5mb" => "≤5MB",
        "2mb" => "≤2MB",
        "1mb" => "≤1MB",
        _ => "Full",
    }
}
fn clamp_position(at: Pos2, size: Vec2, bounds: Rect) -> Pos2 {
    pos2(
        at.x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
        at.y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    )
}
fn dashed_rect(p: &egui::Painter, r: Rect, stroke: Stroke) {
    for (from, to) in [
        (r.left_top(), r.right_top()),
        (r.right_top(), r.right_bottom()),
        (r.right_bottom(), r.left_bottom()),
        (r.left_bottom(), r.left_top()),
    ] {
        let length = from.distance(to);
        if length > 0.0 {
            let direction = (to - from) / length;
            let mut n = 0.0;
            while n < length {
                p.line_segment(
                    [
                        from + direction * n,
                        from + direction * (n + 6.0).min(length),
                    ],
                    stroke,
                );
                n += 10.0;
            }
        }
    }
}
fn paint_annotation(p: &egui::Painter, a: &Annotation, number: Option<usize>) {
    let stroke = Stroke::new(a.width, a.color);
    match a.tool {
        Tool::Rect => {
            p.rect_stroke(a.rect, 0, stroke, StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(
                a.rect.center(),
                a.rect.size() / 2.0,
                stroke,
            ));
        }
        Tool::Line | Tool::Arrow | Tool::Pencil => {
            let points = if a.tool == Tool::Pencil {
                ed::smooth_points(&a.points)
            } else {
                a.points.clone()
            };
            if points.len() > 1 {
                for at in &points {
                    p.circle_filled(*at, a.width / 2.0, a.color);
                }
                p.add(egui::Shape::line(points, stroke));
            }
            if a.tool == Tool::Arrow && a.points.len() >= 2 {
                p.add(egui::Shape::convex_polygon(
                    raster::arrow_head(a.points[0], a.points[1], a.width).to_vec(),
                    a.color,
                    Stroke::NONE,
                ));
            }
        }
        Tool::Text => {
            p.text(
                a.rect.min,
                Align2::LEFT_TOP,
                &a.text,
                FontId::proportional(a.font_size),
                a.color,
            );
        }
        Tool::Bubble => {
            let center = a.rect.min;
            if !a.text.trim().is_empty() {
                p.line_segment(
                    [
                        center + a.label_offset,
                        center + ed::connector(a.label_offset, 15.0),
                    ],
                    Stroke::new(2.0, a.color),
                );
                let r = a.label_rect();
                p.rect_filled(r, 5, Color32::from_rgb(13, 17, 23));
                p.rect_stroke(r, 5, Stroke::new(2.0, a.color), StrokeKind::Middle);
                p.text(
                    r.min + Vec2::splat(6.0),
                    Align2::LEFT_TOP,
                    &a.text,
                    FontId::proportional(14.0),
                    Color32::from_rgb(230, 237, 243),
                );
            }
            p.circle_filled(center, 15.0, a.color);
            let n = number.unwrap_or(1).to_string();
            p.text(
                center,
                Align2::CENTER_CENTER,
                &n,
                FontId::proportional(16.0),
                Color32::WHITE,
            );
            p.text(
                center + vec2(0.5, 0.0),
                Align2::CENTER_CENTER,
                n,
                FontId::proportional(16.0),
                Color32::WHITE,
            );
        }
        Tool::Mosaic => {
            p.rect_filled(a.rect, 0, Color32::from_black_alpha(140));
        }
        Tool::Select => {}
    }
}
fn tool_icon(p: &egui::Painter, r: Rect, tool: Tool, color: Color32) {
    let s = Stroke::new(1.7, color);
    let c = r.center();
    match tool {
        Tool::Select => {
            p.add(egui::Shape::closed_line(
                vec![
                    r.left_top(),
                    pos2(r.left() + 3.0, r.bottom()),
                    c,
                    pos2(r.right(), r.top() + 7.0),
                ],
                s,
            ));
        }
        Tool::Rect => {
            p.rect_stroke(r, 0, s, StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(c, r.size() / 2.0, s));
        }
        Tool::Line => {
            p.line_segment([r.left_bottom(), r.right_top()], s);
        }
        Tool::Arrow => {
            p.line_segment([r.left_bottom(), r.right_top()], s);
            p.line_segment([r.right_top(), pos2(r.right() - 7.0, r.top())], s);
            p.line_segment([r.right_top(), pos2(r.right(), r.top() + 7.0)], s);
        }
        Tool::Pencil => {
            p.add(egui::Shape::line(
                vec![
                    r.left_bottom(),
                    pos2(c.x - 3.0, c.y - 3.0),
                    pos2(c.x + 3.0, c.y + 3.0),
                    r.right_top(),
                ],
                s,
            ));
        }
        Tool::Text => {
            p.line_segment([r.left_bottom(), pos2(c.x, r.top())], s);
            p.line_segment([pos2(c.x, r.top()), r.right_bottom()], s);
            p.line_segment(
                [
                    pos2(r.left() + 3.0, c.y + 3.0),
                    pos2(r.right() - 3.0, c.y + 3.0),
                ],
                s,
            );
        }
        Tool::Bubble => {
            p.circle_stroke(c, r.width() / 2.0, s);
            p.line_segment([pos2(c.x, c.y - 4.0), pos2(c.x, c.y + 4.0)], s);
        }
        Tool::Mosaic => {
            for y in 0..3 {
                for x in 0..3 {
                    p.rect_filled(
                        Rect::from_min_size(
                            r.min + vec2(x as f32 * 6.0, y as f32 * 6.0),
                            Vec2::splat(4.0),
                        ),
                        0,
                        if (x + y) % 2 == 0 {
                            color
                        } else {
                            Color32::GRAY
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn overlay() -> OverlayUi {
        let monitor = Monitor {
            index: 0,
            rect: platform::MonitorRect {
                x: 0,
                y: 0,
                width: 200,
                height: 160,
            },
            scale: 2.0,
        };
        let session = CaptureSession {
            image: RgbaImage::from_pixel(200, 160, image::Rgba([255, 255, 255, 255])),
            window_selections: vec![crate::capture_flow::WindowSelection {
                selection: crate::capture::Rect {
                    x: 20,
                    y: 24,
                    width: 100,
                    height: 80,
                },
                activation: crate::capture::Rect {
                    x: 20,
                    y: 24,
                    width: 100,
                    height: 16,
                },
            }],
            window_capture: None,
            mode: CaptureMode::Image,
            monitor,
        };
        let mut overlay = OverlayUi::new(session, &Settings::default());
        overlay.background = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 80.0));
        overlay.scale = 2.0;
        overlay
    }
    #[test]
    fn auto_selection_uses_activation_not_entire_window() {
        let o = overlay();
        assert_eq!(
            o.auto_at(pos2(20.0, 14.0)),
            Some(Rect::from_min_size(pos2(10.0, 12.0), vec2(50.0, 40.0)))
        );
        assert!(o.auto_at(pos2(20.0, 30.0)).is_none());
        assert!(o.selection.is_none());
    }
    #[test]
    fn inline_commit_trims_empty_and_preserves_history() {
        let mut o = overlay();
        o.start_text(pos2(10.0, 10.0), None);
        o.text.as_mut().unwrap().value = "  \n".into();
        o.commit_text();
        assert!(o.annotations.is_empty());
        assert!(!o.history.can_undo());
        o.start_text(pos2(10.0, 10.0), None);
        o.text.as_mut().unwrap().value = "Hello \n".into();
        o.commit_text();
        assert_eq!(o.annotations[0].text, "Hello");
        o.undo();
        assert!(o.annotations.is_empty());
        o.redo();
        assert_eq!(o.annotations[0].text, "Hello");
    }
    #[test]
    fn letterboxed_export_uses_bitmap_origin() {
        let mut o = overlay();
        o.background = Rect::from_min_size(pos2(8.0, 6.0), vec2(100.0, 80.0));
        o.selection = Some(Rect::from_min_size(pos2(18.0, 16.0), vec2(40.0, 30.0)));
        o.session
            .image
            .put_pixel(20, 20, image::Rgba([1, 2, 3, 255]));
        let image = o.export().unwrap();
        assert_eq!(image.dimensions(), (80, 60));
        assert_eq!(image.get_pixel(0, 0).0, [1, 2, 3, 255]);
    }
}
