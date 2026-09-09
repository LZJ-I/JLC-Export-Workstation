//! 浅色 / 深色卡片主题：系统蓝、大圆角、不透明底板。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontDefinitions, FontFamily, FontId, Sense, Shadow, Stroke,
    Visuals,
};
use std::cell::Cell;

pub const ACCENT: Color32 = Color32::from_rgb(0, 122, 255);
pub const LABEL: Color32 = Color32::from_rgb(29, 29, 31);
pub const SECONDARY: Color32 = Color32::from_rgb(90, 90, 95);
pub const WINDOW_BG: Color32 = Color32::from_rgb(242, 242, 247);
pub const WELL: Color32 = Color32::from_rgb(236, 236, 241);

thread_local! {
    static DARK: Cell<bool> = const { Cell::new(false) };
}

pub fn is_dark() -> bool {
    DARK.with(Cell::get)
}

pub fn set_dark(dark: bool) {
    DARK.with(|c| c.set(dark));
}

pub fn fill() -> Color32 {
    if is_dark() {
        Color32::from_rgb(44, 44, 46)
    } else {
        Color32::from_rgb(255, 255, 255)
    }
}
pub fn fill_strong() -> Color32 {
    if is_dark() {
        Color32::from_rgb(58, 58, 60)
    } else {
        Color32::from_rgb(248, 248, 250)
    }
}
pub fn hairline() -> Color32 {
    if is_dark() {
        Color32::from_rgb(72, 72, 74)
    } else {
        Color32::from_rgb(224, 224, 229)
    }
}
pub fn window_bg() -> Color32 {
    if is_dark() {
        Color32::from_rgb(28, 28, 30)
    } else {
        WINDOW_BG
    }
}
pub fn well() -> Color32 {
    if is_dark() {
        Color32::from_rgb(58, 58, 60)
    } else {
        WELL
    }
}
pub fn label() -> Color32 {
    if is_dark() {
        Color32::from_rgb(242, 242, 247)
    } else {
        LABEL
    }
}
pub fn secondary() -> Color32 {
    if is_dark() {
        Color32::from_rgb(188, 188, 192)
    } else {
        SECONDARY
    }
}

pub fn disabled_text() -> Color32 {
    if is_dark() {
        Color32::from_rgb(174, 174, 178)
    } else {
        Color32::from_rgb(99, 99, 102)
    }
}

pub fn apply(ctx: &egui::Context) {
    install_cjk_fonts(ctx);
    apply_visuals(ctx, false);
}

pub fn apply_visuals(ctx: &egui::Context, dark: bool) {
    set_dark(dark);
    let mut visuals = if dark { Visuals::dark() } else { Visuals::light() };
    visuals.window_fill = window_bg();
    visuals.panel_fill = window_bg();
    visuals.extreme_bg_color = fill();
    visuals.faint_bg_color = well();
    visuals.widgets.inactive.bg_fill = fill_strong();
    visuals.widgets.inactive.weak_bg_fill = fill_strong();
    visuals.widgets.hovered.bg_fill = if dark {
        Color32::from_rgb(72, 72, 74)
    } else {
        Color32::from_rgb(240, 240, 245)
    };
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, label());
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, label());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, label());
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, label());
    visuals.weak_text_color = Some(secondary());
    visuals.widgets.inactive.corner_radius = CornerRadius::same(10);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(10);
    visuals.widgets.active.corner_radius = CornerRadius::same(10);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(12);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, hairline());
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0, 122, 255, 36);
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0_f32, hairline());
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.override_text_color = Some(label());
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

/// `.strong()` 用的是 active 白字，浅色底上会看不见。标题一律走这里。
pub fn heading(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).strong().color(label())
}

/// Markdown 的标题/加粗走 `strong_text_color()`（即 active 白字）。浅色底上必须改回深色。
pub fn show_markdown(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.override_text_color = Some(label());
        v.widgets.active.fg_stroke = Stroke::new(1.0_f32, label());
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, label());
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

pub fn show_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> egui::InnerResponse<()> {
    card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    })
}

/// VS Code Codicons `layout-sidebar-left`，左缘与导航图标对齐。
pub fn menu_toggle(ui: &mut egui::Ui, icon: Option<&egui::TextureHandle>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let bubble = rect.shrink2(egui::vec2(4.0, 3.0));
    if resp.hovered() {
        ui.painter().rect_filled(bubble, 10.0, well());
    }
    let ir = egui::Rect::from_center_size(
        egui::pos2(bubble.left() + 16.0, bubble.center().y),
        egui::vec2(16.0, 16.0),
    );
    let tint = if resp.hovered() { ACCENT } else { label() };
    if let Some(tex) = icon {
        ui.painter().image(
            tex.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp
}

pub fn nav_icon_item(
    ui: &mut egui::Ui,
    icon: Option<&egui::TextureHandle>,
    text: &str,
    selected: bool,
    expanded: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let bubble = rect.shrink2(egui::vec2(4.0, 3.0));
    let bg = if selected {
        Color32::from_rgba_unmultiplied(0, 122, 255, 48)
    } else if resp.hovered() {
        well()
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(bubble, 10.0, bg);
    }
    let text_x = if let Some(tex) = icon {
        let ir = egui::Rect::from_center_size(
            egui::pos2(bubble.left() + 16.0, bubble.center().y),
            egui::vec2(16.0, 16.0),
        );
        ui.painter().image(
            tex.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            label(),
        );
        bubble.left() + 34.0
    } else {
        bubble.left() + 12.0
    };
    if expanded {
        ui.painter().text(
            egui::pos2(text_x, bubble.center().y + 1.0),
            Align2::LEFT_CENTER,
            text,
            FontId::new(14.0, FontFamily::Proportional),
            label(),
        );
        resp
    } else {
        resp.on_hover_text(text)
    }
}

pub fn prepare_menu(ui: &mut egui::Ui) {
    ui.set_min_width(176.0);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
}

pub fn pay_icon(
    ui: &mut egui::Ui,
    brand: &egui::TextureHandle,
    flag: Option<&egui::TextureHandle>,
    title: &str,
    hint: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 8.0, well());
    }
    let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(32.0, 32.0));
    p.image(
        brand.id(),
        icon,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if let Some(flag) = flag {
        let badge = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 19.0, rect.bottom() - 13.0),
            egui::vec2(18.0, 12.0),
        );
        p.rect_filled(badge, 2.0, Color32::from_rgb(238, 28, 37));
        p.image(
            flag.id(),
            badge,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    resp.on_hover_text(format!("{title} - {hint}"))
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
            if is_dark() {
                Color32::from_rgb(58, 58, 60)
            } else {
                Color32::from_rgb(236, 236, 240)
            },
            hairline(),
            disabled_text(),
        )
    } else if filled {
        (ACCENT, ACCENT, Color32::WHITE)
    } else {
        (fill(), hairline(), label())
    };
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            text.to_string(),
            FontId::new(13.5, FontFamily::Proportional),
            text_color,
        )
    });
    let w = if size.x > 0.0 {
        size.x
    } else {
        (galley.size().x + 24.0).max(48.0)
    };
    let h = size.y.max(32.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    let mut fill = fill;
    if enabled && resp.hovered() {
        fill = if filled {
            Color32::from_rgb(10, 132, 255)
        } else {
            well()
        };
    }
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    p.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0_f32, stroke),
        egui::StrokeKind::Inside,
    );
    p.galley(
        egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5 + 1.0,
        ),
        galley,
        text_color,
    );
    resp
}

pub fn edit_single<'a>(text: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .text_color(label())
        .hint_text(egui::RichText::new(hint).color(secondary()))
}

pub fn search_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    id: &'static str,
    size: egui::Vec2,
) -> egui::Response {
    ui.add_sized(
        size,
        edit_single(text, hint)
            .id(egui::Id::new(id))
            .vertical_align(egui::Align::Center)
            .margin(egui::Margin {
                left: 10,
                right: 8,
                top: 8,
                bottom: 4,
            }),
    )
}

/// 深色为开启（滑块在右），浅色为关闭。
pub fn theme_switch(ui: &mut egui::Ui, dark: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 32.0), Sense::click());
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(32.0, 16.0));
    ui.painter().rect_stroke(
        track,
        8.0,
        Stroke::new(1.0_f32, hairline()),
        egui::StrokeKind::Inside,
    );
    if resp.hovered() {
        ui.painter().rect_filled(
            track,
            8.0,
            Color32::from_rgba_unmultiplied(128, 128, 128, 28),
        );
    }
    let thumb_x = if dark {
        track.right() - 8.0
    } else {
        track.left() + 8.0
    };
    ui.painter().circle_filled(
        egui::pos2(thumb_x, track.center().y),
        6.0,
        label(),
    );
    resp
}

pub fn pin_button(ui: &mut egui::Ui, pinned: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, well());
    }
    let c = rect.center();
    let color = if pinned { ACCENT } else { label() };
    let s = Stroke::new(1.5_f32, color);
    let head = egui::Rect::from_center_size(c + egui::vec2(0.0, -2.2), egui::vec2(10.0, 7.0));
    ui.painter().rect_stroke(head, 1.6, s, egui::StrokeKind::Inside);
    ui.painter().line_segment(
        [egui::pos2(head.left() + 1.0, head.bottom()), egui::pos2(head.right() - 1.0, head.bottom())],
        Stroke::new(1.6_f32, color),
    );
    ui.painter().line_segment(
        [egui::pos2(c.x, head.bottom()), egui::pos2(c.x, c.y + 7.0)],
        s,
    );
    resp
}

pub fn caption_min(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, well());
    }
    let y = rect.center().y;
    ui.painter().line_segment(
        [egui::pos2(rect.center().x - 5.0, y), egui::pos2(rect.center().x + 5.0, y)],
        Stroke::new(1.4_f32, label()),
    );
    resp
}

pub fn caption_max(ui: &mut egui::Ui, maximized: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, well());
    }
    let s = Stroke::new(1.4_f32, label());
    if maximized {
        let back = egui::Rect::from_center_size(rect.center() + egui::vec2(1.8, -1.8), egui::vec2(8.0, 8.0));
        let front = egui::Rect::from_center_size(rect.center() + egui::vec2(-1.6, 1.6), egui::vec2(8.0, 8.0));
        let bg = if resp.hovered() { well() } else { fill() };
        ui.painter().rect_stroke(back, 0.8, s, egui::StrokeKind::Inside);
        ui.painter().rect_filled(front.expand(1.0), 0.8, bg);
        ui.painter().rect_stroke(front, 0.8, s, egui::StrokeKind::Inside);
    } else {
        let boxr = egui::Rect::from_center_size(rect.center(), egui::vec2(10.0, 10.0));
        ui.painter().rect_stroke(boxr, 1.2, s, egui::StrokeKind::Inside);
    }
    resp
}

pub fn caption_close(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_rgb(232, 17, 35));
    }
    let c = rect.center();
    let color = if resp.hovered() {
        Color32::WHITE
    } else {
        label()
    };
    let s = Stroke::new(1.4_f32, color);
    ui.painter().line_segment(
        [c + egui::vec2(-5.0, -5.0), c + egui::vec2(5.0, 5.0)],
        s,
    );
    ui.painter().line_segment(
        [c + egui::vec2(5.0, -5.0), c + egui::vec2(-5.0, 5.0)],
        s,
    );
    resp
}
