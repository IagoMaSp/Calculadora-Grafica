//! Interfaz: panel de expresiones a la izquierda y lienzo de la gráfica a la derecha.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, TextEdit, TextureHandle,
    TextureOptions, pos2, vec2,
};

use crate::analysis::{self, Curve, Poi};
use crate::editor;
use crate::help;
use crate::keyboard::{self, KeyboardState};
use crate::latex;
use crate::math::{self, C64, CondPoint, Compiled, Env, Expr, MathError, Restriction, Statement};
use crate::plot::{self, GridColors, Polyline, View};
use crate::points::{self, Coord, ListValue, Lists};
use crate::project;
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
    /// Si existe, la línea es un texto sobre la gráfica (y `text` es su contenido).
    #[serde(default)]
    note: Option<Note>,
}

/// Texto colocado sobre la gráfica.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(default)]
struct Note {
    /// Esquina superior izquierda, en coordenadas de la gráfica (se mueve con ella).
    x: f64,
    y: f64,
    size: f32,
    /// Recuadro semitransparente detrás del texto.
    background: bool,
}

impl Default for Note {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, size: 18.0, background: true }
    }
}

/// Color por defecto de los textos (negro; con tema oscuro se ve claro).
const NOTE_COLOR: [u8; 3] = [0x00, 0x00, 0x00];

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
    /// Texto sobre la gráfica, con los `{…}` ya sustituidos por su valor.
    Note(String),
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
    /// Mostrar las expresiones con escritura matemática (exponentes arriba, fracciones…).
    pretty_math: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: ThemeChoice::Light, expr_size: 19.0, monospace: false, label_size: 13.0, show_pois: true, pretty_math: true }
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
    /// Estado del editor matemático de cada línea.
    #[serde(skip)]
    editors: HashMap<u64, editor::State>,
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
    /// Punto o texto que se está arrastrando.
    #[serde(skip)]
    dragging: Option<Drag>,
    /// Teclado en pantalla.
    keyboard_open: bool,
    keyboard: KeyboardState,
    /// Última expresión que tuvo el cursor: ahí escribe el teclado.
    #[serde(skip)]
    last_edit: Option<u64>,
    /// Líneas borradas (posición, línea), para deshacer.
    #[serde(skip)]
    deleted: Vec<(usize, Item)>,
    /// Si es falso, no se guarda la sesión al cerrar (al abrir con expresiones por
    /// línea de comandos, para no reemplazar la sesión guardada).
    #[serde(skip)]
    persist: bool,
    #[serde(skip)]
    help_open: bool,
    #[serde(skip)]
    help_section: usize,
    /// Archivo del proyecto abierto (None = todavía sin guardar).
    project_path: Option<PathBuf>,
    /// Contenido del proyecto la última vez que se guardó o abrió (para saber si hay cambios).
    saved: String,
    /// Proyectos abiertos o guardados hace poco.
    recent: Vec<PathBuf>,
    /// Acción a la espera de responder "¿Guardar los cambios?".
    #[serde(skip)]
    pending: Option<FileAction>,
    /// Captura de la gráfica en curso (exportar imagen).
    #[serde(skip)]
    capture: Option<Capture>,
    /// Aviso temporal sobre la gráfica: (texto, es un error, hasta qué instante).
    #[serde(skip)]
    toast: Option<(String, bool, f64)>,
    /// Título de la ventana ya aplicado.
    #[serde(skip)]
    title_shown: String,
    /// Herramienta de texto activa: el próximo clic en la gráfica crea un texto.
    #[serde(skip)]
    placing_note: bool,
    /// Texto que se está editando directamente sobre la gráfica.
    #[serde(skip)]
    editing_note: Option<u64>,
    /// Dónde se abrió el menú contextual de la gráfica.
    #[serde(skip)]
    context_pos: Option<Pos2>,
    /// Zona de la gráfica en el último fotograma (para recortar la captura).
    #[serde(skip)]
    canvas_rect: Rect,
}

/// Qué se está arrastrando en la gráfica.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Point(u64),
    Note(u64),
}

/// Acciones que reemplazan el proyecto actual (y piden guardar antes si hay cambios).
#[derive(Clone)]
enum FileAction {
    New,
    /// None = elegir el archivo con un diálogo.
    Open(Option<PathBuf>),
}

enum CaptureTarget {
    File(PathBuf),
    Clipboard,
}

struct Capture {
    target: CaptureTarget,
    /// Aquí deja la imagen el aviso de captura terminada.
    slot: Arc<Mutex<Option<Arc<egui::ColorImage>>>>,
}

/// Proyecto tal como se escribe en el archivo.
#[derive(serde::Serialize)]
struct ProjectOut<'a> {
    app: &'static str,
    version: u32,
    view: View,
    items: &'a [Item],
}

#[derive(serde::Deserialize)]
struct ProjectIn {
    #[serde(default)]
    view: View,
    items: Vec<Item>,
}

/// Recuadro de un texto en pantalla.
struct NoteBox {
    id: u64,
    rect: Rect,
    galley: Arc<egui::Galley>,
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
            editors: HashMap::new(),
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
            persist: true,
            help_open: false,
            help_section: 0,
            project_path: None,
            saved: String::new(),
            recent: Vec::new(),
            pending: None,
            capture: None,
            toast: None,
            title_shown: String::new(),
            placing_note: false,
            editing_note: None,
            context_pos: None,
            canvas_rect: Rect::NOTHING,
        };
        for text in ["y = sin(x)", "a = 0.5", "y = a x^2 - 3", "x^2 + y^2 < 9", "A = (2, 1)"] {
            app.push_item(text);
        }
        app.saved = app.snapshot();
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
        // Un proyecto como argumento (p. ej. al abrir un .agraf desde el explorador de archivos)
        // se abre como desde el menú; si no, los argumentos son expresiones.
        if let Some(path) = initial.first().map(PathBuf::from).filter(|p| project::is_project(p)) {
            app.request(None, FileAction::Open(Some(path)));
        } else if !initial.is_empty() {
            app.persist = false;
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
            note: None,
        }
    }

    fn push_item(&mut self, text: &str) -> u64 {
        let item = self.new_item(text);
        let id = item.id;
        self.items.push(item);
        self.revision += 1;
        id
    }

    /// Añade un texto en (x, y) de la gráfica y lo deja editándose sobre ella.
    fn add_note(&mut self, x: f64, y: f64) -> u64 {
        let mut item = self.new_item("");
        item.note = Some(Note { x, y, ..Default::default() });
        item.style.color = NOTE_COLOR;
        let id = item.id;
        // Sustituye la línea vacía del final, si la hay.
        if self.items.last().is_some_and(|it| it.note.is_none() && it.text.trim().is_empty()) {
            self.items.pop();
        }
        self.items.push(item);
        self.revision += 1;
        self.selected = Some(id);
        self.editing_note = Some(id);
        id
    }

    // ───────────────────────── Proyectos ─────────────────────────

    /// Contenido del proyecto que cuenta como "cambio" (la vista no: moverse no es editar).
    fn snapshot(&self) -> String {
        serde_json::to_string(&self.items).unwrap_or_default()
    }

    fn is_dirty(&self) -> bool {
        // Un proyecto nuevo y vacío no tiene nada que guardar.
        let empty = self.items.iter().all(|it| it.text.trim().is_empty());
        !(empty && self.project_path.is_none()) && self.snapshot() != self.saved
    }

    fn project_name(&self) -> String {
        self.project_path.as_deref().map(project::stem).unwrap_or_else(|| "Sin título".into())
    }

    /// Carpeta donde abrir los diálogos: la del proyecto actual o la del último reciente.
    fn dialog_dir(&self) -> Option<PathBuf> {
        self.project_path.as_ref().or(self.recent.first()).and_then(|p| p.parent()).map(|p| p.to_path_buf())
    }

    fn notify(&mut self, ctx: &egui::Context, text: impl Into<String>, error: bool) {
        let until = ctx.input(|i| i.time) + if error { 6.0 } else { 3.0 };
        self.toast = Some((text.into(), error, until));
    }

    /// Ejecuta `action`, o pregunta antes si hay cambios sin guardar.
    fn request(&mut self, ctx: Option<&egui::Context>, action: FileAction) {
        if self.is_dirty() {
            self.pending = Some(action);
        } else {
            self.pending = None;
            self.run_action(ctx, action);
        }
    }

    fn run_action(&mut self, ctx: Option<&egui::Context>, action: FileAction) {
        match action {
            FileAction::New => self.new_project(),
            FileAction::Open(path) => {
                let Some(path) = path.or_else(|| project::pick_open(self.dialog_dir().as_deref())) else { return };
                if let Err(e) = self.load_from(&path) {
                    let msg = format!("No se pudo abrir {}: {e}", path.display());
                    match ctx {
                        Some(ctx) => self.notify(ctx, msg, true),
                        // Al arrancar todavía no hay contexto: el aviso se muestra en el primer fotograma.
                        None => self.toast = Some((msg, true, f64::INFINITY)),
                    }
                    self.recent.retain(|r| r != &path);
                }
            }
        }
    }

    fn clear_transient(&mut self) {
        self.selected = None;
        self.dragging = None;
        self.editing_note = None;
        self.focus_next = None;
        self.last_edit = None;
        self.deleted.clear();
        self.kinds.clear();
        self.editors.clear();
        self.analyzed_revision = None;
        self.geometry = None;
        self.pois = None;
        self.revision += 1;
    }

    fn new_project(&mut self) {
        self.items.clear();
        self.next_id = 0;
        self.view = View::default();
        self.project_path = None;
        self.clear_transient();
        let id = self.push_item("");
        self.focus_next = Some(id);
        self.saved = self.snapshot();
    }

    fn load_from(&mut self, path: &std::path::Path) -> Result<(), String> {
        let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let p: ProjectIn = serde_json::from_str(&data).map_err(|_| "el archivo no es un proyecto válido".to_string())?;
        self.items = p.items;
        if self.items.is_empty() {
            let item = self.new_item("");
            self.items.push(item);
        }
        self.next_id = self.items.iter().map(|it| it.id + 1).max().unwrap_or(0);
        // Ids repetidos (archivo editado a mano): se renumeran.
        let mut seen = HashSet::new();
        for i in 0..self.items.len() {
            if !seen.insert(self.items[i].id) {
                self.items[i].id = self.next_id;
                self.next_id += 1;
            }
        }
        self.view = p.view;
        self.persist = true;
        self.project_path = Some(path.to_path_buf());
        self.clear_transient();
        self.saved = self.snapshot();
        project::push_recent(&mut self.recent, path);
        Ok(())
    }

    fn save_to(&mut self, ctx: &egui::Context, path: PathBuf) -> bool {
        let out = ProjectOut { app: "AppGrafica", version: 1, view: self.view, items: &self.items };
        let res = serde_json::to_string_pretty(&out).map_err(|e| e.to_string()).and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
        match res {
            Ok(()) => {
                self.saved = self.snapshot();
                project::push_recent(&mut self.recent, &path);
                self.project_path = Some(path);
                self.persist = true;
                self.notify(ctx, format!("Proyecto guardado: {}", self.project_name()), false);
                true
            }
            Err(e) => {
                self.notify(ctx, format!("No se pudo guardar: {e}"), true);
                false
            }
        }
    }

    /// Guardar (pide el nombre si el proyecto aún no tiene archivo). Devuelve si se guardó.
    fn save_project(&mut self, ctx: &egui::Context) -> bool {
        match self.project_path.clone() {
            Some(p) => self.save_to(ctx, p),
            None => self.save_project_as(ctx),
        }
    }

    fn save_project_as(&mut self, ctx: &egui::Context) -> bool {
        match project::pick_save(self.dialog_dir().as_deref(), &self.project_name()) {
            Some(p) => self.save_to(ctx, p),
            None => false,
        }
    }

    /// Exporta la gráfica (sin botones ni marcas de selección) a un PNG o al portapapeles.
    fn export_image(&mut self, ctx: &egui::Context, to_clipboard: bool) {
        let target = if to_clipboard {
            CaptureTarget::Clipboard
        } else {
            match project::pick_png(self.dialog_dir().as_deref(), &self.project_name()) {
                Some(p) => CaptureTarget::File(p),
                None => return,
            }
        };
        let slot = Arc::new(Mutex::new(None));
        let (slot2, ctx2) = (slot.clone(), ctx.clone());
        // La captura es del fotograma siguiente, que se dibuja ya sin los controles.
        ctx.request_screenshot(move |img| {
            if let Ok(mut s) = slot2.lock() {
                *s = Some(img);
            }
            ctx2.request_repaint();
        });
        self.capture = Some(Capture { target, slot });
    }

    /// Recoge la captura cuando llega y la guarda.
    fn finish_capture(&mut self, ctx: &egui::Context) {
        let Some(cap) = &self.capture else { return };
        let Some(img) = cap.slot.lock().ok().and_then(|mut s| s.take()) else { return };
        let Some(cap) = self.capture.take() else { return };
        let ppp = ctx.pixels_per_point();
        let full = Rect::from_min_size(Pos2::ZERO, vec2(img.size[0] as f32, img.size[1] as f32) / ppp);
        let r = self.canvas_rect.intersect(full);
        if !r.is_positive() {
            self.notify(ctx, "No se pudo capturar la gráfica", true);
            return;
        }
        let img = img.region(&r, Some(ppp));
        match cap.target {
            CaptureTarget::Clipboard => {
                ctx.copy_image(img);
                self.notify(ctx, "Imagen copiada al portapapeles", false);
            }
            CaptureTarget::File(path) => match project::write_png(&path, &img) {
                Ok(()) => self.notify(ctx, format!("Imagen guardada: {}", path.display()), false),
                Err(e) => self.notify(ctx, format!("No se pudo guardar la imagen: {e}"), true),
            },
        }
    }

    /// Título de la ventana: nombre del proyecto y ● si hay cambios sin guardar.
    fn update_title(&mut self, ctx: &egui::Context) {
        let dirty = if self.is_dirty() { "● " } else { "" };
        let title = format!("{dirty}{} — AppGrafica", self.project_name());
        if title != self.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title_shown = title;
        }
    }

    /// Diálogo "¿Guardar los cambios?" antes de reemplazar el proyecto.
    fn pending_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending.clone() else { return };
        let mut choice = None;
        let resp = egui::Modal::new(egui::Id::unique("guardar-cambios")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("¿Guardar los cambios?");
            ui.add_space(6.0);
            ui.label(format!("\"{}\" tiene cambios sin guardar. Si continúas sin guardar, se perderán.", self.project_name()));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Guardar").clicked() {
                    choice = Some(true);
                }
                if ui.button("No guardar").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancelar").clicked() {
                    ui.close();
                }
            });
        });
        match choice {
            Some(save) => {
                self.pending = None;
                if !save || self.save_project(ctx) {
                    self.run_action(Some(ctx), action);
                }
            }
            None if resp.should_close() => self.pending = None,
            None => {}
        }
    }

    // ───────────────────────── Análisis de expresiones ─────────────────────────

    fn analyze(&mut self) {
        if self.analyzed_revision == Some(self.revision) {
            return;
        }
        self.analyzed_revision = Some(self.revision);

        // 0) Las líneas en LaTeX (\frac{x}{2}…) se traducen a la sintaxis de la app.
        let sources: Vec<Result<String, MathError>> = self
            .items
            .iter()
            .map(|it| match () {
                // Los textos no son expresiones.
                _ if it.note.is_some() => Ok(String::new()),
                _ if latex::is_latex(&it.text) => latex::to_plain(&it.text),
                _ => Ok(it.text.clone()),
            })
            .collect();

        // 1) Nombres definidos en todas las líneas.
        let mut user_funcs = HashSet::new();
        let mut user_names = HashSet::new();
        for src in sources.iter().flatten() {
            if let Some((name, is_func)) = math::scan_definition(src) {
                if is_func {
                    user_funcs.insert(name.clone());
                }
                user_names.insert(name);
            }
        }

        // 2) Parseo de cada línea.
        let parsed: Vec<Option<Result<(Statement, Vec<Restriction>), MathError>>> = sources
            .iter()
            .map(|src| match src {
                Ok(t) => {
                    let t = t.trim();
                    (!t.is_empty()).then(|| math::parse_statement(t, &user_funcs, &user_names))
                }
                Err(e) => Some(Err(e.clone())),
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
                None if self.items[i].note.is_some() => Kind::Note(interpolate(&self.items[i].text, &env, &lists, &user_funcs, &user_names)),
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
        // Archivos: Ctrl+N nuevo, Ctrl+O abrir, Ctrl+S guardar (Mayús: como…), Ctrl+E exportar imagen.
        let ctrl = egui::Modifiers::COMMAND;
        let (save_as, save, open, new, export) = ctx.input_mut(|i| {
            let save_as = i.consume_key(ctrl | egui::Modifiers::SHIFT, Key::S);
            (save_as, i.consume_key(ctrl, Key::S), i.consume_key(ctrl, Key::O), i.consume_key(ctrl, Key::N), i.consume_key(ctrl, Key::E))
        });
        if save_as {
            self.save_project_as(ctx);
        } else if save {
            self.save_project(ctx);
        }
        if open {
            self.request(Some(ctx), FileAction::Open(None));
        }
        if new {
            self.request(Some(ctx), FileAction::New);
        }
        if export {
            self.export_image(ctx, false);
        }
        // Soltar un proyecto en la ventana lo abre.
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| project::is_project(p)));
        if let Some(p) = dropped {
            self.request(Some(ctx), FileAction::Open(Some(p)));
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.placing_note = false;
        }
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
        // Con el editor matemático, las teclas actúan sobre su árbol (los textos, no).
        if self.settings.pretty_math && self.items[idx].note.is_none() && !matches!(action, keyboard::Action::Enter) {
            let item = &mut self.items[idx];
            let st = self.editors.entry(id).or_insert_with(|| editor::State::new(&item.text));
            st.sync(&item.text);
            match action {
                keyboard::Action::Insert { text, back } => st.insert_template(&text, back),
                keyboard::Action::Backspace => st.backspace(),
                keyboard::Action::Left => st.left(false),
                keyboard::Action::Right => st.right(false),
                keyboard::Action::Enter => {}
            }
            if st.store(&mut item.text) {
                self.revision += 1;
            }
            ctx.memory_mut(|m| m.request_focus(edit_id));
            self.last_edit = Some(id);
            self.selected = Some(id);
            return;
        }
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
                let resp = ui.button(RichText::new("📁").size(16.0)).on_hover_text("Proyecto: guardar, abrir, exportar imagen");
                egui::Popup::menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.project_menu(ui));
            });
        });
        ui.separator();

        let mut remove = None;
        let mut add_after = None;
        let mut add_sliders: Option<(usize, Vec<String>)> = None;
        let mut row_action: Option<(usize, RowAction)> = None;
        // Flechas ↑ ↓ en el borde de una línea: (línea, hacia abajo).
        let mut focus_move: Option<(usize, bool)> = None;
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
                    let is_note = item.note.is_some();
                    if is_note {
                        // Los textos se reconocen por una T de su color (tenue si está oculto).
                        let color = item.style.base_color(dark);
                        let color = if item.visible { color } else { color.gamma_multiply(0.35) };
                        ui.painter().text(c, Align2::CENTER_CENTER, "T", FontId::proportional(r * 2.0), color);
                        if resp.hovered() {
                            ui.painter().circle_stroke(c, r + 3.0, Stroke::new(1.0, ui.visuals().weak_text_color()));
                        }
                    } else if kind.is_graph() {
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
                    if is_note {
                        let resp = resp.on_hover_text("Estilo del texto (clic derecho: más opciones)");
                        let target = EditorTarget { has_line: false, has_fill: false, is_point: false, is_list: false };
                        egui::Popup::from_toggle_button_response(&resp)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                style::style_editor(ui, &mut item.style, &mut item.visible, &target);
                                note_extras_ui(ui, item);
                            });
                    } else if kind.is_graph() {
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
                        // Resultado de una cuenta en el mismo renglón, a la derecha.
                        let result = match kind {
                            Kind::Value(v) => Some(format_number(*v)),
                            Kind::Points { value: Some(z), .. } => Some(format_complex(*z)),
                            _ => None,
                        };
                        if let Some(r) = result {
                            result_chip(ui, &r, self.settings.expr_size);
                        }
                        let edit_id = expr_edit_id(item.id);
                        if is_note {
                            let resp = ui.add(
                                TextEdit::multiline(&mut item.text)
                                    .id(edit_id)
                                    .desired_rows(1)
                                    .desired_width(ui.available_width())
                                    .font(FontId::proportional(self.settings.expr_size))
                                    .margin(vec2(6.0, 4.0))
                                    .hint_text("texto sobre la gráfica…"),
                            )
                            .on_hover_text("Texto sobre la gráfica. Escribe {a} para mostrar el valor de a.");
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
                            changed |= resp.changed();
                            return;
                        }
                        // Editor matemático: se escribe directamente sobre la notación.
                        if self.settings.pretty_math {
                            let st = self.editors.entry(item.id).or_insert_with(|| editor::State::new(&item.text));
                            st.sync(&item.text);
                            if self.focus_next == Some(item.id) {
                                ui.memory_mut(|m| m.request_focus(edit_id));
                                st.cursor_to_end();
                                self.focus_next = None;
                            }
                            let (resp, out) = editor::show(ui, edit_id, st, &mut item.text, self.settings.expr_size, "escribe una expresión…");
                            if resp.gained_focus() {
                                self.selected = Some(item.id);
                            }
                            if resp.has_focus() {
                                self.last_edit = Some(item.id);
                            }
                            changed |= out.changed;
                            if out.enter {
                                add_after = Some(idx);
                            }
                            if out.up {
                                focus_move = Some((idx, false));
                            }
                            if out.down {
                                focus_move = Some((idx, true));
                            }
                            return;
                        }
                        let resp = ui.add(
                            TextEdit::singleline(&mut item.text)
                                .id(edit_id)
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
                        // Con el editor, una casilla sin rellenar se explica mejor así.
                        let empty_slot = self.settings.pretty_math && self.editors.get(&item.id).is_some_and(|st| st.has_empty_slot());
                        let msg = if empty_slot { "completa las casillas vacías" } else { msg.as_str() };
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
                    // Texto con valores: se muestra cómo queda.
                    Kind::Note(shown) if shown != &item.text => {
                        ui.indent(item.id, |ui| {
                            ui.label(RichText::new(format!("→ {shown}")).size(small).weak());
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
        if let Some((idx, down)) = focus_move {
            let target = if down { idx + 1 } else { idx.wrapping_sub(1) };
            if let Some(it) = self.items.get(target) {
                self.focus_next = Some(it.id);
            }
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
                // Un texto duplicado se coloca un poco más abajo para que se vea.
                copy.note = self.items[idx].note.map(|n| Note { y: n.y - 1.5 * n.size as f64 / self.view.scale, ..n });
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

    fn project_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        let ctx = ui.ctx().clone();
        let dirty = if self.is_dirty() { "  (sin guardar)" } else { "" };
        ui.label(RichText::new(format!("{}{dirty}", self.project_name())).strong());
        ui.separator();
        let item = |ui: &mut egui::Ui, text: &str, keys: &str| ui.add(egui::Button::new(text).shortcut_text(keys)).clicked();
        if item(ui, "🗋  Nuevo", "Ctrl+N") {
            ui.close();
            self.request(Some(&ctx), FileAction::New);
        }
        if item(ui, "🗁  Abrir…", "Ctrl+O") {
            ui.close();
            self.request(Some(&ctx), FileAction::Open(None));
        }
        ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
            ui.menu_button("🕘  Recientes", |ui| {
                for p in self.recent.clone() {
                    let resp = ui.button(project::stem(&p)).on_hover_text(p.display().to_string());
                    if resp.clicked() {
                        ui.close();
                        self.request(Some(&ctx), FileAction::Open(Some(p)));
                    }
                }
            });
        });
        ui.separator();
        if item(ui, "💾  Guardar", "Ctrl+S") {
            ui.close();
            self.save_project(&ctx);
        }
        if item(ui, "    Guardar como…", "Ctrl+Mayús+S") {
            ui.close();
            self.save_project_as(&ctx);
        }
        ui.separator();
        if item(ui, "🖼  Exportar imagen PNG…", "Ctrl+E") {
            ui.close();
            self.export_image(&ctx, false);
        }
        if item(ui, "📋  Copiar imagen", "") {
            ui.close();
            self.export_image(&ctx, true);
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
        ui.checkbox(&mut self.settings.pretty_math, "Editor matemático (desactívalo para escribir como texto)");
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
        self.canvas_rect = rect;
        // Mientras se captura la imagen no se dibujan controles ni marcas de selección.
        let clean = self.capture.is_some();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let dark = ui.visuals().dark_mode;
        let painter = ui.painter_at(rect);
        let notes = self.note_boxes(&painter, rect);
        let note_under = |p: Pos2| notes.iter().rev().find(|b| b.rect.expand(4.0).contains(p)).map(|b| b.id);

        // Interacción: arrastrar un punto móvil o un texto lo mueve; arrastrar el fondo desplaza
        // la vista; rueda / pellizco = zoom.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
            self.dragging = origin.and_then(|p| note_under(p).map(Drag::Note).or_else(|| self.draggable_under(rect, p).map(Drag::Point)));
        }
        if let Some(target) = self.dragging {
            if let (true, Some(p)) = (resp.dragged(), pointer) {
                match target {
                    Drag::Point(id) => {
                        self.drag_point_to(id, rect, p);
                        self.analyze();
                    }
                    Drag::Note(id) => {
                        let d = resp.drag_delta();
                        if let Some(n) = self.items.iter_mut().find(|it| it.id == id).and_then(|it| it.note.as_mut()) {
                            n.x += d.x as f64 / self.view.scale;
                            n.y -= d.y as f64 / self.view.scale;
                        }
                    }
                }
            }
            if resp.drag_stopped() || !resp.dragged() {
                self.dragging = None;
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.dragged() {
            let d = resp.drag_delta();
            self.view.cx -= d.x as f64 / self.view.scale;
            self.view.cy += d.y as f64 / self.view.scale;
        } else if self.placing_note && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if pointer.is_some_and(|p| resp.hovered() && (note_under(p).is_some() || self.draggable_under(rect, p).is_some())) {
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
        self.draw_items(&painter, rect, dark, clean);
        self.draw_notes(ui, &painter, &notes, dark, clean);

        // Rastreo bajo el cursor: primero puntos notables, luego curvas (no sobre un texto).
        let mouse = resp.hover_pos().filter(|p| !resp.dragged() && !clean && !self.placing_note && note_under(*p).is_none());
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

        // Clic: crear un texto (herramienta de texto), seleccionar el texto o la curva bajo el
        // cursor, o deseleccionar. Doble clic en un texto: editarlo.
        let click_pos = ui.input(|i| i.pointer.interact_pos());
        if resp.clicked() {
            let note = click_pos.and_then(note_under);
            if let (true, Some(p)) = (self.placing_note, click_pos) {
                let (x, y) = self.view.to_world(rect, p);
                self.add_note(x, y);
                self.placing_note = false;
            } else if note.is_some() {
                self.selected = note;
            } else if poi_hit.is_none() {
                self.selected = curve_hit.map(|h| h.id);
            }
        }
        if resp.double_clicked() {
            if let Some(id) = click_pos.and_then(note_under) {
                self.editing_note = Some(id);
            }
        }
        // Clic derecho: añadir un texto en ese punto.
        if resp.secondary_clicked() {
            self.context_pos = click_pos;
        }
        resp.context_menu(|ui| {
            if ui.button("T  Añadir texto aquí").clicked() {
                if let Some(p) = self.context_pos {
                    let (x, y) = self.view.to_world(rect, p);
                    self.add_note(x, y);
                }
                ui.close();
            }
            if ui.button("🖼  Exportar imagen PNG…").clicked() {
                ui.close();
                self.export_image(ui.ctx(), false);
            }
            if ui.button("📋  Copiar imagen").clicked() {
                ui.close();
                self.export_image(ui.ctx(), true);
            }
        });

        if !clean {
            self.view_buttons(ui, rect);
            self.draw_toast(ui, &painter, rect, dark);
        }
    }

    /// Recuadros en pantalla de los textos visibles (en orden: el último queda encima).
    fn note_boxes(&self, painter: &egui::Painter, rect: Rect) -> Vec<NoteBox> {
        self.items
            .iter()
            .zip(&self.kinds)
            .filter_map(|(it, k)| {
                let (Some(note), Kind::Note(text), true) = (it.note, k, it.visible) else { return None };
                // Mientras se edita se muestra el texto tal cual, con sus {…}.
                let shown = if self.editing_note == Some(it.id) { it.text.clone() } else { text.clone() };
                let galley = painter.layout_no_wrap(shown, FontId::proportional(note.size), Color32::PLACEHOLDER);
                let min = self.view.to_screen(rect, note.x, note.y);
                let size = galley.size().max(vec2(note.size * 0.6, note.size * 1.2));
                Some(NoteBox { id: it.id, rect: Rect::from_min_size(min, size), galley })
            })
            .collect()
    }

    /// Dibuja los textos; el que se está editando se muestra como un campo de texto.
    fn draw_notes(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, notes: &[NoteBox], dark: bool, clean: bool) {
        let mut finished = None;
        for b in notes {
            let Some(item) = self.items.iter_mut().find(|it| it.id == b.id) else { continue };
            let Some(note) = item.note else { continue };
            let color = item.style.base_color(dark);
            if self.editing_note == Some(b.id) && !clean {
                let id = note_edit_id(b.id);
                let r = Rect::from_min_size(b.rect.min - vec2(6.0, 4.0), vec2(b.rect.width().max(180.0) + 30.0, b.rect.height() + 8.0));
                let edit = TextEdit::multiline(&mut item.text)
                    .id(id)
                    .font(FontId::proportional(note.size))
                    .text_color(color)
                    .desired_rows(1)
                    .desired_width(r.width() - 12.0)
                    .hint_text("escribe un texto…");
                let resp = ui.put(r, edit);
                if !resp.has_focus() && !resp.lost_focus() && !ui.memory(|m| m.had_focus_last_frame(id)) {
                    resp.request_focus();
                }
                if resp.changed() {
                    self.revision += 1;
                }
                if resp.lost_focus() {
                    finished = Some(b.id);
                }
                continue;
            }
            if note.background {
                let bg = if dark { Color32::from_black_alpha(170) } else { Color32::from_white_alpha(215) };
                painter.rect_filled(b.rect.expand(4.0), 4.0, bg);
            }
            if self.selected == Some(b.id) && !clean {
                painter.rect_stroke(b.rect.expand(5.0), 4.0, Stroke::new(1.5, color.gamma_multiply(0.6)), egui::StrokeKind::Outside);
            }
            painter.galley(b.rect.min, b.galley.clone(), color);
        }
        // Al terminar de editar, un texto vacío se descarta.
        if let Some(id) = finished {
            self.editing_note = None;
            if let Some(idx) = self.items.iter().position(|it| it.id == id && it.text.trim().is_empty()) {
                self.items.remove(idx);
                if self.items.is_empty() {
                    self.push_item("");
                }
                self.revision += 1;
            }
        }
    }

    /// Aviso temporal abajo en el centro de la gráfica.
    fn draw_toast(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, dark: bool) {
        let now = ui.input(|i| i.time);
        let Some((text, error, until)) = &mut self.toast else { return };
        if until.is_infinite() {
            *until = now + 6.0;
        }
        if now > *until {
            self.toast = None;
            return;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(*until - now));
        let color = if *error { ui.visuals().error_fg_color } else if dark { Color32::from_gray(230) } else { Color32::from_gray(30) };
        let galley = painter.layout(text.clone(), FontId::proportional(15.0), color, rect.width() - 80.0);
        let r = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 40.0), galley.size()).expand2(vec2(14.0, 9.0));
        painter.rect_filled(r, 8.0, if dark { Color32::from_gray(45) } else { Color32::from_gray(245) });
        painter.rect_stroke(r, 8.0, Stroke::new(1.0, color.gamma_multiply(0.4)), egui::StrokeKind::Inside);
        painter.galley(r.min + vec2(14.0, 9.0), galley, color);
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

    fn draw_items(&self, painter: &egui::Painter, rect: Rect, dark: bool, clean: bool) {
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
            if self.selected == Some(item.id) && !clean {
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
        if self.settings.show_pois && !clean {
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
        // Herramienta de texto, separada de los botones de la vista.
        let r = Rect::from_min_size(pos2(x, rect.top() + 24.0 + 3.0 * (size.y + 6.0)), size);
        let tool = egui::Button::new(RichText::new("T").size(18.0).strong()).selected(self.placing_note);
        if ui.put(r, tool).on_hover_text("Añadir texto: haz clic donde quieras ponerlo (Esc cancela)").clicked() {
            self.placing_note = !self.placing_note;
        }
    }
}

/// Ajustes propios de un texto: tamaño y recuadro de fondo.
fn note_extras_ui(ui: &mut egui::Ui, item: &mut Item) {
    let Some(note) = &mut item.note else { return };
    ui.separator();
    ui.label("Texto");
    ui.add(egui::Slider::new(&mut note.size, 10.0..=64.0).step_by(1.0).text("Tamaño"));
    ui.checkbox(&mut note.background, "Fondo detrás del texto");
    ui.label(RichText::new("Arrástralo en la gráfica para moverlo; doble clic para editarlo.").small().weak());
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

/// Expresión con escritura matemática, en un recuadro como el del campo de texto.
/// Si no cabe a lo ancho, se reduce (hasta el 60 %).
/// Resultado "= valor" en un recuadro tenue, como etiqueta al final del renglón.
/// Un número muy largo se acorta (el valor completo queda en la ayuda emergente).
fn result_chip(ui: &mut egui::Ui, value: &str, size: f32) {
    let max_chars = 18;
    let shown = if value.chars().count() > max_chars { format!("{}…", value.chars().take(max_chars - 1).collect::<String>()) } else { value.to_string() };
    let text = RichText::new(format!("= {shown}")).size(size * 0.9).strong();
    let frame = egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 3));
    frame.show(ui, |ui| ui.label(text)).response.on_hover_text(format!("= {value}"));
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
fn note_edit_id(item_id: u64) -> egui::Id {
    egui::Id::unique(("texto", item_id))
}

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

/// Sustituye cada `{expr}` de un texto por su valor: "a = {a}" → "a = 0.5".
/// Un punto con nombre se muestra como (x, y); lo que no se puede calcular queda como "{?}".
fn interpolate(text: &str, env: &Env, lists: &Lists, user_funcs: &HashSet<String>, user_names: &HashSet<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let Some(len) = rest[open..].find('}') else { break };
        out.push_str(&rest[..open]);
        let src = rest[open + 1..open + len].trim();
        let value = match lists.points.get(src).map(Vec::as_slice) {
            Some([(x, y)]) => Some(format!("({}, {})", format_number(*x), format_number(*y))),
            _ => math::parse_expr(src, user_funcs, user_names)
                .and_then(|e| math::compile(&e, env))
                .ok()
                .filter(|c| !(c.uses_x() || c.uses_y() || c.uses_t() || c.uses_theta() || c.uses_r()))
                .map(|c| math::value_of(&c))
                .filter(|z| z.re.is_finite() && z.im.is_finite())
                .map(|z| if z.im == 0.0 { format_number(z.re) } else { format_complex(z) }),
        };
        out.push_str(&value.unwrap_or_else(|| "{?}".into()));
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
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

        let ctx = ui.ctx().clone();
        self.finish_capture(&ctx);
        self.pending_dialog(&ctx);
        self.update_title(&ctx);

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
        if !self.persist {
            return;
        }
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
        // Modo texto (sin el editor matemático).
        app.settings.pretty_math = false;
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

    #[test]
    fn keyboard_in_math_editor() {
        let ctx = egui::Context::default();
        let mut app = GraphApp::default();
        app.items.clear();
        let id = app.push_item("y = ");
        app.last_edit = Some(id);
        let ins = |text: &str, back: usize| keyboard::Action::Insert { text: text.into(), back };
        // ∑ del teclado: el cursor queda en el cuerpo.
        app.apply_key(&ctx, ins("sum(n, , 1, 10)", 8));
        app.apply_key(&ctx, ins("n", 0));
        app.apply_key(&ctx, ins("^2", 0));
        assert_eq!(app.items[0].text, "y = sum(n, n^2, 1, 10)");
        app.analyze();
        let Kind::Curve(Curve::Fx(f)) = &app.kinds[0] else { panic!() };
        assert_eq!(f.eval(0.0, 0.0), 385.0);
        // ⌫ tras el exponente entra en él; el siguiente borra el 2.
        app.apply_key(&ctx, keyboard::Action::Backspace);
        assert_eq!(app.items[0].text, "y = sum(n, n^2, 1, 10)");
        app.apply_key(&ctx, keyboard::Action::Backspace);
        assert_eq!(app.items[0].text, "y = sum(n, n^(), 1, 10)");
    }

    /// Imagen de la app sin ventana (APPGRAFICA_SNAPSHOTS=carpeta para guardarla).
    fn snapshot_of(app: &mut GraphApp, name: &str, frames: Vec<Vec<egui::Event>>, dark: bool, focus: Option<u64>) {
        let img = crate::snapshot::render([1000.0, 560.0], 1.5, frames, |ui| {
            ui.ctx().set_theme(if dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
            if let Some(id) = focus {
                ui.memory_mut(|m| m.request_focus(expr_edit_id(id)));
            }
            app.analyze();
            egui::Panel::left("expresiones").default_size(430.0).show(ui, |ui| app.expressions_panel(ui));
            app.analyze();
            egui::CentralPanel::no_frame().show(ui, |ui| app.canvas(ui));
        });
        crate::snapshot::save(name, &img);
    }

    #[test]
    fn math_editor_snapshot() {
        let mut app = app_with(&[
            "y = x^2/2 - 3",
            "f(x) = sqrt(x + 1) + root(3, x)",
            "sum(n, 1/n^2, 1, 100)",
            "y = |x - 2| {x > 0}",
            "int(x, e^(-x^2), 0, 1)",
            "y = d/dx(sin(x)^2)",
            "a_1 = 2pi",
            "",
        ]);
        let ctx = egui::Context::default();
        let id = app.items.last().unwrap().id;
        app.last_edit = Some(id);
        app.apply_key(&ctx, keyboard::Action::Insert { text: "prod(n, , 1, )".into(), back: 6 });
        app.focus_next = None;
        let frames = vec![vec![], vec![], vec![]];
        snapshot_of(&mut app, "editor-claro", frames.clone(), false, Some(id));
        snapshot_of(&mut app, "editor-oscuro", frames, true, None);
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

    #[test]
    fn notes() {
        let mut app = app_with(&["a = 0.5", "A = (1, 2)", "y = a x"]);
        let id = app.add_note(1.0, 2.0);
        let idx = app.items.iter().position(|it| it.id == id).unwrap();
        // Un texto no se interpreta como expresión aunque lo parezca.
        app.items[idx].text = "y = x^2, a = {a}, A = {A}, {2a + 1}, {z}".into();
        app.revision += 1;
        app.analyze();
        let Kind::Note(shown) = &app.kinds[idx] else { panic!() };
        assert_eq!(shown, "y = x^2, a = 0.5, A = (1, 2), 2, {?}");
        assert!(matches!(app.kinds[2], Kind::Curve(_)));
    }

    /// Un fotograma del lienzo sin ventana, con los eventos de ratón dados.
    fn canvas_frame(ctx: &egui::Context, app: &mut GraphApp, events: Vec<egui::Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            app.analyze();
            app.canvas(ui);
        });
        out.textures_delta.clear();
    }

    fn press(p: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn notes_on_canvas() {
        let ctx = egui::Context::default();
        let mut app = app_with(&["y = x"]);
        canvas_frame(&ctx, &mut app, vec![]);
        let rect = app.canvas_rect;

        // Herramienta T: el clic crea un texto en ese punto, que queda editándose.
        app.placing_note = true;
        let p = pos2(500.0, 200.0);
        canvas_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p), press(p, true)]);
        canvas_frame(&ctx, &mut app, vec![press(p, false)]);
        let note = app.items.last().unwrap();
        let id = note.id;
        assert!(!app.placing_note);
        assert_eq!(app.editing_note, Some(id));
        let n = note.note.unwrap();
        assert!(app.view.to_screen(rect, n.x, n.y).distance(p) < 1.0);

        // Escribir y terminar la edición.
        app.items.last_mut().unwrap().text = "hola".into();
        app.editing_note = None;
        app.revision += 1;
        canvas_frame(&ctx, &mut app, vec![]);

        // Arrastrar el texto lo mueve con el ratón.
        let start = p + vec2(10.0, 8.0);
        let mut evs = vec![egui::Event::PointerMoved(start), press(start, true)];
        canvas_frame(&ctx, &mut app, std::mem::take(&mut evs));
        for k in 1..=10 {
            canvas_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start + vec2(10.0 * k as f32, 5.0 * k as f32))]);
        }
        canvas_frame(&ctx, &mut app, vec![press(start + vec2(100.0, 50.0), false)]);
        let n = app.items.iter().find(|it| it.id == id).unwrap().note.unwrap();
        let moved = app.view.to_screen(rect, n.x, n.y);
        assert!(moved.x > p.x + 50.0 && moved.y > p.y + 20.0, "{moved:?}");
    }

    #[test]
    fn project_roundtrip() {
        let ctx = egui::Context::default();
        let mut app = app_with(&["a = 2", "y = a x"]);
        app.add_note(-1.0, 3.0);
        let id = app.items.last().unwrap().id;
        app.items.last_mut().unwrap().text = "pendiente {a}".into();
        app.view.cx = 4.0;
        assert!(app.is_dirty());
        let path = std::env::temp_dir().join("appgrafica-test.agraf");
        assert!(app.save_to(&ctx, path.clone()));
        assert!(!app.is_dirty());
        assert_eq!(app.recent.first(), Some(&path));

        let mut other = GraphApp::default();
        other.load_from(&path).unwrap();
        other.analyze();
        assert_eq!(other.items.len(), 3);
        assert_eq!(other.view.cx, 4.0);
        assert!(other.next_id > id);
        assert!(matches!(&other.kinds[2], Kind::Note(t) if t == "pendiente 2"));
        assert_eq!(other.items[2].note.map(|n| (n.x, n.y)), Some((-1.0, 3.0)));
        assert!(!other.is_dirty());
        other.items[0].text = "a = 3".into();
        assert!(other.is_dirty());
        // Un archivo que no es un proyecto da error sin tocar lo abierto.
        std::fs::write(&path, "hola").unwrap();
        assert!(other.load_from(&path).is_err());
        assert_eq!(other.items[0].text, "a = 3");
        let _ = std::fs::remove_file(path);

        // Exportar imagen: se recorta la zona de la gráfica de la captura de la ventana.
        let png = std::env::temp_dir().join("appgrafica-export.png");
        let window = egui::ColorImage::filled([300, 200], Color32::WHITE);
        other.canvas_rect = Rect::from_min_max(pos2(100.0, 0.0), pos2(400.0, 150.0));
        let slot = Arc::new(Mutex::new(Some(Arc::new(window))));
        other.capture = Some(Capture { target: CaptureTarget::File(png.clone()), slot });
        other.finish_capture(&ctx);
        assert!(other.capture.is_none());
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&png).unwrap()));
        let info = dec.read_info().unwrap().info().clone();
        assert_eq!((info.width, info.height), (200, 150));
        let _ = std::fs::remove_file(png);

        // Nuevo: una línea vacía y nada que guardar.
        other.new_project();
        assert_eq!(other.items.len(), 1);
        assert!(!other.is_dirty());
        assert!(other.project_path.is_none());
    }

    #[test]
    fn latex_lines() {
        let app = app_with(&[r"f(x) = \frac{x^2}{2}", r"y = f(x) + \sqrt{x}", r"\sum_{n=1}^{4} n", r"\frac{1}{2"]);
        assert!(matches!(app.kinds[0], Kind::Curve(Curve::Fx(_))));
        let Kind::Curve(Curve::Fx(c)) = &app.kinds[1] else { panic!("{:?}", app.items[1].text) };
        assert!((c.eval(4.0, 0.0) - 10.0).abs() < 1e-12);
        assert!(matches!(app.kinds[2], Kind::Value(v) if v == 10.0));
        assert!(matches!(app.kinds[3], Kind::Error { .. }));
    }
}
