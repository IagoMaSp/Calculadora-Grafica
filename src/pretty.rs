//! Composición tipográfica al estilo TeX: cajas con línea base, texto, trazos y
//! delimitadores. La usa el editor matemático para dibujar las expresiones.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Galley, Painter, Pos2, Stroke, Vec2, pos2, vec2};
use eframe::epaint::text::{LayoutJob, TextFormat};


// ───────────────────────────── Composición ─────────────────────────────

/// Caja compuesta: ancho, altura sobre la línea base y profundidad bajo ella.
/// Los elementos se guardan relativos al punto (izquierda, línea base).
pub struct Layout {
    pub w: f32,
    pub asc: f32,
    pub desc: f32,
    pub(crate) prims: Vec<Prim>,
}

pub(crate) enum Prim {
    /// Texto: posición de su línea base y galería ya compuesta (con la distancia de la
    /// parte superior de la galería a su línea base).
    Text { at: Vec2, galley: Arc<Galley>, baseline: f32 },
    /// Polilínea de grosor relativo al tamaño de letra.
    Line { points: Vec<Vec2>, width: f32 },
    /// Casilla vacía del editor (rectángulo tenue).
    Slot { min: Vec2, max: Vec2 },
}

impl Layout {
    pub(crate) fn empty() -> Self {
        Self { w: 0.0, asc: 0.0, desc: 0.0, prims: Vec::new() }
    }

    /// Añade `other` desplazado `(dx, dy)` (dy positivo = hacia abajo).
    pub(crate) fn place(&mut self, other: Layout, dx: f32, dy: f32) {
        for p in other.prims {
            self.prims.push(match p {
                Prim::Text { at, galley, baseline } => Prim::Text { at: at + vec2(dx, dy), galley, baseline },
                Prim::Line { points, width } => Prim::Line { points: points.into_iter().map(|q| q + vec2(dx, dy)).collect(), width },
                Prim::Slot { min, max } => Prim::Slot { min: min + vec2(dx, dy), max: max + vec2(dx, dy) },
            });
        }
    }

    pub fn height(&self) -> f32 {
        self.asc + self.desc
    }

    /// Dibuja la caja con su punto (izquierda, línea base) en `origin`.
    pub fn paint(&self, painter: &Painter, origin: Pos2, color: Color32) {
        for p in &self.prims {
            match p {
                Prim::Text { at, galley, baseline } => {
                    painter.galley(origin + *at - vec2(0.0, *baseline), galley.clone(), color);
                }
                Prim::Line { points, width } => {
                    let pts: Vec<Pos2> = points.iter().map(|q| origin + *q).collect();
                    painter.line(pts, Stroke::new(*width, color));
                }
                Prim::Slot { min, max } => {
                    let r = egui::Rect::from_min_max(origin + *min, origin + *max);
                    painter.rect_filled(r, 2.0, color.gamma_multiply(0.13));
                    painter.rect_stroke(r, 2.0, Stroke::new(1.0, color.gamma_multiply(0.35)), egui::StrokeKind::Inside);
                }
            }
        }
    }
}

/// Composición de un árbol con letra de tamaño `size`.
pub struct Composer<'a> {
    pub painter: &'a Painter,
    pub color: Color32,
}

/// Altura del "eje matemático" (donde van la raya de fracción y el signo menos).
pub(crate) fn axis(size: f32) -> f32 {
    0.27 * size
}

impl Composer<'_> {
    pub(crate) fn text(&self, s: &str, size: f32, italic: bool) -> Layout {
        let format = TextFormat { font_id: FontId::proportional(size), color: self.color, italics: italic, ..Default::default() };
        let galley = self.painter.layout_job(LayoutJob::single_section(s.to_string(), format));
        let baseline = galley
            .rows
            .first()
            .and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y))
            .unwrap_or(galley.size().y * 0.8);
        let w = galley.size().x + if italic { size * 0.04 } else { 0.0 };
        // Métricas "de tinta" aproximadas: más ajustadas que la altura de línea de la fuente.
        Layout { w, asc: 0.74 * size, desc: 0.22 * size, prims: vec![Prim::Text { at: Vec2::ZERO, galley, baseline }] }
    }

    pub(crate) fn line(points: Vec<Vec2>, size: f32) -> Prim {
        Prim::Line { points, width: (size * 0.065).max(1.0) }
    }

    /// Rodea una caja con delimitadores: de la fuente si el contenido es bajo, dibujados
    /// a medida si es alto (fracciones, exponentes grandes…).
    /// Devuelve también dónde quedó el contenido (desplazamiento en x).
    pub(crate) fn delimit_at(&self, open: char, close: char, c: Layout, size: f32) -> (Layout, f32) {
        let tall = c.asc > 0.95 * size || c.desc > 0.4 * size;
        if !tall {
            let (o, cl) = (self.text(&open.to_string(), size, false), self.text(&close.to_string(), size, false));
            let gap = size * 0.03;
            let (ow, cw) = (o.w, cl.w);
            let mut l = Layout { w: ow + gap + c.w + gap + cw, asc: c.asc.max(o.asc), desc: c.desc.max(o.desc), prims: Vec::new() };
            let cwid = c.w;
            l.place(o, 0.0, 0.0);
            l.place(c, ow + gap, 0.0);
            l.place(cl, ow + gap + cwid + gap, 0.0);
            return (l, ow + gap);
        }
        let ext = size * 0.1;
        let top = -(c.asc + ext);
        let bottom = c.desc + ext;
        let dw = size * 0.34;
        let gap = size * 0.05;
        let mut l = Layout { w: dw + gap + c.w + gap + dw, asc: -top, desc: bottom, prims: Vec::new() };
        let cw = c.w;
        l.prims.push(Self::line(delim_points(open, false, top, bottom, dw), size));
        let mut right = delim_points(close, true, top, bottom, dw);
        for p in &mut right {
            p.x += dw + gap + cw + gap;
        }
        l.prims.push(Self::line(right, size));
        l.place(c, dw + gap, 0.0);
        (l, dw + gap)
    }
}

/// Trazo de un delimitador alto entre `top` y `bottom`, de ancho `dw`. Si `mirror`, es el
/// de cierre (reflejado).
fn delim_points(c: char, mirror: bool, top: f32, bottom: f32, dw: f32) -> Vec<Vec2> {
    let h = bottom - top;
    let mid = (top + bottom) / 2.0;
    let pts: Vec<Vec2> = match c {
        '(' | ')' => {
            // Arco cuadrático: de arriba a abajo pasando cerca del borde.
            let (p0, ctrl, p2) = (vec2(dw * 0.85, top), vec2(-dw * 0.05, mid), vec2(dw * 0.85, bottom));
            (0..=20)
                .map(|i| {
                    let t = i as f32 / 20.0;
                    p0 * (1.0 - t) * (1.0 - t) + ctrl * 2.0 * (1.0 - t) * t + p2 * t * t
                })
                .collect()
        }
        '[' | ']' => vec![vec2(dw * 0.8, top), vec2(dw * 0.3, top), vec2(dw * 0.3, bottom), vec2(dw * 0.8, bottom)],
        '{' | '}' => vec![
            vec2(dw * 0.85, top),
            vec2(dw * 0.5, top + h * 0.06),
            vec2(dw * 0.5, mid - h * 0.08),
            vec2(dw * 0.1, mid),
            vec2(dw * 0.5, mid + h * 0.08),
            vec2(dw * 0.5, bottom - h * 0.06),
            vec2(dw * 0.85, bottom),
        ],
        _ => vec![vec2(dw * 0.5, top), vec2(dw * 0.5, bottom)], // |
    };
    // Las formas de arriba son las de apertura; las de cierre son su reflejo.
    if mirror { pts.into_iter().map(|p| pos2(dw - p.x, p.y).to_vec2()).collect() } else { pts }
}
