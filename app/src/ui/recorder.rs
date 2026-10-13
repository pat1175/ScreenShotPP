//! Mini-éditeur natif : trim, crop, prévisualisation et exports réutilisables.
use super::{style, video::Video};
use crate::app::{WinCx, WindowUi};
use crate::record::Recording;
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use screenshotpp_core::record::{
    clamp_trim, display_to_video, effective_duration, DisplayCrop, ExportOptions, ExportPreset,
    Region,
};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver},
    Arc,
};
use std::time::{Duration, Instant};

pub fn attributes() -> winit::window::WindowAttributes {
    winit::window::Window::default_attributes()
        .with_title("ScreenShotPP — Recording")
        .with_inner_size(winit::dpi::LogicalSize::new(960.0, 680.0))
        .with_resizable(true)
        .with_decorations(true)
        .with_window_icon(crate::icon::window_icon())
}
struct Toast {
    text: String,
    error: bool,
    until: Instant,
}
struct Exported {
    path: PathBuf,
    gif: bool,
}
struct ExportJob {
    receiver: Receiver<Result<(), String>>,
    progress: Arc<AtomicU64>,
    total: f64,
    path: PathBuf,
    gif: bool,
    cancelled: bool,
}
#[derive(Clone, Copy)]
enum CropMode {
    Move,
    Corner(usize),
}
struct CropDrag {
    mode: CropMode,
    pointer: Pos2,
    rect: Rect,
}
#[derive(Clone, Copy)]
enum TimelineDrag {
    Start,
    End,
    Seek,
}

pub struct RecorderUi {
    file: PathBuf,
    recording: Recording,
    video: Video,
    initialized: bool,
    start: f64,
    end: f64,
    speed: f64,
    preset: ExportPreset,
    // Coordonnées normalisées : un resize de la fenêtre ne modifie pas l'export.
    crop: Option<Rect>,
    display_size: Vec2,
    crop_drag: Option<CropDrag>,
    timeline_drag: Option<TimelineDrag>,
    exporting: Option<ExportJob>,
    exports: Vec<Exported>,
    toasts: Vec<Toast>,
}
impl RecorderUi {
    pub fn new(file: PathBuf, recording: Recording) -> Self {
        Self {
            video: Video::new(file.clone()),
            file,
            recording,
            initialized: false,
            start: 0.0,
            end: 0.0,
            speed: 1.0,
            preset: ExportPreset::Mp4High,
            crop: None,
            display_size: Vec2::ZERO,
            crop_drag: None,
            timeline_drag: None,
            exporting: None,
            exports: Vec::new(),
            toasts: Vec::new(),
        }
    }
    fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast {
            text: text.into(),
            error,
            until: Instant::now() + Duration::from_millis(if error { 5200 } else { 3200 }),
        });
    }
    fn duration(&self) -> f64 {
        self.video
            .info
            .and_then(|info| info.duration)
            .unwrap_or_default()
    }
    fn effective(&self) -> f64 {
        effective_duration(self.start, self.end, self.speed)
    }
    fn toggle_crop(&mut self) {
        self.crop = if self.crop.is_some() {
            None
        } else {
            Some(Rect::from_min_max(Pos2::new(0.2, 0.2), Pos2::new(0.8, 0.8)))
        };
        self.crop_drag = None;
    }
    fn crop_pixels(&self) -> Option<Region> {
        let info = self.video.info?;
        let crop = self.crop?;
        if self.display_size.x <= 0.0 || self.display_size.y <= 0.0 {
            return None;
        }
        Some(display_to_video(
            DisplayCrop {
                x: f64::from(crop.min.x * self.display_size.x),
                y: f64::from(crop.min.y * self.display_size.y),
                width: f64::from(crop.width() * self.display_size.x),
                height: f64::from(crop.height() * self.display_size.y),
            },
            f64::from(self.display_size.x),
            f64::from(self.display_size.y),
            info.width,
            info.height,
        ))
    }
    fn apply_trim(&mut self, start: f64, end: f64, seek: Option<f64>) {
        (self.start, self.end) = clamp_trim(start, end, self.duration());
        let time = seek.unwrap_or(self.video.time).clamp(self.start, self.end);
        self.video.seek(time);
    }
    fn poll(&mut self, ctx: &egui::Context) {
        for error in self.video.update(ctx, self.end) {
            self.toast(error, true);
        }
        if !self.initialized && self.video.info.is_some() {
            self.initialized = true;
            self.end = self.duration();
        }
        if let Some(job) = self.exporting.as_ref() {
            if job.cancelled {
                self.recording.cancel_export();
            }
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Export worker stopped unexpectedly".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let job = self.exporting.take().unwrap();
                match result {
                    Ok(()) => {
                        self.exports.retain(|item| item.path != job.path);
                        self.exports.insert(
                            0,
                            Exported {
                                path: job.path,
                                gif: job.gif,
                            },
                        );
                        self.toast(if job.gif { "GIF saved" } else { "MP4 saved" }, false);
                    }
                    Err(_) if job.cancelled => self.toast("Export cancelled", false),
                    Err(e) => self.toast(format!("Export failed: {e}"), true),
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        let now = Instant::now();
        self.toasts.retain(|toast| toast.until > now);
        if let Some(next) = self.toasts.iter().map(|toast| toast.until).min() {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }
    fn export(&mut self, gif: bool, cx: &WinCx, ctx: &egui::Context) {
        if self.exporting.is_some() || !self.initialized {
            return;
        }
        self.preset = match (gif, self.preset) {
            (true, ExportPreset::Mp4High | ExportPreset::Mp4Small) => ExportPreset::GifSmooth,
            (false, ExportPreset::GifSmooth | ExportPreset::GifSmall) => ExportPreset::Mp4High,
            (_, preset) => preset,
        };
        let extension = if gif { "gif" } else { "mp4" };
        let name = format!(
            "recording-{}-edited.{extension}",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let mut dialog = rfd::FileDialog::new()
            .set_parent(cx.window)
            .set_file_name(name)
            .add_filter(if gif { "GIF" } else { "MP4" }, &[extension]);
        if let Some(parent) = self.file.parent() {
            dialog = dialog.set_directory(parent);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        // Ne jamais écraser le fichier temporaire, nécessaire aux exports suivants.
        let same_source = path == self.file
            || (std::fs::canonicalize(&path)
                .ok()
                .zip(std::fs::canonicalize(&self.file).ok())
                .is_some_and(|(a, b)| a == b));
        if same_source {
            self.toast("Choose a different path from the source recording", true);
            return;
        }
        let options = ExportOptions {
            trim_start: self.start,
            trim_end: self.end,
            crop: self.crop_pixels(),
            speed: self.speed,
            gif,
            preset: self.preset,
        };
        let total = self.effective();
        let progress = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
        let worker_progress = progress.clone();
        let recording = self.recording.clone();
        let output = path.clone();
        let context = ctx.clone();
        let (tx, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = recording.export(&options, &output, |seconds| {
                worker_progress.store(seconds.to_bits(), Ordering::Relaxed);
                context.request_repaint();
            });
            let _ = tx.send(result);
            context.request_repaint();
        });
        self.exporting = Some(ExportJob {
            receiver,
            progress,
            total,
            path,
            gif,
            cancelled: false,
        });
    }
    fn discard(&mut self, cx: &mut WinCx) {
        if self.exports.is_empty()
            && !crate::dialogs::confirm(
                Some(cx.window),
                "Discard recording",
                "Discard this recording without saving an export?",
            )
        {
            return;
        }
        // Sous Windows, ffmpeg doit avoir libéré le fichier avant sa suppression.
        self.video.shutdown();
        self.recording.discard();
        cx.close();
    }
    fn stage(&mut self, ui: &mut egui::Ui) {
        let window_height = ui.ctx().content_rect().height();
        let height = (window_height - 260.0)
            .clamp(110.0, 560.0)
            .min((window_height * 0.58).max(110.0));
        let (stage, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(stage);
        painter.rect_filled(stage, 8.0, Color32::from_rgb(5, 6, 8));
        let Some(info) = self.video.info else {
            painter.text(
                stage.center(),
                egui::Align2::CENTER_CENTER,
                if self.video.loading() {
                    "Loading recording…"
                } else {
                    "Preview unavailable"
                },
                egui::FontId::proportional(16.0),
                style::MUTED,
            );
            return;
        };
        let scale = (stage.width() / info.width as f32).min(stage.height() / info.height as f32);
        let video_rect = Rect::from_center_size(
            stage.center(),
            Vec2::new(info.width as f32 * scale, info.height as f32 * scale),
        );
        self.display_size = video_rect.size();
        if let Some(texture) = &self.video.texture {
            painter.image(
                texture.id(),
                video_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some(normalized) = self.crop {
            let rect = Rect::from_min_max(
                video_rect.min + normalized.min.to_vec2() * video_rect.size(),
                video_rect.min + normalized.max.to_vec2() * video_rect.size(),
            );
            let corners = [
                rect.left_top(),
                rect.right_top(),
                rect.left_bottom(),
                rect.right_bottom(),
            ];
            for outside in [
                Rect::from_min_max(video_rect.min, Pos2::new(video_rect.right(), rect.top())),
                Rect::from_min_max(Pos2::new(video_rect.left(), rect.bottom()), video_rect.max),
                Rect::from_min_max(Pos2::new(video_rect.left(), rect.top()), rect.left_bottom()),
                Rect::from_min_max(
                    rect.right_top(),
                    Pos2::new(video_rect.right(), rect.bottom()),
                ),
            ] {
                painter.rect_filled(outside, 0.0, Color32::from_black_alpha(100));
            }
            painter.rect_filled(rect, 0.0, style::ACCENT.linear_multiply(0.10));
            painter.rect_stroke(
                rect,
                0.0,
                Stroke::new(1.5, style::ACCENT),
                StrokeKind::Inside,
            );
            for point in corners {
                painter.circle_filled(point, 5.0, style::TEXT);
            }
            if self.exporting.is_none() {
                if response.drag_started() {
                    if let Some(pointer) = ui.input(|input| input.pointer.press_origin()) {
                        let mode = corners
                            .iter()
                            .position(|corner| corner.distance(pointer) <= 12.0)
                            .map(CropMode::Corner)
                            .or_else(|| rect.contains(pointer).then_some(CropMode::Move));
                        self.crop_drag = mode.map(|mode| CropDrag {
                            mode,
                            pointer,
                            rect,
                        });
                    }
                }
                if response.dragged() {
                    if let (Some(drag), Some(pointer)) =
                        (&self.crop_drag, response.interact_pointer_pos())
                    {
                        let delta = pointer - drag.pointer;
                        let mut next = drag.rect;
                        match drag.mode {
                            CropMode::Move => {
                                let x = (next.left() + delta.x)
                                    .clamp(video_rect.left(), video_rect.right() - next.width());
                                let y = (next.top() + delta.y)
                                    .clamp(video_rect.top(), video_rect.bottom() - next.height());
                                next = Rect::from_min_size(Pos2::new(x, y), next.size());
                            }
                            CropMode::Corner(corner) => {
                                let min_w = 20.0_f32.min(if corner == 0 || corner == 2 {
                                    next.max.x - video_rect.left()
                                } else {
                                    video_rect.right() - next.min.x
                                });
                                let min_h = 20.0_f32.min(if corner < 2 {
                                    next.max.y - video_rect.top()
                                } else {
                                    video_rect.bottom() - next.min.y
                                });
                                if corner == 0 || corner == 2 {
                                    next.min.x = (next.min.x + delta.x)
                                        .clamp(video_rect.left(), next.max.x - min_w);
                                } else {
                                    next.max.x = (next.max.x + delta.x)
                                        .clamp(next.min.x + min_w, video_rect.right());
                                }
                                if corner < 2 {
                                    next.min.y = (next.min.y + delta.y)
                                        .clamp(video_rect.top(), next.max.y - min_h);
                                } else {
                                    next.max.y = (next.max.y + delta.y)
                                        .clamp(next.min.y + min_h, video_rect.bottom());
                                }
                            }
                        }
                        self.crop = Some(Rect::from_min_max(
                            Pos2::ZERO + (next.min - video_rect.min) / video_rect.size(),
                            Pos2::ZERO + (next.max - video_rect.min) / video_rect.size(),
                        ));
                    }
                }
                if response.drag_stopped() {
                    self.crop_drag = None;
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                }
            }
        }
    }
    fn timeline(&mut self, ui: &mut egui::Ui) {
        let duration = self.duration();
        let (track, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 36.0),
            Sense::click_and_drag(),
        );
        let painter = ui.painter();
        let x = |time: f64| {
            track.left()
                + if duration > 0.0 {
                    (time / duration) as f32 * track.width()
                } else {
                    0.0
                }
        };
        painter.rect_filled(track, 6.0, style::WIDGET);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x(self.start), track.top()),
                Pos2::new(x(self.end), track.bottom()),
            ),
            0.0,
            style::ACCENT.linear_multiply(0.3),
        );
        for (time, label) in [(self.start, "["), (self.end, "]")] {
            painter.text(
                Pos2::new(x(time), track.center().y),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(30.0),
                style::ACCENT,
            );
        }
        painter.line_segment(
            [
                Pos2::new(x(self.video.time.clamp(self.start, self.end)), track.top()),
                Pos2::new(
                    x(self.video.time.clamp(self.start, self.end)),
                    track.bottom(),
                ),
            ],
            Stroke::new(2.0, style::TEXT),
        );
        if self.exporting.is_none() && duration > 0.0 {
            if response.drag_started() {
                if let Some(pointer) = ui.input(|input| input.pointer.press_origin()) {
                    self.timeline_drag = Some(if (pointer.x - x(self.start)).abs() <= 10.0 {
                        TimelineDrag::Start
                    } else if (pointer.x - x(self.end)).abs() <= 10.0 {
                        TimelineDrag::End
                    } else {
                        TimelineDrag::Seek
                    });
                }
            }
            if response.dragged() || response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let time =
                        f64::from(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0))
                            * duration;
                    match self.timeline_drag.unwrap_or(TimelineDrag::Seek) {
                        TimelineDrag::Start => {
                            let (start, _) = clamp_trim(time, self.end, duration);
                            self.apply_trim(time, self.end, Some(start));
                        }
                        TimelineDrag::End => {
                            let (_, end) = clamp_trim(self.start, time, duration);
                            self.apply_trim(self.start, time, Some(end));
                        }
                        TimelineDrag::Seek => self.video.seek(time.clamp(self.start, self.end)),
                    }
                }
            }
            if response.drag_stopped() {
                self.timeline_drag = None;
            }
        }
        ui.horizontal(|ui| {
            ui.label(format_time(self.start));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format_time(self.end));
                ui.label(format!("{:.1}s", self.effective()));
            });
        });
        ui.add_enabled_ui(self.exporting.is_none() && self.initialized, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button(if self.video.playing { "Pause" } else { "Play" })
                    .clicked()
                {
                    self.video.toggle(ui.ctx(), self.start, self.end);
                }
                ui.label(format_time(self.video.time));
                ui.label("Start");
                let start = ui
                    .add(
                        egui::DragValue::new(&mut self.start)
                            .speed(0.1)
                            .range(0.0..=duration)
                            .fixed_decimals(1),
                    )
                    .changed();
                ui.label("End");
                let end = ui
                    .add(
                        egui::DragValue::new(&mut self.end)
                            .speed(0.1)
                            .range(0.0..=duration)
                            .fixed_decimals(1),
                    )
                    .changed();
                if start || end {
                    self.apply_trim(self.start, self.end, None);
                }
                ui.label(if let Some(crop) = self.crop_pixels() {
                    format!("Crop {}×{}", crop.width, crop.height)
                } else {
                    "Crop off".into()
                });
            });
        });
    }
    fn controls(&mut self, ui: &mut egui::Ui, cx: &mut WinCx) {
        let enabled = self.initialized && self.exporting.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(self.crop.is_some(), "Crop").clicked() {
                    self.toggle_crop();
                }
                if ui
                    .add_enabled(self.crop.is_some(), egui::Button::new("Reset crop"))
                    .clicked()
                {
                    self.crop = None;
                }
                if ui.button("Start here").clicked() {
                    self.apply_trim(self.video.time, self.end, Some(self.video.time));
                }
                if ui.button("End here").clicked() {
                    self.apply_trim(self.start, self.video.time, Some(self.video.time));
                }
                if ui
                    .add_enabled(
                        self.start != 0.0 || (self.end - self.duration()).abs() >= 0.01,
                        egui::Button::new("Reset trim"),
                    )
                    .clicked()
                {
                    self.apply_trim(0.0, self.duration(), Some(0.0));
                }
                ui.label("Speed");
                egui::ComboBox::from_id_salt("speed")
                    .selected_text(format!("{}×", self.speed))
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for speed in [1.0, 2.0, 4.0] {
                            ui.selectable_value(&mut self.speed, speed, format!("{speed}×"));
                        }
                    });
                ui.label("Preset");
                egui::ComboBox::from_id_salt("preset")
                    .selected_text(preset_label(self.preset))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for preset in [
                            ExportPreset::Mp4High,
                            ExportPreset::Mp4Small,
                            ExportPreset::GifSmooth,
                            ExportPreset::GifSmall,
                        ] {
                            ui.selectable_value(&mut self.preset, preset, preset_label(preset));
                        }
                    });
                if ui.button("Save GIF…").clicked() {
                    self.export(true, cx, ui.ctx());
                }
                if ui.add(style::primary_button("Save MP4…")).clicked() {
                    self.export(false, cx, ui.ctx());
                }
            });
        });
        if let Some(job) = self.exporting.as_mut() {
            ui.horizontal(|ui| {
                let seconds = f64::from_bits(job.progress.load(Ordering::Relaxed));
                let value = if job.total > 0.0 {
                    (seconds / job.total).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(value)
                        .desired_width((ui.available_width() - 100.0).max(50.0))
                        .text(format!("{:.0}%", value * 100.0)),
                );
                if ui
                    .add_enabled(!job.cancelled, style::danger_button("Cancel"))
                    .clicked()
                {
                    job.cancelled = true;
                    self.recording.cancel_export();
                }
            });
        }
    }
    fn exports(&mut self, ui: &mut egui::Ui, cx: &WinCx) {
        if self.exports.is_empty() {
            return;
        }
        ui.separator();
        ui.label(format!("Saved · {}", self.exports.len()));
        let mut action = None;
        egui::ScrollArea::horizontal()
            .id_salt("saved-recordings")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, item) in self.exports.iter().enumerate() {
                        egui::Frame::new()
                            .fill(style::PANEL)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let name = item
                                        .path
                                        .file_name()
                                        .unwrap_or(item.path.as_os_str())
                                        .to_string_lossy();
                                    ui.label(format!(
                                        "{} · {name}",
                                        if item.gif { "GIF" } else { "MP4" }
                                    ));
                                    for (kind, label) in
                                        ["Open", "Show", "Copy", "Delete"].iter().enumerate()
                                    {
                                        if ui.small_button(*label).clicked() {
                                            action = Some((index, kind));
                                        }
                                    }
                                });
                            });
                    }
                });
            });
        if let Some((index, kind)) = action {
            let path = self.exports[index].path.clone();
            let result: Result<(), String> = match kind {
                0 => opener::open(&path).map_err(|e| format!("Could not open file: {e}")),
                1 => opener::reveal(&path).map_err(|e| format!("Could not show file: {e}")),
                2 => crate::clipboard::copy_text(&path.to_string_lossy())
                    .map(|()| self.toast("Path copied", false))
                    .map_err(|e| format!("Could not copy path: {e}")),
                _ => {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    if !crate::dialogs::confirm(
                        Some(cx.window),
                        "Delete export",
                        &format!("Delete {name}?"),
                    ) {
                        return;
                    }
                    self.recording
                        .delete_export(&path)
                        .map(|()| {
                            self.exports.remove(index);
                            self.toast("Export deleted", false);
                        })
                        .map_err(|e| format!("Could not delete export: {e}"))
                }
            };
            if let Err(e) = result {
                self.toast(e, true);
            }
        }
    }
}
impl WindowUi for RecorderUi {
    fn clear_color(&self) -> [f32; 4] {
        style::BG.to_normalized_gamma_f32()
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut WinCx) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let typing = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::TextEdit::load_state(&ctx, id).is_some());
        if !typing && !egui::Popup::is_any_open(&ctx) {
            let (play, crop, export) = ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Space),
                    input.key_pressed(egui::Key::C),
                    input.key_pressed(egui::Key::Enter),
                )
            });
            if play && self.initialized {
                self.video.toggle(&ctx, self.start, self.end);
            }
            if self.exporting.is_none() && self.initialized {
                if crop {
                    self.toggle_crop();
                }
                if export {
                    self.export(false, cx, &ctx);
                }
            }
        }
        // Marge intérieure de la fenêtre (l'Ui racine n'en a pas).
        let mut padded = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect().shrink(16.0)));
        let ui = &mut padded;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Recording");
                    let status = if let Some(job) = &self.exporting {
                        if job.cancelled {
                            "Cancelling export…".into()
                        } else {
                            format!("Exporting {}…", if job.gif { "GIF" } else { "MP4" })
                        }
                    } else {
                        format!(
                            "{:.1}s selected · {}x · {}",
                            self.effective(),
                            self.speed,
                            if self.crop.is_some() {
                                "crop on"
                            } else {
                                "no crop"
                            }
                        )
                    };
                    ui.colored_label(style::HEADING, status);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(self.exporting.is_none(), egui::Button::new("Discard"))
                        .clicked()
                    {
                        self.discard(cx);
                    }
                });
            });
            self.stage(ui);
            self.timeline(ui);
            self.controls(ui, cx);
            self.exports(ui, cx);
        });
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("recorder-toasts"))
                .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-14.0, -14.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    ui.set_max_width(420.0);
                    for toast in &self.toasts {
                        egui::Frame::new()
                            .fill(if toast.error {
                                Color32::from_rgb(42, 23, 27)
                            } else {
                                style::PANEL
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if toast.error {
                                    style::DANGER
                                } else {
                                    style::BORDER
                                },
                            ))
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.label(&toast.text);
                            });
                    }
                });
        }
    }
}
fn format_time(time: f64) -> String {
    let seconds = time.max(0.0).floor() as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn preset_label(preset: ExportPreset) -> &'static str {
    match preset {
        ExportPreset::Mp4High => "MP4 high",
        ExportPreset::Mp4Small => "MP4 small",
        ExportPreset::GifSmooth => "GIF smooth",
        ExportPreset::GifSmall => "GIF small",
    }
}
