//! Estilo visual de cada expresión (color, trazo, grosor, opacidad, relleno) y su editor.

use eframe::egui::{self, Color32, Painter, Pos2, Sense, Shape, Stroke, pos2, vec2};

pub const PALETTE: [[u8; 3]; 10] = [
    [0xc7, 0x44, 0x40], // rojo
    [0x2d, 0x70, 0xb3], // azul
    [0x38, 0x8c, 0x46], // verde
    [0x60, 0x42, 0xa6], // morado
    [0xfa, 0x7e, 0x19], // naranja
    [0x00, 0x9c, 0x9c], // turquesa
    [0xd6, 0x3a, 0x8f], // rosa
    [0x8a, 0x6a, 0x3a], // marrón
    [0x70, 0x70, 0x70], // gris
    [0x00, 0x00, 0x00], // negro
];

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum PointStyle {
    Filled,
    Hollow,
    Cross,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct Style {
    pub color: [u8; 3],
    pub width: f32,
    pub line: LineStyle,
    /// Opacidad del trazo (0–1).
    pub opacity: f32,
    /// Opacidad del sombreado de las desigualdades (0–1).
    pub fill_opacity: f32,
    pub point: PointStyle,
    pub point_size: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: PALETTE[0],
            width: 2.5,
            line: LineStyle::Solid,
            opacity: 1.0,
            fill_opacity: 0.25,
            point: PointStyle::Filled,
            point_size: 5.0,
        }
    }
}

impl Style {
    pub fn with_palette(index: usize) -> Self {
        Self { color: PALETTE[index % PALETTE.len()], ..Default::default() }
    }

    /// Color base; los muy oscuros se aclaran con tema oscuro para que se vean.
    pub fn base_color(&self, dark: bool) -> Color32 {
        let [r, g, b] = self.color;
        let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
        if dark && luma < 40.0 { Color32::from_gray(225) } else { Color32::from_rgb(r, g, b) }
    }

    pub fn stroke_color(&self, dark: bool) -> Color32 {
        self.base_color(dark).gamma_multiply(self.opacity)
    }

    pub fn fill_color(&self, dark: bool) -> Color32 {
        self.base_color(dark).gamma_multiply(self.fill_opacity)
    }
}

/// Dibuja una polilínea con el tipo de trazo indicado.
pub fn draw_polyline(painter: &Painter, pts: &[Pos2], line: LineStyle, color: Color32, width: f32) {
    if pts.len() < 2 {
        return;
    }
    match line {
        LineStyle::Solid => {
            painter.line(pts.to_vec(), Stroke::new(width, color));
        }
        LineStyle::Dashed => {
            let dash = 3.0 * width + 6.0;
            let gap = 2.0 * width + 4.0;
            painter.extend(Shape::dashed_line(pts, Stroke::new(width, color), dash, gap));
        }
        LineStyle::Dotted => {
            let spacing = 2.0 * width + 3.0;
            painter.extend(Shape::dotted_line(pts, color, spacing, width * 0.65));
        }
    }
}

pub fn draw_point(painter: &Painter, p: Pos2, style: &Style, color: Color32) {
    let r = style.point_size;
    match style.point {
        PointStyle::Filled => {
            painter.circle_filled(p, r, color);
        }
        PointStyle::Hollow => {
            painter.circle_stroke(p, r - 1.0, Stroke::new(2.0, color));
        }
        PointStyle::Cross => {
            let s = Stroke::new(2.5, color);
            let d = r * 0.9;
            painter.line_segment([p + vec2(-d, -d), p + vec2(d, d)], s);
            painter.line_segment([p + vec2(-d, d), p + vec2(d, -d)], s);
        }
    }
}

/// Qué controles tienen sentido para una expresión.
pub struct EditorTarget {
    pub has_line: bool,
    pub has_fill: bool,
    pub is_point: bool,
}

/// Editor de estilo (se muestra dentro de un popup). Devuelve si algo cambió.
pub fn style_editor(ui: &mut egui::Ui, style: &mut Style, visible: &mut bool, target: &EditorTarget) -> bool {
    let before = (*style, *visible);
    let dark = ui.visuals().dark_mode;
    ui.set_min_width(250.0);

    ui.checkbox(visible, "Visible");
    ui.separator();

    ui.label("Color");
    ui.horizontal_wrapped(|ui| {
        for c in PALETTE {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            let shown = Style { color: c, ..*style }.base_color(dark);
            ui.painter().circle_filled(r.center(), 9.0, shown);
            if style.color == c {
                ui.painter().circle_stroke(r.center(), 10.5, Stroke::new(2.0, ui.visuals().strong_text_color()));
            }
            if resp.clicked() {
                style.color = c;
            }
        }
    });
    ui.collapsing("Color personalizado", |ui| {
        let mut c = Color32::from_rgb(style.color[0], style.color[1], style.color[2]);
        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
            style.color = [c.r(), c.g(), c.b()];
        }
    });

    if target.has_line {
        ui.separator();
        ui.label("Trazo");
        ui.horizontal(|ui| {
            for (ls, tip) in [(LineStyle::Solid, "Sólida"), (LineStyle::Dashed, "Segmentada"), (LineStyle::Dotted, "Punteada")] {
                let (r, resp) = ui.allocate_exact_size(vec2(64.0, 26.0), Sense::click());
                let selected = style.line == ls;
                let vis = if selected { ui.visuals().selection.bg_fill } else { ui.visuals().widgets.inactive.weak_bg_fill };
                ui.painter().rect_filled(r, 4.0, vis);
                let pts = [pos2(r.left() + 8.0, r.center().y), pos2(r.right() - 8.0, r.center().y)];
                draw_polyline(ui.painter(), &pts, ls, style.base_color(dark), 2.5);
                if resp.on_hover_text(tip).clicked() {
                    style.line = ls;
                }
            }
        });
        ui.add(egui::Slider::new(&mut style.width, 0.5..=12.0).text("Grosor").step_by(0.5));
        ui.add(egui::Slider::new(&mut style.opacity, 0.05..=1.0).text("Opacidad"));
    }

    if target.has_fill {
        ui.add(egui::Slider::new(&mut style.fill_opacity, 0.0..=1.0).text("Relleno"));
    }

    if target.is_point {
        ui.separator();
        ui.label("Punto");
        ui.horizontal(|ui| {
            for (ps, tip) in [(PointStyle::Filled, "Relleno"), (PointStyle::Hollow, "Hueco"), (PointStyle::Cross, "Cruz")] {
                let (r, resp) = ui.allocate_exact_size(vec2(40.0, 30.0), Sense::click());
                let vis = if style.point == ps { ui.visuals().selection.bg_fill } else { ui.visuals().widgets.inactive.weak_bg_fill };
                ui.painter().rect_filled(r, 4.0, vis);
                draw_point(ui.painter(), r.center(), &Style { point: ps, point_size: 6.0, ..*style }, style.base_color(dark));
                if resp.on_hover_text(tip).clicked() {
                    style.point = ps;
                }
            }
        });
        ui.add(egui::Slider::new(&mut style.point_size, 2.0..=15.0).text("Tamaño"));
        ui.add(egui::Slider::new(&mut style.opacity, 0.05..=1.0).text("Opacidad"));
    }

    before != (*style, *visible)
}
