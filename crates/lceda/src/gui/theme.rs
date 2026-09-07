//! 浅色卡片主题：系统蓝、大圆角、不透明底板。

use eframe::egui::{self, Color32, CornerRadius, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(0, 122, 255);
pub const LABEL: Color32 = Color32::from_rgb(29, 29, 31);
pub const SECONDARY: Color32 = Color32::from_rgb(134, 134, 139);
pub const WINDOW_BG: Color32 = Color32::from_rgb(242, 242, 247);
pub const WELL: Color32 = Color32::from_rgb(236, 236, 241);

pub fn fill() -> Color32 {
    Color32::from_rgb(255, 255, 255)
}
pub fn fill_strong() -> Color32 {
    Color32::from_rgb(248, 248, 250)
}
pub fn hairline() -> Color32 {
    Color32::from_rgb(224, 224, 229)
}

pub fn apply(ctx: &egui::Context) {
    install_cjk_fonts(ctx);

    let mut visuals = Visuals::light();
    visuals.window_fill = WINDOW_BG;
    visuals.panel_fill = WINDOW_BG;
    visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
    visuals.faint_bg_color = WELL;
    visuals.widgets.inactive.bg_fill = fill_strong();
    visuals.widgets.inactive.weak_bg_fill = fill_strong();
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(240, 240, 245);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, LABEL);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, LABEL);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(10);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(10);
    visuals.widgets.active.corner_radius = CornerRadius::same(10);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(12);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, hairline());
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0, 122, 255, 36);
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::NONE;
    visuals.window_shadow.blur = 0;
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(12);
        style.text_styles.insert(
            egui::TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
    });
}

fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansSC-Regular.otf",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/mnt/c/Windows/Fonts/msyh.ttc",
        "/mnt/c/Windows/Fonts/msyh.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = FontDefinitions::default();
            fonts.font_data.insert(
                "cjk".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".into());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("cjk".into());
            ctx.set_fonts(fonts);
            break;
        }
    }
}

pub fn paint_card(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 12.0, fill());
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0_f32, hairline()),
        egui::StrokeKind::Inside,
    );
}

/// Markdown 的标题/加粗走 `strong_text_color()`（即 active 白字）。浅色底上必须改回深色。
pub fn show_markdown(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.override_text_color = Some(LABEL);
        v.widgets.active.fg_stroke = Stroke::new(1.0_f32, LABEL);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, LABEL);
        add_contents(ui);
    });
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(fill())
        .stroke(Stroke::new(1.0_f32, hairline()))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(12))
}

pub fn pill_button(ui: &mut egui::Ui, text: &str, enabled: bool, filled: bool) -> egui::Response {
    action_button(ui, text, enabled, filled, egui::vec2(0.0, 32.0))
}

pub fn action_button(
    ui: &mut egui::Ui,
    text: &str,
    enabled: bool,
    filled: bool,
    size: egui::Vec2,
) -> egui::Response {
    let (fill, stroke, text_color) = if !enabled {
        (
            Color32::from_rgb(236, 236, 240),
            hairline(),
            Color32::from_rgb(174, 174, 178),
        )
    } else if filled {
        (ACCENT, ACCENT, Color32::WHITE)
    } else {
        (Color32::from_rgb(255, 255, 255), Color32::from_rgb(186, 186, 192), LABEL)
    };
    let button = egui::Button::new(egui::RichText::new(text).color(text_color))
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(8);
    if size.x > 0.0 {
        ui.add_sized(size, button)
    } else {
        ui.add(button.min_size(egui::vec2(0.0, size.y.max(32.0))))
    }
}
