//! Teclado en pantalla: calculadora, funciones, letras, símbolos y letras griegas.

use eframe::egui::{self, RichText, vec2};

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
const SHIFT: &str = "⇧";

const CALC_LEFT: &[&[K]] = &[
    &[t("x"), t("y"), k("a²", "^2", 0, "al cuadrado"), k("aᵇ", "^", 0, "potencia"), f("√", "sqrt()", "raíz cuadrada"), k("|a|", "||", 1, "valor absoluto")],
    &[t("("), t(")"), t("<"), t(">"), k("≤", "<=", 0, "menor o igual"), k("≥", ">=", 0, "mayor o igual")],
    &[k("π", "pi", 0, "pi"), t("e"), k("i", "i", 0, "unidad imaginaria"), t(","), t("="), k("!", "!", 0, "factorial")],
    &[k("t", "t", 0, "parámetro de curvas paramétricas"), k("θ", "θ", 0, "ángulo en polares"), k("r", "r", 0, "radio en polares"), k("∞", "inf", 0, "infinito"), k("{", "{}", 1, "restricción {…}"), k("[", "[]", 1, "lista […]")],
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
    ("Logaritmos y exponencial", &[
        f("ln", "ln()", "logaritmo natural"), f("log", "log()", "logaritmo en base 10"),
        k("logₐ", "log(, )", 3, "logaritmo en base a: log(a, x)"), f("exp", "exp()", "exponencial"),
        k("eˣ", "e^()", 1, "e elevado a"), k("10ˣ", "10^()", 1, "10 elevado a"), k("ⁿ√", "root(, )", 3, "raíz n-ésima: root(n, x)"),
    ]),
    ("Cálculo", &[
        k("d/dx", "d/dx()", 1, "derivada respecto de x"), k("f′", "'", 0, "derivada de una función: f'(x)"),
        k("∑", "sum(n, 1, 10, )", 1, "sumatoria: sum(variable, desde, hasta, expresión)"),
        k("∏", "prod(n, 1, 10, )", 1, "productoria: prod(variable, desde, hasta, expresión)"),
        k("∫", "int(u, 0, 1, )", 1, "integral: int(variable, desde, hasta, expresión)"),
    ]),
    ("Combinatoria", &[
        k("n!", "!", 0, "factorial"), k("nCr", "nCr(, )", 3, "combinaciones"), k("nPr", "nPr(, )", 3, "permutaciones"),
        f("Γ", "gamma()", "función gamma"), k("B", "beta(, )", 3, "función beta"),
    ]),
    ("Redondeo y aritmética", &[
        f("floor", "floor()", "piso"), f("ceil", "ceil()", "techo"), f("round", "round()", "redondeo"), f("trunc", "trunc()", "parte entera"),
        f("abs", "abs()", "valor absoluto"), f("sign", "sign()", "signo"), k("mod", "mod(, )", 3, "resto: mod(a, b)"),
        k("min", "min(, )", 3, ""), k("max", "max(, )", 3, ""), k("mcd", "gcd(, )", 3, "máximo común divisor"), k("mcm", "lcm(, )", 3, "mínimo común múltiplo"),
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
    t("+"), k("−", "-", 0, ""), k("×", "*", 0, ""), k("÷", "/", 0, ""), t("^"), t("="), t("<"), t(">"),
    k("≤", "<=", 0, ""), k("≥", ">=", 0, ""), t("("), t(")"), t("["), t("]"), t("{"), t("}"),
    t("|"), t(","), t(";"), t("."), t("!"), k("′", "'", 0, "derivada: f'(x)"), k("…", "...", 0, "rango: [1...10]"), t("_"),
    k("∞", "inf", 0, "infinito"), k("√", "sqrt()", 1, ""), k("∑", "sum(n, 1, 10, )", 1, ""), k("∏", "prod(n, 1, 10, )", 1, ""),
    k("∫", "int(u, 0, 1, )", 1, ""), k("π", "pi", 0, ""), k("τ", "tau", 0, "2π"), t("·"),
];

const GREEK_LOWER: &str = "αβγδεζηθικλμνξοπρστυφχψω";
const GREEK_UPPER: &str = "ΓΔΘΛΞΠΣΥΦΨΩ";

/// Dibuja el teclado. Devuelve la acción de la tecla pulsada, si la hay.
pub fn keyboard_ui(ui: &mut egui::Ui, state: &mut KeyboardState, open: &mut bool) -> Option<Action> {
    let mut action = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (tab, label) in [
            (Tab::Calc, "123  Calculadora"),
            (Tab::Funcs, "f(x)  Funciones"),
            (Tab::Letters, "abc  Letras"),
            (Tab::Symbols, "∑  Símbolos"),
            (Tab::Greek, "αβγ  Griegas"),
        ] {
            ui.selectable_value(&mut state.tab, tab, RichText::new(label).size(15.0));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Ocultar el teclado").clicked() {
                *open = false;
            }
        });
    });
    ui.separator();

    let key_size = vec2(54.0, 36.0);
    // Tecla genérica (etiqueta, texto, retroceso del cursor, ayuda).
    let mut raw = |ui: &mut egui::Ui, label: &str, text: &str, back: usize, tip: &str, size: egui::Vec2| {
        let mut resp = ui.add_sized(size, egui::Button::new(RichText::new(label).size(17.0)));
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

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match state.tab {
        Tab::Calc => {
            ui.horizontal_top(|ui| {
                grid(ui, "calc-left", CALC_LEFT, key_size, &mut |ui, kk, sz| raw(ui, kk.label, kk.text, kk.back, kk.tip, sz));
                ui.add_space(24.0);
                grid(ui, "calc-right", CALC_RIGHT, key_size, &mut |ui, kk, sz| raw(ui, kk.label, kk.text, kk.back, kk.tip, sz));
            });
        }
        Tab::Funcs => {
            for (title, keys) in FUNC_GROUPS {
                ui.horizontal_wrapped(|ui| {
                    ui.add_sized([150.0, 32.0], egui::Label::new(RichText::new(*title).strong()));
                    for kk in *keys {
                        raw(ui, kk.label, kk.text, kk.back, kk.tip, vec2(66.0, 32.0));
                    }
                });
            }
        }
        Tab::Letters => {
            for (r, row) in LETTER_ROWS.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(r as f32 * 20.0);
                    for c in row.chars() {
                        let s: String = if state.shift { c.to_uppercase().collect() } else { c.to_string() };
                        raw(ui, &s, &s, 0, "", key_size);
                    }
                    if r == 2 {
                        raw(ui, BACKSPACE, "", 0, "borrar", key_size);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.add_sized(key_size, egui::Button::new(RichText::new(SHIFT).size(17.0)).selected(state.shift)).on_hover_text("Mayúsculas").clicked() {
                    state.shift = !state.shift;
                }
                raw(ui, "_", "_", 0, "subíndice: a_1", key_size);
                raw(ui, "espacio", " ", 0, "", vec2(key_size.x * 4.0, key_size.y));
                raw(ui, LEFT, "", 0, "", key_size);
                raw(ui, RIGHT, "", 0, "", key_size);
                raw(ui, ENTER, "", 0, "nueva expresión", key_size);
            });
        }
        Tab::Symbols => {
            ui.horizontal_wrapped(|ui| {
                for kk in SYMBOLS {
                    raw(ui, kk.label, kk.text, kk.back, kk.tip, key_size);
                }
                raw(ui, BACKSPACE, "", 0, "borrar", key_size);
            });
        }
        Tab::Greek => {
            for letters in [GREEK_LOWER, GREEK_UPPER] {
                ui.horizontal_wrapped(|ui| {
                    for c in letters.chars() {
                        let s = c.to_string();
                        let tip = match c {
                            'π' => "constante pi",
                            'τ' => "constante tau = 2π",
                            'θ' => "ángulo en coordenadas polares",
                            'Γ' => "función gamma: Γ(x)",
                            _ => "",
                        };
                        raw(ui, &s, &s, 0, tip, key_size);
                    }
                });
            }
            ui.label(RichText::new("Las letras griegas se pueden usar como nombres: α = 2 crea un deslizador.").weak());
        }
    });
    action
}

fn grid(ui: &mut egui::Ui, id: &str, rows: &[&[K]], size: egui::Vec2, key: &mut dyn FnMut(&mut egui::Ui, &K, egui::Vec2)) {
    egui::Grid::new(id).spacing([4.0, 4.0]).show(ui, |ui| {
        for row in rows {
            for kk in *row {
                key(ui, kk, size);
            }
            ui.end_row();
        }
    });
}
