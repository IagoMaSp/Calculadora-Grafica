//! Escritura matemática: convierte una expresión en un árbol de presentación (exponentes
//! arriba, fracciones apiladas, raíces…) y lo compone y dibuja al estilo TeX.

use std::sync::Arc;

use eframe::egui::{Color32, FontId, Galley, Painter, Pos2, Stroke, Vec2, pos2, vec2};
use eframe::epaint::text::{LayoutJob, TextFormat};

use crate::app::format_number;
use crate::math::{self, BigOp, Comparison, Expr, Func1, Func2, Restriction, Statement};

// ───────────────────────────── Árbol de presentación ─────────────────────────────

/// Nodo de presentación (independiente del tamaño de letra).
#[derive(Debug, Clone)]
pub enum Node {
    /// Texto. `italic` para variables; `op` añade espacio a los lados (+, =, <…).
    Text { s: String, italic: bool, op: bool },
    Row(Vec<Node>),
    Frac(Box<Node>, Box<Node>),
    /// Coeficiente binomial (n sobre r entre paréntesis, sin raya).
    Binom(Box<Node>, Box<Node>),
    Sup(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
    /// Base con subíndice y superíndice (límites de una integral).
    SubSup(Box<Node>, Box<Node>, Box<Node>),
    /// Raíz con índice opcional.
    Sqrt(Box<Node>, Option<Box<Node>>),
    /// Contenido entre delimitadores: ( ), [ ], { }, | |.
    Delim(char, char, Box<Node>),
    /// Símbolo grande (∑, ∏) con límites debajo y encima.
    BigOp(&'static str, Box<Node>, Box<Node>),
    /// Signo de integral grande.
    Integral,
    /// Espacio en "em" (múltiplos del tamaño de letra).
    Space(f32),
}

fn var(s: &str) -> Node {
    Node::Text { s: s.to_string(), italic: true, op: false }
}

fn up(s: &str) -> Node {
    Node::Text { s: s.to_string(), italic: false, op: false }
}

fn op(s: &str) -> Node {
    Node::Text { s: s.to_string(), italic: false, op: true }
}

fn row(v: Vec<Node>) -> Node {
    Node::Row(v)
}

fn paren(n: Node) -> Node {
    Node::Delim('(', ')', Box::new(n))
}

/// Elementos separados por comas.
fn comma_list(items: Vec<Node>) -> Node {
    let mut v = Vec::new();
    for (i, it) in items.into_iter().enumerate() {
        if i > 0 {
            v.push(up(","));
            v.push(Node::Space(0.2));
        }
        v.push(it);
    }
    row(v)
}

/// Línea completa (sentencia + restricciones).
pub fn statement(st: &Statement, rs: &[Restriction]) -> Node {
    let mut v = match st {
        Statement::Constant { name, expr: e } => vec![name_node(name), op("="), expr(e)],
        Statement::FuncDef { name, params, body } => {
            vec![var(name), paren(comma_list(params.iter().map(|p| name_node(p)).collect())), op("="), expr(body)]
        }
        Statement::Equation { lhs, rhs } => vec![expr(lhs), op("="), expr(rhs)],
        Statement::Inequality { lhs, rhs, cmp } => vec![expr(lhs), op(cmp_symbol(*cmp)), expr(rhs)],
        Statement::Bare(e) => vec![expr(e)],
    };
    if !rs.is_empty() {
        v.push(Node::Space(0.3));
        v.push(restrictions(rs));
    }
    row(v)
}

fn cmp_symbol(c: Comparison) -> &'static str {
    match (c.less, c.strict) {
        (true, true) => "<",
        (true, false) => "≤",
        (false, true) => ">",
        (false, false) => "≥",
    }
}

fn restrictions(rs: &[Restriction]) -> Node {
    let mut v = Vec::new();
    for (i, r) in rs.iter().enumerate() {
        if i > 0 {
            v.push(up(";"));
            v.push(Node::Space(0.25));
        }
        v.push(expr(&r.lhs));
        v.push(op(cmp_symbol(r.cmp)));
        v.push(expr(&r.rhs));
    }
    Node::Delim('{', '}', Box::new(row(v)))
}

/// Nombre de variable: a_1 con subíndice, theta como θ, A.x con el campo.
fn name_node(name: &str) -> Node {
    if name == "theta" {
        return var("θ");
    }
    if let Some((base, field)) = name.split_once('.') {
        return row(vec![name_node(base), up("."), var(field)]);
    }
    if let Some((base, sub)) = name.split_once('_') {
        if !base.is_empty() && !sub.is_empty() {
            let sub_node = if sub.chars().all(|c| c.is_ascii_digit()) { up(sub) } else { var(sub) };
            return Node::Sub(Box::new(var(base)), Box::new(sub_node));
        }
    }
    var(name)
}

/// Precedencia visual: cuanto mayor, más "atómico".
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::F2(Func2::Add | Func2::Sub, ..) => 1,
        Expr::Big { .. } | Expr::Deriv { .. } => 1,
        Expr::Neg(_) => 2,
        Expr::F2(Func2::Mul, ..) => 2,
        Expr::F2(Func2::Pow, ..) => 3,
        _ => 4,
    }
}

fn paren_if(e: &Expr, cond: bool) -> Node {
    if cond { paren(expr(e)) } else { expr(e) }
}

/// ¿La expresión, escrita, empieza por una cifra? (para poner 2·3 y no 23).
fn starts_with_digit(e: &Expr) -> bool {
    match e {
        Expr::Num(v) => !is_named_constant(*v),
        Expr::F2(Func2::Mul | Func2::Pow, a, _) => starts_with_digit(a),
        Expr::F1(Func1::Fact, a) => starts_with_digit(a),
        _ => false,
    }
}

/// ¿Empieza por el nombre de una función (sin, log…)? Entonces va con un espacio delante.
fn starts_with_function(e: &Expr) -> bool {
    match e {
        Expr::F1(f, _) => !matches!(f, Func1::Sqrt | Func1::Cbrt | Func1::Abs | Func1::CAbs | Func1::Fact),
        Expr::F2(Func2::Pow | Func2::Mul, a, _) => starts_with_function(a),
        Expr::F2(Func2::LogBase | Func2::LambertWk, ..) => true,
        _ => false,
    }
}

fn is_named_constant(v: f64) -> bool {
    v == std::f64::consts::PI || v == std::f64::consts::E || v == std::f64::consts::TAU || v.is_infinite()
}

fn number(v: f64) -> Node {
    if v == std::f64::consts::PI {
        // Recta: en cursiva sintética se confunde con una n.
        up("π")
    } else if v == std::f64::consts::E {
        var("e")
    } else if v == std::f64::consts::TAU {
        var("τ")
    } else if v == f64::INFINITY {
        up("∞")
    } else {
        up(&format_number(v))
    }
}

/// Árbol de presentación de una expresión.
pub fn expr(e: &Expr) -> Node {
    match e {
        Expr::Num(v) => number(*v),
        Expr::Var(name) => name_node(name),
        Expr::Neg(a) => row(vec![up("−"), paren_if(a, prec(a) <= 2)]),
        Expr::F2(Func2::Add, a, b) => match &**b {
            // a + (−b) se escribe a − b
            Expr::Neg(inner) => row(vec![expr(a), op("−"), paren_if(inner, prec(inner) <= 1)]),
            _ => row(vec![expr(a), op("+"), expr(b)]),
        },
        Expr::F2(Func2::Sub, a, b) => row(vec![expr(a), op("−"), paren_if(b, prec(b) <= 1)]),
        Expr::F2(Func2::Mul, a, b) => {
            let left = paren_if(a, prec(a) <= 1);
            let right = paren_if(b, prec(b) <= 2);
            // Entre dos números (o antes de una cifra) va un punto; si no, yuxtaposición.
            let sep = if starts_with_digit(b) && prec(b) > 2 {
                op("·")
            } else if starts_with_function(b) {
                Node::Space(0.18)
            } else {
                Node::Space(0.06)
            };
            row(vec![left, sep, right])
        }
        Expr::F2(Func2::Div, a, b) => Node::Frac(Box::new(expr(a)), Box::new(expr(b))),
        Expr::F2(Func2::Pow, a, b) => match &**a {
            // sin(x)^2 se escribe sin²(x), como en los libros.
            Expr::F1(f, arg) if !matches!(f, Func1::Sqrt | Func1::Cbrt | Func1::Abs | Func1::CAbs | Func1::Fact) => row(vec![
                Node::Sup(Box::new(up(math::func1_display(*f))), Box::new(expr(b))),
                Node::Space(0.05),
                paren(expr(arg)),
            ]),
            // |x|² no necesita paréntesis.
            Expr::F1(Func1::Abs | Func1::CAbs, _) => Node::Sup(Box::new(expr(a)), Box::new(expr(b))),
            // La base va entre paréntesis si no es atómica: (x + 1)², (a/b)², (√x)².
            _ => {
                let needs = prec(a) < 4 || matches!(&**a, Expr::F1(..) | Expr::F2(..) | Expr::Call(..) | Expr::Prime { .. });
                Node::Sup(Box::new(paren_if(a, needs)), Box::new(expr(b)))
            }
        },
        Expr::F2(Func2::NCr, n, r) => Node::Binom(Box::new(expr(n)), Box::new(expr(r))),
        Expr::F2(Func2::LogBase, b, x) => row(vec![Node::Sub(Box::new(up("log")), Box::new(expr(b))), paren(expr(x))]),
        Expr::F2(Func2::Root, n, x) => Node::Sqrt(Box::new(expr(x)), Some(Box::new(expr(n)))),
        Expr::F2(Func2::LambertWk, k, x) => row(vec![Node::Sub(Box::new(up("W")), Box::new(expr(k))), paren(expr(x))]),
        Expr::F2(f, a, b) => row(vec![up(math::func2_display(*f)), paren(comma_list(vec![expr(a), expr(b)]))]),
        Expr::F1(Func1::Sqrt, a) => Node::Sqrt(Box::new(expr(a)), None),
        Expr::F1(Func1::Cbrt, a) => Node::Sqrt(Box::new(expr(a)), Some(Box::new(up("3")))),
        Expr::F1(Func1::Abs | Func1::CAbs, a) => Node::Delim('|', '|', Box::new(expr(a))),
        Expr::F1(Func1::Fact, a) => row(vec![paren_if(a, prec(a) < 4), up("!")]),
        Expr::F1(f, a) => row(vec![up(math::func1_display(*f)), Node::Space(0.05), paren(expr(a))]),
        Expr::Call(name, args) => row(vec![var(name), paren(comma_list(args.iter().map(expr).collect()))]),
        Expr::Prime { name, order, arg } => row(vec![var(name), up(&"′".repeat(*order as usize)), paren(expr(arg))]),
        Expr::Tuple(items) => paren(comma_list(items.iter().map(expr).collect())),
        Expr::List(items) => Node::Delim('[', ']', Box::new(comma_list(items.iter().map(expr).collect()))),
        Expr::Range(a, b, c) => {
            let mut items = vec![expr(a)];
            if let Some(b) = b {
                items.push(expr(b));
            }
            let mut v = vec![comma_list(items), Node::Space(0.15), up("…"), Node::Space(0.15), expr(c)];
            v.shrink_to_fit();
            Node::Delim('[', ']', Box::new(row(v)))
        }
        Expr::For { body, var: v, list } => row(vec![
            expr(body),
            Node::Space(0.35),
            up("for"),
            Node::Space(0.35),
            name_node(v),
            op("="),
            expr(list),
        ]),
        Expr::Restrict(body, rs) => row(vec![expr(body), Node::Space(0.3), restrictions(rs)]),
        Expr::Big { op: big, var: v, from, to, body } => match big {
            BigOp::Sum | BigOp::Prod => {
                let symbol = if *big == BigOp::Sum { "∑" } else { "∏" };
                let lower = row(vec![name_node(v), up("="), expr(from)]);
                row(vec![
                    Node::BigOp(symbol, Box::new(lower), Box::new(expr(to))),
                    Node::Space(0.1),
                    paren_if(body, prec(body) <= 1),
                ])
            }
            BigOp::Int => row(vec![
                Node::SubSup(Box::new(Node::Integral), Box::new(expr(from)), Box::new(expr(to))),
                Node::Space(0.1),
                paren_if(body, prec(body) <= 1),
                Node::Space(0.15),
                up("d"),
                name_node(v),
            ]),
        },
        Expr::Deriv { body, order } => {
            let (d_top, d_bottom) = if *order == 1 {
                (up("d"), row(vec![up("d"), var("x")]))
            } else {
                let n = up(&order.to_string());
                (Node::Sup(Box::new(up("d")), Box::new(n.clone())), row(vec![up("d"), Node::Sup(Box::new(var("x")), Box::new(n))]))
            };
            row(vec![Node::Frac(Box::new(d_top), Box::new(d_bottom)), paren(expr(body))])
        }
    }
}

// ───────────────────────────── Composición ─────────────────────────────

/// Caja compuesta: ancho, altura sobre la línea base y profundidad bajo ella.
/// Los elementos se guardan relativos al punto (izquierda, línea base).
pub struct Layout {
    pub w: f32,
    pub asc: f32,
    pub desc: f32,
    prims: Vec<Prim>,
}

enum Prim {
    /// Texto: posición de su línea base y galería ya compuesta (con la distancia de la
    /// parte superior de la galería a su línea base).
    Text { at: Vec2, galley: Arc<Galley>, baseline: f32 },
    /// Polilínea de grosor relativo al tamaño de letra.
    Line { points: Vec<Vec2>, width: f32 },
}

impl Layout {
    fn empty() -> Self {
        Self { w: 0.0, asc: 0.0, desc: 0.0, prims: Vec::new() }
    }

    /// Añade `other` desplazado `(dx, dy)` (dy positivo = hacia abajo).
    fn place(&mut self, other: Layout, dx: f32, dy: f32) {
        for p in other.prims {
            self.prims.push(match p {
                Prim::Text { at, galley, baseline } => Prim::Text { at: at + vec2(dx, dy), galley, baseline },
                Prim::Line { points, width } => Prim::Line { points: points.into_iter().map(|q| q + vec2(dx, dy)).collect(), width },
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
fn axis(size: f32) -> f32 {
    0.27 * size
}

impl Composer<'_> {
    fn text(&self, s: &str, size: f32, italic: bool) -> Layout {
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

    fn line(points: Vec<Vec2>, size: f32) -> Prim {
        Prim::Line { points, width: (size * 0.065).max(1.0) }
    }

    pub fn layout(&self, n: &Node, size: f32) -> Layout {
        let script = (size * 0.7).max(8.0);
        match n {
            Node::Text { s, italic, op } => {
                let t = self.text(s, size, *italic);
                if *op {
                    let pad = size * 0.27;
                    let mut l = Layout { w: t.w + 2.0 * pad, asc: t.asc, desc: t.desc, prims: Vec::new() };
                    l.place(t, pad, 0.0);
                    l
                } else {
                    t
                }
            }
            Node::Space(em) => Layout { w: em * size, ..Layout::empty() },
            Node::Row(items) => {
                let mut l = Layout::empty();
                for it in items {
                    let c = self.layout(it, size);
                    l.asc = l.asc.max(c.asc);
                    l.desc = l.desc.max(c.desc);
                    let x = l.w;
                    l.w += c.w;
                    l.place(c, x, 0.0);
                }
                l
            }
            Node::Frac(num, den) | Node::Binom(num, den) => {
                let is_frac = matches!(n, Node::Frac(..));
                let s = (size * 0.92).max(9.0);
                let (a, b) = (self.layout(num, s), self.layout(den, s));
                let (aw, bw) = (a.w, b.w);
                let gap = size * 0.14;
                let pad = size * 0.12;
                let inner_w = aw.max(bw);
                let w = inner_w + 2.0 * pad;
                let ax = axis(size);
                // Numerador por encima de la raya y denominador por debajo, centrados.
                let num_base = -ax - gap - a.desc;
                let den_base = -ax + gap + b.asc;
                let mut l = Layout { w, asc: ax + gap + a.height(), desc: gap + b.height() - ax, prims: Vec::new() };
                l.place(a, pad + (inner_w - aw) / 2.0, num_base);
                l.place(b, pad + (inner_w - bw) / 2.0, den_base);
                if is_frac {
                    l.prims.push(Self::line(vec![vec2(size * 0.05, -ax), vec2(w - size * 0.05, -ax)], size));
                    l
                } else {
                    self.delimit('(', ')', l, size)
                }
            }
            Node::Sup(base, sup) => {
                let b = self.layout(base, size);
                let s = self.layout(sup, script);
                let shift = (0.42 * size).max(b.asc - 0.5 * s.asc);
                let x = b.w + size * 0.04;
                let mut l = Layout { w: x + s.w, asc: b.asc.max(shift + s.asc), desc: b.desc.max(s.desc - shift), prims: Vec::new() };
                l.place(b, 0.0, 0.0);
                l.place(s, x, -shift);
                l
            }
            Node::Sub(base, sub) => {
                let b = self.layout(base, size);
                let s = self.layout(sub, script);
                let shift = (0.22 * size).max(b.desc - 0.2 * s.desc);
                let x = b.w + size * 0.02;
                let mut l = Layout { w: x + s.w, asc: b.asc.max(s.asc - shift), desc: b.desc.max(shift + s.desc), prims: Vec::new() };
                l.place(b, 0.0, 0.0);
                l.place(s, x, shift);
                l
            }
            Node::SubSup(base, sub, sup) => {
                let b = self.layout(base, size);
                let lo = self.layout(sub, script);
                let hi = self.layout(sup, script);
                let up_shift = b.asc - 0.55 * hi.asc;
                let down_shift = b.desc - 0.1 * lo.asc;
                let x = b.w + size * 0.02;
                let w = x + lo.w.max(hi.w);
                let mut l = Layout {
                    w,
                    asc: b.asc.max(up_shift + hi.asc),
                    desc: b.desc.max(down_shift + lo.desc),
                    prims: Vec::new(),
                };
                l.place(b, 0.0, 0.0);
                l.place(hi, x, -up_shift);
                l.place(lo, x - size * 0.12, down_shift);
                l
            }
            Node::Integral => {
                // ∫ grande, centrado en el eje.
                let big = size * 1.6;
                let t = self.text("∫", big, false);
                let center = (t.asc - t.desc) / 2.0;
                let dy = center - axis(size);
                let mut l = Layout { w: t.w, asc: t.asc - dy, desc: t.desc + dy, prims: Vec::new() };
                l.place(t, 0.0, dy);
                l
            }
            Node::BigOp(symbol, lower, upper) => {
                let big = size * 1.45;
                let t = self.text(symbol, big, false);
                let center = (t.asc - t.desc) / 2.0;
                let dy = center - axis(size);
                let (sym_asc, sym_desc) = (t.asc - dy, t.desc + dy);
                let lo = self.layout(lower, script);
                let hi = self.layout(upper, script);
                let w = t.w.max(lo.w).max(hi.w);
                let gap = size * 0.08;
                let hi_base = -sym_asc - gap - hi.desc;
                let lo_base = sym_desc + gap + lo.asc;
                let mut l = Layout { w, asc: -hi_base + hi.asc, desc: lo_base + lo.desc, prims: Vec::new() };
                let (tw, low, hiw) = (t.w, lo.w, hi.w);
                l.place(t, (w - tw) / 2.0, dy);
                l.place(lo, (w - low) / 2.0, lo_base);
                l.place(hi, (w - hiw) / 2.0, hi_base);
                l
            }
            Node::Sqrt(content, index) => {
                let c = self.layout(content, size);
                let pad = size * 0.12;
                let top = -(c.asc + pad);
                let bottom = c.desc + size * 0.04;
                let mid = (top + bottom) / 2.0;
                let rw = size * 0.6;
                let idx = index.as_ref().map(|i| self.layout(i, (size * 0.55).max(7.0)));
                // Desplazamiento si el índice es más ancho que el gancho del signo.
                let shift = idx.as_ref().map_or(0.0, |i| (i.w - rw * 0.45).max(0.0));
                let x0 = shift;
                let x_end = x0 + rw + c.w + size * 0.1;
                let points = vec![
                    vec2(x0, mid + (bottom - top) * 0.05),
                    vec2(x0 + rw * 0.25, mid - (bottom - top) * 0.02),
                    vec2(x0 + rw * 0.55, bottom),
                    vec2(x0 + rw, top),
                    vec2(x_end, top),
                ];
                let mut l = Layout { w: x_end + size * 0.05, asc: -top + size * 0.07, desc: bottom, prims: vec![Self::line(points, size)] };
                if let Some(i) = idx {
                    let base = mid - (bottom - top) * 0.08 - i.desc;
                    l.asc = l.asc.max(-base + i.asc);
                    // El índice termina justo donde empieza el gancho del signo.
                    let iw = i.w;
                    l.place(i, x0 + rw * 0.45 - iw, base);
                }
                l.place(c, x0 + rw, 0.0);
                l
            }
            Node::Delim(open, close, content) => {
                let c = self.layout(content, size);
                self.delimit(*open, *close, c, size)
            }
        }
    }

    /// Rodea una caja con delimitadores: de la fuente si el contenido es bajo, dibujados
    /// a medida si es alto (fracciones, exponentes grandes…).
    fn delimit(&self, open: char, close: char, c: Layout, size: f32) -> Layout {
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
            return l;
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
        l
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Volcado tipo LaTeX del árbol, para comprobar la estructura.
    fn tex(n: &Node) -> String {
        match n {
            Node::Text { s, .. } => s.clone(),
            Node::Row(v) => v.iter().map(tex).collect(),
            Node::Frac(a, b) => format!("\\frac{{{}}}{{{}}}", tex(a), tex(b)),
            Node::Binom(a, b) => format!("\\binom{{{}}}{{{}}}", tex(a), tex(b)),
            Node::Sup(a, b) => format!("{}^{{{}}}", tex(a), tex(b)),
            Node::Sub(a, b) => format!("{}_{{{}}}", tex(a), tex(b)),
            Node::SubSup(a, b, c) => format!("{}_{{{}}}^{{{}}}", tex(a), tex(b), tex(c)),
            Node::Sqrt(a, None) => format!("\\sqrt{{{}}}", tex(a)),
            Node::Sqrt(a, Some(i)) => format!("\\sqrt[{}]{{{}}}", tex(i), tex(a)),
            Node::Delim(o, c, a) => format!("{o}{}{c}", tex(a)),
            Node::BigOp(s, lo, hi) => format!("{s}_{{{}}}^{{{}}}", tex(lo), tex(hi)),
            Node::Integral => "∫".into(),
            Node::Space(_) => String::new(),
        }
    }

    fn show(src: &str) -> String {
        let funcs: HashSet<String> = ["f".to_string()].into();
        let (st, rs) = math::parse_statement(src, &funcs, &funcs).unwrap();
        tex(&statement(&st, &rs))
    }

    #[test]
    fn structure() {
        assert_eq!(show("5^2"), "5^{2}");
        assert_eq!(show("y = (x^2 + 1)/(x - 3)"), "y=\\frac{x^{2}+1}{x−3}");
        assert_eq!(show("(x + 1)^2"), "(x+1)^{2}");
        assert_eq!(show("2^(x + 1)"), "2^{x+1}");
        assert_eq!(show("sin(x)^2"), "sin^{2}(x)");
        assert_eq!(show("sqrt(x)^3"), "(\\sqrt{x})^{3}");
        assert_eq!(show("2 * 3"), "2·3");
        assert_eq!(show("2x"), "2x");
        assert_eq!(show("2pi"), "2π");
        assert_eq!(show("a - (b + c)"), "a−(b+c)");
        assert_eq!(show("a + -b"), "a−b");
        assert_eq!(show("-x^2"), "−x^{2}");
        assert_eq!(show("(-x)^2"), "(−x)^{2}");
        assert_eq!(show("root(3, x) + |x|"), "\\sqrt[3]{x}+|x|");
        assert_eq!(show("nCr(5, 2)"), "\\binom{5}{2}");
        assert_eq!(show("log(2, 8)"), "log_{2}(8)");
        assert_eq!(show("a_1 + x"), "a_{1}+x");
        assert_eq!(show("sum(n, n^2, 1, 10)"), "∑_{n=1}^{10}n^{2}");
        assert_eq!(show("int(x, x^2, 0, 1)"), "∫_{0}^{1}x^{2}dx");
        assert_eq!(show("d/dx(x^3)"), "\\frac{d}{dx}(x^{3})");
        assert_eq!(show("f'(x) + f(2)"), "f′(x)+f(2)");
        assert_eq!(show("y = x {x > 0; y < 2}"), "y=x{x>0;y<2}");
        assert_eq!(show("y <= x"), "y≤x");
        assert_eq!(show("(1, 2)"), "(1,2)");
        assert_eq!(show("[1...5]"), "[1…5]");
    }
}
