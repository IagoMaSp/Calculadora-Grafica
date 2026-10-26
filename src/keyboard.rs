//! Teclado en pantalla: calculadora, funciones, letras, símbolos y letras griegas.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2, vec2};

/// Lo que pide el teclado al pulsar una tecla.
pub enum Action {
    /// Insertar texto en el cursor; luego mover el cursor `back` caracteres hacia atrás
    /// (para dejarlo dentro de los paréntesis de una función).
    Insert { text: String, back: usize },
    Backspace,
    Left,
    Right,
    /// Nueva expresión debajo (como Enter).
    Enter,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Default, Debug)]
pub enum Tab {
    #[default]
    Calc,
    Funcs,
    Letters,
    Symbols,
    Greek,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct KeyboardState {
    pub tab: Tab,
    /// Categoría elegida en la pestaña de funciones.
    func_group: usize,
    #[serde(skip)]
    shift: bool,
}

/// Una tecla: etiqueta, texto que inserta, retroceso del cursor y ayuda.
#[derive(Clone, Copy)]
struct K {
    label: &'static str,
    text: &'static str,
    back: usize,
    tip: &'static str,
}

/// Tecla que inserta su propia etiqueta.
const fn t(label: &'static str) -> K {
    K { label, text: label, back: 0, tip: "" }
}

/// Tecla con texto propio y retroceso del cursor.
const fn k(label: &'static str, text: &'static str, back: usize, tip: &'static str) -> K {
    K { label, text, back, tip }
}

/// Función de un argumento: inserta "nombre()" con el cursor dentro.
const fn f(name: &'static str, text: &'static str, tip: &'static str) -> K {
    K { label: name, text, back: 1, tip }
}

// Teclas especiales (no insertan texto).
const BACKSPACE: &str = "⌫";
const LEFT: &str = "◀";
const RIGHT: &str = "▶";
const ENTER: &str = "↵";

const CALC_LEFT: &[&[K]] = &[
    &[t("x"), t("y"), k("a²", "^2", 0, "al cuadrado"), k("aᵇ", "^", 0, "potencia"), f("√", "sqrt()", "raíz cuadrada"), k("|a|", "||", 1, "valor absoluto")],
    &[t("("), t(")"), t("<"), t(">"), k("≤", "<=", 0, "menor o igual"), k("≥", ">=", 0, "mayor o igual")],
    &[k("π", "pi", 0, "pi"), t("e"), k("i", "i", 0, "unidad imaginaria"), t(","), t("="), k("!", "!", 0, "factorial")],
    &[k("t", "t", 0, "parámetro de curvas paramétricas"), k("θ", "θ", 0, "ángulo en polares"), k("r", "r", 0, "radio en polares"), k("∞", "inf", 0, "infinito"), k("{ }", "{}", 1, "restricción {…}"), k("[ ]", "[]", 1, "lista […]")],
];

const CALC_RIGHT: &[&[K]] = &[
    &[t("7"), t("8"), t("9"), k("÷", "/", 0, "dividir"), k(BACKSPACE, "", 0, "borrar")],
    &[t("4"), t("5"), t("6"), k("×", "*", 0, "multiplicar"), k(LEFT, "", 0, "mover el cursor a la izquierda")],
    &[t("1"), t("2"), t("3"), k("−", "-", 0, "restar"), k(RIGHT, "", 0, "mover el cursor a la derecha")],
    &[t("0"), t("."), k("^", "^", 0, "potencia"), t("+"), k(ENTER, "", 0, "nueva expresión")],
];

const FUNC_GROUPS: &[(&str, &[K])] = &[
    ("Trigonométricas", &[
        f("sin", "sin()", "seno"), f("cos", "cos()", "coseno"), f("tan", "tan()", "tangente"),
        f("sec", "sec()", "secante"), f("csc", "csc()", "cosecante"), f("cot", "cot()", "cotangente"),
        f("arcsin", "arcsin()", "arcoseno"), f("arccos", "arccos()", "arcocoseno"), f("arctan", "arctan()", "arcotangente"),
        f("arcsec", "arcsec()", ""), f("arccsc", "arccsc()", ""), f("arccot", "arccot()", ""),
    ]),
    ("Hiperbólicas", &[
        f("sinh", "sinh()", "seno hiperbólico"), f("cosh", "cosh()", "coseno hiperbólico"), f("tanh", "tanh()", "tangente hiperbólica"),
        f("sech", "sech()", ""), f("csch", "csch()", ""), f("coth", "coth()", ""),
        f("arsinh", "arsinh()", ""), f("arcosh", "arcosh()", ""), f("artanh", "artanh()", ""),
    ]),
    ("Logaritmos", &[
        f("ln", "ln()", "logaritmo natural"), f("log", "log()", "logaritmo en base 10"),
        k("logₐ", "log(, )", 3, "logaritmo en base a: log(a, x)"), f("exp", "exp()", "exponencial"),
        k("eˣ", "e^()", 1, "e elevado a"), k("10ˣ", "10^()", 1, "10 elevado a"), k("ⁿ√", "root(, )", 3, "raíz n-ésima: root(n, x)"),
        f("√", "sqrt()", "raíz cuadrada"), f("∛", "cbrt()", "raíz cúbica"),
    ]),
    ("Cálculo", &[
        k("d/dx", "d/dx()", 1, "derivada respecto de x"), k("f′", "'", 0, "derivada de una función: f'(x)"),
        k("∫", "int(x, , 0, 1)", 7, "integral definida: int(variable, función, desde, hasta)"),
        k("∫dx", "int(x, )", 1, "primitiva (constante 0): int(x, f)"),
        k("∑", "sum(n, , 1, 10)", 8, "sumatoria: sum(variable, expresión, desde, hasta)"),
        k("∏", "prod(n, , 1, 10)", 8, "productoria: prod(variable, expresión, desde, hasta)"),
    ]),
    ("Combinatoria", &[
        k("n!", "!", 0, "factorial"), k("nCr", "nCr(, )", 3, "combinaciones"), k("nPr", "nPr(, )", 3, "permutaciones"),
        f("Γ", "gamma()", "función gamma"), k("B", "beta(, )", 3, "función beta"), k("mcd", "gcd(, )", 3, "máximo común divisor"),
        k("mcm", "lcm(, )", 3, "mínimo común múltiplo"),
    ]),
    ("Redondeo", &[
        f("floor", "floor()", "piso"), f("ceil", "ceil()", "techo"), f("round", "round()", "redondeo"), f("trunc", "trunc()", "parte entera"),
        f("abs", "abs()", "valor absoluto"), f("sign", "sign()", "signo"), k("mod", "mod(, )", 3, "resto: mod(a, b)"),
        k("min", "min(, )", 3, ""), k("max", "max(, )", 3, ""),
    ]),
    ("Especiales", &[
        f("W", "W()", "función W de Lambert"), k("W₋₁", "W(-1, )", 1, "rama −1 de la W de Lambert"),
        f("erf", "erf()", "función error"), f("erfc", "erfc()", "función error complementaria"), f("lnΓ", "lgamma()", "logaritmo de gamma"),
    ]),
    ("Complejos", &[
        f("re", "re()", "parte real"), f("im", "im()", "parte imaginaria"), f("arg", "arg()", "argumento"),
        f("conj", "conj()", "conjugado"), f("|z|", "abs()", "módulo"), t("i"),
    ]),
];

const LETTER_ROWS: &[&str] = &["qwertyuiop", "asdfghjklñ", "zxcvbnm"];

const SYMBOLS: &[K] = &[
    t("+"), k("−", "-", 0, ""), k("×", "*", 0, ""), k("÷", "/", 0, ""), t("^"), t("="), t("<"), t(">"), k("≤", "<=", 0, ""), k("≥", ">=", 0, ""),
    t("("), t(")"), t("["), t("]"), t("{"), t("}"), t("|"), t(","), t(";"), t("."),
    t("!"), k("′", "'", 0, "derivada: f'(x)"), k("…", "...", 0, "rango: [1...10]"), t("_"), t("·"), k("∞", "inf", 0, "infinito"),
    k("√", "sqrt()", 1, ""), k("∑", "sum(n, , 1, 10)", 8, ""), k("∏", "prod(n, , 1, 10)", 8, ""), k("∫", "int(x, , 0, 1)", 7, ""),
];

const GREEK_LOWER: &str = "αβγδεζηθικλμνξοπρστυφχψω";
const GREEK_UPPER: &str = "ΓΔΘΛΞΠΣΥΦΨΩ";

/// Aspecto de una tecla según lo que hace.
#[derive(Clone, Copy)]
enum Look {
    Normal,
    Number,
    Operator,
    Special,
    Accent,
    Danger,
}

fn look_of(label: &str) -> Look {
    match label {
        ENTER => Look::Accent,
        BACKSPACE => Look::Danger,
        LEFT | RIGHT => Look::Special,
        "+" | "−" | "×" | "÷" | "^" | "=" => Look::Operator,
        l if l.chars().all(|c| c.is_ascii_digit() || c == '.') => Look::Number,
        _ => Look::Normal,
    }
}

/// (fondo, texto) de cada tipo de tecla.
fn colors(look: Look, dark: bool) -> (Color32, Color32) {
    let text = if dark { Color32::from_gray(235) } else { Color32::from_gray(25) };
    match (look, dark) {
        (Look::Normal, false) => (Color32::from_gray(236), text),
        (Look::Normal, true) => (Color32::from_gray(52), text),
        (Look::Number, false) => (Color32::WHITE, text),
        (Look::Number, true) => (Color32::from_gray(72), text),
        (Look::Operator, false) => (Color32::from_rgb(218, 230, 246), text),
        (Look::Operator, true) => (Color32::from_rgb(38, 58, 88), text),
        (Look::Special, false) => (Color32::from_gray(214), text),
        (Look::Special, true) => (Color32::from_gray(40), text),
        (Look::Accent, _) => (Color32::from_rgb(45, 112, 179), Color32::WHITE),
        (Look::Danger, false) => (Color32::from_rgb(246, 222, 220), Color32::from_rgb(160, 40, 35)),
        (Look::Danger, true) => (Color32::from_rgb(90, 40, 38), Color32::from_rgb(255, 200, 195)),
    }
}

const GAP: f32 = 6.0;

/// Ancho de una fila de `n` teclas.
fn row_width(n: usize, key: Vec2) -> f32 {
    n as f32 * key.x + n.saturating_sub(1) as f32 * GAP
}

/// Contenido centrado horizontalmente, de ancho `width`.
fn centered(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        add(ui);
    });
}

/// Dibuja el teclado. Devuelve la acción de la tecla pulsada, si la hay.
pub fn keyboard_ui(ui: &mut egui::Ui, state: &mut KeyboardState, open: &mut bool) -> Option<Action> {
    let mut action = None;
    let dark = ui.visuals().dark_mode;

    // Pestañas centradas y el botón de cerrar a la derecha.
    ui.add_space(6.0);
    let tabs = [
        (Tab::Calc, "123  Calculadora"),
        (Tab::Funcs, "f(x)  Funciones"),
        (Tab::Letters, "abc  Letras"),
        (Tab::Symbols, "∑  Símbolos"),
        (Tab::Greek, "αβγ  Griegas"),
    ];
    let tab_w = 140.0;
    ui.horizontal(|ui| {
        let width = row_width(tabs.len(), vec2(tab_w, 0.0));
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = GAP;
        for (tab, label) in tabs {
            let look = if state.tab == tab { Look::Accent } else { Look::Normal };
            let (fill, text) = colors(look, dark);
            let b = egui::Button::new(RichText::new(label).size(14.5).color(text)).fill(fill).corner_radius(CornerRadius::same(14));
            if ui.add_sized([tab_w, 28.0], b).clicked() {
                state.tab = tab;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Ocultar el teclado (Ctrl+K)").clicked() {
                *open = false;
            }
        });
    });
    ui.add_space(8.0);

    let key = vec2(60.0, 40.0);
    // Tecla genérica: etiqueta, texto, retroceso del cursor, ayuda, tamaño.
    let mut press = |ui: &mut egui::Ui, label: &str, text: &str, back: usize, tip: &str, size: Vec2| {
        let (fill, color) = colors(look_of(label), dark);
        let border = if dark { Color32::from_gray(30) } else { Color32::from_gray(205) };
        let button = egui::Button::new(RichText::new(label).size(18.0).color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(8));
        let mut resp = ui.add_sized(size, button);
        if !tip.is_empty() {
            resp = resp.on_hover_text(tip);
        }
        if resp.clicked() {
            action = Some(match label {
                BACKSPACE => Action::Backspace,
                LEFT => Action::Left,
                RIGHT => Action::Right,
                ENTER => Action::Enter,
                _ => Action::Insert { text: text.to_string(), back },
            });
        }
    };

    match state.tab {
        Tab::Calc => {
            let gap = 28.0;
            let width = row_width(6, key) + gap + row_width(5, key);
            for (left, right) in CALC_LEFT.iter().zip(CALC_RIGHT) {
                centered(ui, width, |ui| {
                    for kk in *left {
                        press(ui, kk.label, kk.text, kk.back, kk.tip, key);
                    }
                    ui.add_space(gap - GAP);
                    for kk in *right {
                        press(ui, kk.label, kk.text, kk.back, kk.tip, key);
                    }
                });
            }
        }
        Tab::Funcs => {
            // Categorías a la izquierda; las teclas de la categoría, en cuadrícula, a la derecha.
            let cols = 6;
            let fkey = vec2(84.0, 40.0);
            let side = 170.0;
            let width = side + 20.0 + row_width(cols, fkey);
            let group = state.func_group.min(FUNC_GROUPS.len() - 1);
            centered(ui, width, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(side);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, (title, _)) in FUNC_GROUPS.iter().enumerate() {
                        if ui.add_sized([side, 24.0], egui::Button::selectable(group == i, *title)).clicked() {
                            state.func_group = i;
                        }
                    }
                });
                ui.add_space(20.0 - GAP);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                    for row in FUNC_GROUPS[group].1.chunks(cols) {
                        ui.horizontal(|ui| {
                            for kk in row {
                                press(ui, kk.label, kk.text, kk.back, kk.tip, fkey);
                            }
                        });
                    }
                });
            });
        }
        Tab::Letters => {
            for (r, row) in LETTER_ROWS.iter().enumerate() {
                let n = row.chars().count() + usize::from(r == 2);
                centered(ui, row_width(n, key), |ui| {
                    for c in row.chars() {
                        let s: String = if state.shift { c.to_uppercase().collect() } else { c.to_string() };
                        press(ui, &s, &s, 0, "", key);
                    }
                    if r == 2 {
                        press(ui, BACKSPACE, "", 0, "borrar", key);
                    }
                });
            }
            let space = vec2(key.x * 4.0 + GAP * 3.0, key.y);
            centered(ui, row_width(5, key) + GAP + space.x, |ui| {
                let look = if state.shift { Look::Accent } else { Look::Special };
                let (fill, color) = colors(look, dark);
                let shift = egui::Button::new(RichText::new("⇧").size(18.0).color(color)).fill(fill).corner_radius(CornerRadius::same(8));
                if ui.add_sized(key, shift).on_hover_text("Mayúsculas").clicked() {
                    state.shift = !state.shift;
                }
                press(ui, "_", "_", 0, "subíndice: a_1", key);
                press(ui, "espacio", " ", 0, "", space);
                press(ui, LEFT, "", 0, "", key);
                press(ui, RIGHT, "", 0, "", key);
                press(ui, ENTER, "", 0, "nueva expresión", key);
            });
        }
        Tab::Symbols => {
            let cols = 10;
            let mut keys: Vec<K> = SYMBOLS.to_vec();
            keys.push(k(BACKSPACE, "", 0, "borrar"));
            for row in keys.chunks(cols) {
                centered(ui, row_width(cols, key), |ui| {
                    for kk in row {
                        press(ui, kk.label, kk.text, kk.back, kk.tip, key);
                    }
                });
            }
        }
        Tab::Greek => {
            let cols = 12;
            let letters: Vec<char> = GREEK_LOWER.chars().chain(GREEK_UPPER.chars()).collect();
            for row in letters.chunks(cols) {
                centered(ui, row_width(cols, key), |ui| {
                    for &c in row {
                        let s = c.to_string();
                        let tip = match c {
                            'π' => "constante pi",
                            'τ' => "constante tau = 2π",
                            'θ' => "ángulo en coordenadas polares",
                            'Γ' => "función gamma: Γ(x)",
                            _ => "",
                        };
                        press(ui, &s, &s, 0, tip, key);
                    }
                });
            }
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Las letras griegas se pueden usar como nombres: α = 2 crea un deslizador.").weak());
            });
        }
    }
    action
}
