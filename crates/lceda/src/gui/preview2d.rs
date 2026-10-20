use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use lceda_core::altium::SchColors;
use lceda_core::altium::sch_preview::{SchPreview, TextAnchor, MM_PER_POINT};
use lceda_core::ir::{FootprintIr, IrPad, SymbolIr};

#[derive(Default)]
pub struct SymbolView {
    zoom: f32,
    pan: Vec2,
    scene: Option<SceneCache>,
}

impl SymbolView {
    pub fn reset(&mut self) { self.zoom = 1.0; self.pan = Vec2::ZERO; self.scene = None; }

    fn transform(&mut self, ui: &mut Ui, rect: Rect, bounds: Rect, id: &str, locked: bool, flip_y: bool) -> ViewTransform {
        if self.zoom <= 0.0 { self.zoom = 1.0; self.pan = Vec2::ZERO; }
        let response = ui.interact(rect, ui.id().with(id), if locked { Sense::hover() } else { Sense::click_and_drag() });
        if !locked && response.double_clicked() { self.zoom = 1.0; self.pan = Vec2::ZERO; }
        if !locked && response.dragged() { self.pan += response.drag_delta(); }
        if !locked && response.hovered() {
            let delta = ui.input(|i| i.raw_scroll_delta.y);
            if delta.abs() > 0.01 {
                let next = scroll_zoom(self.zoom, delta, 0.75, 8.0);
                if let Some(pointer) = response.hover_pos() {
                    self.pan = pointer - rect.center() - (pointer - rect.center() - self.pan) * (next / self.zoom);
                }
                self.zoom = next;
            }
        }
        let fit = ((rect.width() - 40.0).max(1.0) / bounds.width().max(1.0))
            .min((rect.height() - 40.0).max(1.0) / bounds.height().max(1.0));
        let scale = (fit * self.zoom).clamp(0.01, 2000.0);
        self.pan = clamp_pan(self.pan, bounds.size() * scale, rect.size());
        ViewTransform { origin: rect.center() + self.pan, center: bounds.center(), scale, flip_y }
    }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, symbol: &SymbolIr, colors: SchColors, locked: bool) {
        let dpi = ui.ctx().pixels_per_point();
        if self.scene.as_ref().is_none_or(|cache| cache.colors != colors || cache.dpi != dpi) {
            let prepared = lceda_core::altium::schlib::preview(symbol, colors)
                .map(|scene| prepare_scene(ui, scene)).map_err(|error| error.to_string());
            self.scene = Some(SceneCache { colors, dpi, prepared });
        }
        self.paint_scene(ui,rect,colors,locked);
    }

    pub fn show_native(&mut self,ui:&mut Ui,rect:Rect,scene:&SchPreview,locked:bool){
        let colors=SchColors{background:scene.background,..SchColors::default()};
        let dpi=ui.ctx().pixels_per_point();
        if self.scene.as_ref().is_none_or(|cache|cache.dpi!=dpi){
            self.scene=Some(SceneCache{colors,dpi,prepared:Ok(prepare_scene(ui,scene.clone()))});
        }
        self.paint_scene(ui,rect,colors,locked);
    }

    fn paint_scene(&mut self,ui:&mut Ui,rect:Rect,colors:SchColors,locked:bool){
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6, ad_color(colors.background));
        let bounds = match &self.scene.as_ref().unwrap().prepared {
            Ok(prepared) => prepared.bounds,
            Err(error) => {
                painter.text(rect.center(), Align2::CENTER_CENTER, error,
                    FontId::proportional(13.0), Color32::from_rgb(100, 60, 60));
                return;
            }
        };
        let view = self.transform(ui, rect, bounds, "symbol_canvas", locked, true);
        let Ok(prepared) = &self.scene.as_ref().unwrap().prepared else { return; };
        painter.rect_filled(rect, 6, ad_color(prepared.scene.background));
        for stroke in &prepared.scene.strokes {
            let points: Vec<_> = stroke.points.iter().map(|&(x,y)| view.map(x,y)).collect();
            if points.len() >= 2 {
                painter.add(egui::Shape::line(points,
                    Stroke::new((stroke.width as f32 * view.scale).max(0.7), ad_color(stroke.color))));
            }
        }
        for e in &prepared.scene.ellipses {
            let points: Vec<_> = (0..=48).map(|i| {
                let t = i as f64 * std::f64::consts::TAU / 48.0;
                view.map(e.center.0 + e.radii.0 * t.cos(), e.center.1 + e.radii.1 * t.sin())
            }).collect();
            painter.add(egui::Shape::line(points,
                Stroke::new((e.width as f32 * view.scale).max(0.7), ad_color(e.color))));
        }
        // Rasterize at the visible size for crisp small text. At extreme zoom,
        // scale a bounded glyph mesh instead of exhausting the font atlas.
        let raster_scale = view.scale.min(64.0);
        for label in &prepared.scene.texts {
            let mut shape = layout_label(ui, label, raster_scale);
            shape.transform(egui::emath::TSTransform::new(view.map(0.0,0.0).to_vec2(), view.scale/raster_scale));
            painter.add(shape);
        }
    }
}

struct SceneCache {
    colors: SchColors,
    dpi: f32,
    prepared: Result<PreparedScene, String>,
}

struct PreparedScene {
    scene: SchPreview,
    #[cfg(test)]
    labels: Vec<egui::epaint::TextShape>,
    bounds: Rect,
}

fn scene_geometry_bounds(scene: &SchPreview) -> Rect {
    let mut bounds = Rect::NOTHING;
    let mut add = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() { bounds.extend_with(Pos2::new(x as f32, y as f32)); }
    };
    for stroke in &scene.strokes { for &(x,y) in &stroke.points { add(x,y); } }
    for e in &scene.ellipses {
        add(e.center.0-e.radii.0, e.center.1-e.radii.1);
        add(e.center.0+e.radii.0, e.center.1+e.radii.1);
    }
    bounds
}

fn prepare_scene(ui: &Ui, scene: SchPreview) -> PreparedScene {
    // Measure text at a stable resolution to keep fit independent of zoom.
    const REFERENCE_SCALE: f32 = 24.0;
    let mut bounds = scene_geometry_bounds(&scene);
    let labels = scene.texts.iter().map(|label| {
        let mut shape = layout_label(ui, label, REFERENCE_SCALE);
        shape.transform(egui::emath::TSTransform::from_scaling(1.0/REFERENCE_SCALE));
        let text_bounds = shape.visual_bounding_rect();
        if text_bounds.is_finite() {
            bounds.extend_with(Pos2::new(text_bounds.min.x, -text_bounds.max.y));
            bounds.extend_with(Pos2::new(text_bounds.max.x, -text_bounds.min.y));
        }
        shape
    }).collect::<Vec<_>>();
    let _ = &labels;
    if !bounds.is_finite() { bounds = Rect::from_center_size(Pos2::ZERO, Vec2::splat(1.0)); }
    PreparedScene { scene, #[cfg(test)] labels, bounds: bounds.expand(0.5) }
}

fn layout_label(ui: &Ui, label: &lceda_core::altium::sch_preview::SchText, scale: f32) -> egui::epaint::TextShape {
    let color = ad_color(label.color);
    let requested = egui::FontFamily::Name(label.font.name.clone().into());
    let family = ui.fonts(|fonts| if fonts.families().contains(&requested){requested}else{egui::FontFamily::Proportional});
    let font = FontId::new((label.font.points * MM_PER_POINT) as f32 * scale, family);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(label.text.clone(), font, color));
    let anchor = match label.anchor {
        TextAnchor::LeftBottom => Align2::LEFT_BOTTOM,
        TextAnchor::LeftCenter => Align2::LEFT_CENTER,
        TextAnchor::RightCenter => Align2::RIGHT_CENTER,
        TextAnchor::CenterBottom => Align2::CENTER_BOTTOM,
    };
    let position = Pos2::new(label.position.0 as f32, -label.position.1 as f32) * scale;
    let offset = anchor.pos_in_rect(&galley.rect).to_vec2();
    egui::epaint::TextShape::new(position-offset, galley, color)
        .with_angle_and_anchor(-label.rotation.to_radians() as f32, anchor)
}

struct ViewTransform {
    origin: Pos2,
    center: Pos2,
    scale: f32,
    flip_y: bool,
}

impl ViewTransform {
    fn map(&self, x: f64, y: f64) -> Pos2 {
        self.origin + Vec2::new((x as f32 - self.center.x) * self.scale,
            (y as f32 - self.center.y) * self.scale * if self.flip_y { -1.0 } else { 1.0 })
    }
}

#[derive(Default)]
pub struct FootprintView {
    camera: SymbolView,
}

impl FootprintView {
    pub fn reset(&mut self) { self.camera.reset(); }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, footprint: &FootprintIr, locked: bool) {
        let painter = ui.painter_at(rect);
        let background = Color32::from_rgb(26, 35, 46);
        painter.rect_filled(rect, 6, background);
        let Some(bounds) = footprint_bounds(footprint) else { return; };
        // Footprint IR uses the same downward Y axis as its exported KiCad footprint.
        let view = self.camera.transform(ui, rect, bounds, "footprint_canvas", locked, false);
        let map_points = |points: &[(f64, f64)]| points.iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite()).map(|&(x, y)| view.map(x, y)).collect::<Vec<_>>();
        for region in &footprint.regions {
            fill_polygon(&painter, map_points(&region.points), layer_color(region.layer).gamma_multiply(0.55));
        }
        for track in &footprint.tracks {
            let points = map_points(&track.points);
            if points.len() >= 2 {
                painter.add(egui::Shape::line(points, Stroke::new((track.width as f32 * view.scale).max(1.0), layer_color(track.layer))));
            }
        }
        for circle in &footprint.circles {
            if circle.radius.is_finite() && circle.radius > 0.0 {
                painter.circle_stroke(view.map(circle.x, circle.y), circle.radius as f32 * view.scale,
                    Stroke::new((circle.width as f32 * view.scale).max(1.0), layer_color(circle.layer)));
            }
        }
        for arc in &footprint.arcs {
            let sweep = (arc.end - arc.start).rem_euclid(360.0);
            let steps = (sweep / 3.0).ceil().clamp(2.0, 120.0) as usize;
            let points = (0..=steps).map(|i| {
                let angle = (arc.start + sweep * i as f64 / steps as f64).to_radians();
                view.map(arc.x + arc.radius * angle.cos(), arc.y + arc.radius * angle.sin())
            }).collect();
            painter.add(egui::Shape::line(points, Stroke::new((arc.width as f32 * view.scale).max(1.0), layer_color(arc.layer))));
        }
        for pad in &footprint.pads {
            let copper = if pad.layer == 2 { Color32::from_rgb(115, 175, 245) } else { Color32::from_rgb(232, 185, 91) };
            fill_polygon(&painter, map_points(&pad_outline(pad)), copper);
            if pad.hole > 1e-6 {
                let slot = if pad.hole_slot > 1e-6 { pad.hole_slot } else { pad.hole };
                let hole_shape = pad.hole_shape.to_ascii_uppercase();
                let radius = if hole_shape.contains("SQUARE") || hole_shape.contains("RECT") { 0.0 } else { pad.hole.min(slot) * 0.5 };
                let drill = rounded_outline(pad.hole, slot, radius);
                let points = drill.into_iter().map(|(x, y)| rotate_point(pad.x, pad.y, x, y, pad.rotation)).collect::<Vec<_>>();
                fill_polygon(&painter, map_points(&points), background);
            }
            let size = pad.width.min(pad.height) as f32 * view.scale;
            if size >= 12.0 && !pad.designator.is_empty() {
                painter.text(view.map(pad.x, pad.y), Align2::CENTER_CENTER, &pad.designator,
                    FontId::proportional((size * 0.35).clamp(7.0, 16.0)),
                    if pad.hole > 1e-6 { Color32::WHITE } else { Color32::from_rgb(40, 34, 20) });
            }
        }
        // Dimensions stay in mm regardless of zoom.
        painter.text(rect.left_bottom() + Vec2::new(12.0, -10.0), Align2::LEFT_BOTTOM,
            format!("{:.2} × {:.2} mm", bounds.width(), bounds.height()), FontId::proportional(12.0), Color32::from_rgb(171, 184, 200));
    }
}

fn layer_color(layer: i32) -> Color32 {
    match layer {
        1 => Color32::from_rgb(232, 185, 91),
        2 => Color32::from_rgb(115, 175, 245),
        3 | 4 | 49 => Color32::from_rgb(231, 237, 224),
        5..=8 => Color32::from_rgb(110, 175, 158),
        _ => Color32::from_rgb(160, 144, 190),
    }
}

fn rotate_point(cx: f64, cy: f64, x: f64, y: f64, rotation: f64) -> (f64, f64) {
    let (sin, cos) = rotation.to_radians().sin_cos();
    (cx + x * cos - y * sin, cy + x * sin + y * cos)
}

/// Rounded rectangle, including circles, ellipses and oblong pads, in local coordinates.
fn rounded_outline(width: f64, height: f64, radius: f64) -> Vec<(f64, f64)> {
    let hw = width.max(0.0) * 0.5;
    let hh = height.max(0.0) * 0.5;
    let r = radius.max(0.0).min(hw).min(hh);
    let mut points = Vec::with_capacity(36);
    for (cx, cy, start) in [(hw - r, hh - r, 0.0), (-hw + r, hh - r, 90.0),
        (-hw + r, -hh + r, 180.0), (hw - r, -hh + r, 270.0)] {
        for i in 0..=8 {
            let angle = (start + i as f64 * 90.0 / 8.0).to_radians();
            let point = (cx + r * angle.cos(), cy + r * angle.sin());
            if points.last() != Some(&point) { points.push(point); }
        }
    }
    let coincident = |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).hypot(a.1 - b.1) < 1e-9;
    points.dedup_by(|a, b| coincident(a, b));
    if points.len() > 1 && coincident(&points[0], points.last().unwrap()) { points.pop(); }
    points
}

fn pad_outline(pad: &IrPad) -> Vec<(f64, f64)> {
    if pad.is_custom_poly() { return pad.polygon.clone().unwrap_or_default(); }
    let radius = match pad.shape.to_ascii_uppercase().as_str() {
        "ROUND" | "OVAL" | "ELLIPSE" | "CIRCLE" => pad.width.min(pad.height) * 0.5,
        _ => 0.0,
    };
    rounded_outline(pad.width, pad.height, radius).into_iter()
        .map(|(x, y)| rotate_point(pad.x, pad.y, x, y, pad.rotation)).collect()
}

fn footprint_bounds(footprint: &FootprintIr) -> Option<Rect> {
    let mut points = Vec::new();
    let mut add = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() { points.push(Pos2::new(x as f32, y as f32)); }
    };
    for pad in &footprint.pads { for (x, y) in pad_outline(pad) { add(x, y); } }
    for track in &footprint.tracks {
        for &(x, y) in &track.points {
            let half = track.width.max(0.0) * 0.5;
            add(x - half, y - half); add(x + half, y + half);
        }
    }
    for circle in &footprint.circles {
        let r = circle.radius + circle.width.max(0.0) * 0.5;
        add(circle.x - r, circle.y - r); add(circle.x + r, circle.y + r);
    }
    for arc in &footprint.arcs {
        let sweep = (arc.end - arc.start).rem_euclid(360.0);
        let r = arc.radius;
        let half = arc.width.max(0.0) * 0.5;
        for angle in [arc.start, arc.end, 0.0, 90.0, 180.0, 270.0] {
            if angle == arc.start || angle == arc.end || (angle - arc.start).rem_euclid(360.0) <= sweep {
                let angle = angle.to_radians();
                let (x, y) = (arc.x + r * angle.cos(), arc.y + r * angle.sin());
                add(x - half, y - half); add(x + half, y + half);
            }
        }
    }
    for region in &footprint.regions { for &(x, y) in &region.points { add(x, y); } }
    if points.is_empty() { None } else { Some(Rect::from_points(&points)) }
}

/// Ear clipping handles concave custom copper pads without filling their cutouts.
fn polygon_triangles(points: &[Pos2]) -> Vec<[usize; 3]> {
    if points.len() < 3 { return Vec::new(); }
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let area: f32 = points.iter().zip(points.iter().cycle().skip(1)).take(points.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    let sign = if area < 0.0 { -1.0 } else { 1.0 };
    let mut remaining: Vec<_> = (0..points.len()).collect();
    let mut triangles = Vec::new();
    while remaining.len() >= 3 {
        let mut ear = None;
        for i in 0..remaining.len() {
            let a = remaining[(i + remaining.len() - 1) % remaining.len()];
            let b = remaining[i]; let c = remaining[(i + 1) % remaining.len()];
            if cross(points[a], points[b], points[c]) * sign <= 0.0001 { continue; }
            let occupied = remaining.iter().any(|&p| p != a && p != b && p != c
                && cross(points[a], points[b], points[p]) * sign >= -0.0001
                && cross(points[b], points[c], points[p]) * sign >= -0.0001
                && cross(points[c], points[a], points[p]) * sign >= -0.0001);
            if !occupied { ear = Some((i, [a, b, c])); break; }
        }
        let Some((i, triangle)) = ear else { break; };
        triangles.push(triangle);
        remaining.remove(i);
    }
    triangles
}

fn fill_polygon(painter: &egui::Painter, mut points: Vec<Pos2>, color: Color32) {
    points.dedup_by(|a, b| a.distance_sq(*b) < 0.0001);
    if points.len() > 1 && points[0].distance_sq(*points.last().unwrap()) < 0.0001 { points.pop(); }
    let triangles = polygon_triangles(&points);
    let mut mesh = egui::epaint::Mesh::default();
    for point in &points { mesh.colored_vertex(*point, color); }
    for [a, b, c] in triangles { mesh.add_triangle(a as u32, b as u32, c as u32); }
    painter.add(egui::Shape::mesh(mesh));
    if points.len() >= 2 { painter.add(egui::Shape::closed_line(points, Stroke::new(0.7_f32, color))); }
}

/// Use raw wheel input once, with reciprocal zoom-in/out steps and no inertial drift.
pub(super) fn scroll_zoom(zoom: f32, delta: f32, minimum: f32, maximum: f32) -> f32 {
    if !delta.is_finite() { return zoom.clamp(minimum, maximum); }
    (zoom * (delta * 0.0012).clamp(-1.0, 1.0).exp()).clamp(minimum, maximum)
}

fn clamp_pan(pan: Vec2, content: Vec2, viewport: Vec2) -> Vec2 {
    let limit = ((content - viewport + Vec2::splat(40.0)) * 0.5).max(Vec2::ZERO);
    Vec2::new(pan.x.clamp(-limit.x, limit.x), pan.y.clamp(-limit.y, limit.y))
}

fn ad_color(value: i32) -> Color32 {
    Color32::from_rgb((value & 255) as u8, ((value >> 8) & 255) as u8, ((value >> 16) & 255) as u8)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_includes_exported_designator_long_comment_and_rotated_pin_text() {
        let symbol = SymbolIr { name: "ESP32-S3-WROOM-1U-N16R8".into(), description: String::new(), designator: "U?".into(),
            meta: Default::default(), pins: vec![lceda_core::ir::IrPin {
                name: "GPIO_WITH_LONG_NAME".into(), number: "12".into(), rotation: 270.0, ..Default::default() }],
            rects: vec![lceda_core::ir::IrRect { x1: -2.54, y1: -2.54, x2: 2.54, y2: 2.54 }],
            polys: vec![], ellipses: vec![] };
        let ctx = egui::Context::default();
        super::super::theme::apply(&ctx);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let prepared = prepare_scene(ui, lceda_core::altium::schlib::preview(&symbol, SchColors::default()).unwrap());
                assert!(prepared.bounds.width() > 20.0);
                assert!(prepared.bounds.height() > 10.0);
                for label in &prepared.labels {
                    let b = label.visual_bounding_rect();
                    assert!(prepared.bounds.contains(Pos2::new(b.min.x, -b.max.y)));
                    assert!(prepared.bounds.contains(Pos2::new(b.max.x, -b.min.y)));
                }
            });
        });
    }

    fn footprint_fixture() -> FootprintIr {
        let json = serde_json::from_str(include_str!("../../../lceda-core/tests/fixtures/FNR3015S2R2MT_footprint_easyeda.json")).unwrap();
        let source = lceda_core::easyeda::parse_footprint(&json).unwrap();
        lceda_core::ir::footprint_ir("FNR3015", "", source, Default::default())
    }

    #[test]
    fn footprint_fit_includes_silkscreen_and_rotated_pads() {
        let mut footprint = footprint_fixture();
        let bounds = footprint_bounds(&footprint).unwrap();
        assert!(bounds.width() > 3.0 && bounds.height() > 2.5);
        for pad in &footprint.pads {
            for (x, y) in pad_outline(pad) { assert!(bounds.contains(Pos2::new(x as f32, y as f32))); }
        }
        footprint.pads.truncate(1);
        let pad = &mut footprint.pads[0];
        pad.x = 0.0; pad.y = 0.0; pad.width = 4.0; pad.height = 2.0; pad.rotation = 90.0;
        let points = pad_outline(pad).into_iter().map(|(x, y)| Pos2::new(x as f32, y as f32)).collect::<Vec<_>>();
        let rotated = Rect::from_points(&points);
        assert!((rotated.width() - 2.0).abs() < 0.001);
        assert!((rotated.height() - 4.0).abs() < 0.001);
    }

    #[test]
    fn concave_custom_pad_retains_its_notch_in_both_windings() {
        let outline = vec![Pos2::new(0.0, 0.0), Pos2::new(3.0, 0.0), Pos2::new(3.0, 1.0),
            Pos2::new(1.0, 1.0), Pos2::new(1.0, 3.0), Pos2::new(0.0, 3.0)];
        for points in [outline.clone(), outline.into_iter().rev().collect()] {
            let triangles = polygon_triangles(&points);
            assert_eq!(triangles.len(), 4);
            let area: f32 = triangles.iter().map(|&[a, b, c]| {
                let ab = points[b] - points[a]; let ac = points[c] - points[a];
                (ab.x * ac.y - ab.y * ac.x).abs() * 0.5
            }).sum();
            assert!((area - 5.0).abs() < 0.0001, "must not fill the 2 by 2 notch");
        }
    }

    #[test]
    fn slots_and_oval_pads_keep_their_physical_dimensions() {
        for (width, height) in [(1.0, 3.0), (3.0, 1.0), (2.0, 2.0)] {
            let points: Vec<_> = rounded_outline(width, height, width.min(height) * 0.5)
                .into_iter().map(|(x, y)| Pos2::new(x as f32, y as f32)).collect();
            let bounds = Rect::from_points(&points);
            assert!((bounds.width() - width as f32).abs() < 0.001);
            assert!((bounds.height() - height as f32).abs() < 0.001);
            assert_eq!(polygon_triangles(&points).len(), points.len() - 2);
        }
    }

    #[test]
    fn oval_shape_aliases_preview_with_round_ends_without_square_corners() {
        let mut pad = footprint_fixture().pads.remove(0);
        pad.x = 0.0; pad.y = 0.0; pad.rotation = 0.0;
        pad.width = 1.0; pad.height = 2.0; pad.polygon = None;
        for shape in ["OVAL", "ELLIPSE", "ROUND", "CIRCLE"] {
            pad.shape = shape.into();
            let points: Vec<_> = pad_outline(&pad).into_iter()
                .map(|(x, y)| Pos2::new(x as f32, y as f32)).collect();
            let bounds = Rect::from_points(&points);
            assert!((bounds.width() - 1.0).abs() < 0.001, "{shape}");
            assert!((bounds.height() - 2.0).abs() < 0.001, "{shape}");
            assert!(points.len() > 4, "{shape} must have curved ends");
            assert!(points.iter().all(|p| p.x.abs() < 0.49 || p.y.abs() < 0.99),
                "{shape} must not include the rectangle's corners");
        }
    }

    #[test]
    fn wheel_steps_are_reversible_and_do_not_drift_without_new_input() {
        let next = scroll_zoom(1.0, 120.0, 0.75, 8.0);
        assert!(next > 1.0 && next < 1.2);
        let restored = scroll_zoom(next, -120.0, 0.75, 8.0);
        assert!((restored - 1.0).abs() < 0.00001);
        assert_eq!(scroll_zoom(next, 0.0, 0.75, 8.0), next);
        assert_eq!(scroll_zoom(1.0, -100_000.0, 0.75, 8.0), 0.75);
        assert_eq!(scroll_zoom(8.0, 120.0, 0.75, 8.0), 8.0);
    }

    #[test]
    fn small_content_is_centered_and_zoomed_content_cannot_pan_out_of_view() {
        assert_eq!(clamp_pan(Vec2::new(500.0, -500.0), Vec2::new(150.0, 200.0), Vec2::new(220.0, 300.0)), Vec2::ZERO);
        assert_eq!(clamp_pan(Vec2::new(500.0, -500.0), Vec2::new(440.0, 600.0), Vec2::new(220.0, 300.0)), Vec2::new(130.0, -170.0));
    }
    #[test]
    fn bounds_include_electrical_endpoints_and_ignore_nonfinite_geometry() {
        let symbol = SymbolIr { name: "R".into(), description: String::new(), designator: "R?".into(), meta: Default::default(),
            pins: vec![lceda_core::ir::IrPin { x: 0.0, y: 0.0, length: 10.0, rotation: 180.0, ..Default::default() }],
            rects: vec![], polys: vec![vec![(f64::NAN, 0.0)]], ellipses: vec![] };
        let scene = lceda_core::altium::schlib::preview(&symbol, SchColors::default()).unwrap();
        let bounds = scene_geometry_bounds(&scene);
        // Pins use the quantized DXP coordinates and lengths of the AD record.
        assert!(bounds.min.x <= -9.9 && bounds.max.x >= 0.0);
    }
}
