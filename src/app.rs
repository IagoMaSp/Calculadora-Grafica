//! Interfaz: panel de expresiones a la izquierda y lienzo de la gráfica a la derecha.

use std::collections::{HashMap, HashSet};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, TextEdit, TextureHandle,
    TextureOptions, pos2, vec2,
};

use crate::analysis::{self, Curve, Poi};
use crate::help;
use crate::keyboard::{self, KeyboardState};
use crate::math::{self, C64, CondPoint, Compiled, Env, Expr, MathError, Restriction, Statement};
use crate::plot::{self, GridColors, Polyline, View};
use crate::points::{self, Coord, ListValue, Lists};
use crate::style::{self, EditorTarget, LineStyle, Style};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Item {
    id: u64,
    text: String,
    #[serde(default)]
    style: Style,
    visible: bool,
    /// Configuración del deslizador, si la línea es `a = número`.
    #[serde(default)]
    slider: SliderCfg,
    /// Rango del parámetro (t o θ) de una curva paramétrica o polar.
    #[serde(default)]
    domain: Option<(f64, f64)>,
    #[serde(skip)]
    anim: AnimState,
    /// Etiqueta de los puntos.
    #[serde(default)]
    label: Label,
    /// Dejar rastro cuando el punto se mueve.
    #[serde(default)]
    trace: bool,
    #[serde(skip)]
    trail: Vec<(f64, f64)>,
    /// Animación de dibujo de una curva paramétrica.
    #[serde(skip)]
    draw: DrawAnim,
    /// ¿Se muestran los ajustes del deslizador (rango, paso, velocidad)?
    #[serde(skip)]
    slider_open: bool,
}

/// Acciones del menú contextual de una línea.
#[derive(Clone, Copy)]
enum RowAction {
    Duplicate,
    Up,
    Down,
    Delete,
}

/// Etiqueta de un punto.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(default)]
struct Label {
    /// None = automático (se muestra si el punto tiene nombre).
    show: Option<bool>,
    /// Texto; vacío = el nombre del punto.
    text: String,
    /// Añadir las coordenadas al texto.
    coords: bool,
}

/// Animación que va dibujando una curva paramétrica desde t0 hasta t1.
#[derive(Clone, Copy)]
struct DrawAnim {
    /// Mientras está activa, la curva se dibuja solo hasta `progress`.
    active: bool,
    playing: bool,
    /// Fracción del rango ya dibujada (0–1).
    progress: f64,
    speed: f64,
    looping: bool,
}

impl Default for DrawAnim {
    fn default() -> Self {
        Self { active: false, playing: false, progress: 0.0, speed: 1.0, looping: false }
    }
}

/// Máximo de puntos guardados en el rastro de un punto.
const MAX_TRAIL: usize = 20_000;

/// Rango por defecto de t y θ.
const DEFAULT_DOMAIN: (f64, f64) = (0.0, std::f64::consts::TAU);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
enum AnimMode {
    /// Al llegar al máximo vuelve al mínimo.
    Loop,
    /// Va y vuelve entre mínimo y máximo.
    Bounce,
    /// Se detiene al llegar al máximo.
    Once,
}

const SPEEDS: [f64; 5] = [0.25, 0.5, 1.0, 1.5, 2.0];

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(default)]
struct SliderCfg {
    min: f64,
    max: f64,
    /// Incremento del deslizador; 0 = continuo.
    step: f64,
    speed: f64,
    mode: AnimMode,
}

impl Default for SliderCfg {
    fn default() -> Self {
        Self { min: -10.0, max: 10.0, step: 0.0, speed: 1.0, mode: AnimMode::Loop }
    }
}

impl SliderCfg {
    /// Ajusta `v` al paso (contando desde el mínimo) y al rango.
    fn snap(&self, v: f64) -> f64 {
        if !(self.min <= self.max) {
            return v;
        }
        let v = v.clamp(self.min, self.max);
        if self.step > 0.0 {
            let k = ((v - self.min) / self.step).round();
            (self.min + k * self.step).min(self.max)
        } else {
            v
        }
    }

    /// Texto del valor sin ruido de coma flotante (0.30000000004 → 0.3).
    fn format(&self, v: f64) -> String {
        let decimals = if self.step > 0.0 { (-self.step.log10()).ceil().clamp(0.0, 10.0) as i32 + 1 } else { 4 };
        let m = 10f64.powi(decimals);
        format_number((v * m).round() / m)
    }
}

/// Estado de reproducción de un deslizador (no se guarda).
#[derive(Clone, Copy, Default)]
struct AnimState {
    playing: bool,
    /// Posición continua de la animación (el valor mostrado es esta posición ajustada al paso).
    pos: Option<f64>,
    /// Sentido en el modo rebote: true = hacia atrás.
    backwards: bool,
}

/// Interpretación de una línea, recalculada cuando cambia algún texto.
enum Kind {
    Empty,
    /// Error; `missing` son las variables sin definir (se ofrecen como deslizadores).
    Error { msg: String, missing: Vec<String> },
    /// Constante calculada (o expresión sin x): se muestra su valor.
    Value(f64),
    Slider { name: String, value: f64 },
    /// Definición de función con varios parámetros (no se grafica).
    Definition,
    Curve(Curve),
    /// Desigualdad: región a un lado de `curve`. Para y = f(x), `positive` es "por encima";
    /// para x = g(y), "a la derecha"; para F(x, y) = 0, la región F > 0.
    /// `mask`: si existe, la región se pinta como {mask > 0} (o < 0) en vez de rellenar
    /// hasta el borde (curvas implícitas y desigualdades con restricciones).
    Ineq { curve: Curve, positive: bool, strict: bool, mask: Option<Compiled> },
    /// Uno o varios puntos. `name` si se definió como `A = (1, 2)`; `drag` indica qué
    /// coordenadas se pueden arrastrar con el ratón; `list` si es una lista.
    /// `value`: el número complejo que representa el punto, si viene de uno (z = 1 + 2i).
    Points { pts: Vec<(f64, f64)>, name: Option<String>, drag: Option<[Coord; 2]>, list: bool, value: Option<C64> },
    /// Lista de números: `[1, 2, 3]`.
    Numbers(Vec<f64>),
}

impl Kind {
    fn curve(&self) -> Option<&Curve> {
        match self {
            Kind::Curve(c) | Kind::Ineq { curve: c, .. } => Some(c),
            _ => None,
        }
    }

    fn is_graph(&self) -> bool {
        self.curve().is_some() || matches!(self, Kind::Points { .. })
    }
}

/// Lo que se dibuja de cada línea, en coordenadas de pantalla.
#[derive(Default)]
struct Geometry {
    lines: Vec<Polyline>,
    /// Región sombreada de una desigualdad implícita (máscara blanca, se tiñe al dibujar).
    region: Option<TextureHandle>,
    points: Vec<Pos2>,
}

#[derive(PartialEq, Clone, Copy)]
struct CacheKey {
    revision: u64,
    view: View,
    rect: Rect,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
enum ThemeChoice {
    Light,
    Dark,
    System,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
#[serde(default)]
struct Settings {
    theme: ThemeChoice,
    /// Tamaño de letra de las expresiones.
    expr_size: f32,
    monospace: bool,
    /// Tamaño de los números de los ejes.
    label_size: f32,
    show_pois: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: ThemeChoice::Light, expr_size: 19.0, monospace: false, label_size: 13.0, show_pois: true }
    }
}

impl Settings {
    fn expr_font(&self) -> FontId {
        if self.monospace { FontId::monospace(self.expr_size) } else { FontId::proportional(self.expr_size) }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GraphApp {
    items: Vec<Item>,
    next_id: u64,
    view: View,
    settings: Settings,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    kinds: Vec<Kind>,
    #[serde(skip)]
    analyzed_revision: Option<u64>,
    #[serde(skip)]
    geometry: Option<(CacheKey, Vec<Geometry>)>,
    #[serde(skip)]
    pois: Option<((CacheKey, Option<u64>), Vec<Poi>)>,
    /// Expresión seleccionada (se muestran sus puntos notables).
    #[serde(skip)]
    selected: Option<u64>,
    #[serde(skip)]
    focus_next: Option<u64>,
    #[serde(skip)]
    theme_applied: Option<ThemeChoice>,
    /// Punto que se está arrastrando.
    #[serde(skip)]
    dragging: Option<u64>,
    /// Teclado en pantalla.
    keyboard_open: bool,
    keyboard: KeyboardState,
    /// Última expresión que tuvo el cursor: ahí escribe el teclado.
    #[serde(skip)]
    last_edit: Option<u64>,
    /// Líneas borradas (posición, línea), para deshacer.
    #[serde(skip)]
    deleted: Vec<(usize, Item)>,
    #[serde(skip)]
    help_open: bool,
    #[serde(skip)]
    help_section: usize,
}

impl Default for GraphApp {
    fn default() -> Self {
        let mut app = Self {
            items: Vec::new(),
            next_id: 0,
            view: View::default(),
            settings: Settings::default(),
            revision: 1,
            kinds: Vec::new(),
            analyzed_revision: None,
            geometry: None,
            pois: None,
            selected: None,
            focus_next: None,
            theme_applied: None,
            dragging: None,
            keyboard_open: false,
            keyboard: KeyboardState::default(),
            last_edit: None,
            deleted: Vec::new(),
            help_open: false,
            help_section: 0,
        };
        for text in ["y = sin(x)", "a = 0.5", "y = a x^2 - 3", "x^2 + y^2 < 9", "A = (2, 1)"] {
            app.push_item(text);
        }
        app
    }
}

/// Resultado del rastreo bajo el cursor.
struct Hit {
    id: u64,
    x: f64,
    y: f64,
    color: Color32,
    /// El punto es un número complejo: se muestra como a + bi.
    complex: bool,
}

impl GraphApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Vec<String>) -> Self {
        let mut app: Self = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        app.revision = 1;
        if !initial.is_empty() {
            app.items.clear();
            app.next_id = 0;
            for text in &initial {
                app.push_item(text);
            }
        }
        if app.items.is_empty() {
            app.push_item("");
        }
        // Sin escalado de interfaz (ni con Ctrl +/-): la app siempre a escala 1.
        cc.egui_ctx.set_zoom_factor(1.0);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        app
    }

    fn new_item(&mut self, text: &str) -> Item {
        let id = self.next_id;
        self.next_id += 1;
        Item {
            id,
            text: text.into(),
            style: Style::with_palette(id as usize),
            visible: true,
            slider: SliderCfg::default(),
            domain: None,
            anim: AnimState::default(),
            label: Label::default(),
            trace: false,
            trail: Vec::new(),
            draw: DrawAnim::default(),
            slider_open: false,
        }
    }

    fn push_item(&mut self, text: &str) -> u64 {
        let item = self.new_item(text);
        let id = item.id;
        self.items.push(item);
        self.revision += 1;
        id
    }

    // ───────────────────────── Análisis de expresiones ─────────────────────────

    fn analyze(&mut self) {
        if self.analyzed_revision == Some(self.revision) {
            return;
        }
        self.analyzed_revision = Some(self.revision);

        // 1) Nombres definidos en todas las líneas.
        let mut user_funcs = HashSet::new();
        let mut user_names = HashSet::new();
        for it in &self.items {
            if let Some((name, is_func)) = math::scan_definition(&it.text) {
                if is_func {
                    user_funcs.insert(name.clone());
                }
                user_names.insert(name);
            }
        }

        // 2) Parseo de cada línea.
        let parsed: Vec<Option<Result<(Statement, Vec<Restriction>), MathError>>> = self
            .items
            .iter()
            .map(|it| {
                let t = it.text.trim();
                (!t.is_empty()).then(|| math::parse_statement(t, &user_funcs, &user_names))
            })
            .collect();

        // 3) Entorno: funciones y constantes (resueltas en el orden que haga falta).
        let mut env = Env::default();
        let mut dup: HashSet<usize> = HashSet::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (i, p) in parsed.iter().enumerate() {
            let name = match p {
                Some(Ok((Statement::FuncDef { name, .. }, _))) | Some(Ok((Statement::Constant { name, .. }, _))) => name,
                _ => continue,
            };
            if seen.insert(name.clone(), i).is_some() {
                dup.insert(i);
            }
            if let Some(Ok((Statement::FuncDef { name, params, body }, _))) = p {
                if !dup.contains(&i) {
                    env.funcs.insert(name.clone(), (params.clone(), body.clone()));
                }
            }
        }
        // Puntos y listas con nombre (A = (1, 2), L = [1, 2, 3]) y deslizadores.
        let is_list_def = |i: usize| {
            matches!(&parsed[i], Some(Ok((Statement::Constant { expr, .. }, _))) if points::is_list_syntax(expr))
        };
        let mut sliders = HashSet::new();
        for (i, p) in parsed.iter().enumerate() {
            let Some(Ok((Statement::Constant { name, expr }, _))) = p else { continue };
            if dup.contains(&i) {
                continue;
            }
            if is_list_def(i) {
                env.points.insert(name.clone());
            } else if matches!(expr, Expr::Num(_)) || matches!(expr, Expr::Neg(e) if matches!(**e, Expr::Num(_))) {
                sliders.insert(name.clone());
            }
        }
        let mut lists = Lists::default();
        let mut list_results: HashMap<usize, Result<ListValue, MathError>> = HashMap::new();
        let mut const_results: HashMap<usize, Result<C64, MathError>> = HashMap::new();
        loop {
            let mut progress = false;
            for (i, p) in parsed.iter().enumerate() {
                let Some(Ok((Statement::Constant { name, expr }, _))) = p else { continue };
                if dup.contains(&i) || matches!(const_results.get(&i), Some(Ok(_))) || matches!(list_results.get(&i), Some(Ok(_))) {
                    continue;
                }
                if is_list_def(i) {
                    let res = if points::is_pointish(expr, &lists) {
                        points::eval_points(expr, &env, &lists).map(ListValue::Points)
                    } else {
                        points::eval_number_list(expr, &env, &lists)
                    };
                    match &res {
                        Ok(ListValue::Points(pts)) => {
                            if let [(x, y)] = pts.as_slice() {
                                env.constants.insert(format!("{name}.x"), *x);
                                env.constants.insert(format!("{name}.y"), *y);
                            }
                            lists.points.insert(name.clone(), pts.clone());
                            progress = true;
                        }
                        Ok(ListValue::Numbers(v)) => {
                            lists.numbers.insert(name.clone(), v.clone());
                            progress = true;
                        }
                        Err(_) => {}
                    }
                    list_results.insert(i, res);
                    continue;
                }
                let res = math::compile(expr, &env).and_then(|c| {
                    if c.uses_x() || c.uses_y() {
                        Err(MathError(format!("'{name}' no puede depender de x ni y")))
                    } else if c.uses_t() || c.uses_theta() || c.uses_r() {
                        // Curva compleja con nombre (z = e^(it)): no es una constante.
                        Err(MathError("depende de t".into()))
                    } else {
                        Ok(math::value_of(&c))
                    }
                });
                match &res {
                    Ok(z) if z.im == 0.0 => {
                        env.constants.insert(name.clone(), z.re);
                        progress = true;
                    }
                    Ok(z) => {
                        env.complex.insert(name.clone(), *z);
                        progress = true;
                    }
                    Err(_) => {}
                }
                const_results.insert(i, res);
            }
            if !progress {
                break;
            }
        }

        // 4) Tipo de cada línea.
        let mut kinds = Vec::with_capacity(parsed.len());
        for (i, p) in parsed.into_iter().enumerate() {
            let kind = match p {
                None => Kind::Empty,
                Some(Err(e)) => Kind::Error { msg: e.0, missing: Vec::new() },
                Some(Ok(_)) if dup.contains(&i) => Kind::Error { msg: "nombre ya definido en otra línea".into(), missing: Vec::new() },
                Some(Ok((st, rs))) => {
                    let it = &self.items[i];
                    let (t0, t1) = it.domain.unwrap_or(DEFAULT_DOMAIN);
                    // Durante la animación de dibujo, la curva llega solo hasta el progreso actual.
                    let domain = if it.draw.active { (t0, t0 + (t1 - t0) * it.draw.progress.max(1e-9)) } else { (t0, t1) };
                    let ctx = Ctx { env: &env, lists: &lists, sliders: &sliders, domain };
                    let free = points::statement_free_vars(&st, &rs);
                    classify(st, &rs, &ctx, const_results.remove(&i), list_results.remove(&i)).unwrap_or_else(|e| {
                        let mut missing: Vec<String> = free.into_iter().filter(|n| is_missing(n, &env, &lists)).collect();
                        missing.sort();
                        Kind::Error { msg: e.0, missing }
                    })
                }
            };
            kinds.push(kind);
        }

        // Rastro de los puntos que se mueven.
        for (it, k) in self.items.iter_mut().zip(&kinds) {
            match k {
                Kind::Points { pts, list: false, .. } if it.trace => {
                    if let [(x, y)] = pts.as_slice() {
                        if x.is_finite() && y.is_finite() && it.trail.last() != Some(&(*x, *y)) && it.trail.len() < MAX_TRAIL {
                            it.trail.push((*x, *y));
                        }
                    }
                }
                _ => it.trail.clear(),
            }
        }

        // Deslizadores: si el valor escrito cae fuera del rango, se amplía el rango.
        for (it, k) in self.items.iter_mut().zip(&kinds) {
            if let Kind::Slider { value, .. } = k {
                it.slider.min = it.slider.min.min(*value);
                it.slider.max = it.slider.max.max(*value);
            } else {
                it.anim = AnimState::default();
            }
        }
        self.kinds = kinds;
    }

    /// Avanza los deslizadores que se están reproduciendo.
    fn animate(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1) as f64;
        let mut playing = false;
        let mut changed = false;
        for (item, kind) in self.items.iter_mut().zip(&self.kinds) {
            let Kind::Slider { name, value } = kind else { continue };
            if !item.anim.playing {
                continue;
            }
            playing = true;
            let cfg = item.slider;
            let span = cfg.max - cfg.min;
            if span <= 0.0 {
                continue;
            }
            // Si el valor cambió por otra vía (texto, arrastre), seguir desde ahí.
            let tol = if cfg.step > 0.0 { cfg.step * 0.51 } else { span * 1e-3 };
            let mut pos = match item.anim.pos {
                Some(p) if (cfg.snap(p) - value).abs() <= tol => p,
                _ => *value,
            };
            // A velocidad 1× se recorre el rango completo en 8 segundos.
            let delta = span / 8.0 * cfg.speed * dt;
            match cfg.mode {
                AnimMode::Loop => {
                    pos += delta;
                    if pos > cfg.max {
                        pos = cfg.min + (pos - cfg.max) % span;
                    }
                }
                AnimMode::Bounce => {
                    pos += if item.anim.backwards { -delta } else { delta };
                    if pos > cfg.max {
                        pos = cfg.max - (pos - cfg.max);
                        item.anim.backwards = true;
                    } else if pos < cfg.min {
                        pos = cfg.min + (cfg.min - pos);
                        item.anim.backwards = false;
                    }
                }
                AnimMode::Once => {
                    pos += delta;
                    if pos >= cfg.max {
                        pos = cfg.max;
                        item.anim.playing = false;
                    }
                }
            }
            item.anim.pos = Some(pos);
            let v = cfg.snap(pos);
            if (v - value).abs() > 1e-12 * span {
                item.text = format!("{name} = {}", cfg.format(v));
                changed = true;
            }
        }
        // Curvas paramétricas que se van dibujando: a 1× el recorrido completo dura 4 s.
        for item in &mut self.items {
            let d = &mut item.draw;
            if !(d.active && d.playing) {
                continue;
            }
            playing = true;
            d.progress += dt * d.speed / 4.0;
            if d.progress >= 1.0 {
                if d.looping {
                    d.progress = 0.0;
                } else {
                    d.progress = 1.0;
                    d.playing = false;
                }
            }
            changed = true;
        }
        if changed {
            self.revision += 1;
        }
        if playing {
            ctx.request_repaint();
        }
    }

    /// Atajos de teclado: F1 guía, Ctrl+K teclado, Ctrl+Z deshacer borrado (fuera de un campo).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let editing = ctx.memory(|m| m.focused().is_some());
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, Key::F1) {
                self.help_open = !self.help_open;
            }
            if i.consume_key(egui::Modifiers::COMMAND, Key::K) {
                self.keyboard_open = !self.keyboard_open;
            }
            if !editing && i.consume_key(egui::Modifiers::COMMAND, Key::Z) {
                self.undo_delete();
            }
        });
    }

    /// Borra una línea guardándola para poder deshacer.
    fn delete_item(&mut self, idx: usize) {
        let removed = self.items.remove(idx);
        if self.selected == Some(removed.id) {
            self.selected = None;
        }
        self.deleted.push((idx, removed));
        if self.deleted.len() > 30 {
            self.deleted.remove(0);
        }
        if self.items.is_empty() {
            let id = self.push_item("");
            self.focus_next = Some(id);
        }
        self.revision += 1;
    }

    /// Recupera la última línea borrada.
    fn undo_delete(&mut self) {
        let Some((idx, item)) = self.deleted.pop() else { return };
        // Si solo queda una línea vacía (creada al borrar todo), se reemplaza.
        if self.items.len() == 1 && self.items[0].text.trim().is_empty() {
            self.items.clear();
        }
        let idx = idx.min(self.items.len());
        self.items.insert(idx, item);
        self.revision += 1;
    }

    /// Nombre parecido a `base` que no esté usado (f → f_2, a → a_2…).
    fn free_name(&self, base: &str) -> String {
        let used: HashSet<String> = self.items.iter().filter_map(|it| math::scan_definition(&it.text).map(|(n, _)| n)).collect();
        (2..).map(|k| format!("{base}_{k}")).find(|n| !used.contains(n)).unwrap()
    }

    /// Aplica una tecla del teclado en pantalla a la expresión que tenía el cursor
    /// (o a una nueva, si no había ninguna).
    fn apply_key(&mut self, ctx: &egui::Context, action: keyboard::Action) {
        let idx = match self.last_edit.and_then(|id| self.items.iter().position(|it| it.id == id)) {
            Some(i) => i,
            None => {
                if !self.items.last().is_some_and(|it| it.text.trim().is_empty()) {
                    self.push_item("");
                }
                self.items.len() - 1
            }
        };
        let id = self.items[idx].id;
        let edit_id = expr_edit_id(id);
        let mut chars: Vec<char> = self.items[idx].text.chars().collect();
        let mut state = egui::text_edit::TextEditState::load(ctx, edit_id).unwrap_or_default();
        // Cursor y selección actuales (por defecto, al final del texto).
        let (mut lo, mut hi) = state
            .cursor
            .char_range()
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b), a.max(b))
            })
            .unwrap_or((chars.len(), chars.len()));
        lo = lo.min(chars.len());
        hi = hi.min(chars.len());

        let cursor = match action {
            keyboard::Action::Insert { text, back } => {
                let ins: Vec<char> = text.chars().collect();
                chars.splice(lo..hi, ins.iter().copied());
                lo + ins.len() - back.min(ins.len())
            }
            keyboard::Action::Backspace => {
                if hi > lo {
                    chars.drain(lo..hi);
                    lo
                } else if lo > 0 {
                    chars.remove(lo - 1);
                    lo - 1
                } else {
                    0
                }
            }
            keyboard::Action::Left => lo.saturating_sub(if hi > lo { 0 } else { 1 }),
            keyboard::Action::Right => (hi + if hi > lo { 0 } else { 1 }).min(chars.len()),
            keyboard::Action::Enter => {
                let item = self.new_item("");
                let new_id = item.id;
                self.items.insert(idx + 1, item);
                self.focus_next = Some(new_id);
                self.last_edit = Some(new_id);
                self.revision += 1;
                return;
            }
        };
        let text: String = chars.into_iter().collect();
        if text != self.items[idx].text {
            self.items[idx].text = text;
            self.revision += 1;
        }
        let c = egui::text::CCursor::new(cursor);
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(c)));
        state.store(ctx, edit_id);
        // Devolver el foco al campo para seguir escribiendo.
        ctx.memory_mut(|m| m.request_focus(edit_id));
        self.last_edit = Some(id);
        self.selected = Some(id);
    }

    // ───────────────────────── Panel de expresiones ─────────────────────────

    fn expressions_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Expresiones");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("❓").size(16.0)).on_hover_text("Guía de uso (F1)").clicked() {
                    self.help_open = true;
                }
                if ui.add(egui::Button::new(RichText::new("⌨").size(16.0)).selected(self.keyboard_open)).on_hover_text("Teclado (Ctrl+K)").clicked() {
                    self.keyboard_open = !self.keyboard_open;
                }
                if !self.deleted.is_empty() && ui.button(RichText::new("↶").size(16.0)).on_hover_text("Deshacer el borrado (Ctrl+Z)").clicked() {
                    self.undo_delete();
                }
                let resp = ui.button(RichText::new("⚙").size(16.0)).on_hover_text("Ajustes");
                egui::Popup::menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.settings_ui(ui));
                if ui.button(RichText::new("＋").size(16.0)).on_hover_text("Añadir expresión").clicked() {
                    let id = self.push_item("");
                    self.focus_next = Some(id);
                }
            });
        });
        ui.separator();

        let mut remove = None;
        let mut add_after = None;
        let mut add_sliders: Option<(usize, Vec<String>)> = None;
        let mut row_action: Option<(usize, RowAction)> = None;
        let mut changed = false;
        let font = self.settings.expr_font();
        let row_h = self.settings.expr_size + 10.0;
        let dark = ui.visuals().dark_mode;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for idx in 0..self.items.len() {
                let kind = self.kinds.get(idx).unwrap_or(&Kind::Empty);
                let item = &mut self.items[idx];
                let selected = self.selected == Some(item.id);

                let row = ui.horizontal(|ui| {
                    // Indicador de estilo: clic = editor de estilo.
                    let (rect, resp) = ui.allocate_exact_size(vec2(row_h, row_h), Sense::click());
                    let c = rect.center();
                    let r = (row_h * 0.32).min(11.0);
                    if kind.is_graph() {
                        let color = item.style.base_color(dark);
                        if item.visible {
                            ui.painter().circle_filled(c, r, color);
                        } else {
                            ui.painter().circle_stroke(c, r - 1.0, Stroke::new(2.0, color));
                        }
                        if resp.hovered() {
                            ui.painter().circle_stroke(c, r + 3.0, Stroke::new(1.0, ui.visuals().weak_text_color()));
                        }
                    } else {
                        ui.painter().text(
                            c,
                            Align2::CENTER_CENTER,
                            format!("{}", idx + 1),
                            FontId::proportional(self.settings.expr_size * 0.6),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    // Clic derecho: acciones de la línea.
                    resp.context_menu(|ui| {
                        for (label, action) in [
                            ("⧉  Duplicar", RowAction::Duplicate),
                            ("⏶  Subir", RowAction::Up),
                            ("⏷  Bajar", RowAction::Down),
                            ("✕  Eliminar", RowAction::Delete),
                        ] {
                            if ui.button(label).clicked() {
                                row_action = Some((idx, action));
                                ui.close();
                            }
                        }
                    });
                    if kind.is_graph() {
                        let resp = resp.on_hover_text("Estilo y visibilidad (clic derecho: más opciones)");
                        let target = EditorTarget {
                            has_line: kind.curve().is_some(),
                            has_fill: matches!(kind, Kind::Ineq { .. }),
                            is_point: matches!(kind, Kind::Points { .. }),
                            is_list: matches!(kind, Kind::Points { list: true, .. }),
                        };
                        egui::Popup::from_toggle_button_response(&resp)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                let was_visible = item.visible;
                                style::style_editor(ui, &mut item.style, &mut item.visible, &target);
                                if was_visible != item.visible {
                                    changed = true;
                                }
                                if let Kind::Points { name, list, .. } = kind {
                                    changed |= point_extras_ui(ui, item, name.as_deref(), !*list);
                                }
                            });
                    }

                    // De derecha a izquierda: primero el botón, luego el texto ocupa el resto.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("✕").size(self.settings.expr_size * 0.7)).on_hover_text("Eliminar").clicked() {
                            remove = Some(idx);
                        }
                        let resp = ui.add(
                            TextEdit::singleline(&mut item.text)
                                .id(expr_edit_id(item.id))
                                .desired_width(ui.available_width())
                                .font(font.clone())
                                .margin(vec2(6.0, 4.0))
                                .hint_text("escribe una expresión…"),
                        );
                        if self.focus_next == Some(item.id) {
                            resp.request_focus();
                            self.focus_next = None;
                        }
                        if resp.gained_focus() {
                            self.selected = Some(item.id);
                        }
                        if resp.has_focus() {
                            self.last_edit = Some(item.id);
                        }
                        if resp.changed() {
                            changed = true;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            add_after = Some(idx);
                        }
                    });
                });
                if selected {
                    // Barra de selección a la izquierda de la fila.
                    let r = row.response.rect;
                    ui.painter().rect_filled(
                        Rect::from_min_max(pos2(r.left() - 6.0, r.top()), pos2(r.left() - 3.0, r.bottom())),
                        1.0,
                        item.style.base_color(dark),
                    );
                }

                // Información bajo la expresión.
                let small = self.settings.expr_size * 0.75;
                match kind {
                    Kind::Error { msg, missing } => {
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("⚠ {msg}")).color(ui.visuals().error_fg_color).size(small));
                            if !missing.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new("Crear deslizador:").size(small).weak());
                                    for name in missing.iter().take(6) {
                                        if ui.button(RichText::new(format!("＋ {name}")).size(small)).on_hover_text(format!("Añade {name} = 1")).clicked() {
                                            add_sliders = Some((idx, vec![name.clone()]));
                                        }
                                    }
                                    if missing.len() > 1 && ui.button(RichText::new("＋ todos").size(small)).clicked() {
                                        add_sliders = Some((idx, missing.clone()));
                                    }
                                });
                            }
                        });
                    }
                    Kind::Value(v) => {
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("= {}", format_number(*v))).strong().size(self.settings.expr_size * 0.9));
                        });
                    }
                    Kind::Slider { name, value } => {
                        ui.indent(item.id, |ui| {
                            if slider_ui(ui, item, name, *value) {
                                changed = true;
                            }
                        });
                    }
                    Kind::Points { value: Some(z), .. } => {
                        ui.indent(item.id, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("= {}", format_complex(*z))).strong().size(self.settings.expr_size * 0.9));
                                let polar = format!("|z| = {}   arg = {}", format_number(z.norm()), format_number(z.arg()));
                                ui.label(RichText::new(polar).size(self.settings.expr_size * 0.7).weak());
                            });
                        });
                    }
                    Kind::Numbers(v) => {
                        let shown: Vec<String> = v.iter().take(12).map(|x| format_number(*x)).collect();
                        let more = if v.len() > 12 { format!(", … ({} elementos)", v.len()) } else { String::new() };
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("= [{}{more}]", shown.join(", "))).size(self.settings.expr_size * 0.8));
                        });
                    }
                    Kind::Curve(Curve::Param { var, .. }) | Kind::Ineq { curve: Curve::Param { var, .. }, .. } => {
                        let can_draw = matches!(kind, Kind::Curve(_));
                        ui.indent(item.id, |ui| {
                            if domain_ui(ui, item, var, can_draw) {
                                changed = true;
                            }
                        });
                    }
                    _ => {}
                }
                ui.add_space(4.0);
            }


        });

        if let Some((idx, names)) = add_sliders {
            for (k, name) in names.iter().enumerate() {
                let item = self.new_item(&format!("{name} = 1"));
                self.items.insert(idx + 1 + k, item);
            }
            changed = true;
        }
        if let Some(idx) = add_after {
            let item = self.new_item("");
            self.focus_next = Some(item.id);
            self.items.insert(idx + 1, item);
            changed = true;
        }
        match row_action {
            Some((idx, RowAction::Delete)) => remove = Some(idx),
            Some((idx, RowAction::Duplicate)) => {
                let mut copy = self.new_item(&self.items[idx].text.clone());
                copy.style = self.items[idx].style;
                copy.slider = self.items[idx].slider;
                copy.domain = self.items[idx].domain;
                copy.label = self.items[idx].label.clone();
                // Una definición duplicada chocaría con la original: se renombra.
                if let Some((name, _)) = math::scan_definition(&copy.text) {
                    let new_name = self.free_name(&name);
                    copy.text = copy.text.replacen(&name, &new_name, 1);
                }
                self.items.insert(idx + 1, copy);
                changed = true;
            }
            Some((idx, RowAction::Up)) if idx > 0 => {
                self.items.swap(idx, idx - 1);
                changed = true;
            }
            Some((idx, RowAction::Down)) if idx + 1 < self.items.len() => {
                self.items.swap(idx, idx + 1);
                changed = true;
            }
            _ => {}
        }
        if let Some(idx) = remove {
            self.delete_item(idx);
            changed = true;
        }
        if changed {
            self.revision += 1;
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(260.0);
        ui.label(RichText::new("Ajustes").strong());
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Tema:");
            ui.selectable_value(&mut self.settings.theme, ThemeChoice::Light, "☀ Claro");
            ui.selectable_value(&mut self.settings.theme, ThemeChoice::Dark, "🌙 Oscuro");
            ui.selectable_value(&mut self.settings.theme, ThemeChoice::System, "Sistema");
        });
        ui.add_space(4.0);
        ui.add(egui::Slider::new(&mut self.settings.expr_size, 12.0..=36.0).step_by(1.0).text("Texto de expresiones"));
        ui.checkbox(&mut self.settings.monospace, "Fuente monoespaciada");
        ui.add(egui::Slider::new(&mut self.settings.label_size, 9.0..=24.0).step_by(1.0).text("Números de los ejes"));
        ui.checkbox(&mut self.settings.show_pois, "Puntos notables al seleccionar una curva");
        ui.add_space(4.0);
        if ui.button("Restablecer ajustes").clicked() {
            self.settings = Settings::default();
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.theme_applied == Some(self.settings.theme) {
            return;
        }
        self.theme_applied = Some(self.settings.theme);
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::System => egui::ThemePreference::System,
        });
    }

    // ───────────────────────── Lienzo ─────────────────────────

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let dark = ui.visuals().dark_mode;
        let painter = ui.painter_at(rect);

        // Interacción: arrastrar un punto móvil lo mueve; arrastrar el fondo desplaza la vista;
        // rueda / pellizco = zoom.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
            self.dragging = origin.and_then(|p| self.draggable_under(rect, p));
        }
        if let Some(id) = self.dragging {
            if let (true, Some(p)) = (resp.dragged(), pointer) {
                self.drag_point_to(id, rect, p);
                self.analyze();
            }
            if resp.drag_stopped() || !resp.dragged() {
                self.dragging = None;
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.dragged() {
            let d = resp.drag_delta();
            self.view.cx -= d.x as f64 / self.view.scale;
            self.view.cy += d.y as f64 / self.view.scale;
        } else if pointer.is_some_and(|p| resp.hovered() && self.draggable_under(rect, p).is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if resp.hovered() {
            let (scroll, zoom, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            let factor = zoom as f64 * (scroll.y as f64 * 0.0025).exp();
            if (factor - 1.0).abs() > 1e-6 {
                if let Some(p) = pointer {
                    self.view.zoom_at(rect, p, factor);
                }
            }
        }

        // Fondo y cuadrícula.
        let label_size = self.settings.label_size;
        let (bg, colors) = if dark {
            (
                Color32::from_gray(24),
                GridColors {
                    minor: Color32::from_gray(36),
                    major: Color32::from_gray(58),
                    axis: Color32::from_gray(190),
                    label: Color32::from_gray(200),
                    label_bg: Color32::from_black_alpha(160),
                    label_size,
                },
            )
        } else {
            (
                Color32::WHITE,
                GridColors {
                    minor: Color32::from_gray(242),
                    major: Color32::from_gray(215),
                    axis: Color32::from_gray(40),
                    label: Color32::from_gray(50),
                    label_bg: Color32::from_white_alpha(200),
                    label_size,
                },
            )
        };
        painter.rect_filled(rect, CornerRadius::ZERO, bg);
        plot::draw_grid(&painter, &self.view, rect, &colors);

        self.update_geometry(ui.ctx(), rect);
        self.update_pois(rect);
        self.draw_items(&painter, rect, dark);

        // Rastreo bajo el cursor: primero puntos notables, luego curvas.
        let mouse = resp.hover_pos().filter(|_| !resp.dragged());
        let poi_hit = mouse.and_then(|m| self.poi_under(rect, m));
        let curve_hit = if poi_hit.is_none() { mouse.and_then(|m| self.curve_under(rect, m, dark)) } else { None };

        if let Some(poi) = poi_hit {
            let p = self.view.to_screen(rect, poi.x, poi.y);
            painter.circle_filled(p, 6.0, Color32::from_gray(110));
            painter.circle_stroke(p, 6.0, Stroke::new(1.5, Color32::WHITE));
            let text = format!("{}\n({}, {})", poi.kind.label(), format_number(poi.x), format_number(poi.y));
            draw_tooltip(&painter, rect, p, text, Color32::from_gray(120), dark);
        } else if let Some(hit) = &curve_hit {
            let p = self.view.to_screen(rect, hit.x, hit.y);
            painter.circle_filled(p, 5.5, hit.color);
            painter.circle_stroke(p, 5.5, Stroke::new(1.5, Color32::WHITE));
            let text = if hit.complex {
                format_complex(C64::new(hit.x, hit.y))
            } else {
                format!("({}, {})", format_number(hit.x), format_number(hit.y))
            };
            draw_tooltip(&painter, rect, p, text, hit.color, dark);
        }

        // Clic: seleccionar la curva bajo el cursor (o deseleccionar).
        if resp.clicked() && poi_hit.is_none() {
            self.selected = curve_hit.map(|h| h.id);
        }

        self.view_buttons(ui, rect);
    }

    /// Recalcula las curvas si cambió la vista, el tamaño o alguna expresión.
    fn update_geometry(&mut self, ctx: &egui::Context, rect: Rect) {
        let key = CacheKey { revision: self.revision, view: self.view, rect };
        if self.geometry.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let view = self.view;
        let lines_of = |c: &Curve| match c {
            Curve::Fx(f) => plot::trace_fx(f, &view, rect),
            Curve::Fy(g) => plot::trace_fy(g, &view, rect),
            Curve::Implicit(f) => plot::trace_implicit(f, &view, rect),
            Curve::Param { x, y, t0, t1, .. } => plot::trace_param(x, y, *t0, *t1, &view, rect),
        };
        let geo = self
            .kinds
            .iter()
            .enumerate()
            .map(|(i, k)| match k {
                Kind::Curve(c) => Geometry { lines: lines_of(c), ..Default::default() },
                Kind::Ineq { curve, positive, mask, .. } => {
                    let region = mask.as_ref().map(|f| {
                        let img = plot::region_mask(f, *positive, &view, rect);
                        ctx.load_texture(format!("region-{i}"), img, TextureOptions::LINEAR)
                    });
                    Geometry { lines: lines_of(curve), region, points: Vec::new() }
                }
                Kind::Points { pts, .. } => Geometry {
                    points: pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).map(|&(x, y)| view.to_screen(rect, x, y)).collect(),
                    ..Default::default()
                },
                _ => Geometry::default(),
            })
            .collect();
        self.geometry = Some((key, geo));
    }

    /// Puntos notables de la curva seleccionada (cortes, extremos, intersecciones).
    fn update_pois(&mut self, rect: Rect) {
        let Some((key, geo)) = &self.geometry else { return };
        let full_key = (*key, self.selected);
        if self.pois.as_ref().is_some_and(|(k, _)| *k == full_key) {
            return;
        }
        let mut out = Vec::new();
        let sel_idx = self.selected.and_then(|id| self.items.iter().position(|it| it.id == id));
        if let Some(si) = sel_idx {
            if let (true, Some(curve)) = (self.items[si].visible, self.kinds.get(si).and_then(Kind::curve)) {
                analysis::curve_pois(curve, &self.view, rect, &mut out);
                for (oi, (other, kind)) in self.items.iter().zip(&self.kinds).enumerate() {
                    if oi == si || !other.visible {
                        continue;
                    }
                    if let Some(oc) = kind.curve() {
                        analysis::intersections(curve, oc, &geo[si].lines, &geo[oi].lines, &self.view, rect, &mut out);
                    }
                }
            }
        }
        // Solo los visibles y sin duplicados (a menos de 2 px).
        let mut kept: Vec<Poi> = Vec::new();
        for p in out {
            let s = self.view.to_screen(rect, p.x, p.y);
            if !(p.x.is_finite() && p.y.is_finite() && rect.contains(s)) {
                continue;
            }
            if kept.iter().all(|q| self.view.to_screen(rect, q.x, q.y).distance(s) > 2.0) {
                kept.push(p);
            }
            if kept.len() >= 300 {
                break;
            }
        }
        self.pois = Some((full_key, kept));
    }

    fn draw_items(&self, painter: &egui::Painter, rect: Rect, dark: bool) {
        let Some((_, geo)) = &self.geometry else { return };

        // 1) Rellenos de desigualdades (debajo de todo).
        for ((item, kind), g) in self.items.iter().zip(&self.kinds).zip(geo) {
            let Kind::Ineq { curve, positive, .. } = kind else { continue };
            if !item.visible || item.style.fill_opacity <= 0.0 {
                continue;
            }
            let fill = item.style.fill_color(dark);
            if let Some(tex) = &g.region {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                let size = tex.size_vec2() * 3.0;
                painter.image(tex.id(), Rect::from_min_size(rect.min, size), uv, fill);
                continue;
            }
            match curve {
                Curve::Fx(_) => {
                    let edge_y = if *positive { rect.top() } else { rect.bottom() };
                    painter.add(plot::fill_to_edge(&g.lines, fill, |p| pos2(p.x, edge_y)));
                }
                Curve::Fy(_) => {
                    let edge_x = if *positive { rect.right() } else { rect.left() };
                    painter.add(plot::fill_to_edge(&g.lines, fill, |p| pos2(edge_x, p.y)));
                }
                // Siempre tienen máscara.
                Curve::Implicit(_) | Curve::Param { .. } => {}
            }
        }

        // 2) Curvas (la seleccionada con un halo).
        for ((item, kind), g) in self.items.iter().zip(&self.kinds).zip(geo) {
            if !item.visible || g.lines.is_empty() {
                continue;
            }
            let st = &item.style;
            let color = st.stroke_color(dark);
            // Las desigualdades estrictas se dibujan discontinuas, como es habitual.
            let line = match kind {
                Kind::Ineq { strict: true, .. } if st.line == LineStyle::Solid => LineStyle::Dashed,
                _ => st.line,
            };
            if self.selected == Some(item.id) {
                let halo = st.base_color(dark).gamma_multiply(0.25);
                for l in &g.lines {
                    style::draw_polyline(painter, l, LineStyle::Solid, halo, st.width + 6.0);
                }
            }
            for l in &g.lines {
                style::draw_polyline(painter, l, line, color, st.width);
            }
        }

        // 3) Rastros, líneas entre puntos, puntos y etiquetas.
        for ((item, kind), g) in self.items.iter().zip(&self.kinds).zip(geo) {
            if !item.visible {
                continue;
            }
            let st = &item.style;
            let color = st.stroke_color(dark);
            if item.trail.len() >= 2 {
                let trail: Vec<Pos2> = item.trail.iter().map(|&(x, y)| self.view.to_screen(rect, x, y)).collect();
                style::draw_polyline(painter, &trail, LineStyle::Solid, color.gamma_multiply(0.7), (st.width * 0.8).max(1.5));
            }
            if st.connect && g.points.len() >= 2 {
                style::draw_polyline(painter, &g.points, st.line, color, st.width);
            }
            for p in &g.points {
                style::draw_point(painter, *p, st, color);
            }
            // Punta de una curva que se está dibujando.
            if let (true, Kind::Curve(Curve::Param { x, y, t1, .. })) = (item.draw.active, kind) {
                let (px, py) = (x.eval(*t1, 0.0), y.eval(*t1, 0.0));
                if px.is_finite() && py.is_finite() {
                    let p = self.view.to_screen(rect, px, py);
                    painter.circle_filled(p, st.width + 3.5, st.base_color(dark));
                    painter.circle_stroke(p, st.width + 3.5, Stroke::new(1.5, if dark { Color32::from_gray(24) } else { Color32::WHITE }));
                }
            }
            if let Kind::Points { pts, name, list, .. } = kind {
                self.draw_labels(painter, rect, item, pts, name.as_deref(), !*list, dark);
            }
        }

        // 4) Puntos notables de la curva seleccionada.
        if self.settings.show_pois {
            if let Some((_, pois)) = &self.pois {
                let ring = if dark { Color32::from_gray(24) } else { Color32::WHITE };
                for p in pois {
                    let s = self.view.to_screen(rect, p.x, p.y);
                    painter.circle_filled(s, 4.5, Color32::from_gray(120));
                    painter.circle_stroke(s, 4.5, Stroke::new(1.2, ring));
                }
            }
        }
    }

    /// Etiquetas de los puntos. Por defecto solo se muestran en puntos sueltos con nombre.
    #[allow(clippy::too_many_arguments)]
    fn draw_labels(&self, painter: &egui::Painter, rect: Rect, item: &Item, pts: &[(f64, f64)], name: Option<&str>, single: bool, dark: bool) {
        if !item.label.show.unwrap_or(name.is_some() && single) {
            return;
        }
        let base = if item.label.text.trim().is_empty() { name.unwrap_or("") } else { item.label.text.trim() };
        let text_color = if dark { Color32::from_gray(235) } else { Color32::from_gray(25) };
        let bg = if dark { Color32::from_black_alpha(150) } else { Color32::from_white_alpha(190) };
        let font = FontId::proportional(self.settings.label_size + 3.0);
        for &(x, y) in pts.iter().take(500) {
            if !(x.is_finite() && y.is_finite()) {
                continue;
            }
            let text = match (base.is_empty(), item.label.coords) {
                (_, false) => base.to_string(),
                (true, true) => format!("({}, {})", format_number(x), format_number(y)),
                (false, true) => format!("{base} ({}, {})", format_number(x), format_number(y)),
            };
            if text.is_empty() {
                continue;
            }
            let p = self.view.to_screen(rect, x, y);
            let galley = painter.layout_no_wrap(text, font.clone(), text_color);
            let offset = item.style.point_size + 4.0;
            let r = Rect::from_min_size(p + vec2(offset, -offset - galley.size().y), galley.size()).expand(2.0);
            painter.rect_filled(r, 3.0, bg);
            painter.galley(r.min + vec2(2.0, 2.0), galley, text_color);
        }
    }

    /// Punto arrastrable bajo el cursor (a menos de 12 px).
    fn draggable_under(&self, rect: Rect, mouse: Pos2) -> Option<u64> {
        self.items
            .iter()
            .zip(&self.kinds)
            .filter(|(it, _)| it.visible)
            .filter_map(|(it, k)| match k {
                Kind::Points { pts, drag: Some(_), .. } => {
                    let &(x, y) = pts.first()?;
                    let d = self.view.to_screen(rect, x, y).distance(mouse);
                    (d < 12.0).then_some((d, it.id))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// Mueve el punto `id` a la posición del cursor: reescribe sus números o mueve los
    /// deslizadores de sus coordenadas. Se ajusta a la cuadrícula si está cerca.
    fn drag_point_to(&mut self, id: u64, rect: Rect, mouse: Pos2) {
        let Some(idx) = self.items.iter().position(|it| it.id == id) else { return };
        let Some(Kind::Points { drag: Some(spec), .. }) = self.kinds.get(idx) else { return };
        let spec = spec.clone();
        let (wx, wy) = self.view.to_world(rect, mouse);
        let scale = self.view.scale;
        let minor = plot::minor_step(&self.view, self.settings.label_size);
        let precision = 10f64.powf((1.0 / scale).log10().floor());
        let snap = |v: f64| {
            let g = (v / minor).round() * minor;
            let v = if ((v - g) * scale).abs() < 7.0 { g } else { (v / precision).round() * precision };
            (v * 1e10).round() / 1e10
        };
        let (wx, wy) = (snap(wx), snap(wy));

        let literal = |c: &Coord, v: f64| matches!(c, Coord::Literal).then(|| format_number(v));
        let (nx, ny) = (literal(&spec[0], wx), literal(&spec[1], wy));
        if nx.is_some() || ny.is_some() {
            if let Some(t) = points::rewrite_point_text(&self.items[idx].text, nx, ny) {
                self.items[idx].text = t;
            }
        }
        for (c, v) in spec.iter().zip([wx, wy]) {
            if let Coord::Slider(name) = c {
                self.set_slider(name, v);
            }
        }
        self.revision += 1;
    }

    /// Pone el deslizador `name` en `v` (ajustado a su paso; amplía el rango si hace falta).
    fn set_slider(&mut self, name: &str, v: f64) {
        let Some(it) = self.items.iter_mut().find(|it| math::scan_definition(&it.text).is_some_and(|(n, f)| !f && n == name)) else {
            return;
        };
        let cfg = &mut it.slider;
        cfg.min = cfg.min.min(v);
        cfg.max = cfg.max.max(v);
        let v = if cfg.step > 0.0 { cfg.snap(v) } else { v };
        it.text = format!("{name} = {}", cfg.format(v));
        it.anim.pos = None;
    }

    fn poi_under(&self, rect: Rect, mouse: Pos2) -> Option<Poi> {
        if !self.settings.show_pois {
            return None;
        }
        let (_, pois) = self.pois.as_ref()?;
        pois.iter()
            .map(|p| (self.view.to_screen(rect, p.x, p.y).distance(mouse), p))
            .filter(|(d, _)| *d < 10.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| *p)
    }

    /// Punto de la curva más cercana al cursor (a menos de 14 px).
    fn curve_under(&self, rect: Rect, mouse: Pos2, dark: bool) -> Option<Hit> {
        let (mx, my) = self.view.to_world(rect, mouse);
        let geo = self.geometry.as_ref().map(|(_, g)| g);
        let mut best: Option<(f32, Hit)> = None;
        for (idx, (item, kind)) in self.items.iter().zip(&self.kinds).enumerate() {
            if !item.visible {
                continue;
            }
            let candidate = match (kind, kind.curve()) {
                (_, Some(Curve::Fx(f))) => Some((mx, f.eval(mx, 0.0))),
                (_, Some(Curve::Fy(g))) => Some((g.eval(0.0, my), my)),
                (_, Some(Curve::Implicit(_) | Curve::Param { .. })) => geo
                    .and_then(|g| g.get(idx))
                    .and_then(|g| g.lines.iter().flatten().min_by(|a, b| a.distance(mouse).total_cmp(&b.distance(mouse))))
                    .map(|p| self.view.to_world(rect, *p)),
                (Kind::Points { pts, .. }, _) => pts
                    .iter()
                    .copied()
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .min_by(|a, b| {
                        let da = self.view.to_screen(rect, a.0, a.1).distance(mouse);
                        let db = self.view.to_screen(rect, b.0, b.1).distance(mouse);
                        da.total_cmp(&db)
                    }),
                _ => None,
            };
            let Some((x, y)) = candidate else { continue };
            if !(x.is_finite() && y.is_finite()) {
                continue;
            }
            let d = self.view.to_screen(rect, x, y).distance(mouse);
            if d < 14.0 && best.as_ref().is_none_or(|b| d < b.0) {
                let complex = matches!(kind, Kind::Points { value: Some(_), .. });
                best = Some((d, Hit { id: item.id, x, y, color: item.style.base_color(dark), complex }));
            }
        }
        best.map(|(_, h)| h)
    }

    fn view_buttons(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let size = vec2(34.0, 34.0);
        let x = rect.right() - size.x - 12.0;
        let buttons = [("＋", "Acercar"), ("－", "Alejar"), ("⌂", "Vista inicial")];
        for (i, (label, tip)) in buttons.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, rect.top() + 12.0 + i as f32 * (size.y + 6.0)), size);
            let resp = ui.put(r, egui::Button::new(RichText::new(*label).size(18.0))).on_hover_text(*tip);
            if resp.clicked() {
                match i {
                    0 => self.view.zoom_at(rect, rect.center(), 1.5),
                    1 => self.view.zoom_at(rect, rect.center(), 1.0 / 1.5),
                    _ => self.view = View::default(),
                }
            }
        }
    }
}

/// Deslizador con reproducción y sus ajustes. Devuelve si cambió el texto o el rango.
fn slider_ui(ui: &mut egui::Ui, item: &mut Item, name: &str, value: f64) -> bool {
    let mut changed = false;
    let cfg = &mut item.slider;
    ui.horizontal(|ui| {
        let play = if item.anim.playing { "⏸" } else { "▶" };
        let tip = if item.anim.playing { "Pausar" } else { "Animar" };
        if ui.add(egui::Button::new(RichText::new(play).size(15.0)).min_size(vec2(30.0, 24.0))).on_hover_text(tip).clicked() {
            item.anim.playing = !item.anim.playing;
            // Al empezar desde el final en modo bucle/una vez, arrancar desde el mínimo.
            if item.anim.playing && cfg.mode != AnimMode::Bounce && value >= cfg.max {
                item.anim.pos = Some(cfg.min);
            }
        }
        let gear = ui.add(egui::Button::new("⚙").selected(item.slider_open)).on_hover_text("Rango, paso y velocidad");
        if gear.clicked() {
            item.slider_open = !item.slider_open;
        }

        let mut v = value;
        ui.spacing_mut().slider_width = (ui.available_width() - 100.0).max(60.0);
        let mut slider = egui::Slider::new(&mut v, cfg.min..=cfg.max).custom_formatter(|v, _| cfg.format(v));
        slider = if cfg.step > 0.0 { slider.step_by(cfg.step) } else { slider.smart_aim(true) };
        if ui.add(slider).changed() {
            let v = cfg.snap(v);
            item.text = format!("{name} = {}", cfg.format(v));
            item.anim.pos = None;
            changed = true;
        }
    });

    if item.slider_open {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new(("slider-cfg", item.id)).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                let drag_speed = ((cfg.max - cfg.min).abs() / 200.0).max(1e-3);
                ui.label("Rango");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut cfg.min).speed(drag_speed)).changed();
                    ui.label(format!("≤ {name} ≤"));
                    changed |= ui.add(egui::DragValue::new(&mut cfg.max).speed(drag_speed)).changed();
                });
                ui.end_row();

                ui.label("Paso");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut cfg.step).speed(0.01).range(0.0..=f64::MAX)).changed();
                    for s in [0.01, 0.1, 1.0] {
                        if ui.selectable_label(cfg.step == s, format_number(s)).clicked() {
                            cfg.step = s;
                            changed = true;
                        }
                    }
                    if ui.selectable_label(cfg.step == 0.0, "libre").on_hover_text("Sin paso: valores continuos").clicked() {
                        cfg.step = 0.0;
                        changed = true;
                    }
                });
                ui.end_row();

                ui.label("Velocidad");
                ui.horizontal(|ui| {
                    for sp in SPEEDS {
                        ui.selectable_value(&mut cfg.speed, sp, format!("{}×", format_number(sp)));
                    }
                });
                ui.end_row();

                ui.label("Modo");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut cfg.mode, AnimMode::Loop, "↻ Bucle").on_hover_text("Al llegar al máximo vuelve al mínimo");
                    ui.selectable_value(&mut cfg.mode, AnimMode::Bounce, "⇄ Rebote").on_hover_text("Va y vuelve");
                    ui.selectable_value(&mut cfg.mode, AnimMode::Once, "→ Una vez").on_hover_text("Se detiene en el máximo");
                });
                ui.end_row();
            });
        });
        if cfg.max <= cfg.min {
            cfg.max = cfg.min + if cfg.step > 0.0 { cfg.step } else { 1.0 };
        }
        // Si el rango o el paso dejan el valor fuera, se ajusta.
        let snapped = cfg.snap(value);
        if changed && snapped != value {
            item.text = format!("{name} = {}", cfg.format(snapped));
            item.anim.pos = None;
        }
    }
    changed
}

/// Rango del parámetro de una curva paramétrica o polar y, si `can_draw`, los controles
/// para animar su dibujo. Devuelve si cambió algo que obliga a recalcular la curva.
fn domain_ui(ui: &mut egui::Ui, item: &mut Item, var: &str, can_draw: bool) -> bool {
    let (mut t0, mut t1) = item.domain.unwrap_or(DEFAULT_DOMAIN);
    let mut changed = false;
    ui.horizontal(|ui| {
        let speed = ((t1 - t0).abs() / 200.0).max(1e-3);
        changed |= ui.add(egui::DragValue::new(&mut t0).speed(speed)).changed();
        ui.label(format!("≤ {var} ≤"));
        changed |= ui.add(egui::DragValue::new(&mut t1).speed(speed)).changed();
        for (label, k) in [("π", 1.0), ("2π", 2.0), ("12π", 12.0)] {
            let target = (0.0, k * std::f64::consts::PI);
            if ui.selectable_label((t0, t1) == target, format!("0–{label}")).clicked() {
                (t0, t1) = target;
                changed = true;
            }
        }
    });
    if changed {
        if t1 <= t0 {
            t1 = t0 + 1.0;
        }
        item.domain = Some((t0, t1));
    }

    if can_draw {
        let d = &mut item.draw;
        ui.horizontal(|ui| {
            let (icon, tip) = if d.playing { ("⏸", "Pausar") } else { ("▶", "Dibujar la curva paso a paso") };
            let label = if d.active { icon.to_string() } else { format!("{icon} Dibujar") };
            if ui.button(label).on_hover_text(tip).clicked() {
                if !d.active || d.progress >= 1.0 {
                    d.progress = 0.0;
                }
                d.active = true;
                d.playing = !d.playing;
                changed = true;
            }
            if d.active {
                if ui.button("■").on_hover_text("Terminar: mostrar la curva completa").clicked() {
                    *d = DrawAnim { speed: d.speed, looping: d.looping, ..DrawAnim::default() };
                    changed = true;
                    return;
                }
                // Ancho fijo con margen para el texto: si el contenido excede el panel, el
                // panel se ensancha y el deslizador con él, en bucle.
                ui.spacing_mut().slider_width = (ui.available_width() - 110.0).max(60.0);
                let shown = t0 + (t1 - t0) * d.progress;
                let slider = egui::Slider::new(&mut d.progress, 0.0..=1.0)
                    .custom_formatter(|_, _| format!("{var}={}", format_number((shown * 100.0).round() / 100.0)));
                if ui.add(slider).changed() {
                    d.playing = false;
                    changed = true;
                }
            }
        });
        if d.active {
            ui.horizontal(|ui| {
                ui.label("Velocidad");
                for sp in SPEEDS {
                    ui.selectable_value(&mut d.speed, sp, format!("{}×", format_number(sp)));
                }
                ui.toggle_value(&mut d.looping, "↻").on_hover_text("Repetir");
            });
        }
    }
    changed
}

/// Opciones extra del editor de estilo de un punto: etiqueta y rastro.
/// Devuelve si cambió algo que obliga a reanalizar.
fn point_extras_ui(ui: &mut egui::Ui, item: &mut Item, name: Option<&str>, single: bool) -> bool {
    let mut changed = false;
    ui.separator();
    ui.label("Etiqueta");
    let mut show = item.label.show.unwrap_or(name.is_some() && single);
    if ui.checkbox(&mut show, "Mostrar etiqueta").changed() {
        item.label.show = Some(show);
    }
    ui.add_enabled_ui(show, |ui| {
        ui.horizontal(|ui| {
            ui.label("Texto:");
            let hint = name.unwrap_or("(sin nombre)");
            ui.add(TextEdit::singleline(&mut item.label.text).hint_text(hint).desired_width(150.0));
        });
        ui.checkbox(&mut item.label.coords, "Mostrar coordenadas");
    });
    if single {
        ui.separator();
        if ui.checkbox(&mut item.trace, "Dejar rastro al moverse").on_hover_text("Dibuja el camino del punto cuando cambia (p. ej. con un deslizador animado)").changed() {
            changed = true;
        }
        if item.trace && ui.button("Borrar rastro").clicked() {
            item.trail.clear();
        }
    }
    changed
}

/// ¿`name` es una variable sin definir (candidata a deslizador)?
fn is_missing(name: &str, env: &Env, lists: &Lists) -> bool {
    const KNOWN: &[&str] = &["x", "y", "θ", "theta", "r", "i", "e", "pi", "π", "tau", "τ", "inf"];
    name.chars().next().is_some_and(char::is_alphabetic)
        && !name.contains('.')
        && !KNOWN.contains(&name)
        && !env.constants.contains_key(name)
        && !env.complex.contains_key(name)
        && !env.funcs.contains_key(name)
        && !env.points.contains(name)
        && !lists.numbers.contains_key(name)
        && !lists.points.contains_key(name)
}

/// Id del campo de texto de una expresión (para que el teclado sepa dónde escribir).
fn expr_edit_id(item_id: u64) -> egui::Id {
    egui::Id::unique(("expresion", item_id))
}

fn draw_tooltip(painter: &egui::Painter, rect: Rect, p: Pos2, text: String, accent: Color32, dark: bool) {
    let text_color = if dark { Color32::from_gray(230) } else { Color32::from_gray(20) };
    let galley = painter.layout_no_wrap(text, FontId::proportional(14.0), text_color);
    let mut r = Rect::from_min_size(p + vec2(12.0, -12.0 - galley.size().y), galley.size()).expand(5.0);
    if r.right() > rect.right() {
        r = r.translate(vec2(-r.width() - 24.0, 0.0));
    }
    if r.top() < rect.top() {
        r = r.translate(vec2(0.0, r.height() + 24.0));
    }
    let bg = if dark { Color32::from_gray(45) } else { Color32::WHITE };
    painter.rect_filled(r, 5.0, bg);
    painter.rect_stroke(r, 5.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    painter.galley(r.min + vec2(5.0, 5.0), galley, text_color);
}

const T_ERROR: &str = "t solo se usa en curvas paramétricas: (x(t), y(t))";
const POLAR_ERROR: &str = "θ y r solo se usan en ecuaciones polares, p. ej. r = 1 + cos(θ)";

/// Decide qué es una sentencia ya parseada. `restrictions` son las condiciones `{…}`
/// y `domain` el rango del parámetro si resulta ser una curva paramétrica o polar.
/// Contexto de evaluación de una línea.
struct Ctx<'a> {
    env: &'a Env,
    lists: &'a Lists,
    /// Nombres de los deslizadores (para saber qué puntos se pueden arrastrar).
    sliders: &'a HashSet<String>,
    /// Rango del parámetro si la línea es una curva paramétrica o polar.
    domain: (f64, f64),
}

/// Clasifica la línea y comprueba que ninguna gráfica real reciba valores complejos.
fn classify(
    st: Statement,
    restrictions: &[Restriction],
    ctx: &Ctx,
    const_result: Option<Result<C64, MathError>>,
    list_result: Option<Result<ListValue, MathError>>,
) -> Result<Kind, MathError> {
    let kind = classify_inner(st, restrictions, ctx, const_result, list_result)?;
    let leaks = match &kind {
        Kind::Curve(c) => curve_leaks(c),
        Kind::Ineq { curve, mask, .. } => curve_leaks(curve) || mask.as_ref().is_some_and(Compiled::leaks_complex),
        _ => false,
    };
    if leaks {
        return Err(MathError("da valores complejos: para graficarla usa re(…), im(…), abs(…) o arg(…)".into()));
    }
    Ok(kind)
}

fn curve_leaks(c: &Curve) -> bool {
    match c {
        Curve::Fx(f) | Curve::Fy(f) | Curve::Implicit(f) => f.leaks_complex(),
        Curve::Param { x, y, .. } => x.leaks_complex() || y.leaks_complex(),
    }
}

/// Punto del plano complejo que representa a `z`.
fn complex_point(z: C64, name: Option<String>) -> Kind {
    Kind::Points { pts: vec![(z.re, z.im)], name, drag: None, list: false, value: Some(z) }
}

fn classify_inner(
    st: Statement,
    restrictions: &[Restriction],
    ctx: &Ctx,
    const_result: Option<Result<C64, MathError>>,
    list_result: Option<Result<ListValue, MathError>>,
) -> Result<Kind, MathError> {
    let (env, domain) = (ctx.env, ctx.domain);
    let is_y = |c: &Compiled| matches!(c, Compiled::Y);
    let is_x = |c: &Compiled| matches!(c, Compiled::X);
    let raw_conds = restrictions
        .iter()
        .map(|r| Ok((implicit_any(math::compile(&r.lhs, env)?, math::compile(&r.rhs, env)?), r.cmp)))
        .collect::<Result<Vec<_>, MathError>>()?;
    let restricted = !raw_conds.is_empty();
    // Condiciones para curvas en x/y: r y θ se expresan en función de x e y.
    let conds_xy = || -> Result<Vec<(Compiled, math::Comparison)>, MathError> {
        if raw_conds.iter().any(|(c, _)| c.uses_t()) {
            return Err(MathError(T_ERROR.into()));
        }
        Ok(raw_conds.iter().map(|(c, cmp)| (polar_xy(c), *cmp)).collect())
    };
    // Condiciones para curvas paramétricas: solo pueden depender del parámetro.
    let conds_param = |var: &Compiled| -> Result<Vec<(Compiled, math::Comparison)>, MathError> {
        raw_conds
            .iter()
            .map(|(c, cmp)| {
                let other = c.uses_x() || c.uses_y() || c.uses_r() || if *var == Compiled::T { c.uses_theta() } else { c.uses_t() };
                if other {
                    let name = if *var == Compiled::T { "t" } else { "θ" };
                    return Err(MathError(format!("en esta curva las restricciones solo pueden usar {name}")));
                }
                Ok((param_to_x(c, var), *cmp))
            })
            .collect()
    };
    let not_graph = || Err(MathError("las restricciones {…} solo se aplican a gráficas".into()));
    // Aplica las restricciones a la curva, comprobándolas en el punto de la curva.
    let restrict_curve = |c: Curve, conds: Vec<(Compiled, math::Comparison)>| match c {
        Curve::Fx(f) => Curve::Fx(math::restrict(f, conds, CondPoint::ValueIsY)),
        Curve::Fy(g) => Curve::Fy(math::restrict(g, conds, CondPoint::ValueIsX)),
        Curve::Implicit(f) => Curve::Implicit(math::restrict(f, conds, CondPoint::Xy)),
        p @ Curve::Param { .. } => p,
    };
    // r = f(θ) como curva paramétrica en θ.
    let polar_curve = |f: Compiled| -> Result<Curve, MathError> {
        let conds = conds_param(&Compiled::Theta)?;
        let x = Compiled::F2(math::Func2::Mul, Box::new(f.clone()), Box::new(Compiled::F1(math::Func1::Cos, Box::new(Compiled::Theta))));
        let y = Compiled::F2(math::Func2::Mul, Box::new(f), Box::new(Compiled::F1(math::Func1::Sin, Box::new(Compiled::Theta))));
        Ok(Curve::Param {
            x: math::restrict(param_to_x(&x, &Compiled::Theta), conds.clone(), CondPoint::Xy),
            y: math::restrict(param_to_x(&y, &Compiled::Theta), conds, CondPoint::Xy),
            t0: domain.0,
            t1: domain.1,
            var: "θ",
        })
    };
    let only_theta = |c: &Compiled| !(c.uses_x() || c.uses_y() || c.uses_r() || c.uses_t());
    let check_free = |c: &Compiled| -> Result<(), MathError> {
        if c.uses_t() {
            Err(MathError(T_ERROR.into()))
        } else if c.uses_theta() || c.uses_r() {
            Err(MathError(POLAR_ERROR.into()))
        } else {
            Ok(())
        }
    };

    // Expresión compleja en t (o θ), como e^(it): curva t ↦ (re, im) en el plano complejo.
    let complex_curve = |c: &Compiled| -> Option<Result<Kind, MathError>> {
        if c.uses_x() || c.uses_y() || c.uses_r() || !(c.uses_t() || c.uses_theta()) {
            return None;
        }
        Some((|| {
            if c.uses_t() && c.uses_theta() {
                return Err(MathError("usa un solo parámetro: t o θ".into()));
            }
            let (var, name) = if c.uses_t() { (Compiled::T, "t") } else { (Compiled::Theta, "θ") };
            let conds = conds_param(&var)?;
            let part = |f| param_to_x(&Compiled::F1(f, Box::new(c.clone())), &var);
            Ok(Kind::Curve(Curve::Param {
                x: math::restrict(part(math::Func1::Re), conds.clone(), CondPoint::Xy),
                y: math::restrict(part(math::Func1::Im), conds, CondPoint::Xy),
                t0: domain.0,
                t1: domain.1,
                var: name,
            }))
        })())
    };
    // (x(t), y(t)): curva paramétrica, si las coordenadas usan t o θ.
    let param_curve = |items: &[Expr]| -> Option<Result<Kind, MathError>> {
        let [ex, ey] = items else { return None };
        let (Ok(x), Ok(y)) = (math::compile(ex, env), math::compile(ey, env)) else { return None };
        let uses_t = x.uses_t() || y.uses_t();
        let uses_theta = x.uses_theta() || y.uses_theta();
        if !uses_t && !uses_theta {
            return None;
        }
        Some((|| {
            if [&x, &y].iter().any(|c| c.uses_x() || c.uses_y() || c.uses_r()) {
                return Err(MathError("una paramétrica solo puede depender de t (o de θ)".into()));
            }
            if uses_t && uses_theta {
                return Err(MathError("usa un solo parámetro: t o θ".into()));
            }
            let (var, name) = if uses_t { (Compiled::T, "t") } else { (Compiled::Theta, "θ") };
            let conds = conds_param(&var)?;
            Ok(Kind::Curve(Curve::Param {
                x: math::restrict(param_to_x(&x, &var), conds.clone(), CondPoint::Xy),
                y: math::restrict(param_to_x(&y, &var), conds, CondPoint::Xy),
                t0: domain.0,
                t1: domain.1,
                var: name,
            }))
        })())
    };
    // Puntos ya evaluados: se filtran por las restricciones y se ve si son arrastrables.
    let points_kind = |pts: Vec<(f64, f64)>, name: Option<String>, expr: &Expr| -> Result<Kind, MathError> {
        let conds = conds_xy()?;
        let pts: Vec<(f64, f64)> = pts.into_iter().filter(|&(x, y)| math::conds_hold(&conds, x, y)).collect();
        let list = !matches!(expr, Expr::Tuple(_));
        let drag = if list { None } else { points::drag_spec(expr, ctx.sliders) };
        Ok(Kind::Points { pts, name, drag, list, value: None })
    };

    Ok(match st {
        Statement::Constant { name, expr } if points::is_list_syntax(&expr) => {
            if let Expr::Tuple(items) = &expr {
                if let Some(k) = param_curve(items) {
                    return k;
                }
            }
            match list_result.unwrap_or_else(|| Err(MathError("no se pudo evaluar".into())))? {
                ListValue::Points(pts) => points_kind(pts, Some(name), &expr)?,
                ListValue::Numbers(_) if restricted => return not_graph(),
                ListValue::Numbers(v) => Kind::Numbers(v),
            }
        }
        Statement::Constant { name, expr } => {
            // z = e^(it): curva en el plano complejo con nombre.
            if let Ok(c) = math::compile(&expr, env) {
                if let Some(k) = complex_curve(&c) {
                    return k;
                }
            }
            if restricted {
                return not_graph();
            }
            let value = const_result.unwrap_or_else(|| Err(MathError("no se pudo evaluar".into())))?;
            if value.im != 0.0 {
                return Ok(complex_point(value, Some(name)));
            }
            let value = value.re;
            let is_literal = matches!(&expr, Expr::Num(_)) || matches!(&expr, Expr::Neg(inner) if matches!(**inner, Expr::Num(_)));
            if is_literal { Kind::Slider { name, value } } else { Kind::Value(value) }
        }
        // Las restricciones de una función ya forman parte de su cuerpo.
        Statement::FuncDef { name, params, .. } => {
            if params.len() == 1 {
                let call = Expr::Call(name, vec![Expr::Var("x".into())]);
                let c = math::compile(&call, env)?;
                if c.uses_y() {
                    return Err(MathError("la función usa y".into()));
                }
                check_free(&c)?;
                Kind::Curve(Curve::Fx(c))
            } else {
                // Comprueba que el cuerpo compile con valores de prueba.
                let call = Expr::Call(name, params.iter().map(|_| Expr::Num(1.0)).collect());
                math::compile(&call, env)?;
                Kind::Definition
            }
        }
        Statement::Equation { lhs, rhs } => {
            let l = math::compile(&lhs, env)?;
            let r = math::compile(&rhs, env)?;
            if l.uses_t() || r.uses_t() {
                return Err(MathError(T_ERROR.into()));
            }
            if matches!(l, Compiled::R) && only_theta(&r) {
                return Ok(Kind::Curve(polar_curve(r)?));
            }
            if matches!(r, Compiled::R) && only_theta(&l) {
                return Ok(Kind::Curve(polar_curve(l)?));
            }
            let (l, r) = (polar_xy(&l), polar_xy(&r));
            Kind::Curve(restrict_curve(
                match (l, r) {
                    (l, r) if is_y(&l) && !r.uses_y() => Curve::Fx(r),
                    (l, r) if is_y(&r) && !l.uses_y() => Curve::Fx(l),
                    (l, r) if is_x(&l) && !r.uses_x() => Curve::Fy(r),
                    (l, r) if is_x(&r) && !l.uses_x() => Curve::Fy(l),
                    (l, r) => Curve::Implicit(implicit(l, r)?),
                },
                conds_xy()?,
            ))
        }
        Statement::Inequality { lhs, rhs, cmp } => {
            let l = math::compile(&lhs, env)?;
            let r = math::compile(&rhs, env)?;
            if l.uses_t() || r.uses_t() {
                return Err(MathError(T_ERROR.into()));
            }
            // r < f(θ): borde polar y región {r - f(θ) < 0}.
            let polar = if matches!(l, Compiled::R) && only_theta(&r) {
                Some((r.clone(), !cmp.less))
            } else if matches!(r, Compiled::R) && only_theta(&l) {
                Some((l.clone(), cmp.less))
            } else {
                None
            };
            if let Some((f, positive)) = polar {
                let mask = math::restrict(polar_xy(&implicit_any(Compiled::R, f.clone())), conds_xy()?, CondPoint::Xy);
                return Ok(Kind::Ineq { curve: polar_curve(f)?, positive, strict: cmp.strict, mask: Some(mask) });
            }
            let (l, r) = (polar_xy(&l), polar_xy(&r));
            let conds = conds_xy()?;
            // `positive`: región y > f(x), x > g(y) o F > 0 según el caso.
            let (curve, positive) = match (l, r) {
                (l, r) if is_y(&l) && !r.uses_y() => (Curve::Fx(r), !cmp.less),
                (l, r) if is_y(&r) && !l.uses_y() => (Curve::Fx(l), cmp.less),
                (l, r) if is_x(&l) && !r.uses_x() => (Curve::Fy(r), !cmp.less),
                (l, r) if is_x(&r) && !l.uses_x() => (Curve::Fy(l), cmp.less),
                (l, r) => (Curve::Implicit(implicit(l, r)?), !cmp.less),
            };
            // La región explícita se rellena hasta el borde; con restricciones (o si es
            // implícita) se pinta punto a punto como {F > 0} o {F < 0}.
            let mask = match &curve {
                Curve::Implicit(f) => Some(math::restrict(f.clone(), conds.clone(), CondPoint::Xy)),
                Curve::Fx(f) if restricted => Some(math::restrict(implicit_any(Compiled::Y, f.clone()), conds.clone(), CondPoint::Xy)),
                Curve::Fy(g) if restricted => Some(math::restrict(implicit_any(Compiled::X, g.clone()), conds.clone(), CondPoint::Xy)),
                _ => None,
            };
            Kind::Ineq { curve: restrict_curve(curve, conds), positive, strict: cmp.strict, mask }
        }
        Statement::Bare(Expr::Tuple(items)) => {
            if items.len() != 2 {
                return Err(MathError("un punto tiene 2 coordenadas".into()));
            }
            if let Some(k) = param_curve(&items) {
                return k;
            }
            let expr = Expr::Tuple(items);
            points_kind(points::eval_points(&expr, env, ctx.lists)?, None, &expr)?
        }
        Statement::Bare(e) if points::is_list_syntax(&e) || points::is_number_list(&e, ctx.lists) || points::is_pointish(&e, ctx.lists) => {
            if points::is_pointish(&e, ctx.lists) {
                points_kind(points::eval_points(&e, env, ctx.lists)?, None, &e)?
            } else if restricted {
                return not_graph();
            } else {
                match points::eval_number_list(&e, env, ctx.lists)? {
                    ListValue::Numbers(v) => Kind::Numbers(v),
                    ListValue::Points(pts) => points_kind(pts, None, &e)?,
                }
            }
        }
        Statement::Bare(e) => {
            let c = math::compile(&e, env)?;
            if c.uses_y() {
                return Err(MathError("usa una ecuación (con '=') para expresiones con y".into()));
            }
            if let Some(k) = complex_curve(&c) {
                return k;
            }
            check_free(&c)?;
            // "3 {x > 0}" es una gráfica (y = 3 restringida), no un valor.
            if c.uses_x() || restricted {
                Kind::Curve(restrict_curve(Curve::Fx(c), conds_xy()?))
            } else {
                let z = math::value_of(&c);
                if z.im == 0.0 { Kind::Value(z.re) } else { complex_point(z, None) }
            }
        }
    })
}

/// Sustituye r y θ por sus expresiones en x e y (θ en [0, 2π)).
fn polar_xy(c: &Compiled) -> Compiled {
    use math::{Func1, Func2};
    if !c.uses_r() && !c.uses_theta() {
        return c.clone();
    }
    let sq = |v: Compiled| Compiled::F2(Func2::Mul, Box::new(v.clone()), Box::new(v));
    let r = Compiled::F1(Func1::Sqrt, Box::new(Compiled::F2(Func2::Add, Box::new(sq(Compiled::X)), Box::new(sq(Compiled::Y)))));
    let theta = Compiled::F2(
        Func2::Mod,
        Box::new(Compiled::F2(Func2::Atan2, Box::new(Compiled::Y), Box::new(Compiled::X))),
        Box::new(Compiled::Num(std::f64::consts::TAU)),
    );
    c.subst(&|n| match n {
        Compiled::R => Some(r.clone()),
        Compiled::Theta => Some(theta.clone()),
        _ => None,
    })
}

/// Pone el parámetro (`T` o `Theta`) en el lugar de x, para evaluar con `eval(t, 0)`.
fn param_to_x(c: &Compiled, var: &Compiled) -> Compiled {
    c.subst(&|n| (n == var).then_some(Compiled::X))
}

/// lhs - rhs, sin comprobaciones.
fn implicit_any(l: Compiled, r: Compiled) -> Compiled {
    Compiled::F2(math::Func2::Sub, Box::new(l), Box::new(r))
}

/// F = lhs - rhs, que debe depender de x o de y.
fn implicit(l: Compiled, r: Compiled) -> Result<Compiled, MathError> {
    let f = Compiled::F2(math::Func2::Sub, Box::new(l), Box::new(r));
    if !f.uses_x() && !f.uses_y() {
        return Err(MathError("no depende de x ni de y".into()));
    }
    Ok(f)
}

/// Complejo en forma binómica: 3 + 2i, -i, 0.5 - 1.25i…
pub fn format_complex(z: C64) -> String {
    if z.im == 0.0 || z.im.is_nan() && z.re.is_nan() {
        return format_number(z.re);
    }
    let im_abs = z.im.abs();
    let im = if im_abs == 1.0 { "i".to_string() } else { format!("{}i", format_number(im_abs)) };
    match (z.re == 0.0, z.im < 0.0) {
        (true, false) => im,
        (true, true) => format!("-{im}"),
        (false, neg) => format!("{} {} {im}", format_number(z.re), if neg { "-" } else { "+" }),
    }
}

pub fn format_number(v: f64) -> String {
    if v.is_nan() {
        return "indefinido".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let a = v.abs();
    if a != 0.0 && !(1e-5..1e9).contains(&a) {
        return format!("{v:.4e}");
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

impl eframe::App for GraphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_theme(&ui.ctx().clone());
        self.shortcuts(ui.ctx());
        self.analyze();
        self.animate(&ui.ctx().clone());
        self.analyze();

        // Teclado abajo, a todo el ancho.
        if self.keyboard_open {
            let mut action = None;
            egui::Panel::bottom("teclado").resizable(false).exact_size(262.0).show(ui, |ui| {
                action = keyboard::keyboard_ui(ui, &mut self.keyboard, &mut self.keyboard_open);
            });
            if let Some(a) = action {
                self.apply_key(ui.ctx(), a);
            }
        }

        egui::Panel::left("expresiones")
            .resizable(true)
            .default_size(380.0)
            .size_range(260.0..=(ui.available_width() * 0.6).max(260.0))
            .show(ui, |ui| self.expressions_panel(ui));

        // El panel puede haber cambiado textos: reanalizar antes de dibujar.
        self.analyze();
        self.apply_theme(&ui.ctx().clone());

        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));

        // Guía de uso (ventana aparte). "Probar" añade el ejemplo a la lista.
        for example in help::help_window(&ui.ctx().clone(), &mut self.help_open, &mut self.help_section) {
            match self.items.last_mut() {
                Some(last) if last.text.trim().is_empty() => last.text = example,
                _ => {
                    self.push_item(&example);
                }
            }
            self.revision += 1;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_inserts_at_cursor() {
        let ctx = egui::Context::default();
        let mut app = GraphApp::default();
        app.items.clear();
        let id = app.push_item("y = 2");
        app.last_edit = Some(id);
        let ins = |text: &str, back: usize| keyboard::Action::Insert { text: text.into(), back };

        // Sin cursor guardado se escribe al final; la función deja el cursor dentro.
        app.apply_key(&ctx, ins("sin()", 1));
        assert_eq!(app.items[0].text, "y = 2sin()");
        app.apply_key(&ctx, ins("x", 0));
        assert_eq!(app.items[0].text, "y = 2sin(x)");
        // Plantillas con varios huecos: el cursor queda en el primero.
        app.apply_key(&ctx, keyboard::Action::Right);
        app.apply_key(&ctx, ins(" + ", 0));
        app.apply_key(&ctx, ins("nCr(, )", 3));
        app.apply_key(&ctx, ins("5", 0));
        assert_eq!(app.items[0].text, "y = 2sin(x) + nCr(5, )");
        app.apply_key(&ctx, keyboard::Action::Backspace);
        assert_eq!(app.items[0].text, "y = 2sin(x) + nCr(, )");
        app.apply_key(&ctx, keyboard::Action::Left);
        app.apply_key(&ctx, keyboard::Action::Backspace);
        assert_eq!(app.items[0].text, "y = 2sin(x) + nC(, )");
        // ↵ crea una línea nueva y pasa a escribir en ella.
        app.apply_key(&ctx, keyboard::Action::Enter);
        app.apply_key(&ctx, ins("α", 0));
        assert_eq!(app.items.len(), 2);
        assert_eq!(app.items[1].text, "α");
    }

    fn app_with(lines: &[&str]) -> GraphApp {
        let mut app = GraphApp::default();
        app.items.clear();
        for l in lines {
            app.push_item(l);
        }
        app.analyze();
        app
    }

    #[test]
    fn qol_behaviour() {
        // Variables sin definir: se ofrecen como deslizadores.
        let app = app_with(&["y = a x + b c"]);
        let Kind::Error { missing, .. } = &app.kinds[0] else { panic!("debería ser un error") };
        assert_eq!(missing, &vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        // Los parámetros de funciones y las variables ligadas no cuentan.
        let app = app_with(&["f(u) = u k", "sum(n, n m, 1, 3)"]);
        let Kind::Error { missing, .. } = &app.kinds[0] else { panic!() };
        assert_eq!(missing, &vec!["k".to_string()]);
        let Kind::Error { missing, .. } = &app.kinds[1] else { panic!() };
        assert_eq!(missing, &vec!["m".to_string()]);

        // t puede ser un deslizador; sin definirlo sigue siendo el parámetro.
        let app = app_with(&["t = 2", "y = t x", "(cos(t), sin(t))"]);
        assert!(matches!(app.kinds[0], Kind::Slider { .. }));
        assert!(matches!(app.kinds[1], Kind::Curve(Curve::Fx(_))));
        assert!(matches!(app.kinds[2], Kind::Points { .. })); // con t definido, es un punto
        let app = app_with(&["(cos(t), sin(t))"]);
        assert!(matches!(app.kinds[0], Kind::Curve(Curve::Param { .. })));

        // Paréntesis de agrupación sin cerrar.
        let app = app_with(&["y = (x + 2", "y = sin(x"]);
        assert!(matches!(app.kinds[0], Kind::Curve(_)));
        assert!(matches!(app.kinds[1], Kind::Error { .. }));

        // Primitiva de una función con nombre.
        let app = app_with(&["f(x) = 2x", "y = int(x, f)"]);
        let Kind::Curve(Curve::Fx(c)) = &app.kinds[1] else { panic!() };
        assert!((c.eval(3.0, 0.0) - 9.0).abs() < 1e-9);

        // Duplicar renombra las definiciones; borrar se puede deshacer.
        let mut app = app_with(&["a = 1", "y = a x"]);
        assert_eq!(app.free_name("a"), "a_2");
        app.delete_item(1);
        assert_eq!(app.items.len(), 1);
        app.undo_delete();
        assert_eq!(app.items[1].text, "y = a x");
    }
}
