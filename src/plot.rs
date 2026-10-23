//! Geometría de la gráfica: transformación de vista, muestreo de curvas y cuadrícula.

use std::collections::HashMap;

use eframe::egui::{Align2, Color32, ColorImage, FontId, Mesh, Painter, Pos2, Rect, Stroke, pos2};

use crate::math::{self, CondPoint, Compiled};

/// Vista del plano: centro en coordenadas del mundo y escala en píxeles por unidad.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct View {
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
}

impl Default for View {
    fn default() -> Self {
        Self { cx: 0.0, cy: 0.0, scale: 50.0 }
    }
}

impl View {
    pub fn to_screen(&self, rect: Rect, x: f64, y: f64) -> Pos2 {
        let c = rect.center();
        pos2(
            (c.x as f64 + (x - self.cx) * self.scale) as f32,
            (c.y as f64 - (y - self.cy) * self.scale) as f32,
        )
    }

    pub fn to_world(&self, rect: Rect, p: Pos2) -> (f64, f64) {
        let c = rect.center();
        (
            self.cx + (p.x - c.x) as f64 / self.scale,
            self.cy - (p.y - c.y) as f64 / self.scale,
        )
    }

    /// Zoom manteniendo fijo el punto del mundo bajo `anchor`.
    pub fn zoom_at(&mut self, rect: Rect, anchor: Pos2, factor: f64) {
        let (wx, wy) = self.to_world(rect, anchor);
        self.scale = (self.scale * factor).clamp(1e-6, 1e12);
        let (nx, ny) = self.to_world(rect, anchor);
        self.cx += wx - nx;
        self.cy += wy - ny;
    }

    pub fn x_range(&self, rect: Rect) -> (f64, f64) {
        (self.to_world(rect, rect.left_top()).0, self.to_world(rect, rect.right_bottom()).0)
    }

    pub fn y_range(&self, rect: Rect) -> (f64, f64) {
        (self.to_world(rect, rect.right_bottom()).1, self.to_world(rect, rect.left_top()).1)
    }
}

pub type Polyline = Vec<Pos2>;

// ───────────────────────────── Curvas explícitas ─────────────────────────────

/// Traza `v = f(t)` con muestreo adaptativo. `to_screen(t, v)` decide qué eje es cuál,
/// así sirve tanto para y = f(x) como para x = g(y).
struct Tracer<'a, F: Fn(f64) -> f64, S: Fn(f64, f64) -> Pos2> {
    f: F,
    to_screen: S,
    /// Coordenada de pantalla del eje de valores (y para y=f(x)); límites visibles.
    value_axis: fn(Pos2) -> f32,
    lo: f32,
    hi: f32,
    clip: &'a dyn Fn(Pos2) -> Pos2,
    lines: Vec<Polyline>,
    cur: Polyline,
}

impl<F: Fn(f64) -> f64, S: Fn(f64, f64) -> Pos2> Tracer<'_, F, S> {
    fn push(&mut self, t: f64, v: f64) {
        if v.is_finite() {
            let p = (self.clip)((self.to_screen)(t, v));
            self.cur.push(p);
        } else {
            self.break_line();
        }
    }

    fn break_line(&mut self) {
        if self.cur.len() >= 2 {
            self.lines.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }

    fn screen_v(&self, t: f64, v: f64) -> f32 {
        (self.value_axis)((self.to_screen)(t, v))
    }

    /// ¿Hay una discontinuidad de salto (p. ej. floor) en [t0, t1]? Se biseca siguiendo
    /// la mitad con mayor salto: en una función continua el salto se desvanece, en un
    /// escalón se mantiene por pequeño que sea el intervalo.
    fn is_jump(&self, mut t0: f64, mut v0: f64, mut t1: f64, mut v1: f64) -> bool {
        for _ in 0..40 {
            let tm = 0.5 * (t0 + t1);
            if tm <= t0 || tm >= t1 {
                break; // límite de precisión de f64
            }
            let vm = (self.f)(tm);
            if !vm.is_finite() {
                return true;
            }
            let sm = self.screen_v(tm, vm);
            let left = (sm - self.screen_v(t0, v0)).abs();
            let right = (self.screen_v(t1, v1) - sm).abs();
            if left.max(right) < 1.0 {
                return false;
            }
            if left > right {
                (t1, v1) = (tm, vm);
            } else {
                (t0, v0) = (tm, vm);
            }
        }
        true
    }

    /// Supone que (t0, v0) ya está emitido; emite el tramo hasta (t1, v1) inclusive.
    fn segment(&mut self, t0: f64, v0: f64, t1: f64, v1: f64, depth: u32) {
        let (f0, f1) = (v0.is_finite(), v1.is_finite());
        if !f0 && !f1 {
            self.push(t1, v1);
            return;
        }
        if f0 && f1 {
            let (s0, s1) = (self.screen_v(t0, v0), self.screen_v(t1, v1));
            let jump = (s1 - s0).abs();
            let span = self.hi - self.lo;
            // Ambos fuera de pantalla por el mismo lado: no hace falta precisión.
            let same_side_out = (s0 < self.lo && s1 < self.lo) || (s0 > self.hi && s1 > self.hi);
            if jump <= 1.5 || same_side_out {
                self.push(t1, v1);
                return;
            }
            if depth == 0 {
                if jump > span || self.is_jump(t0, v0, t1, v1) {
                    self.break_line();
                }
                self.push(t1, v1);
                return;
            }
        } else if depth == 0 {
            self.push(t1, v1);
            return;
        }
        let tm = 0.5 * (t0 + t1);
        let vm = (self.f)(tm);
        self.segment(t0, v0, tm, vm, depth - 1);
        self.segment(tm, vm, t1, v1, depth - 1);
    }
}

fn clamp_point(rect: Rect) -> impl Fn(Pos2) -> Pos2 {
    let m = rect.width().max(rect.height()) * 2.0;
    move |p: Pos2| pos2(p.x.clamp(rect.left() - m, rect.right() + m), p.y.clamp(rect.top() - m, rect.bottom() + m))
}

/// y = f(x)
pub fn trace_fx(f: &Compiled, view: &View, rect: Rect) -> Vec<Polyline> {
    let (x0, x1) = view.x_range(rect);
    let clip = clamp_point(rect);
    let mut tr = Tracer {
        f: |x| f.eval(x, 0.0),
        to_screen: |x, y| view.to_screen(rect, x, y),
        value_axis: |p| p.y,
        lo: rect.top(),
        hi: rect.bottom(),
        clip: &clip,
        lines: Vec::new(),
        cur: Vec::new(),
    };
    run_tracer(&mut tr, x0, x1, rect.width());
    tr.lines
}

/// x = g(y)
pub fn trace_fy(g: &Compiled, view: &View, rect: Rect) -> Vec<Polyline> {
    let (y0, y1) = view.y_range(rect);
    let clip = clamp_point(rect);
    let mut tr = Tracer {
        f: |y| g.eval(0.0, y),
        to_screen: |y, x| view.to_screen(rect, x, y),
        value_axis: |p| p.x,
        lo: rect.left(),
        hi: rect.right(),
        clip: &clip,
        lines: Vec::new(),
        cur: Vec::new(),
    };
    run_tracer(&mut tr, y0, y1, rect.height());
    tr.lines
}

fn run_tracer<F: Fn(f64) -> f64, S: Fn(f64, f64) -> Pos2>(tr: &mut Tracer<'_, F, S>, t0: f64, t1: f64, pixels: f32) {
    let n = (pixels.ceil() as usize).max(2);
    let dt = (t1 - t0) / n as f64;
    let mut t_prev = t0;
    let mut v_prev = (tr.f)(t_prev);
    tr.push(t_prev, v_prev);
    for i in 1..=n {
        let t = t0 + dt * i as f64;
        let v = (tr.f)(t);
        tr.segment(t_prev, v_prev, t, v, 8);
        t_prev = t;
        v_prev = v;
    }
    tr.break_line();
}

// ───────────────────────────── Curvas paramétricas ─────────────────────────────

/// Curva (x(t), y(t)) para t en [t0, t1]. `fx`/`fy` reciben t en el lugar de x.
pub fn trace_param(fx: &Compiled, fy: &Compiled, t0: f64, t1: f64, view: &View, rect: Rect) -> Vec<Polyline> {
    let point = |t: f64| {
        let (x, y) = (fx.eval(t, 0.0), fy.eval(t, 0.0));
        (x.is_finite() && y.is_finite()).then(|| view.to_screen(rect, x, y))
    };
    let mut tr = ParamTracer {
        point: &point,
        clip: &clamp_point(rect),
        rect,
        jump: rect.size().length() * 0.5,
        lines: Vec::new(),
        cur: Vec::new(),
    };
    const N: usize = 1000;
    let mut prev_t = t0;
    let mut prev = point(t0);
    if let Some(p) = prev {
        tr.push(p);
    }
    for i in 1..=N {
        let t = t0 + (t1 - t0) * i as f64 / N as f64;
        let p = point(t);
        tr.segment(prev_t, prev, t, p, 12);
        prev_t = t;
        prev = p;
    }
    tr.flush();
    tr.lines
}

struct ParamTracer<'a> {
    point: &'a dyn Fn(f64) -> Option<Pos2>,
    clip: &'a dyn Fn(Pos2) -> Pos2,
    rect: Rect,
    /// Distancia en pantalla a partir de la cual un salto se considera discontinuidad.
    jump: f32,
    lines: Vec<Polyline>,
    cur: Polyline,
}

impl ParamTracer<'_> {
    fn push(&mut self, p: Pos2) {
        self.cur.push((self.clip)(p));
    }

    fn flush(&mut self) {
        if self.cur.len() >= 2 {
            self.lines.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }

    /// Supone emitido el punto en t0; emite el tramo hasta t1 inclusive.
    fn segment(&mut self, t0: f64, p0: Option<Pos2>, t1: f64, p1: Option<Pos2>, depth: u32) {
        match (p0, p1) {
            (None, None) => {
                self.flush();
                return;
            }
            (Some(a), Some(b)) => {
                let r = self.rect;
                let same_side_out = (a.x < r.left() && b.x < r.left())
                    || (a.x > r.right() && b.x > r.right())
                    || (a.y < r.top() && b.y < r.top())
                    || (a.y > r.bottom() && b.y > r.bottom());
                let d = a.distance(b);
                if d <= 2.0 || same_side_out {
                    self.push(b);
                    return;
                }
                if depth == 0 {
                    if d > self.jump {
                        self.flush();
                    }
                    self.push(b);
                    return;
                }
            }
            (_, p1) if depth == 0 => {
                match p1 {
                    Some(b) => self.push(b),
                    None => self.flush(),
                }
                return;
            }
            _ => {}
        }
        let tm = 0.5 * (t0 + t1);
        let pm = (self.point)(tm);
        self.segment(t0, p0, tm, pm, depth - 1);
        self.segment(tm, pm, t1, p1, depth - 1);
    }
}

// ───────────────────────────── Curvas implícitas ─────────────────────────────

/// Curva F(x, y) = 0 mediante "marching squares" sobre una rejilla de píxeles.
/// Los segmentos se encadenan en polilíneas para poder dibujarlas con guiones.
pub fn trace_implicit(f: &Compiled, view: &View, rect: Rect) -> Vec<Polyline> {
    const CELL: f32 = 4.0;
    // Con restricciones: la rejilla usa la función sin restringir y luego se recorta
    // cada segmento justo en el borde de la región permitida.
    let (f, conds) = match f {
        Compiled::Cond { value, conds, sub: CondPoint::Xy } => (&**value, Some(conds.as_slice())),
        _ => (f, None),
    };
    let nx = (rect.width() / CELL).ceil() as usize + 1;
    let ny = (rect.height() / CELL).ceil() as usize + 1;
    let world = |i: usize, j: usize| view.to_world(rect, pos2(rect.left() + i as f32 * CELL, rect.top() + j as f32 * CELL));

    let mut grid = vec![0.0f64; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = world(i, j);
            grid[j * nx + i] = f.eval(x, y);
        }
    }

    // Punto de corte sobre la arista (a, b). Se calcula siempre con las esquinas en el
    // mismo orden para que las dos celdas vecinas obtengan exactamente el mismo punto.
    let crossing = |a: (usize, usize), b: (usize, usize)| -> Option<Pos2> {
        let (a, b) = if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) };
        let (va, vb) = (grid[a.1 * nx + a.0], grid[b.1 * nx + b.0]);
        if (va > 0.0) == (vb > 0.0) {
            return None;
        }
        let (xa, ya) = world(a.0, a.1);
        let (xb, yb) = world(b.0, b.1);
        // Descarta polos (p. ej. 1/x): el valor no se acerca a 0 en medio.
        let vm = f.eval(0.5 * (xa + xb), 0.5 * (ya + yb));
        if !vm.is_finite() || vm.abs() > va.abs().max(vb.abs()) {
            return None;
        }
        let t = va / (va - vb);
        Some(view.to_screen(rect, xa + (xb - xa) * t, ya + (yb - ya) * t))
    };

    let mut segs = Vec::new();
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            // Esquinas en sentido horario: arriba-izq, arriba-der, abajo-der, abajo-izq.
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let vals = corners.map(|(a, b)| grid[b * nx + a]);
            if vals.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let pts: Vec<Pos2> = (0..4).filter_map(|k| crossing(corners[k], corners[(k + 1) % 4])).collect();
            match pts.len() {
                2 => segs.push([pts[0], pts[1]]),
                4 => {
                    // Punto de silla: emparejar según el valor en el centro.
                    let (cx, cy) = view.to_world(rect, pos2(rect.left() + (i as f32 + 0.5) * CELL, rect.top() + (j as f32 + 0.5) * CELL));
                    if (f.eval(cx, cy) > 0.0) == (vals[0] > 0.0) {
                        segs.push([pts[0], pts[3]]);
                        segs.push([pts[1], pts[2]]);
                    } else {
                        segs.push([pts[0], pts[1]]);
                        segs.push([pts[2], pts[3]]);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(conds) = conds {
        let keep = |p: Pos2| {
            let (x, y) = view.to_world(rect, p);
            math::conds_hold(conds, x, y)
        };
        segs = segs.into_iter().filter_map(|s| clip_segment(s, &keep)).collect();
    }
    chain_segments(segs)
}

/// Recorta un segmento a la parte donde `keep` es cierto (se supone un solo cruce).
fn clip_segment([a, b]: [Pos2; 2], keep: &impl Fn(Pos2) -> bool) -> Option<[Pos2; 2]> {
    let (ka, kb) = (keep(a), keep(b));
    if ka && kb {
        return Some([a, b]);
    }
    if !ka && !kb {
        return None;
    }
    // Bisección entre el extremo que vale y el que no.
    let (mut inside, mut outside) = if ka { (a, b) } else { (b, a) };
    for _ in 0..16 {
        let m = inside.lerp(outside, 0.5);
        if keep(m) {
            inside = m;
        } else {
            outside = m;
        }
    }
    Some(if ka { [a, inside] } else { [inside, b] })
}

/// Une segmentos que comparten extremos en polilíneas lo más largas posible.
fn chain_segments(segs: Vec<[Pos2; 2]>) -> Vec<Polyline> {
    let key = |p: Pos2| (p.x.to_bits(), p.y.to_bits());
    let mut adj: HashMap<(u32, u32), Vec<usize>> = HashMap::with_capacity(segs.len() * 2);
    for (i, s) in segs.iter().enumerate() {
        adj.entry(key(s[0])).or_default().push(i);
        adj.entry(key(s[1])).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut lines = Vec::new();

    // Sigue la cadena desde `end` añadiendo segmentos no usados.
    let walk = |end: Pos2, used: &mut Vec<bool>, out: &mut Vec<Pos2>| {
        let mut end = end;
        while let Some(&i) = adj.get(&key(end)).and_then(|v| v.iter().find(|&&i| !used[i])) {
            used[i] = true;
            let s = segs[i];
            end = if key(s[0]) == key(end) { s[1] } else { s[0] };
            out.push(end);
        }
    };

    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let [a, b] = segs[start];
        let mut forward = vec![a, b];
        walk(b, &mut used, &mut forward);
        let mut backward = Vec::new();
        walk(a, &mut used, &mut backward);
        backward.reverse();
        backward.extend(forward);
        lines.push(backward);
    }
    lines
}

/// Máscara de la región F(x, y) > 0 (o < 0): blanco opaco dentro, transparente fuera.
/// Se dibuja como textura teñida, así cambiar color u opacidad no exige recalcular.
pub fn region_mask(f: &Compiled, positive: bool, view: &View, rect: Rect) -> ColorImage {
    const CELL: f32 = 3.0;
    let w = (rect.width() / CELL).ceil().max(1.0) as usize;
    let h = (rect.height() / CELL).ceil().max(1.0) as usize;
    let mut pixels = vec![Color32::TRANSPARENT; w * h];
    for j in 0..h {
        for i in 0..w {
            let p = pos2(rect.left() + (i as f32 + 0.5) * CELL, rect.top() + (j as f32 + 0.5) * CELL);
            let (x, y) = view.to_world(rect, p);
            let v = f.eval(x, y);
            if v.is_finite() && (v > 0.0) == positive && v != 0.0 {
                pixels[j * w + i] = Color32::WHITE;
            }
        }
    }
    ColorImage::new([w, h], pixels)
}

/// Relleno entre una curva explícita y el borde de la vista, como malla de trapecios.
/// `edge(p)` da el punto del borde correspondiente a `p` (arriba/abajo o izquierda/derecha).
pub fn fill_to_edge(lines: &[Polyline], color: Color32, edge: impl Fn(Pos2) -> Pos2) -> Mesh {
    let mut mesh = Mesh::default();
    for l in lines {
        for w in l.windows(2) {
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(w[0], color);
            mesh.colored_vertex(w[1], color);
            mesh.colored_vertex(edge(w[1]), color);
            mesh.colored_vertex(edge(w[0]), color);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    mesh
}

// ───────────────────────────── Cuadrícula ─────────────────────────────

/// Paso "bonito" (1, 2 o 5 × 10^n) cercano a `raw`. Devuelve (paso, subdivisiones).
fn nice_step(raw: f64) -> (f64, u32) {
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let m = raw / base;
    if m < 1.5 {
        (base, 5)
    } else if m < 3.5 {
        (2.0 * base, 4)
    } else if m < 7.5 {
        (5.0 * base, 5)
    } else {
        (10.0 * base, 5)
    }
}

pub fn format_tick(v: f64, step: f64) -> String {
    if v.abs() < step * 1e-6 {
        return "0".into();
    }
    if step >= 1e-4 && v.abs() < 1e7 {
        let decimals = (-step.log10()).ceil().max(0.0) as usize;
        format!("{v:.decimals$}")
    } else {
        let s = format!("{v:.2e}");
        s.replace(".00e", "e")
    }
}

pub struct GridColors {
    pub minor: Color32,
    pub major: Color32,
    pub axis: Color32,
    pub label: Color32,
    pub label_bg: Color32,
    pub label_size: f32,
}

pub fn draw_grid(painter: &Painter, view: &View, rect: Rect, colors: &GridColors) {
    let (step, subdiv) = nice_step((60.0 + 2.5 * colors.label_size as f64) / view.scale);
    let minor = step / subdiv as f64;
    let (x0, x1) = view.x_range(rect);
    let (y0, y1) = view.y_range(rect);

    let lines = |lo: f64, hi: f64, s: f64| {
        let start = (lo / s).floor() as i64;
        let end = (hi / s).ceil() as i64;
        (start..=end).map(move |k| k as f64 * s)
    };

    // Líneas menores y mayores.
    for (s, color) in [(minor, colors.minor), (step, colors.major)] {
        let stroke = Stroke::new(1.0, color);
        for x in lines(x0, x1, s) {
            let px = view.to_screen(rect, x, 0.0).x.round() + 0.5;
            painter.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], stroke);
        }
        for y in lines(y0, y1, s) {
            let py = view.to_screen(rect, 0.0, y).y.round() + 0.5;
            painter.line_segment([pos2(rect.left(), py), pos2(rect.right(), py)], stroke);
        }
    }

    // Ejes.
    let origin = view.to_screen(rect, 0.0, 0.0);
    let axis = Stroke::new(1.5, colors.axis);
    if rect.x_range().contains(origin.x) {
        painter.line_segment([pos2(origin.x, rect.top()), pos2(origin.x, rect.bottom())], axis);
    }
    if rect.y_range().contains(origin.y) {
        painter.line_segment([pos2(rect.left(), origin.y), pos2(rect.right(), origin.y)], axis);
    }

    // Etiquetas: pegadas al eje, o al borde si el eje no se ve.
    let font = FontId::proportional(colors.label_size);
    let label_y = origin.y.clamp(rect.top() + 2.0, rect.bottom() - colors.label_size - 8.0);
    let label_x = origin.x.clamp(rect.left() + colors.label_size * 2.8, rect.right() - 4.0);
    let draw_label = |pos: Pos2, align: Align2, text: String| {
        let galley = painter.layout_no_wrap(text, font.clone(), colors.label);
        let r = align.anchor_size(pos, galley.size()).expand(1.5);
        painter.rect_filled(r, 2.0, colors.label_bg);
        painter.galley(r.min + eframe::egui::vec2(1.5, 1.5), galley, colors.label);
    };
    for x in lines(x0, x1, step) {
        if x.abs() < step * 1e-6 {
            continue;
        }
        let p = view.to_screen(rect, x, 0.0);
        draw_label(pos2(p.x, label_y + 3.0), Align2::CENTER_TOP, format_tick(x, step));
    }
    for y in lines(y0, y1, step) {
        if y.abs() < step * 1e-6 {
            continue;
        }
        let p = view.to_screen(rect, 0.0, y);
        draw_label(pos2(label_x - 4.0, p.y), Align2::RIGHT_CENTER, format_tick(y, step));
    }
    if rect.contains(origin) {
        draw_label(origin + eframe::egui::vec2(-4.0, 3.0), Align2::RIGHT_TOP, "0".into());
    }
}
