//! Interfaz: panel de expresiones a la izquierda y lienzo de la gráfica a la derecha.

use std::collections::{HashMap, HashSet};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, TextEdit, TextureHandle,
    TextureOptions, pos2, vec2,
};

use crate::analysis::{self, Curve, Poi};
use crate::math::{self, CondPoint, Compiled, Env, Expr, MathError, Restriction, Statement};
use crate::plot::{self, GridColors, Polyline, View};
use crate::style::{self, EditorTarget, LineStyle, Style};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Item {
    id: u64,
    text: String,
    #[serde(default)]
    style: Style,
    visible: bool,
    /// Rango del deslizador, si la línea es `a = número`.
    slider: Option<(f64, f64)>,
}

/// Interpretación de una línea, recalculada cuando cambia algún texto.
enum Kind {
    Empty,
    Error(String),
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
    Point(f64, f64),
}

impl Kind {
    fn curve(&self) -> Option<&Curve> {
        match self {
            Kind::Curve(c) | Kind::Ineq { curve: c, .. } => Some(c),
            _ => None,
        }
    }

    fn is_graph(&self) -> bool {
        self.curve().is_some() || matches!(self, Kind::Point(..))
    }
}

/// Lo que se dibuja de cada línea, en coordenadas de pantalla.
#[derive(Default)]
struct Geometry {
    lines: Vec<Polyline>,
    /// Región sombreada de una desigualdad implícita (máscara blanca, se tiñe al dibujar).
    region: Option<TextureHandle>,
    point: Option<Pos2>,
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
    /// Escala general de la interfaz.
    ui_scale: f32,
    show_pois: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: ThemeChoice::Light, expr_size: 19.0, monospace: false, label_size: 13.0, ui_scale: 1.0, show_pois: true }
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
        };
        for text in ["y = sin(x)", "a = 0.5", "y = a x^2 - 3", "x^2 + y^2 < 9", "(2, 1)"] {
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
        cc.egui_ctx.set_zoom_factor(app.settings.ui_scale);
        app
    }

    fn new_item(&mut self, text: &str) -> Item {
        let id = self.next_id;
        self.next_id += 1;
        Item { id, text: text.into(), style: Style::with_palette(id as usize), visible: true, slider: None }
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
        let mut const_results: HashMap<usize, Result<f64, MathError>> = HashMap::new();
        loop {
            let mut progress = false;
            for (i, p) in parsed.iter().enumerate() {
                let Some(Ok((Statement::Constant { name, expr }, _))) = p else { continue };
                if dup.contains(&i) || matches!(const_results.get(&i), Some(Ok(_))) {
                    continue;
                }
                let res = math::compile(expr, &env).and_then(|c| {
                    if c.uses_x() || c.uses_y() {
                        Err(MathError(format!("'{name}' no puede depender de x ni y")))
                    } else {
                        Ok(c.eval(0.0, 0.0))
                    }
                });
                if let Ok(v) = res {
                    env.constants.insert(name.clone(), v);
                    progress = true;
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
                Some(Err(e)) => Kind::Error(e.0),
                Some(Ok(_)) if dup.contains(&i) => Kind::Error("nombre ya definido en otra línea".into()),
                Some(Ok((st, rs))) => classify(st, &rs, &env, const_results.remove(&i)).unwrap_or_else(|e| Kind::Error(e.0)),
            };
            kinds.push(kind);
        }

        // Deslizadores: crear/ajustar rango.
        for (it, k) in self.items.iter_mut().zip(&kinds) {
            if let Kind::Slider { value, .. } = k {
                let (lo, hi) = it.slider.unwrap_or((-10.0, 10.0));
                let (lo, hi) = if *value < lo || *value > hi {
                    let m = value.abs().max(1.0) * 2.0;
                    (lo.min(-m), hi.max(m))
                } else {
                    (lo, hi)
                };
                it.slider = Some((lo, hi));
            } else {
                it.slider = None;
            }
        }
        self.kinds = kinds;
    }

    // ───────────────────────── Panel de expresiones ─────────────────────────

    fn expressions_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Expresiones");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    if kind.is_graph() {
                        let resp = resp.on_hover_text("Estilo y visibilidad");
                        let target = EditorTarget {
                            has_line: kind.curve().is_some(),
                            has_fill: matches!(kind, Kind::Ineq { .. }),
                            is_point: matches!(kind, Kind::Point(..)),
                        };
                        egui::Popup::from_toggle_button_response(&resp)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                let was_visible = item.visible;
                                style::style_editor(ui, &mut item.style, &mut item.visible, &target);
                                if was_visible != item.visible {
                                    changed = true;
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
                    Kind::Error(msg) => {
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("⚠ {msg}")).color(ui.visuals().error_fg_color).size(small));
                        });
                    }
                    Kind::Value(v) => {
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("= {}", format_number(*v))).strong().size(self.settings.expr_size * 0.9));
                        });
                    }
                    Kind::Slider { name, value } => {
                        let (lo, hi) = item.slider.unwrap_or((-10.0, 10.0));
                        let mut v = *value;
                        ui.indent(item.id, |ui| {
                            ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(60.0);
                            let resp = ui.add(egui::Slider::new(&mut v, lo..=hi).smart_aim(true));
                            if resp.changed() {
                                item.text = format!("{name} = {}", format_number(v));
                                changed = true;
                            }
                        });
                    }
                    _ => {}
                }
                ui.add_space(4.0);
            }

            ui.add_space(8.0);
            ui.collapsing("Ayuda", help_ui);
        });

        if let Some(idx) = add_after {
            let item = self.new_item("");
            self.focus_next = Some(item.id);
            self.items.insert(idx + 1, item);
            changed = true;
        }
        if let Some(idx) = remove {
            let removed = self.items.remove(idx);
            if self.selected == Some(removed.id) {
                self.selected = None;
            }
            if self.items.is_empty() {
                let id = self.push_item("");
                self.focus_next = Some(id);
            }
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
        let mut scale = ui.ctx().zoom_factor();
        if ui.add(egui::Slider::new(&mut scale, 0.6..=2.5).step_by(0.05).text("Escala de la interfaz")).changed() {
            ui.ctx().set_zoom_factor(scale);
        }
        self.settings.ui_scale = scale;
        ui.checkbox(&mut self.settings.show_pois, "Puntos notables al seleccionar una curva");
        ui.add_space(4.0);
        if ui.button("Restablecer ajustes").clicked() {
            self.settings = Settings::default();
            ui.ctx().set_zoom_factor(1.0);
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

        // Interacción: arrastrar = desplazar, rueda / pellizco = zoom.
        if resp.dragged() {
            let d = resp.drag_delta();
            self.view.cx -= d.x as f64 / self.view.scale;
            self.view.cy += d.y as f64 / self.view.scale;
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
            let text = format!("({}, {})", format_number(hit.x), format_number(hit.y));
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
                    Geometry { lines: lines_of(curve), region, point: None }
                }
                Kind::Point(x, y) if x.is_finite() && y.is_finite() => {
                    Geometry { point: Some(view.to_screen(rect, *x, *y)), ..Default::default() }
                }
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
                Curve::Implicit(_) => {}
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

        // 3) Puntos.
        for (item, g) in self.items.iter().zip(geo) {
            if let (true, Some(p)) = (item.visible, g.point) {
                style::draw_point(painter, p, &item.style, item.style.stroke_color(dark));
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
                (_, Some(Curve::Implicit(_))) => geo
                    .and_then(|g| g.get(idx))
                    .and_then(|g| g.lines.iter().flatten().min_by(|a, b| a.distance(mouse).total_cmp(&b.distance(mouse))))
                    .map(|p| self.view.to_world(rect, *p)),
                (Kind::Point(x, y), _) => Some((*x, *y)),
                _ => None,
            };
            let Some((x, y)) = candidate else { continue };
            if !(x.is_finite() && y.is_finite()) {
                continue;
            }
            let d = self.view.to_screen(rect, x, y).distance(mouse);
            if d < 14.0 && best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, Hit { id: item.id, x, y, color: item.style.base_color(dark) }));
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

/// Decide qué es una sentencia ya parseada. `restrictions` son las condiciones `{…}`.
fn classify(
    st: Statement,
    restrictions: &[Restriction],
    env: &Env,
    const_result: Option<Result<f64, MathError>>,
) -> Result<Kind, MathError> {
    let is_y = |c: &Compiled| matches!(c, Compiled::Y);
    let is_x = |c: &Compiled| matches!(c, Compiled::X);
    let conds = restrictions
        .iter()
        .map(|r| Ok((implicit_any(math::compile(&r.lhs, env)?, math::compile(&r.rhs, env)?), r.cmp)))
        .collect::<Result<Vec<_>, MathError>>()?;
    let restricted = !conds.is_empty();
    let not_graph = || Err(MathError("las restricciones {…} solo se aplican a gráficas".into()));
    // Aplica las restricciones a la curva, comprobándolas en el punto de la curva.
    let restrict_curve = |c: Curve| match c {
        Curve::Fx(f) => Curve::Fx(math::restrict(f, conds.clone(), CondPoint::ValueIsY)),
        Curve::Fy(g) => Curve::Fy(math::restrict(g, conds.clone(), CondPoint::ValueIsX)),
        Curve::Implicit(f) => Curve::Implicit(math::restrict(f, conds.clone(), CondPoint::Xy)),
    };

    Ok(match st {
        Statement::Constant { name, expr } => {
            if restricted {
                return not_graph();
            }
            let value = const_result.unwrap_or_else(|| Err(MathError("no se pudo evaluar".into())))?;
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
            Kind::Curve(restrict_curve(match (l, r) {
                (l, r) if is_y(&l) && !r.uses_y() => Curve::Fx(r),
                (l, r) if is_y(&r) && !l.uses_y() => Curve::Fx(l),
                (l, r) if is_x(&l) && !r.uses_x() => Curve::Fy(r),
                (l, r) if is_x(&r) && !l.uses_x() => Curve::Fy(l),
                (l, r) => Curve::Implicit(implicit(l, r)?),
            }))
        }
        Statement::Inequality { lhs, rhs, cmp } => {
            let l = math::compile(&lhs, env)?;
            let r = math::compile(&rhs, env)?;
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
            Kind::Ineq { curve: restrict_curve(curve), positive, strict: cmp.strict, mask }
        }
        Statement::Bare(Expr::Tuple(items)) => {
            if items.len() != 2 {
                return Err(MathError("un punto tiene 2 coordenadas".into()));
            }
            let x = math::compile(&items[0], env)?;
            let y = math::compile(&items[1], env)?;
            if x.uses_x() || x.uses_y() || y.uses_x() || y.uses_y() {
                return Err(MathError("las coordenadas no pueden depender de x ni y".into()));
            }
            let (px, py) = (x.eval(0.0, 0.0), y.eval(0.0, 0.0));
            // Con restricciones, el punto solo se muestra si las cumple.
            if math::conds_hold(&conds, px, py) { Kind::Point(px, py) } else { Kind::Point(f64::NAN, f64::NAN) }
        }
        Statement::Bare(e) => {
            let c = math::compile(&e, env)?;
            if c.uses_y() {
                return Err(MathError("usa una ecuación (con '=') para expresiones con y".into()));
            }
            // "3 {x > 0}" es una gráfica (y = 3 restringida), no un valor.
            if c.uses_x() || restricted {
                Kind::Curve(restrict_curve(Curve::Fx(c)))
            } else {
                Kind::Value(c.eval(0.0, 0.0))
            }
        }
    })
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

fn help_ui(ui: &mut egui::Ui) {
    let rows = [
        ("y = x^2 + 1", "función de x"),
        ("x = sin(y)", "función de y"),
        ("x^2 + y^2 = 4", "curva implícita"),
        ("y < 2x + 1", "desigualdad (región)"),
        ("x^2 + y^2 ≤ 9", "región implícita"),
        ("y = x^2 {x > 2; y < 12}", "restringir la gráfica"),
        ("y = x {-1 < x < 3}", "restricción doble"),
        ("a = 2", "deslizador"),
        ("f(x) = a x^3", "función con nombre"),
        ("f(x - 1)", "usar una función"),
        ("(1, f(1))", "punto"),
        ("2 pi", "calcular un valor"),
    ];
    egui::Grid::new("help").striped(true).show(ui, |ui| {
        for (code, desc) in rows {
            ui.monospace(code);
            ui.label(desc);
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Funciones: sin cos tan sec csc cot asin acos atan sinh cosh tanh sqrt cbrt abs ln log exp floor ceil round sign min max mod root. \
             Constantes: pi, e, tau. También |x| y n!.",
        )
        .small(),
    );
    ui.label(
        RichText::new(
            "Arrastra para mover, rueda para hacer zoom. Haz clic en una curva para ver sus cortes, máximos, mínimos e \
             intersecciones. Clic en el círculo de color: estilo y visibilidad.",
        )
        .small(),
    );
}

impl eframe::App for GraphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_theme(&ui.ctx().clone());
        self.analyze();

        egui::Panel::left("expresiones")
            .resizable(true)
            .default_size(380.0)
            .min_size(260.0)
            .show(ui, |ui| self.expressions_panel(ui));

        // El panel puede haber cambiado textos: reanalizar antes de dibujar.
        self.analyze();
        self.apply_theme(&ui.ctx().clone());

        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }
}
