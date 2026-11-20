//! Thème sombre commun à toutes les fenêtres (palette reprise de l'ancienne UI).

use egui::{Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(0x10, 0x13, 0x18);
pub const PANEL: Color32 = Color32::from_rgb(0x16, 0x1b, 0x22);
pub const WIDGET: Color32 = Color32::from_rgb(0x21, 0x26, 0x2d);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x2b, 0x31, 0x3a);
pub const BORDER: Color32 = Color32::from_rgb(0x30, 0x36, 0x3d);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf3);
pub const MUTED: Color32 = Color32::from_rgb(0x9d, 0xa9, 0xb8);
pub const HEADING: Color32 = Color32::from_rgb(0x8f, 0xb1, 0xd1);
pub const ACCENT: Color32 = Color32::from_rgb(0x4d, 0xa3, 0xff);
pub const DANGER: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(TEXT);
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = Color32::from_rgb(0x0d, 0x11, 0x17);
        v.faint_bg_color = PANEL;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.linear_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        let radius = CornerRadius::same(6);
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = radius;
            w.bg_stroke = Stroke::new(1.0, BORDER);
        }
        v.widgets.inactive.bg_fill = WIDGET;
        v.widgets.inactive.weak_bg_fill = WIDGET;
        v.widgets.hovered.bg_fill = WIDGET_HOVER;
        v.widgets.hovered.weak_bg_fill = WIDGET_HOVER;
        v.widgets.active.bg_fill = WIDGET_HOVER;
        v.widgets.active.weak_bg_fill = WIDGET_HOVER;
        v.widgets.open.weak_bg_fill = WIDGET_HOVER;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.corner_radius = radius;
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.interact_size.y = 28.0;
    });
}

/// Section de réglages : petit titre en capitales, comme l'ancienne UI.
pub fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(12.0)
            .strong()
            .color(HEADING),
    );
}

/// Bouton plein d'accent (action principale).
pub fn primary_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).fill(Color32::from_rgb(0x1f, 0x6f, 0xeb))
}

/// Bouton rouge (action destructive / stop).
pub fn danger_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).fill(Color32::from_rgb(0xb6, 0x2b, 0x2f))
}
