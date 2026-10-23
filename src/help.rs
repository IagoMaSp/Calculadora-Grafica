//! Guía de uso: ventana con secciones, explicaciones y ejemplos que se pueden probar.

use eframe::egui::{self, RichText, ViewportBuilder, ViewportId};

enum Block {
    /// Párrafo.
    P(&'static str),
    /// Subtítulo.
    H(&'static str),
    /// Ejemplo: expresión y explicación (con botón "Probar").
    Ex(&'static str, &'static str),
    /// Consejo destacado.
    Tip(&'static str),
    /// Tabla de dos columnas.
    Table(&'static [(&'static str, &'static str)]),
}

struct Section {
    title: &'static str,
    blocks: &'static [Block],
}

use Block::*;

const SECTIONS: &[Section] = &[
    Section {
        title: "Primeros pasos",
        blocks: &[
            P("AppGrafica es una calculadora gráfica: escribe expresiones en el panel de la izquierda y se dibujan al instante en el plano de la derecha."),
            H("El panel de expresiones"),
            P("Cada línea es una expresión. Pulsa Enter al final de una línea para crear otra debajo, o usa el botón ＋ de arriba. El botón ✕ borra la línea."),
            P("Si una expresión tiene un error, aparece un aviso en rojo debajo explicando qué falta o qué está mal."),
            P("Fuera de edición, las expresiones se muestran con escritura matemática: los exponentes arriba, las fracciones apiladas, las raíces con su signo… Haz clic sobre una para volver a editarla como texto."),
            P("El círculo de color a la izquierda de cada línea abre su estilo: color, tipo de trazo, grosor, opacidad y visibilidad."),
            H("Pruébalo"),
            Ex("y = x^2 - 2", "una parábola"),
            Ex("x^2 + y^2 = 9", "un círculo de radio 3"),
            Ex("a = 1", "un deslizador: muévelo y mira cómo cambian las gráficas que usan a"),
            Tip("Los botones \"Probar\" de esta guía añaden el ejemplo a tu lista de expresiones."),
            H("Atajos y trucos"),
            Table(&[
                ("Variable sin definir", "debajo del error aparece \"＋ a\": crea el deslizador con un clic"),
                ("Clic derecho en el círculo", "duplicar, subir, bajar o eliminar la línea"),
                ("↶ o Ctrl+Z", "deshacer el borrado de una línea"),
                ("F1", "abrir esta guía"),
                ("Ctrl+K", "mostrar u ocultar el teclado en pantalla"),
                ("Enter", "nueva línea debajo"),
            ]),
        ],
    },
    Section {
        title: "Funciones y ecuaciones",
        blocks: &[
            H("Funciones de x"),
            P("Escribe y = … o directamente la expresión en x."),
            Ex("y = sin(x)", "función seno"),
            Ex("x^3 - 2x", "sin \"y =\" también se grafica como y = x³ − 2x"),
            H("Funciones de y"),
            Ex("x = y^2 - 4", "parábola horizontal"),
            H("Curvas implícitas"),
            P("Cualquier ecuación con x e y mezcladas se dibuja como curva implícita."),
            Ex("x^2/9 + y^2/4 = 1", "elipse"),
            Ex("x y = 1", "hipérbola"),
            H("Funciones con nombre"),
            P("Define una función una vez y úsala en otras líneas. Si tiene un solo parámetro, también se grafica."),
            Ex("f(x) = x^2 - 1", "define y grafica f"),
            Ex("y = f(x - 2) + 1", "f trasladada"),
            Ex("g(a, b) = a^2 + b^2", "funciones de varias variables (no se grafican, pero se pueden usar)"),
            H("Calcular valores"),
            P("Una expresión sin x se evalúa y muestra su resultado."),
            Ex("2^10 + 3!", "= 1030"),
            Ex("sqrt(2) pi", "= 4.442883"),
        ],
    },
    Section {
        title: "Cómo escribir",
        blocks: &[
            H("Reglas de escritura"),
            Table(&[
                ("2x, 3(x+1), (x+1)(x-1)", "multiplicación implícita"),
                ("x^2, 2^-x, e^(2x)", "potencias"),
                ("|x - 1|", "valor absoluto"),
                ("5!", "factorial (también con decimales)"),
                ("sin x, sin 2x", "funciones sin paréntesis"),
                ("sin^2(x)", "(sin x)²"),
                ("log(2, 8)", "logaritmo en base 2"),
                ("x^(1/3)", "raíz cúbica (también de negativos)"),
                ("a_1, a_2", "nombres con subíndice"),
            ]),
            H("Constantes"),
            Table(&[("pi, π", "3.14159…"), ("e", "2.71828…"), ("tau, τ", "2π"), ("i", "unidad imaginaria")]),
            H("Funciones disponibles"),
            Table(&[
                ("sin cos tan sec csc cot", "trigonométricas (sen también vale)"),
                ("arcsin arccos arctan arcsec arccsc arccot", "inversas (también asin, acos…)"),
                ("sinh cosh tanh sech csch coth", "hiperbólicas"),
                ("arsinh arcosh artanh", "hiperbólicas inversas (también asinh…)"),
                ("sqrt cbrt root(n, x)", "raíces"),
                ("ln log log(b, x) exp", "logaritmos y exponencial"),
                ("floor ceil round trunc", "piso, techo, redondeo, parte entera"),
                ("abs sign mod(a, b)", "valor absoluto, signo, resto"),
                ("min max gcd lcm (mcd mcm)", "dos argumentos"),
                ("nCr(n, r) nPr(n, r) n!", "combinaciones, permutaciones, factorial"),
                ("gamma(x) o Γ(x), lgamma, beta(a, b)", "funciones gamma y beta"),
                ("W(x), W(-1, x)", "función W de Lambert (ramas 0 y −1)"),
                ("erf erfc", "función error"),
                ("atan2(y, x)", "ángulo de un punto"),
                ("re im arg conj", "números complejos"),
            ]),
            Tip("Cualquier letra puede ser un deslizador, también t (si no la defines, t es el parámetro de las paramétricas). r y θ (theta) se reservan para las polares."),
            Tip("Los paréntesis que agrupan se cierran solos al final: (x + 2 funciona. Los de una función sí hay que cerrarlos: sin(x da error."),
            Tip("La letra i es la unidad imaginaria (mira la sección Números complejos)."),
        ],
    },
    Section {
        title: "Cálculo",
        blocks: &[
            H("Sumatorias y productorias"),
            P("Se escriben sum(variable, expresión, desde, hasta) y prod(…). También ∑ y ∏."),
            Ex("sum(n, n, 1, 100)", "= 5050"),
            Ex("prod(n, n, 1, 5)", "= 120"),
            Ex("y = sum(n, sin(n x)/n, 1, 10)", "una serie de Fourier"),
            Ex("y = sum(n, x^n/n!, 0, 8)", "polinomio de Taylor de eˣ"),
            H("Integrales"),
            P("int(variable, función, desde, hasta), o ∫. Los límites pueden depender de x, y pueden ser infinitos (inf o ∞)."),
            Ex("int(x, x^2, 0, 1)", "= 1/3"),
            Ex("int(u, e^(-u^2), -inf, inf)", "= √π"),
            Ex("y = int(u, e^(-u^2), 0, x)", "la integral como función del límite superior"),
            H("Primitivas"),
            P("Sin límites, int(x, función) es la primitiva con constante 0 (es decir, la integral desde 0 hasta x). Puedes poner directamente el nombre de una función."),
            Ex("f(x) = cos(x)", "una función…"),
            Ex("y = int(x, f)", "…su primitiva: sin(x)"),
            Ex("y = int(x, x^2)", "x³/3"),
            H("Derivadas"),
            P("d/dx(expresión) deriva respecto de x. Para una función con nombre se puede usar f'(x), f''(x)…"),
            Ex("f(x) = x^3 - 3x", "una función…"),
            Ex("y = f'(x)", "…su derivada"),
            Ex("y = f''(x)", "…y su segunda derivada"),
            Ex("y = d/dx(sin(x) x)", "derivada de una expresión"),
            Ex("f'(2)", "derivada en un punto: = 9"),
            Tip("Las derivadas e integrales se calculan numéricamente con mucha precisión (unos 10 decimales correctos)."),
        ],
    },
    Section {
        title: "Desigualdades",
        blocks: &[
            P("Usa <, >, <=, >=, ≤ o ≥ para sombrear la región que cumple la condición."),
            Ex("y < 2x + 1", "debajo de una recta"),
            Ex("y >= x^2 - 4", "encima de una parábola"),
            Ex("x^2 + y^2 < 9", "interior de un círculo"),
            Ex("x > 2", "a la derecha de x = 2"),
            Tip("Con < y > (estrictas) el borde se dibuja segmentado; con ≤ y ≥ el borde es sólido."),
            P("La opacidad del sombreado se cambia en el estilo de la expresión (círculo de color → Relleno)."),
        ],
    },
    Section {
        title: "Restricciones",
        blocks: &[
            P("Añade condiciones entre llaves al final de una expresión para dibujar solo la parte que las cumple."),
            Ex("y = x^2 {x > 2; y < 12}", "varias condiciones separadas por ;"),
            Ex("y = sin(x) {0 < x < 2pi}", "desigualdad doble"),
            Ex("x^2 + y^2 = 16 {y > 0}", "semicírculo"),
            Ex("y < x {x > -2}{x < 3}", "también funciona en regiones y con varios grupos {…}{…}"),
            Ex("f(x) = sqrt(x) {x < 6}", "en una función: la restricción forma parte de f"),
            Tip("En las curvas y = f(x), las condiciones sobre y usan la altura de la curva."),
        ],
    },
    Section {
        title: "Puntos y listas",
        blocks: &[
            H("Puntos"),
            Ex("(2, 1)", "un punto"),
            Ex("A = (1, 2)", "un punto con nombre: se etiqueta como A"),
            Ex("B = (A.x + 2, A.y)", "usa las coordenadas de otro punto con .x y .y"),
            H("Listas de puntos"),
            Ex("[(0, 0), (2, 1), (3, 3)]", "varios puntos en una línea"),
            Ex("P = [(0, 0), (4, 0), (2, 3), (0, 0)]", "lista con nombre; activa \"Unir los puntos con líneas\" en su estilo para dibujar el triángulo"),
            H("Listas de números y rangos"),
            Ex("L = [1, 2, 3, 4]", "lista de números"),
            Ex("[1...10]", "rango del 1 al 10"),
            Ex("[0, 0.5 ... 3]", "rango con paso 0.5 (el paso es la diferencia entre los dos primeros)"),
            H("Generar puntos"),
            Ex("(n, n^2) for n = [-3...3]", "un punto por cada valor de n"),
            Ex("(L, L^2)", "con una lista con nombre se evalúa elemento a elemento"),
            Ex("([1...5], 0)", "una coordenada fija se repite"),
            H("Etiquetas"),
            P("En el estilo de un punto (círculo de color) puedes mostrar u ocultar su etiqueta, cambiar su texto y añadir sus coordenadas."),
        ],
    },
    Section {
        title: "Puntos móviles",
        blocks: &[
            P("Los puntos se pueden arrastrar con el ratón. Al pasar por encima de un punto móvil el cursor cambia a una mano."),
            Ex("A = (1, 2)", "si las coordenadas son números, al arrastrar se reescriben"),
            Ex("a = 1", "un deslizador…"),
            Ex("(a, a^2)", "…si una coordenada es un deslizador, al arrastrar se mueve el deslizador"),
            Ex("(2, b)", "una coordenada fija y otra móvil: el punto se mueve solo en vertical (crea antes el deslizador b)"),
            Tip("Mientras arrastras, el punto se ajusta a la cuadrícula si está cerca de una línea."),
            H("Dejar rastro"),
            P("En el estilo de un punto activa \"Dejar rastro al moverse\": el punto irá dibujando su camino cada vez que cambie, por ejemplo al animar un deslizador. Así puedes ver cómo se genera una figura."),
            Ex("k = 0", "deslizador (pon el rango de 0 a 6.28 y dale a ▶)"),
            Ex("(cos(3k), sin(2k))", "punto que depende de k: activa su rastro y anima k"),
        ],
    },
    Section {
        title: "Paramétricas y polares",
        blocks: &[
            H("Curvas paramétricas"),
            P("Escribe un punto cuyas coordenadas dependan de t."),
            Ex("(cos(t), sin(2t))", "curva de Lissajous"),
            Ex("(t, t^2) {-2 < t < 2}", "el rango de t también se puede restringir con {…}"),
            H("Curvas polares"),
            P("Escribe r = f(θ). Puedes escribir θ o theta."),
            Ex("r = 1 + cos(theta)", "cardioide"),
            Ex("r = theta / 4", "espiral (pon el rango 0–12π)"),
            Ex("r < 2 + sin(3theta)", "región polar"),
            P("Debajo de cada curva paramétrica o polar aparece su rango, por ejemplo 0 ≤ t ≤ 6.28. Edítalo o usa los atajos 0–π, 0–2π y 0–12π."),
            H("Animar el dibujo"),
            P("Pulsa ▶ Dibujar bajo una curva paramétrica o polar para verla dibujarse poco a poco, con un punto que marca la posición actual. Puedes pausar, mover la barra para ir a cualquier momento, elegir la velocidad y activar ↻ para repetir. El botón ■ vuelve a mostrar la curva completa."),
        ],
    },
    Section {
        title: "Números complejos",
        blocks: &[
            P("La letra i es la unidad imaginaria (i² = −1). Las operaciones, potencias, raíces, exponenciales, logaritmos y funciones trigonométricas funcionan también con números complejos."),
            H("Calcular"),
            Ex("sqrt(-1)", "= i"),
            Ex("(1 + 2i)(3 - i)", "= 5 + 5i"),
            Ex("e^(i pi) + 1", "= 0 (identidad de Euler)"),
            Ex("i^i", "= 0.20788 (un número real)"),
            Ex("ln(-1)", "= πi"),
            Ex("sqrt(-4) + 2", "= 2 + 2i"),
            P("Debajo del resultado se muestran también su módulo |z| y su argumento (en radianes)."),
            H("Funciones para complejos"),
            Table(&[
                ("re(z), im(z)", "parte real e imaginaria (también real(z), imag(z))"),
                ("abs(z), |z|", "módulo"),
                ("arg(z)", "argumento, entre −π y π"),
                ("conj(z)", "conjugado"),
                ("sign(z)", "z / |z|"),
            ]),
            H("En el plano complejo"),
            P("Los números complejos se dibujan como puntos: a + bi está en (a, b). Con nombre llevan etiqueta."),
            Ex("z = 1 + 2i", "un complejo con nombre"),
            Ex("w = z^2", "se puede operar con él"),
            Ex("[1, i, -1, -i]", "una lista de complejos son varios puntos"),
            Ex("e^(i 2pi n/6) for n = [0...5]", "las raíces sextas de la unidad"),
            H("Curvas complejas"),
            P("Una expresión compleja que depende de t se dibuja como la curva que recorre en el plano complejo."),
            Ex("e^(i t)", "la circunferencia unidad"),
            Ex("e^((0.1 + i) t)", "espiral (prueba el rango 0–12π)"),
            H("En las gráficas"),
            P("Las gráficas de x e y son reales. Para usar complejos en ellas, extrae un número real con re, im, abs o arg."),
            Ex("y = re(sqrt(x))", "vale 0 para x < 0, donde √x es imaginaria"),
            Ex("y = abs(e^(i x) + 1)", "módulo de una expresión compleja"),
            Tip("Si defines una variable llamada i (por ejemplo en (i, i^2) for i = [1...5]), esa i es la tuya y no la unidad imaginaria."),
        ],
    },
    Section {
        title: "Deslizadores y animación",
        blocks: &[
            P("Una línea del tipo nombre = número crea un deslizador. Cualquier expresión que use ese nombre se actualiza al moverlo."),
            Ex("a = 2", "deslizador a"),
            Ex("y = a sin(x)", "usa a"),
            H("Animar"),
            P("Pulsa ▶ junto al deslizador para que se mueva solo, y ⏸ para pausarlo."),
            H("Ajustes del deslizador (⚙)"),
            Table(&[
                ("Rango", "valores mínimo y máximo"),
                ("Paso", "cada cuánto salta: 0.01, 0.1, 1, cualquier valor, o \"libre\" (continuo)"),
                ("Velocidad", "0.25×, 0.5×, 1×, 1.5× o 2× (a 1× recorre el rango en 8 s)"),
                ("Modo", "↻ bucle, ⇄ rebote (va y vuelve) o → una vez"),
            ]),
        ],
    },
    Section {
        title: "Estilo",
        blocks: &[
            P("Haz clic en el círculo de color de una expresión para abrir su editor de estilo."),
            Table(&[
                ("Visible", "mostrar u ocultar la gráfica (oculta, el círculo se ve hueco)"),
                ("Color", "paleta o color personalizado"),
                ("Trazo", "sólido, segmentado o punteado"),
                ("Grosor y opacidad", "de la línea"),
                ("Relleno", "opacidad del sombreado de las desigualdades"),
                ("Punto", "relleno, hueco o cruz, y su tamaño"),
                ("Unir los puntos", "en las listas, dibuja líneas entre puntos consecutivos"),
                ("Etiqueta", "texto y coordenadas junto a los puntos"),
            ]),
        ],
    },
    Section {
        title: "Puntos notables",
        blocks: &[
            P("Haz clic en una curva (o en su expresión) para seleccionarla. Aparecen puntos grises en sus lugares importantes:"),
            Table(&[
                ("Cortes con los ejes", "raíces y corte con el eje y"),
                ("Máximos y mínimos", "extremos locales"),
                ("Intersecciones", "con todas las demás curvas visibles"),
            ]),
            P("Pasa el ratón por encima de un punto gris para ver qué es y sus coordenadas. Haz clic en el fondo para quitar la selección."),
            Tip("Pasar el ratón cerca de cualquier curva muestra las coordenadas del punto de la curva."),
        ],
    },
    Section {
        title: "Teclado",
        blocks: &[
            P("El botón ⌨ (arriba del panel) muestra un teclado en pantalla. Cada tecla escribe en la expresión donde estaba el cursor; las funciones dejan el cursor dentro de los paréntesis."),
            Table(&[
                ("123 Calculadora", "números, operaciones, x, y, π, e, i, potencias y raíces; ⌫ borra, ◀ ▶ mueven el cursor, ↵ crea una línea nueva"),
                ("f(x) Funciones", "trigonométricas, hiperbólicas, logaritmos, cálculo, combinatoria, redondeo, especiales y complejos"),
                ("abc Letras", "teclado de letras con ⇧ para mayúsculas"),
                ("∑ Símbolos", "comparaciones, paréntesis, corchetes, llaves, ∑ ∏ ∫ √ ∞…"),
                ("αβγ Griegas", "letras griegas para usarlas como nombres (α = 2)"),
            ]),
            Tip("Pasa el ratón sobre una tecla para ver qué hace. Ctrl+K muestra u oculta el teclado."),
        ],
    },
    Section {
        title: "Navegación y ajustes",
        blocks: &[
            H("Moverse por el plano"),
            Table(&[
                ("Arrastrar el fondo", "desplazar la vista"),
                ("Rueda del ratón / pellizco", "acercar o alejar hacia el cursor"),
                ("Botones ＋ − ⌂", "acercar, alejar y volver a la vista inicial"),
            ]),
            H("Ajustes (⚙ arriba del panel)"),
            Table(&[
                ("Tema", "claro, oscuro o el del sistema"),
                ("Texto de expresiones", "tamaño de letra del panel"),
                ("Fuente monoespaciada", "letra de ancho fijo para las expresiones"),
                ("Números de los ejes", "tamaño de las etiquetas de la cuadrícula"),
                ("Puntos notables", "activarlos o desactivarlos"),
                ("Escritura matemática", "mostrar las expresiones con formato matemático o como texto"),
            ]),
            P("Tus expresiones, estilos y ajustes se guardan automáticamente al cerrar la aplicación."),
        ],
    },
];

/// Muestra la ventana de ayuda si `open`. Devuelve las expresiones que el usuario pidió probar.
pub fn help_window(ctx: &egui::Context, open: &mut bool, section: &mut usize) -> Vec<String> {
    let mut to_add = Vec::new();
    if !*open {
        return to_add;
    }
    let builder = ViewportBuilder::default()
        .with_title("Guía de uso — AppGrafica")
        .with_inner_size([860.0, 620.0])
        .with_min_inner_size([520.0, 360.0]);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("guia-de-uso"), builder, |ui, _class| {
        if ui.input(|i| i.viewport().close_requested()) {
            *open = false;
        }
        egui::Panel::left("secciones").resizable(false).exact_size(210.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading("Guía de uso");
            ui.add_space(6.0);
            for (i, s) in SECTIONS.iter().enumerate() {
                let label = RichText::new(s.title).size(15.0);
                if ui.add_sized([ui.available_width(), 28.0], egui::Button::selectable(*section == i, label)).clicked() {
                    *section = i;
                }
            }
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let s = &SECTIONS[(*section).min(SECTIONS.len() - 1)];
            egui::ScrollArea::vertical().id_salt(*section).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(620.0);
                ui.add_space(4.0);
                ui.label(RichText::new(s.title).size(26.0).strong());
                ui.add_space(8.0);
                for b in s.blocks {
                    block_ui(ui, b, &mut to_add);
                }
                ui.add_space(16.0);
            });
        });
    });
    to_add
}

fn block_ui(ui: &mut egui::Ui, b: &Block, to_add: &mut Vec<String>) {
    match b {
        P(text) => {
            ui.label(RichText::new(*text).size(15.0));
            ui.add_space(6.0);
        }
        H(text) => {
            ui.add_space(8.0);
            ui.label(RichText::new(*text).size(18.0).strong());
            ui.add_space(4.0);
        }
        Tip(text) => {
            egui::Frame::new()
                .fill(ui.visuals().selection.bg_fill.gamma_multiply(0.25))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("Consejo: {text}")).size(14.5));
                });
            ui.add_space(6.0);
        }
        Ex(expr, desc) => {
            egui::Frame::group(ui.style()).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let button_w = 70.0;
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - button_w).max(100.0));
                        ui.label(RichText::new(*expr).monospace().size(15.5).strong());
                        ui.label(RichText::new(*desc).size(14.0).weak());
                    });
                    if ui.button("Probar").on_hover_text("Añadir a la lista de expresiones").clicked() {
                        to_add.push(expr.to_string());
                    }
                });
            });
            ui.add_space(4.0);
        }
        Table(rows) => {
            egui::Grid::new(rows.as_ptr()).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                for (a, b) in rows.iter() {
                    ui.label(RichText::new(*a).monospace().size(14.0));
                    ui.label(RichText::new(*b).size(14.0));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
        }
    }
}
