//! Puntos notables: cortes con los ejes, máximos/mínimos e intersecciones entre curvas.
//!
//! Todo se reduce a buscar raíces o extremos de funciones de una variable sobre el
//! rango visible; solo la intersección de dos curvas implícitas se hace en 2D.

use eframe::egui::{Pos2, Rect};

use crate::math::Compiled;
use crate::plot::{Polyline, View};

/// Una curva graficable (también el borde de una desigualdad).
pub enum Curve {
    /// y = f(x)
    Fx(Compiled),
    /// x = g(y)
    Fy(Compiled),
    /// F(x, y) = 0
    Implicit(Compiled),
    /// (x(t), y(t)) con t en [t0, t1]; `x` e `y` se evalúan con t en el lugar de x.
    /// Las curvas polares r = f(θ) también se guardan así. `var` es "t" o "θ" (para la interfaz).
    Param { x: Compiled, y: Compiled, t0: f64, t1: f64, var: &'static str },
}

/// Muestras para buscar raíces/extremos en el parámetro de una paramétrica.
const PARAM_SAMPLES: usize = 1000;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PoiKind {
    XIntercept,
    YIntercept,
    Max,
    Min,
    /// Punto más a la izquierda/derecha de x = g(y).
    Extreme,
    Intersection,
}

impl PoiKind {
    pub fn label(self) -> &'static str {
        match self {
            PoiKind::XIntercept => "Corte con eje x",
            PoiKind::YIntercept => "Corte con eje y",
            PoiKind::Max => "Máximo",
            PoiKind::Min => "Mínimo",
            PoiKind::Extreme => "Extremo",
            PoiKind::Intersection => "Intersección",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Poi {
    pub x: f64,
    pub y: f64,
    pub kind: PoiKind,
}

// ───────────────────────────── Búsquedas 1D ─────────────────────────────

fn sample(h: &dyn Fn(f64) -> f64, a: f64, b: f64, n: usize) -> Vec<(f64, f64)> {
    (0..=n).map(|i| a + (b - a) * i as f64 / n as f64).map(|t| (t, h(t))).collect()
}

fn bisect(h: &dyn Fn(f64) -> f64, mut a: f64, va: f64, mut b: f64) -> f64 {
    let neg = va < 0.0;
    for _ in 0..80 {
        let m = 0.5 * (a + b);
        if m <= a || m >= b {
            break;
        }
        if (h(m) < 0.0) == neg {
            a = m;
        } else {
            b = m;
        }
    }
    0.5 * (a + b)
}

/// Sección áurea: mínimo de `h` en [a, b] (o máximo si `maximize`).
fn golden(h: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64, maximize: bool) -> f64 {
    const R: f64 = 0.618_033_988_749_894_8;
    let g = |t: f64| if maximize { -h(t) } else { h(t) };
    let mut c = b - R * (b - a);
    let mut d = a + R * (b - a);
    let (mut gc, mut gd) = (g(c), g(d));
    for _ in 0..100 {
        if (b - a).abs() <= 1e-15 * (a.abs() + b.abs()).max(1e-300) {
            break;
        }
        if gc < gd {
            b = d;
            d = c;
            gd = gc;
            c = b - R * (b - a);
            gc = g(c);
        } else {
            a = c;
            c = d;
            gc = gd;
            d = a + R * (b - a);
            gd = g(d);
        }
    }
    0.5 * (a + b)
}

/// Raíces de `h` en [a, b]: cambios de signo (sin polos) y tangencias (x² en 0).
pub fn roots(h: &dyn Fn(f64) -> f64, a: f64, b: f64, n: usize) -> Vec<f64> {
    let s = sample(h, a, b, n);
    let mut out = Vec::new();
    for w in s.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if !(v0.is_finite() && v1.is_finite()) {
            continue;
        }
        if v0 == 0.0 {
            out.push(t0);
        } else if v1 != 0.0 && (v0 < 0.0) != (v1 < 0.0) {
            let r = bisect(h, t0, v0, t1);
            let hr = h(r);
            // En un polo (1/x) el valor crece en vez de acercarse a 0.
            if hr.is_finite() && hr.abs() <= v0.abs().min(v1.abs()) {
                out.push(r);
            }
        }
    }
    for w in s.windows(3) {
        let (p, m, q) = (w[0], w[1], w[2]);
        if !(p.1.is_finite() && m.1.is_finite() && q.1.is_finite()) {
            continue;
        }
        let same_sign = (p.1 > 0.0) == (m.1 > 0.0) && (m.1 > 0.0) == (q.1 > 0.0);
        if same_sign && m.1 != 0.0 && m.1.abs() < p.1.abs() && m.1.abs() <= q.1.abs() {
            let t = golden(&|t| h(t).abs(), p.0, q.0, false);
            if h(t).abs() < 1e-9 {
                out.push(t);
            }
        }
    }
    out.sort_by(f64::total_cmp);
    let eps = (b - a).abs() * 1e-7;
    out.dedup_by(|x, y| (*x - *y).abs() < eps);
    out
}

/// Extremos locales estrictos de `h` en [a, b]: (t, es_máximo).
pub fn extrema(h: &dyn Fn(f64) -> f64, a: f64, b: f64, n: usize) -> Vec<(f64, bool)> {
    let s = sample(h, a, b, n);
    let mut out = Vec::new();
    for w in s.windows(3) {
        let (p, m, q) = (w[0], w[1], w[2]);
        if !(p.1.is_finite() && m.1.is_finite() && q.1.is_finite()) {
            continue;
        }
        let (d1, d2) = (m.1 - p.1, q.1 - m.1);
        let is_max = if d1 > 0.0 && d2 < 0.0 {
            true
        } else if d1 < 0.0 && d2 > 0.0 {
            false
        } else {
            continue;
        };
        let t = golden(h, p.0, q.0, is_max);
        let v = h(t);
        let ok = v.is_finite() && if is_max { v >= p.1.max(q.1) } else { v <= p.1.min(q.1) };
        if ok {
            out.push((t, is_max));
        }
    }
    out
}

// ───────────────────────────── Puntos de una curva ─────────────────────────────

fn samples_for(pixels: f32) -> usize {
    ((pixels / 2.0) as usize).clamp(50, 2000)
}

/// Cortes con los ejes y extremos de una curva dentro de la vista.
pub fn curve_pois(c: &Curve, view: &View, rect: Rect, out: &mut Vec<Poi>) {
    let (x0, x1) = view.x_range(rect);
    let (y0, y1) = view.y_range(rect);
    let (nx, ny) = (samples_for(rect.width()), samples_for(rect.height()));
    let mut push = |x: f64, y: f64, kind| out.push(Poi { x, y, kind });

    match c {
        Curve::Fx(f) => {
            let h = |x: f64| f.eval(x, 0.0);
            for r in roots(&h, x0, x1, nx) {
                push(r, 0.0, PoiKind::XIntercept);
            }
            if x0 <= 0.0 && 0.0 <= x1 {
                push(0.0, h(0.0), PoiKind::YIntercept);
            }
            for (t, is_max) in extrema(&h, x0, x1, nx) {
                push(t, h(t), if is_max { PoiKind::Max } else { PoiKind::Min });
            }
        }
        Curve::Fy(g) => {
            let h = |y: f64| g.eval(0.0, y);
            for r in roots(&h, y0, y1, ny) {
                push(0.0, r, PoiKind::YIntercept);
            }
            if y0 <= 0.0 && 0.0 <= y1 {
                push(h(0.0), 0.0, PoiKind::XIntercept);
            }
            for (t, _) in extrema(&h, y0, y1, ny) {
                push(h(t), t, PoiKind::Extreme);
            }
        }
        Curve::Implicit(f) => {
            for r in roots(&|x| f.eval(x, 0.0), x0, x1, nx) {
                push(r, 0.0, PoiKind::XIntercept);
            }
            for r in roots(&|y| f.eval(0.0, y), y0, y1, ny) {
                push(0.0, r, PoiKind::YIntercept);
            }
        }
        Curve::Param { x, y, t0, t1, .. } => {
            let (px, py) = (|t: f64| x.eval(t, 0.0), |t: f64| y.eval(t, 0.0));
            for t in roots(&py, *t0, *t1, PARAM_SAMPLES) {
                push(px(t), 0.0, PoiKind::XIntercept);
            }
            for t in roots(&px, *t0, *t1, PARAM_SAMPLES) {
                push(0.0, py(t), PoiKind::YIntercept);
            }
            for (t, is_max) in extrema(&py, *t0, *t1, PARAM_SAMPLES) {
                push(px(t), py(t), if is_max { PoiKind::Max } else { PoiKind::Min });
            }
        }
    }
}

/// Intersecciones entre dos curvas dentro de la vista. `a_lines`/`b_lines` son sus
/// polilíneas en pantalla (solo se usan para el caso implícita–implícita).
pub fn intersections(a: &Curve, b: &Curve, a_lines: &[Polyline], b_lines: &[Polyline], view: &View, rect: Rect, out: &mut Vec<Poi>) {
    let (x0, x1) = view.x_range(rect);
    let (y0, y1) = view.y_range(rect);
    let (nx, ny) = (samples_for(rect.width()), samples_for(rect.height()));
    let mut push = |x: f64, y: f64| out.push(Poi { x, y, kind: PoiKind::Intersection });

    use Curve::*;
    match (a, b) {
        (Fx(f), Fx(g)) => {
            for t in roots(&|t| f.eval(t, 0.0) - g.eval(t, 0.0), x0, x1, nx) {
                push(t, f.eval(t, 0.0));
            }
        }
        (Fx(f), Fy(g)) | (Fy(g), Fx(f)) => {
            for t in roots(&|t| t - g.eval(0.0, f.eval(t, 0.0)), x0, x1, nx) {
                push(t, f.eval(t, 0.0));
            }
        }
        (Fx(f), Implicit(imp)) | (Implicit(imp), Fx(f)) => {
            for t in roots(&|t| imp.eval(t, f.eval(t, 0.0)), x0, x1, nx) {
                push(t, f.eval(t, 0.0));
            }
        }
        (Fy(g), Fy(h)) => {
            for t in roots(&|t| g.eval(0.0, t) - h.eval(0.0, t), y0, y1, ny) {
                push(g.eval(0.0, t), t);
            }
        }
        (Fy(g), Implicit(imp)) | (Implicit(imp), Fy(g)) => {
            for t in roots(&|t| imp.eval(g.eval(0.0, t), t), y0, y1, ny) {
                push(g.eval(0.0, t), t);
            }
        }
        (Param { .. }, Param { .. }) => {
            for p in polyline_crossings(a_lines, b_lines) {
                let (x, y) = view.to_world(rect, p);
                push(x, y);
            }
        }
        // Paramétrica contra explícita/implícita: raíces en t.
        (Param { x, y, t0, t1, .. }, other) | (other, Param { x, y, t0, t1, .. }) => {
            let (px, py) = (|t: f64| x.eval(t, 0.0), |t: f64| y.eval(t, 0.0));
            let h: Box<dyn Fn(f64) -> f64> = match other {
                Fx(f) => Box::new(move |t| py(t) - f.eval(px(t), 0.0)),
                Fy(g) => Box::new(move |t| px(t) - g.eval(0.0, py(t))),
                Implicit(imp) => Box::new(move |t| imp.eval(px(t), py(t))),
                Param { .. } => unreachable!(),
            };
            for t in roots(&*h, *t0, *t1, PARAM_SAMPLES) {
                push(px(t), py(t));
            }
        }
        (Implicit(f), Implicit(g)) => {
            for p in polyline_crossings(a_lines, b_lines) {
                let guess = view.to_world(rect, p);
                let (x, y) = newton_2d(f, g, guess, 1.0 / view.scale).unwrap_or(guess);
                push(x, y);
            }
        }
    }
}

/// Cruces entre dos conjuntos de polilíneas (en pantalla), con descarte por cajas.
fn polyline_crossings(a: &[Polyline], b: &[Polyline]) -> Vec<Pos2> {
    const CHUNK: usize = 16;
    let chunks = |lines: &[Polyline]| -> Vec<(Rect, Vec<[Pos2; 2]>)> {
        let mut out = Vec::new();
        for l in lines {
            let segs: Vec<[Pos2; 2]> = l.windows(2).map(|w| [w[0], w[1]]).collect();
            for c in segs.chunks(CHUNK) {
                let r = Rect::from_points(&c.iter().flatten().copied().collect::<Vec<_>>());
                out.push((r, c.to_vec()));
            }
        }
        out
    };
    let (ca, cb) = (chunks(a), chunks(b));
    let mut out = Vec::new();
    for (ra, sa) in &ca {
        for (rb, sb) in &cb {
            if !ra.expand(0.5).intersects(*rb) {
                continue;
            }
            for s in sa {
                for t in sb {
                    if let Some(p) = segment_intersection(*s, *t) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

fn segment_intersection([p, p2]: [Pos2; 2], [q, q2]: [Pos2; 2]) -> Option<Pos2> {
    let r = p2 - p;
    let s = q2 - q;
    let denom = r.x * s.y - r.y * s.x;
    if denom.abs() < 1e-9 {
        return None;
    }
    let qp = q - p;
    let t = (qp.x * s.y - qp.y * s.x) / denom;
    let u = (qp.x * r.y - qp.y * r.x) / denom;
    ((0.0..1.0).contains(&t) && (0.0..1.0).contains(&u)).then(|| p + r * t)
}

/// Newton en 2D para F = G = 0 partiendo de `guess`. Falla si se aleja más de unos píxeles.
fn newton_2d(f: &Compiled, g: &Compiled, guess: (f64, f64), px: f64) -> Option<(f64, f64)> {
    let (mut x, mut y) = guess;
    for _ in 0..20 {
        let (fv, gv) = (f.eval(x, y), g.eval(x, y));
        let h = 1e-7 * (1.0 + x.abs().max(y.abs()));
        let (fx, fy) = ((f.eval(x + h, y) - fv) / h, (f.eval(x, y + h) - fv) / h);
        let (gx, gy) = ((g.eval(x + h, y) - gv) / h, (g.eval(x, y + h) - gv) / h);
        let det = fx * gy - fy * gx;
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let dx = (fv * gy - fy * gv) / det;
        let dy = (fx * gv - fv * gx) / det;
        x -= dx;
        y -= dy;
        if dx.abs().max(dy.abs()) < 1e-13 * (1.0 + x.abs().max(y.abs())) {
            break;
        }
    }
    let moved = ((x - guess.0).powi(2) + (y - guess.1).powi(2)).sqrt();
    (x.is_finite() && y.is_finite() && moved < 3.0 * px).then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_and_extrema() {
        let r = roots(&|x: f64| x * x - 2.0, -3.0, 3.0, 100);
        assert_eq!(r.len(), 2);
        assert!((r[1] - 2f64.sqrt()).abs() < 1e-12);
        // Polo de 1/x: no es raíz.
        assert!(roots(&|x: f64| 1.0 / x, -1.0, 1.3, 100).is_empty());
        // Tangencia.
        let r = roots(&|x: f64| (x - 0.3) * (x - 0.3), -1.0, 1.0, 101);
        assert_eq!(r.len(), 1);
        assert!((r[0] - 0.3).abs() < 1e-6);
        let e = extrema(&|x: f64| x.sin(), 0.0, 7.0, 200);
        assert_eq!(e.len(), 2);
        assert!((e[0].0 - std::f64::consts::FRAC_PI_2).abs() < 1e-7 && e[0].1);
        assert!(!e[1].1);
    }
}
