//! Editor matemático visual (al estilo de Desmos): se escribe directamente sobre la
//! notación. `/` crea una fracción, `^` un exponente, `sqrt` una raíz, `sum` una sumatoria
//! con sus límites como casillas… El árbol se traduce al texto que entiende el analizador
//! (y al revés), así que el texto de cada línea sigue siendo la fuente de verdad.

use eframe::egui::{self, Color32, Event, Key, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::math;
use crate::pretty::{Composer, Layout, Prim, axis};

// ───────────────────────────── Modelo ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Delim {
    Paren,
    Bracket,
    Brace,
    Abs,
}

impl Delim {
    fn chars(self) -> (char, char) {
        match self {
            Delim::Paren => ('(', ')'),
            Delim::Bracket => ('[', ']'),
            Delim::Brace => ('{', '}'),
            Delim::Abs => ('|', '|'),
        }
    }

    fn of_open(c: char) -> Option<Self> {
        match c {
            '(' => Some(Delim::Paren),
            '[' => Some(Delim::Bracket),
            '{' => Some(Delim::Brace),
            '|' => Some(Delim::Abs),
            _ => None,
        }
    }

    fn of_close(c: char) -> Option<Self> {
        match c {
            ')' => Some(Delim::Paren),
            ']' => Some(Delim::Bracket),
            '}' => Some(Delim::Brace),
            _ => None,
        }
    }
}

/// Estructuras con casillas. Las casillas (filas) de cada una, en orden de lectura:
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// numerador, denominador
    Frac,
    /// exponente (del elemento anterior)
    Sup,
    /// subíndice de un nombre: a₁
    Sub,
    Sqrt,
    /// índice, radicando
    Root,
    Delim(Delim),
    /// límite inferior ("n=1"), superior, cuerpo
    Sum,
    Prod,
    /// límite inferior, superior, cuerpo, variable (la x de "dx")
    Int,
    /// d/dx( cuerpo )
    Deriv,
}

impl Kind {
    fn arity(self) -> usize {
        match self {
            Kind::Frac | Kind::Root => 2,
            Kind::Sum | Kind::Prod => 3,
            Kind::Int => 4,
            _ => 1,
        }
    }

    /// Casilla a la que lleva la flecha ↑ desde la casilla `child`.
    fn above(self, child: usize) -> Option<usize> {
        match (self, child) {
            (Kind::Frac | Kind::Root, 1) => Some(0),
            (Kind::Sum | Kind::Prod, 0 | 2) => Some(1),
            (Kind::Int, 0 | 2 | 3) => Some(1),
            _ => None,
        }
    }

    fn below(self, child: usize) -> Option<usize> {
        match (self, child) {
            (Kind::Frac | Kind::Root, 0) => Some(1),
            (Kind::Sum | Kind::Prod, 1 | 2) => Some(0),
            (Kind::Int, 1 | 2 | 3) => Some(0),
            _ => None,
        }
    }

    /// Al borrar hacia atrás desde el principio de la casilla, ¿se deshace la estructura
    /// dejando su contenido? (Si no, el cursor solo retrocede.)
    fn unwraps(self) -> bool {
        matches!(self, Kind::Frac | Kind::Sup | Kind::Sub | Kind::Sqrt | Kind::Delim(_) | Kind::Deriv)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Atom {
    Ch(char),
    Node(Kind, Vec<Row>),
}

pub type Row = Vec<Atom>;

fn node(kind: Kind, rows: Vec<Row>) -> Atom {
    Atom::Node(kind, rows)
}

fn empty_node(kind: Kind) -> Atom {
    Atom::Node(kind, vec![Vec::new(); kind.arity()])
}

fn chars(s: &str) -> Row {
    s.chars().map(Atom::Ch).collect()
}

/// Camino hasta una fila: por cada nivel, (índice del átomo, índice de su casilla).
type Path = Vec<(usize, usize)>;

#[derive(Clone, PartialEq, Debug, Default)]
struct Cursor {
    path: Path,
    pos: usize,
}

fn row_at<'a>(root: &'a Row, path: &[(usize, usize)]) -> &'a Row {
    let mut r = root;
    for &(a, c) in path {
        match &r[a] {
            Atom::Node(_, rows) => r = &rows[c],
            Atom::Ch(_) => unreachable!("camino inválido"),
        }
    }
    r
}

fn row_at_mut<'a>(root: &'a mut Row, path: &[(usize, usize)]) -> &'a mut Row {
    let mut r = root;
    for &(a, c) in path {
        match &mut r[a] {
            Atom::Node(_, rows) => r = &mut rows[c],
            Atom::Ch(_) => unreachable!("camino inválido"),
        }
    }
    r
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic() && !matches!(c, 'π' | '∞')
}

/// Caracteres que forman parte de un operando (para saber qué va al numerador).
fn is_operand_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '!' | '\'' | '′' | '_' | '∞')
}

/// Inicio del operando que termina en `end` (lo que pasa al numerador al escribir `/`).
fn operand_start(row: &[Atom], end: usize) -> usize {
    let mut i = end;
    while i > 0 {
        match &row[i - 1] {
            Atom::Ch(c) if is_operand_char(*c) => i -= 1,
            Atom::Node(..) => i -= 1,
            Atom::Ch(_) => break,
        }
    }
    i
}

/// Corta una fila por sus comas de primer nivel.
fn split_commas(row: Row) -> Vec<Row> {
    let mut parts = vec![Vec::new()];
    for a in row {
        if a == Atom::Ch(',') {
            parts.push(Vec::new());
        } else {
            parts.last_mut().unwrap().push(a);
        }
    }
    parts.into_iter().map(trim_spaces).collect()
}

fn trim_spaces(mut r: Row) -> Row {
    while r.first() == Some(&Atom::Ch(' ')) {
        r.remove(0);
    }
    while r.last() == Some(&Atom::Ch(' ')) {
        r.pop();
    }
    r
}

/// ¿La fila es un solo número o una sola letra? (Entonces no necesita paréntesis.)
fn is_simple(r: &[Atom]) -> bool {
    let cs: Option<Vec<char>> = r.iter().map(|a| if let Atom::Ch(c) = a { Some(*c) } else { None }).collect();
    let Some(cs) = cs else { return false };
    match cs.as_slice() {
        [] => false,
        [c] => c.is_alphanumeric() || matches!(c, 'π' | '∞'),
        _ => cs.iter().all(|c| c.is_ascii_digit() || *c == '.'),
    }
}

/// Las letras seguidas que terminan en `end`.
fn letters_before(row: &[Atom], end: usize) -> (usize, String) {
    let mut i = end;
    while i > 0 && matches!(row[i - 1], Atom::Ch(c) if is_letter(c)) {
        i -= 1;
    }
    let word = row[i..end].iter().map(|a| if let Atom::Ch(c) = a { *c } else { ' ' }).collect();
    (i, word)
}

// ───────────────────────────── Texto → árbol ─────────────────────────────

struct Reader {
    c: Vec<char>,
    i: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        let s: Vec<char> = s.chars().collect();
        self.c[self.i..].starts_with(&s)
    }

    /// Lee hasta `close` (que no consume) o hasta el final.
    fn row(&mut self, close: Option<char>) -> Row {
        let mut out: Row = Vec::new();
        while let Some(ch) = self.peek() {
            if Some(ch) == close {
                break;
            }
            if self.starts_with("d/dx(") {
                self.i += 5;
                let inner = self.row(Some(')'));
                self.eat(')');
                out.push(node(Kind::Deriv, vec![inner]));
                continue;
            }
            match ch {
                '(' | '[' | '{' => {
                    let d = Delim::of_open(ch).unwrap();
                    self.i += 1;
                    let inner = self.row(Some(d.chars().1));
                    self.eat(d.chars().1);
                    if d == Delim::Paren {
                        function_call(&mut out, inner);
                    } else {
                        out.push(node(Kind::Delim(d), vec![inner]));
                    }
                }
                '|' => {
                    self.i += 1;
                    let inner = self.row(Some('|'));
                    if self.eat('|') {
                        out.push(node(Kind::Delim(Delim::Abs), vec![inner]));
                    } else {
                        out.push(Atom::Ch('|'));
                        out.extend(inner);
                    }
                }
                '^' => {
                    self.i += 1;
                    pop_spaces(&mut out);
                    self.skip_spaces();
                    let e = self.power_operand();
                    out.push(node(Kind::Sup, vec![e]));
                }
                '_' if self.c.get(self.i + 1).is_some_and(|c| c.is_alphanumeric()) && matches!(out.last(), Some(Atom::Ch(c)) if c.is_alphanumeric()) => {
                    self.i += 1;
                    let mut sub = Vec::new();
                    while let Some(c) = self.peek().filter(|c| c.is_alphanumeric()) {
                        sub.push(Atom::Ch(c));
                        self.i += 1;
                    }
                    out.push(node(Kind::Sub, vec![sub]));
                }
                '/' => {
                    self.i += 1;
                    pop_spaces(&mut out);
                    self.skip_spaces();
                    let start = operand_start(&out, out.len());
                    let num = unwrap_paren(out.split_off(start));
                    let den = self.power_operand();
                    out.push(node(Kind::Frac, vec![num, den]));
                }
                '<' | '>' if self.c.get(self.i + 1) == Some(&'=') => {
                    self.i += 2;
                    out.push(Atom::Ch(if ch == '<' { '≤' } else { '≥' }));
                }
                '*' => {
                    self.i += 1;
                    out.push(Atom::Ch('·'));
                }
                _ => {
                    self.i += 1;
                    out.push(Atom::Ch(ch));
                }
            }
        }
        symbols(&mut out);
        out
    }

    fn skip_spaces(&mut self) {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// Operando de `^` o denominador de `/`: un número, una letra (con subíndice), una
    /// función con su paréntesis o un paréntesis; incluye los `!` y los `^` que le siguen
    /// (2^3! = 2^(3!), 1/x^2 = 1/(x²)).
    fn power_operand(&mut self) -> Row {
        let mut r = self.operand();
        while self.peek() == Some('!') {
            self.i += 1;
            r.push(Atom::Ch('!'));
        }
        if self.peek() == Some('^') {
            self.i += 1;
            let e = self.power_operand();
            r.push(node(Kind::Sup, vec![e]));
        }
        r
    }

    fn operand(&mut self) -> Row {
        let Some(ch) = self.peek() else { return Vec::new() };
        match ch {
            '(' => {
                self.i += 1;
                let inner = self.row(Some(')'));
                self.eat(')');
                inner
            }
            '-' | '+' | '−' => {
                self.i += 1;
                let mut r = vec![Atom::Ch(ch)];
                r.extend(self.power_operand());
                r
            }
            '|' => {
                let mut tmp = Reader { c: self.c.clone(), i: self.i };
                tmp.i += 1;
                let inner = tmp.row(Some('|'));
                if tmp.eat('|') {
                    self.i = tmp.i;
                    vec![node(Kind::Delim(Delim::Abs), vec![inner])]
                } else {
                    Vec::new()
                }
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut r = Vec::new();
                while let Some(d) = self.peek().filter(|d| d.is_ascii_digit() || *d == '.') {
                    r.push(Atom::Ch(d));
                    self.i += 1;
                }
                r
            }
            c if c.is_alphabetic() => {
                // Nombre de función o constante completo (sin, pi…); si no, una sola letra.
                let mut j = self.i;
                while self.c.get(j).is_some_and(|c| c.is_alphabetic()) {
                    j += 1;
                }
                let word: String = self.c[self.i..j].iter().collect();
                let known = |w: &str| math::is_builtin_func(w) || matches!(w, "pi" | "tau" | "inf" | "theta");
                let end = if known(&word) { j } else { self.i + 1 };
                let mut r: Row = self.c[self.i..end].iter().map(|c| Atom::Ch(*c)).collect();
                self.i = end;
                if self.peek() == Some('_') && self.c.get(self.i + 1).is_some_and(|c| c.is_alphanumeric()) {
                    self.i += 1;
                    let mut sub = Vec::new();
                    while let Some(c) = self.peek().filter(|c| c.is_alphanumeric()) {
                        sub.push(Atom::Ch(c));
                        self.i += 1;
                    }
                    r.push(node(Kind::Sub, vec![sub]));
                }
                if self.peek() == Some('(') && known(&word) {
                    self.i += 1;
                    let inner = self.row(Some(')'));
                    self.eat(')');
                    function_call(&mut r, inner);
                }
                symbols(&mut r);
                r
            }
            _ => {
                self.i += 1;
                vec![Atom::Ch(ch)]
            }
        }
    }
}

/// Los espacios alrededor de `/` y `^` no cuentan ("x / 2" es una fracción).
fn pop_spaces(r: &mut Row) {
    while r.last() == Some(&Atom::Ch(' ')) {
        r.pop();
    }
}

/// Un paréntesis que sigue a un nombre: sqrt(…), root(…), sum(…)… se convierten en su
/// estructura; los demás quedan como paréntesis (sin(…), f(…)).
fn function_call(out: &mut Row, inner: Row) {
    let (start, word) = letters_before(out, out.len());
    let rebuilt = match word.as_str() {
        "sqrt" => Some(node(Kind::Sqrt, vec![inner.clone()])),
        "abs" => Some(node(Kind::Delim(Delim::Abs), vec![inner.clone()])),
        "root" => match <[Row; 2]>::try_from(split_commas(inner.clone())) {
            Ok([i, r]) => Some(node(Kind::Root, vec![i, r])),
            Err(_) => None,
        },
        "sum" | "prod" => match <[Row; 4]>::try_from(split_commas(inner.clone())) {
            Ok([v, body, from, to]) => {
                let mut lower = v;
                lower.push(Atom::Ch('='));
                lower.extend(from);
                Some(node(if word == "sum" { Kind::Sum } else { Kind::Prod }, vec![lower, to, body]))
            }
            Err(_) => None,
        },
        "int" | "integral" => match split_commas(inner.clone()).as_slice() {
            [v, body] => Some(node(Kind::Int, vec![Vec::new(), Vec::new(), body.clone(), v.clone()])),
            [v, body, from, to] => Some(node(Kind::Int, vec![from.clone(), to.clone(), body.clone(), v.clone()])),
            _ => None,
        },
        _ => None,
    };
    match rebuilt {
        Some(a) => {
            out.truncate(start);
            out.push(a);
        }
        None => out.push(node(Kind::Delim(Delim::Paren), vec![inner])),
    }
}

/// Un paréntesis alrededor de todo el numerador sobra (la raya ya agrupa).
fn unwrap_paren(mut r: Row) -> Row {
    if r.len() == 1 {
        if let Atom::Node(Kind::Delim(Delim::Paren), rows) = &mut r[0] {
            return std::mem::take(&mut rows[0]);
        }
    }
    r
}

/// Palabras que se muestran con su símbolo: pi → π, theta → θ…
fn symbols(row: &mut Row) {
    let mut i = 0;
    while i < row.len() {
        let mut j = i;
        while j < row.len() && matches!(row[j], Atom::Ch(c) if c.is_ascii_alphabetic()) {
            j += 1;
        }
        if j == i {
            i += 1;
            continue;
        }
        let word: String = row[i..j].iter().map(|a| if let Atom::Ch(c) = a { *c } else { ' ' }).collect();
        let sym = match word.as_str() {
            "pi" => Some('π'),
            "theta" => Some('θ'),
            "tau" => Some('τ'),
            "inf" => Some('∞'),
            _ => None,
        };
        // No si sigue un subíndice o un paréntesis de llamada (theta_1, pi(x) son nombres).
        let next_ok = !matches!(row.get(j), Some(Atom::Node(Kind::Sub | Kind::Delim(Delim::Paren), _)));
        if let (Some(s), true) = (sym, next_ok) {
            row.splice(i..j, [Atom::Ch(s)]);
            i += 1;
        } else {
            i = j;
        }
    }
}

/// Quita los paréntesis que solo rodean una estructura que ya agrupa por sí misma
/// (((x^2)/(2)) → la fracción), salvo que cambie el sentido: si les sigue un exponente o
/// un factorial, o si son los de una llamada a función.
fn drop_redundant_parens(row: &mut Row) {
    let mut i = 0;
    while i < row.len() {
        if let Atom::Node(_, rows) = &mut row[i] {
            for r in rows.iter_mut() {
                drop_redundant_parens(r);
            }
        }
        let groups = matches!(&row[i], Atom::Node(Kind::Delim(Delim::Paren), rows)
            if matches!(&rows[0][..], [Atom::Node(Kind::Frac | Kind::Sqrt | Kind::Root | Kind::Delim(_), _)]));
        let next_binds = matches!(row.get(i + 1), Some(Atom::Node(Kind::Sup, _) | Atom::Ch('!' | '\'' | '′')));
        let call = i > 0 && matches!(row[i - 1], Atom::Ch(c) if c.is_alphanumeric() || c == '_') || matches!(row.get(i.wrapping_sub(1)), Some(Atom::Node(Kind::Sub, _)));
        if groups && !next_binds && !call {
            if let Atom::Node(_, rows) = &mut row[i] {
                let inner = std::mem::take(&mut rows[0]);
                row.splice(i..i + 1, inner);
            }
        }
        i += 1;
    }
}

pub fn from_text(s: &str) -> Row {
    let mut r = Reader { c: s.chars().collect(), i: 0 };
    let mut out = Vec::new();
    // Un cierre suelto no detiene la lectura: se guarda tal cual.
    loop {
        out.extend(r.row(None));
        match r.peek() {
            Some(c) => {
                out.push(Atom::Ch(c));
                r.i += 1;
            }
            None => break,
        }
    }
    drop_redundant_parens(&mut out);
    out
}

// ───────────────────────────── Árbol → texto ─────────────────────────────

/// Escritura del texto, anotando en qué carácter cae cada posición del cursor.
#[derive(Default)]
struct Writer {
    out: String,
    len: usize,
    marks: Vec<(Path, usize, usize)>,
}

impl Writer {
    fn push(&mut self, s: &str) {
        self.out.push_str(s);
        self.len += s.chars().count();
    }

    fn row(&mut self, r: &[Atom], path: &mut Path) {
        for (i, a) in r.iter().enumerate() {
            self.marks.push((path.clone(), i, self.len));
            self.atom(a, path, i);
        }
        self.marks.push((path.clone(), r.len(), self.len));
    }

    fn child(&mut self, rows: &[Row], path: &mut Path, at: usize, c: usize) {
        path.push((at, c));
        self.row(&rows[c], path);
        path.pop();
    }

    /// Casilla que va entre paréntesis salvo que sea un número o una letra.
    fn group(&mut self, rows: &[Row], path: &mut Path, at: usize, c: usize) {
        if is_simple(&rows[c]) {
            self.child(rows, path, at, c);
        } else {
            self.push("(");
            self.child(rows, path, at, c);
            self.push(")");
        }
    }

    /// Una casilla escrita aparte (para reordenar las de sum/prod/int); sin marcas.
    fn plain(r: &[Atom]) -> String {
        let mut w = Writer::default();
        w.row(r, &mut Vec::new());
        w.out
    }

    fn atom(&mut self, a: &Atom, path: &mut Path, at: usize) {
        let Atom::Node(kind, rows) = a else {
            if let Atom::Ch(c) = a {
                let mut b = [0; 4];
                self.push(c.encode_utf8(&mut b));
            }
            return;
        };
        match kind {
            Kind::Frac => {
                self.group(rows, path, at, 0);
                self.push("/");
                self.group(rows, path, at, 1);
            }
            Kind::Sup => {
                self.push("^");
                self.group(rows, path, at, 0);
            }
            Kind::Sub => {
                self.push("_");
                self.child(rows, path, at, 0);
            }
            Kind::Sqrt => {
                self.push("sqrt(");
                self.child(rows, path, at, 0);
                self.push(")");
            }
            Kind::Root => {
                self.push("root(");
                self.child(rows, path, at, 0);
                self.push(", ");
                self.child(rows, path, at, 1);
                self.push(")");
            }
            Kind::Delim(d) => {
                // |…| salvo que dentro haya otro valor absoluto (sería ambiguo).
                let nested = rows[0].iter().any(|a| matches!(a, Atom::Ch('|') | Atom::Node(Kind::Delim(Delim::Abs), _)));
                let (o, c) = if *d == Delim::Abs && nested { ("abs(".to_string(), ")".to_string()) } else { (d.chars().0.to_string(), d.chars().1.to_string()) };
                self.push(&o);
                self.child(rows, path, at, 0);
                self.push(&c);
            }
            Kind::Sum | Kind::Prod => {
                let name = if *kind == Kind::Sum { "sum" } else { "prod" };
                let lower = &rows[0];
                let (var, from) = match lower.iter().position(|a| *a == Atom::Ch('=')) {
                    Some(eq) => (Writer::plain(&lower[..eq]), Writer::plain(&lower[eq + 1..])),
                    None => ("n".to_string(), Writer::plain(lower)),
                };
                let var = if var.trim().is_empty() { "n".to_string() } else { var.trim().to_string() };
                let s = format!("{name}({var}, {}, {}, {})", Writer::plain(&rows[2]), from.trim(), Writer::plain(&rows[1]));
                self.push(&s);
            }
            Kind::Int => {
                let var = Writer::plain(&rows[3]);
                let var = if var.trim().is_empty() { "x".to_string() } else { var.trim().to_string() };
                let body = Writer::plain(&rows[2]);
                let s = if rows[0].is_empty() && rows[1].is_empty() {
                    format!("int({var}, {body})")
                } else {
                    format!("int({var}, {body}, {}, {})", Writer::plain(&rows[0]), Writer::plain(&rows[1]))
                };
                self.push(&s);
            }
            Kind::Deriv => {
                self.push("d/dx(");
                self.child(rows, path, at, 0);
                self.push(")");
            }
        }
    }
}

pub fn to_text(row: &[Atom]) -> String {
    Writer::plain(row)
}

/// ¿Queda alguna casilla sin rellenar?
pub fn has_empty_slot(row: &[Atom]) -> bool {
    row.iter().any(|a| match a {
        Atom::Node(Kind::Int, rows) => {
            // La integral sin límites es la primitiva: solo cuentan cuerpo y variable.
            let limits_half = rows[0].is_empty() != rows[1].is_empty();
            limits_half || rows[2].is_empty() || rows.iter().any(|r| has_empty_slot(r))
        }
        Atom::Node(_, rows) => rows.iter().any(|r| r.is_empty() || has_empty_slot(r)),
        Atom::Ch(_) => false,
    })
}

// ───────────────────────────── Composición ─────────────────────────────

/// Geometría de una fila: posiciones del cursor (x), línea base y alto.
#[derive(Clone, Debug)]
struct RowGeo {
    path: Path,
    xs: Vec<f32>,
    y: f32,
    asc: f32,
    desc: f32,
}

struct Boxed {
    lay: Layout,
    rows: Vec<RowGeo>,
}

impl Boxed {
    fn place(&mut self, other: Boxed, dx: f32, dy: f32) {
        self.lay.place(other.lay, dx, dy);
        for mut g in other.rows {
            for x in &mut g.xs {
                *x += dx;
            }
            g.y += dy;
            self.rows.push(g);
        }
    }
}

/// Cómo se dibuja cada carácter de una fila.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Var,
    Upright,
    /// Operador con espacio a los lados.
    Op,
    /// Coma o punto y coma: espacio después.
    Sep,
    Space,
}

/// Aspecto de los caracteres: los nombres de función (sin, log…) van rectos y los
/// operadores binarios espaciados.
fn looks(row: &[Atom]) -> Vec<Look> {
    let mut out = vec![Look::Upright; row.len()];
    let mut i = 0;
    while i < row.len() {
        if let Atom::Ch(c) = row[i] {
            if is_letter(c) {
                let mut j = i;
                while j < row.len() && matches!(row[j], Atom::Ch(c) if is_letter(c)) {
                    j += 1;
                }
                let word: String = row[i..j].iter().map(|a| if let Atom::Ch(c) = a { *c } else { ' ' }).collect();
                let upright = math::is_builtin_func(&word) || word == "for";
                for o in &mut out[i..j] {
                    *o = if upright { Look::Upright } else { Look::Var };
                }
                i = j;
                continue;
            }
            out[i] = match c {
                '=' | '<' | '>' | '≤' | '≥' | '·' | '×' | '÷' => Look::Op,
                // + y − binarios (no al principio ni tras otro operador o una apertura).
                '+' | '-' | '−' => {
                    let unary = i == 0 || matches!(row[i - 1], Atom::Ch(p) if matches!(p, '=' | '<' | '>' | '≤' | '≥' | '(' | ',' | '+' | '-' | '−' | '·' | ' ' | ';'));
                    if unary { Look::Upright } else { Look::Op }
                }
                ',' | ';' => Look::Sep,
                ' ' => Look::Space,
                _ => Look::Upright,
            };
        }
        i += 1;
    }
    out
}

fn shown_char(c: char) -> char {
    match c {
        '-' => '−',
        '\'' => '′',
        _ => c,
    }
}

struct Typesetter<'a> {
    comp: Composer<'a>,
}

impl Typesetter<'_> {
    /// Casilla vacía: un recuadro tenue del tamaño de una letra.
    fn slot(&self, size: f32) -> Layout {
        // Tamaño mínimo para que se pueda hacer clic también en límites y exponentes.
        let size = size.max(14.0);
        let w = size * 0.55;
        let (asc, desc) = (size * 0.72, size * 0.08);
        let mut l = Layout { w: w + size * 0.08, asc, desc, ..Layout::empty() };
        l.prims.push(Prim::Slot { min: vec2(size * 0.04, -asc), max: vec2(size * 0.04 + w, desc) });
        l
    }

    fn row(&self, row: &[Atom], path: &mut Path, size: f32, slot: bool) -> Boxed {
        if row.is_empty() {
            let lay = if slot { self.slot(size) } else { Layout { w: 1.0, asc: 0.74 * size, desc: 0.22 * size, ..Layout::empty() } };
            let x = lay.w / 2.0;
            let g = RowGeo { path: path.clone(), xs: vec![if slot { x } else { 0.0 }], y: 0.0, asc: lay.asc, desc: lay.desc };
            return Boxed { lay, rows: vec![g] };
        }
        let looks = looks(row);
        let mut b = Boxed { lay: Layout::empty(), rows: Vec::new() };
        let mut xs = Vec::with_capacity(row.len() + 1);
        // Métricas del elemento anterior (para colocar exponentes y subíndices).
        let mut prev = (0.74 * size, 0.22 * size);
        for (i, a) in row.iter().enumerate() {
            xs.push(b.lay.w);
            let (child, dy) = match a {
                Atom::Ch(c) => {
                    let look = looks[i];
                    let t = self.comp.text(&shown_char(*c).to_string(), size, look == Look::Var);
                    let l = match look {
                        Look::Op => pad(t, size * 0.24, size * 0.24),
                        Look::Sep => pad(t, 0.0, size * 0.2),
                        Look::Space => Layout { w: size * 0.22, ..Layout::empty() },
                        _ => t,
                    };
                    (Boxed { lay: l, rows: Vec::new() }, 0.0)
                }
                Atom::Node(kind, rows) => {
                    path.push((i, 0));
                    let n = self.node(*kind, rows, path, i, size, prev);
                    path.pop();
                    n
                }
            };
            if !matches!(a, Atom::Node(Kind::Sup | Kind::Sub, _)) {
                prev = (child.lay.asc - dy, child.lay.desc + dy);
            }
            b.lay.asc = b.lay.asc.max(child.lay.asc - dy);
            b.lay.desc = b.lay.desc.max(child.lay.desc + dy);
            let x = b.lay.w;
            b.lay.w += child.lay.w;
            b.place(child, x, dy);
        }
        xs.push(b.lay.w);
        b.lay.asc = b.lay.asc.max(0.74 * size);
        b.lay.desc = b.lay.desc.max(0.22 * size);
        b.rows.push(RowGeo { path: path.clone(), xs, y: 0.0, asc: b.lay.asc, desc: b.lay.desc });
        b
    }

    /// Casilla `c` del átomo `at` (el último elemento de `path` es (at, _)).
    fn child(&self, rows: &[Row], path: &mut Path, c: usize, size: f32) -> Boxed {
        let at = path.last().unwrap().0;
        *path.last_mut().unwrap() = (at, c);
        self.row(&rows[c], path, size, true)
    }

    /// Devuelve la caja y su desplazamiento vertical (los exponentes van hacia arriba).
    fn node(&self, kind: Kind, rows: &[Row], path: &mut Path, _at: usize, size: f32, prev: (f32, f32)) -> (Boxed, f32) {
        let script = (size * 0.7).max(8.0);
        let empty = || Boxed { lay: Layout::empty(), rows: Vec::new() };
        match kind {
            Kind::Sup => {
                let s = self.child(rows, path, 0, script);
                let shift = (0.42 * size).max(prev.0 - 0.5 * s.lay.asc);
                let mut b = empty();
                let w = s.lay.w + size * 0.04;
                b.lay = Layout { w, asc: s.lay.asc, desc: s.lay.desc, ..Layout::empty() };
                b.place(s, size * 0.04, 0.0);
                (b, -shift)
            }
            Kind::Sub => {
                let s = self.child(rows, path, 0, script);
                let shift = (0.22 * size).max(prev.1 - 0.2 * s.lay.desc);
                let mut b = empty();
                b.lay = Layout { w: s.lay.w + size * 0.02, asc: s.lay.asc, desc: s.lay.desc, ..Layout::empty() };
                b.place(s, size * 0.02, 0.0);
                (b, shift)
            }
            Kind::Frac => {
                let s = (size * 0.92).max(9.0);
                let a = self.child(rows, path, 0, s);
                let d = self.child(rows, path, 1, s);
                let (aw, dw) = (a.lay.w, d.lay.w);
                let gap = size * 0.14;
                let padx = size * 0.12;
                let inner = aw.max(dw);
                let w = inner + 2.0 * padx;
                let ax = axis(size);
                let num_base = -ax - gap - a.lay.desc;
                let den_base = -ax + gap + d.lay.asc;
                let mut b = empty();
                b.lay = Layout { w, asc: ax + gap + a.lay.height(), desc: gap + d.lay.height() - ax, ..Layout::empty() };
                b.place(a, padx + (inner - aw) / 2.0, num_base);
                b.place(d, padx + (inner - dw) / 2.0, den_base);
                b.lay.prims.push(Composer::line(vec![vec2(size * 0.05, -ax), vec2(w - size * 0.05, -ax)], size));
                (b, 0.0)
            }
            Kind::Sqrt | Kind::Root => {
                let (c, idx) = if kind == Kind::Root {
                    let i = self.child(rows, path, 0, (size * 0.55).max(7.0));
                    (self.child(rows, path, 1, size), Some(i))
                } else {
                    (self.child(rows, path, 0, size), None)
                };
                let pad_top = size * 0.12;
                let top = -(c.lay.asc + pad_top);
                let bottom = c.lay.desc + size * 0.04;
                let mid = (top + bottom) / 2.0;
                let rw = size * 0.6;
                let shift = idx.as_ref().map_or(0.0, |i| (i.lay.w - rw * 0.45).max(0.0));
                let x0 = shift;
                let x_end = x0 + rw + c.lay.w + size * 0.1;
                let points = vec![
                    vec2(x0, mid + (bottom - top) * 0.05),
                    vec2(x0 + rw * 0.25, mid - (bottom - top) * 0.02),
                    vec2(x0 + rw * 0.55, bottom),
                    vec2(x0 + rw, top),
                    vec2(x_end, top),
                ];
                let mut b = empty();
                b.lay = Layout { w: x_end + size * 0.05, asc: -top + size * 0.07, desc: bottom, ..Layout::empty() };
                b.lay.prims.push(Composer::line(points, size));
                if let Some(i) = idx {
                    let base = mid - (bottom - top) * 0.08 - i.lay.desc;
                    b.lay.asc = b.lay.asc.max(-base + i.lay.asc);
                    let iw = i.lay.w;
                    b.place(i, x0 + rw * 0.45 - iw, base);
                }
                b.place(c, x0 + rw, 0.0);
                (b, 0.0)
            }
            Kind::Delim(d) => {
                let c = self.child(rows, path, 0, size);
                (self.delimit(d.chars(), c, size), 0.0)
            }
            Kind::Deriv => {
                // d/dx como fracción pequeña, seguido del cuerpo entre paréntesis.
                let s = (size * 0.92).max(9.0);
                let top = self.comp.text("d", s, false);
                let mut bot = self.comp.text("d", s, false);
                let dw = bot.w;
                bot.w += self.comp.text("x", s, true).w;
                bot.place(self.comp.text("x", s, true), dw, 0.0);
                let (tw, bw) = (top.w, bot.w);
                let (gap, padx, ax) = (size * 0.14, size * 0.12, axis(size));
                let inner = tw.max(bw);
                let fw = inner + 2.0 * padx;
                let mut frac = Layout { w: fw, asc: ax + gap + top.height(), desc: gap + bot.height() - ax, ..Layout::empty() };
                let (top_desc, bot_asc) = (top.desc, bot.asc);
                frac.place(top, padx + (inner - tw) / 2.0, -ax - gap - top_desc);
                frac.place(bot, padx + (inner - bw) / 2.0, -ax + gap + bot_asc);
                frac.prims.push(Composer::line(vec![vec2(size * 0.05, -ax), vec2(fw - size * 0.05, -ax)], size));
                let c = self.child(rows, path, 0, size);
                let body = self.delimit(('(', ')'), c, size);
                let mut b = empty();
                b.lay = Layout { w: fw + body.lay.w, asc: frac.asc.max(body.lay.asc), desc: frac.desc.max(body.lay.desc), ..Layout::empty() };
                b.lay.place(frac, 0.0, 0.0);
                b.place(body, fw, 0.0);
                (b, 0.0)
            }
            Kind::Sum | Kind::Prod | Kind::Int => {
                let lo = self.child(rows, path, 0, script);
                let hi = self.child(rows, path, 1, script);
                let body = self.child(rows, path, 2, size);
                let mut b = empty();
                let (sym_w, sym_asc, sym_desc, lo_x, hi_x, lo_y, hi_y);
                if kind == Kind::Int {
                    let t = self.comp.text("∫", size * 1.6, false);
                    let dy = (t.asc - t.desc) / 2.0 - axis(size);
                    let (a, de) = (t.asc - dy, t.desc + dy);
                    let x = t.w + size * 0.02;
                    sym_w = x + lo.lay.w.max(hi.lay.w);
                    hi_y = -(a - 0.55 * hi.lay.asc);
                    lo_y = de - 0.1 * lo.lay.asc;
                    hi_x = x;
                    lo_x = x - size * 0.12;
                    sym_asc = a.max(-hi_y + hi.lay.asc);
                    sym_desc = de.max(lo_y + lo.lay.desc);
                    b.lay.place(t, 0.0, dy);
                } else {
                    let t = self.comp.text(if kind == Kind::Sum { "∑" } else { "∏" }, size * 1.45, false);
                    let dy = (t.asc - t.desc) / 2.0 - axis(size);
                    let (a, de) = (t.asc - dy, t.desc + dy);
                    let gap = size * 0.08;
                    sym_w = t.w.max(lo.lay.w).max(hi.lay.w);
                    hi_y = -a - gap - hi.lay.desc;
                    lo_y = de + gap + lo.lay.asc;
                    hi_x = (sym_w - hi.lay.w) / 2.0;
                    lo_x = (sym_w - lo.lay.w) / 2.0;
                    sym_asc = -hi_y + hi.lay.asc;
                    sym_desc = lo_y + lo.lay.desc;
                    let tw = t.w;
                    b.lay.place(t, (sym_w - tw) / 2.0, dy);
                }
                b.place(lo, lo_x, lo_y);
                b.place(hi, hi_x, hi_y);
                // El cuerpo va entre paréntesis si tiene sumas o restas (si no, no se sabría
                // dónde termina).
                let needs = rows[2].iter().enumerate().any(|(i, a)| i > 0 && matches!(a, Atom::Ch('+' | '-' | '−' | '=' | '<' | '>')));
                let body = if needs { self.delimit(('(', ')'), body, size) } else { body };
                let x = sym_w + size * 0.12;
                let mut w = x + body.lay.w;
                let (mut asc, mut desc) = (sym_asc.max(body.lay.asc), sym_desc.max(body.lay.desc));
                b.place(body, x, 0.0);
                if kind == Kind::Int {
                    let d = self.comp.text("d", size, false);
                    let dw = d.w;
                    b.lay.place(d, w + size * 0.15, 0.0);
                    let v = self.child(rows, path, 3, size);
                    asc = asc.max(v.lay.asc);
                    desc = desc.max(v.lay.desc);
                    let vx = w + size * 0.15 + dw;
                    w = vx + v.lay.w;
                    b.place(v, vx, 0.0);
                }
                b.lay.w = w;
                b.lay.asc = asc;
                b.lay.desc = desc;
                (b, 0.0)
            }
        }
    }

    fn delimit(&self, (o, c): (char, char), inner: Boxed, size: f32) -> Boxed {
        let Boxed { lay, rows } = inner;
        let (l, dx) = self.comp.delimit_at(o, c, lay, size);
        let mut b = Boxed { lay: l, rows: Vec::new() };
        // Las filas del contenido se desplazan igual que su dibujo.
        for mut g in rows {
            for x in &mut g.xs {
                *x += dx;
            }
            b.rows.push(g);
        }
        b
    }
}

fn pad(t: Layout, left: f32, right: f32) -> Layout {
    let mut l = Layout { w: t.w + left + right, asc: t.asc, desc: t.desc, ..Layout::empty() };
    l.place(t, left, 0.0);
    l
}

// ───────────────────────────── Edición ─────────────────────────────

/// Estado de edición de una línea.
pub struct State {
    root: Row,
    cur: Cursor,
    /// Otro extremo de la selección (en la misma fila que el cursor).
    anchor: Option<usize>,
    /// Texto del que salió el árbol (para notar cambios hechos desde fuera).
    synced: String,
    /// Desplazamiento horizontal cuando la expresión no cabe.
    scroll: f32,
    /// Geometría del último dibujo (ratón y flechas ↑ ↓), relativa al origen del dibujo.
    geo: Vec<RowGeo>,
    /// Dónde se empezó a arrastrar el ratón (para seleccionar).
    press: Option<Cursor>,
}

/// Lo que pasó al editar en este fotograma.
#[derive(Default)]
pub struct Output {
    pub changed: bool,
    /// Enter: nueva línea debajo.
    pub enter: bool,
    /// Flecha ↑ / ↓ sin más casillas por encima / debajo: pasar a la línea anterior / siguiente.
    pub up: bool,
    pub down: bool,
}

impl State {
    pub fn new(text: &str) -> Self {
        let root = parse_any(text);
        let cur = Cursor { path: Vec::new(), pos: root.len() };
        Self { root, cur, anchor: None, synced: text.to_string(), scroll: 0.0, geo: Vec::new(), press: None }
    }

    /// Rehace el árbol si el texto cambió desde fuera (deslizadores, puntos arrastrados…).
    pub fn sync(&mut self, text: &str) {
        if text != self.synced {
            let at_end = self.cur.path.is_empty() && self.cur.pos == self.root.len();
            *self = Self { scroll: self.scroll, ..Self::new(text) };
            if !at_end {
                self.cur.pos = self.root.len();
            }
        }
    }

    pub fn text(&self) -> String {
        to_text(&self.root)
    }

    pub fn has_empty_slot(&self) -> bool {
        has_empty_slot(&self.root)
    }

    /// Escribe el texto del árbol en `text` (si cambió).
    pub fn store(&mut self, text: &mut String) -> bool {
        let t = self.text();
        self.synced = t.clone();
        if *text != t {
            *text = t;
            true
        } else {
            false
        }
    }

    pub fn cursor_to_end(&mut self) {
        self.cur = Cursor { path: Vec::new(), pos: self.root.len() };
        self.anchor = None;
    }

    fn row(&self) -> &Row {
        row_at(&self.root, &self.cur.path)
    }

    fn row_mut(&mut self) -> &mut Row {
        row_at_mut(&mut self.root, &self.cur.path)
    }

    fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cur.pos).then(|| (a.min(self.cur.pos), a.max(self.cur.pos)))
    }

    /// Quita la selección y la devuelve.
    fn take_selection(&mut self) -> Option<Row> {
        let (lo, hi) = self.selection()?;
        self.anchor = None;
        self.cur.pos = lo;
        Some(self.row_mut().drain(lo..hi).collect())
    }

    fn enter(&mut self, at: usize, child: usize, end: bool) {
        self.cur.path.push((at, child));
        self.cur.pos = if end { self.row().len() } else { 0 };
    }

    /// Inserta un átomo y deja el cursor detrás.
    fn put(&mut self, a: Atom) {
        let pos = self.cur.pos;
        self.row_mut().insert(pos, a);
        self.cur.pos += 1;
    }

    /// Inserta una estructura con `content` en su casilla `slot` y entra en la primera casilla vacía.
    fn put_node(&mut self, kind: Kind, slot: usize, content: Row) {
        let mut rows = vec![Vec::new(); kind.arity()];
        rows[slot] = content;
        let first_empty = rows.iter().position(Vec::is_empty);
        let at = self.cur.pos;
        self.put(node(kind, rows));
        if let Some(c) = first_empty {
            self.enter(at, c, false);
        }
    }

    pub fn insert_char(&mut self, c: char) {
        match c {
            '/' => {
                let num = match self.take_selection() {
                    Some(sel) => sel,
                    None => {
                        while self.cur.pos > 0 && self.row()[self.cur.pos - 1] == Atom::Ch(' ') {
                            let i = self.cur.pos - 1;
                            self.row_mut().remove(i);
                            self.cur.pos = i;
                        }
                        let start = operand_start(self.row(), self.cur.pos);
                        let pos = self.cur.pos;
                        let num: Row = self.row_mut().drain(start..pos).collect();
                        self.cur.pos = start;
                        unwrap_paren(num)
                    }
                };
                let filled = !num.is_empty();
                let at = self.cur.pos;
                self.put(node(Kind::Frac, vec![num, Vec::new()]));
                self.enter(at, if filled { 1 } else { 0 }, false);
            }
            '^' | '_' => {
                let kind = if c == '^' { Kind::Sup } else { Kind::Sub };
                let sel = self.take_selection().unwrap_or_default();
                // Si ya hay un exponente justo antes, se sigue escribiendo en él.
                if sel.is_empty() && self.cur.pos > 0 && matches!(&self.row()[self.cur.pos - 1], Atom::Node(k, _) if *k == kind) {
                    let at = self.cur.pos - 1;
                    self.enter(at, 0, true);
                    return;
                }
                let at = self.cur.pos;
                self.put(node(kind, vec![sel]));
                self.enter(at, 0, true);
            }
            '(' | '[' | '{' | '|' => {
                let d = Delim::of_open(c).unwrap();
                // | al final de un valor absoluto lo cierra.
                if d == Delim::Abs && self.selection().is_none() && self.close_delim(Delim::Abs) {
                    return;
                }
                match self.take_selection() {
                    Some(sel) => self.put(node(Kind::Delim(d), vec![sel])),
                    None => self.put_node(Kind::Delim(d), 0, Vec::new()),
                }
            }
            ')' | ']' | '}' => {
                let d = Delim::of_close(c).unwrap();
                self.anchor = None;
                if !self.close_delim(d) {
                    // Sin paréntesis abierto: encierra lo escrito antes del cursor.
                    let pos = self.cur.pos;
                    let inner: Row = self.row_mut().drain(..pos).collect();
                    self.cur.pos = 0;
                    self.put(node(Kind::Delim(d), vec![inner]));
                }
            }
            '*' => self.insert_plain('·'),
            '=' => {
                let prev = self.cur.pos.checked_sub(1).and_then(|i| match self.row()[i] {
                    Atom::Ch(p @ ('<' | '>')) => Some(p),
                    _ => None,
                });
                match prev {
                    Some(p) => {
                        let i = self.cur.pos - 1;
                        self.row_mut()[i] = Atom::Ch(if p == '<' { '≤' } else { '≥' });
                    }
                    None => self.insert_plain('='),
                }
            }
            c if c.is_control() => {}
            c => {
                self.insert_plain(c);
                if is_letter(c) {
                    self.auto_command();
                }
            }
        }
    }

    fn insert_plain(&mut self, c: char) {
        self.take_selection();
        self.put(Atom::Ch(c));
    }

    /// Sale del delimitador `d` más cercano que contiene al cursor (dejándolo detrás).
    fn close_delim(&mut self, d: Delim) -> bool {
        for depth in (0..self.cur.path.len()).rev() {
            let (at, _) = self.cur.path[depth];
            let parent = row_at(&self.root, &self.cur.path[..depth]);
            if matches!(parent[at], Atom::Node(Kind::Delim(k), _) if k == d) {
                // | solo cierra si el cursor está al final del contenido.
                if d == Delim::Abs && (depth + 1 != self.cur.path.len() || self.cur.pos != self.row().len()) {
                    return false;
                }
                self.cur.path.truncate(depth);
                self.cur.pos = at + 1;
                return true;
            }
        }
        false
    }

    /// Palabras que se convierten al escribirlas: sqrt → √, sum → ∑, pi → π…
    fn auto_command(&mut self) {
        let (start, word) = letters_before(self.row(), self.cur.pos);
        let replace = |s: &mut Self, a: Atom| {
            let pos = s.cur.pos;
            s.row_mut().splice(start..pos, [a]);
            s.cur.pos = start + 1;
        };
        match word.as_str() {
            "pi" => replace(self, Atom::Ch('π')),
            "theta" => replace(self, Atom::Ch('θ')),
            "tau" => replace(self, Atom::Ch('τ')),
            "sqrt" => {
                replace(self, empty_node(Kind::Sqrt));
                self.enter(start, 0, false);
            }
            "root" => {
                replace(self, empty_node(Kind::Root));
                self.enter(start, 0, false);
            }
            "sum" | "prod" => {
                let kind = if word == "sum" { Kind::Sum } else { Kind::Prod };
                replace(self, node(kind, vec![chars("n="), Vec::new(), Vec::new()]));
                self.enter(start, 0, true);
            }
            "int" => {
                replace(self, node(Kind::Int, vec![Vec::new(), Vec::new(), Vec::new(), chars("x")]));
                self.enter(start, 0, false);
            }
            _ => {}
        }
    }

    /// Inserta un texto del teclado en pantalla ("sqrt()", "sum(n, , 1, 10)"…). El cursor
    /// queda en la primera casilla vacía de lo insertado o, si no hay, `back` caracteres
    /// antes del final.
    pub fn insert_template(&mut self, text: &str, back: usize) {
        self.take_selection();
        let atoms = from_text(text);
        // Si lo insertado es solo texto plano, se escribe carácter a carácter (así "^" o
        // "/" crean su estructura y "pi" se convierte en π).
        if atoms.iter().all(|a| matches!(a, Atom::Ch(_))) && back == 0 {
            for c in text.chars() {
                self.insert_char(c);
            }
            return;
        }
        let start = self.cur.pos;
        let n = atoms.len();
        let pos = self.cur.pos;
        self.row_mut().splice(pos..pos, atoms);
        self.cur.pos = start + n;
        let base = self.cur.path.clone();
        // Primera casilla vacía dentro de lo insertado.
        let row = self.row().clone();
        for (i, a) in row.iter().enumerate().take(start + n).skip(start) {
            if let Some(mut p) = first_empty(a) {
                let mut path = base.clone();
                path.push((i, p.remove(0).1));
                path.extend(p);
                self.cur = Cursor { path, pos: 0 };
                return;
            }
        }
        if back > 0 {
            // Posición del cursor por caracteres: la marca del texto insertado que cae en
            // len - back.
            let mut w = Writer::default();
            let inserted: Row = row[start..start + n].to_vec();
            w.row(&inserted, &mut Vec::new());
            let target = w.len.saturating_sub(back);
            if let Some((p, pos, _)) = w.marks.iter().filter(|m| m.2 == target).max_by_key(|m| m.0.len()) {
                let mut path = base.clone();
                let mut p = p.clone();
                if p.is_empty() {
                    self.cur = Cursor { path, pos: start + pos };
                } else {
                    p[0].0 += start;
                    path.extend(p);
                    self.cur = Cursor { path, pos: *pos };
                }
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.take_selection().is_some() {
            return;
        }
        if self.cur.pos > 0 {
            let i = self.cur.pos - 1;
            match &self.row()[i] {
                Atom::Ch(_) => {
                    self.row_mut().remove(i);
                    self.cur.pos = i;
                }
                Atom::Node(_, rows) if rows.iter().all(Vec::is_empty) => {
                    self.row_mut().remove(i);
                    self.cur.pos = i;
                }
                Atom::Node(_, rows) => {
                    let last = rows.len() - 1;
                    // ∑ ∏ ∫: se entra al cuerpo; el resto, a su última casilla.
                    let c = if rows.len() >= 3 { 2 } else { last };
                    self.enter(i, c, true);
                }
            }
            return;
        }
        let Some(&(at, child)) = self.cur.path.last() else { return };
        let parent_path = self.cur.path[..self.cur.path.len() - 1].to_vec();
        let parent = row_at(&self.root, &parent_path);
        let Atom::Node(kind, rows) = &parent[at] else { return };
        let (kind, rows) = (*kind, rows.clone());
        if rows.iter().all(Vec::is_empty) {
            row_at_mut(&mut self.root, &parent_path).remove(at);
            self.cur = Cursor { path: parent_path, pos: at };
        } else if kind.unwraps() {
            // Se deshace la estructura y su contenido queda en su lugar.
            let before: usize = rows[..child].iter().map(Vec::len).sum();
            let flat: Row = rows.into_iter().flatten().collect();
            row_at_mut(&mut self.root, &parent_path).splice(at..at + 1, flat);
            self.cur = Cursor { path: parent_path, pos: at + before };
        } else if child > 0 {
            self.cur.path.pop();
            self.enter(at, child - 1, true);
        } else {
            self.cur = Cursor { path: parent_path, pos: at };
        }
    }

    pub fn delete(&mut self) {
        if self.take_selection().is_some() {
            return;
        }
        let pos = self.cur.pos;
        match self.row().get(pos) {
            Some(Atom::Ch(_)) => {
                self.row_mut().remove(pos);
            }
            Some(Atom::Node(_, rows)) if rows.iter().all(Vec::is_empty) => {
                self.row_mut().remove(pos);
            }
            Some(Atom::Node(..)) => self.enter(pos, 0, false),
            None => {}
        }
    }

    pub fn left(&mut self, shift: bool) {
        if shift {
            self.anchor.get_or_insert(self.cur.pos);
            self.cur.pos = self.cur.pos.saturating_sub(1);
            return;
        }
        if let Some((lo, _)) = self.selection() {
            self.anchor = None;
            self.cur.pos = lo;
            return;
        }
        self.anchor = None;
        if self.cur.pos > 0 {
            let i = self.cur.pos - 1;
            if let Atom::Node(_, rows) = &self.row()[i] {
                let last = rows.len() - 1;
                self.enter(i, last, true);
            } else {
                self.cur.pos = i;
            }
        } else if let Some(&(at, child)) = self.cur.path.last() {
            self.cur.path.pop();
            if child > 0 {
                self.enter(at, child - 1, true);
            } else {
                self.cur.pos = at;
            }
        }
    }

    pub fn right(&mut self, shift: bool) {
        if shift {
            self.anchor.get_or_insert(self.cur.pos);
            self.cur.pos = (self.cur.pos + 1).min(self.row().len());
            return;
        }
        if let Some((_, hi)) = self.selection() {
            self.anchor = None;
            self.cur.pos = hi;
            return;
        }
        self.anchor = None;
        let pos = self.cur.pos;
        if pos < self.row().len() {
            if let Atom::Node(..) = &self.row()[pos] {
                self.enter(pos, 0, false);
            } else {
                self.cur.pos += 1;
            }
        } else if let Some(&(at, child)) = self.cur.path.last() {
            self.cur.path.pop();
            let arity = match &self.row()[at] {
                Atom::Node(_, rows) => rows.len(),
                Atom::Ch(_) => 1,
            };
            if child + 1 < arity {
                self.enter(at, child + 1, false);
            } else {
                self.cur.pos = at + 1;
            }
        }
    }

    /// Flecha ↑ (`up`) o ↓: a la casilla de encima/debajo de la estructura más cercana.
    /// Devuelve false si no hay ninguna (entonces se pasa a otra línea).
    fn vertical(&mut self, up: bool) -> bool {
        let x = self.caret_x();
        for depth in (0..self.cur.path.len()).rev() {
            let (at, child) = self.cur.path[depth];
            let parent = row_at(&self.root, &self.cur.path[..depth]);
            let Atom::Node(kind, _) = parent[at] else { continue };
            let target = if up { kind.above(child) } else { kind.below(child) };
            if let Some(c) = target {
                self.cur.path.truncate(depth);
                self.cur.path.push((at, c));
                self.anchor = None;
                self.cur.pos = self.nearest_pos(&self.cur.path.clone(), x);
                return true;
            }
        }
        false
    }

    fn geo_of(&self, path: &[(usize, usize)]) -> Option<&RowGeo> {
        self.geo.iter().find(|g| g.path == path)
    }

    fn caret_x(&self) -> f32 {
        self.geo_of(&self.cur.path).and_then(|g| g.xs.get(self.cur.pos).copied()).unwrap_or(0.0)
    }

    fn nearest_pos(&self, path: &[(usize, usize)], x: f32) -> usize {
        let len = row_at(&self.root, path).len();
        match self.geo_of(path) {
            Some(g) if !g.xs.is_empty() => {
                (0..g.xs.len()).min_by(|a, b| (g.xs[*a] - x).abs().total_cmp(&(g.xs[*b] - x).abs())).unwrap_or(0).min(len)
            }
            _ => len,
        }
    }

    /// Posición del cursor más cercana a un punto (relativo al origen del dibujo).
    fn hit(&self, p: Vec2) -> Cursor {
        let mut best: Option<&RowGeo> = None;
        for g in &self.geo {
            let (x0, x1) = (g.xs[0] - 4.0, g.xs[g.xs.len() - 1] + 4.0);
            let inside = p.x >= x0 && p.x <= x1 && p.y >= g.y - g.asc - 2.0 && p.y <= g.y + g.desc + 2.0;
            // La fila más pequeña que contiene el punto es la más profunda.
            if inside && best.is_none_or(|b| g.asc + g.desc < b.asc + b.desc) {
                best = Some(g);
            }
        }
        match best {
            Some(g) => Cursor { path: g.path.clone(), pos: self.nearest_pos(&g.path, p.x) },
            None => Cursor { path: Vec::new(), pos: self.nearest_pos(&[], p.x) },
        }
    }

    fn select_all(&mut self) {
        self.cur = Cursor { path: Vec::new(), pos: self.root.len() };
        self.anchor = Some(0);
    }

    fn selected_text(&self) -> Option<String> {
        let (lo, hi) = self.selection()?;
        Some(to_text(&self.row()[lo..hi]))
    }

    fn paste(&mut self, s: &str) {
        self.take_selection();
        let atoms = parse_any(s.trim());
        let n = atoms.len();
        let pos = self.cur.pos;
        self.row_mut().splice(pos..pos, atoms);
        self.cur.pos += n;
    }

    /// Siguiente casilla vacía (Tab), recorriendo toda la expresión.
    fn next_slot(&mut self) {
        let mut all = Vec::new();
        collect_slots(&self.root, &mut Vec::new(), &mut all);
        if let Some(p) = all.iter().find(|p| **p > self.cur.path).or(all.first()) {
            self.cur = Cursor { path: p.clone(), pos: 0 };
            self.anchor = None;
        }
    }
}

/// Camino a la primera casilla vacía dentro de un átomo.
fn first_empty(a: &Atom) -> Option<Path> {
    let Atom::Node(_, rows) = a else { return None };
    for (c, r) in rows.iter().enumerate() {
        if r.is_empty() {
            return Some(vec![(0, c)]);
        }
        for (i, inner) in r.iter().enumerate() {
            if let Some(mut p) = first_empty(inner) {
                p[0].0 = i;
                let mut out = vec![(0, c)];
                out.extend(p);
                return Some(out);
            }
        }
    }
    None
}

fn collect_slots(row: &Row, path: &mut Path, out: &mut Vec<Path>) {
    for (i, a) in row.iter().enumerate() {
        if let Atom::Node(_, rows) = a {
            for (c, r) in rows.iter().enumerate() {
                path.push((i, c));
                if r.is_empty() {
                    out.push(path.clone());
                }
                collect_slots(r, path, out);
                path.pop();
            }
        }
    }
}

/// Texto de la app o LaTeX (que se traduce primero).
fn parse_any(text: &str) -> Row {
    if crate::latex::is_latex(text) {
        if let Ok(plain) = crate::latex::to_plain(text) {
            return from_text(&plain);
        }
    }
    from_text(text)
}

// ───────────────────────────── Widget ─────────────────────────────

/// Dibuja y edita una línea. `text` se actualiza con cada cambio.
pub fn show(ui: &mut egui::Ui, id: egui::Id, state: &mut State, text: &mut String, size: f32, hint: &str) -> (egui::Response, Output) {
    let mut out = Output::default();
    let color = ui.visuals().strong_text_color();
    let focused = ui.memory(|m| m.has_focus(id));

    // 1) Teclado (antes de componer, para dibujar ya el resultado).
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        let events = ui.input(|i| i.events.clone());
        let mut edited = false;
        for ev in events {
            match ev {
                Event::Text(s) | Event::Ime(egui::ImeEvent::Commit(s)) => {
                    for c in s.chars() {
                        state.insert_char(c);
                    }
                    edited = true;
                }
                Event::Paste(s) => {
                    state.paste(&s);
                    edited = true;
                }
                Event::Copy => {
                    if let Some(t) = state.selected_text() {
                        ui.ctx().copy_text(t);
                    }
                }
                Event::Cut => {
                    if let Some(t) = state.selected_text() {
                        ui.ctx().copy_text(t);
                        state.take_selection();
                        edited = true;
                    }
                }
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let shift = modifiers.shift;
                    match key {
                        Key::ArrowLeft => state.left(shift),
                        Key::ArrowRight => state.right(shift),
                        Key::ArrowUp => out.up |= !state.vertical(true),
                        Key::ArrowDown => out.down |= !state.vertical(false),
                        Key::Home => {
                            state.cur = Cursor::default();
                            state.anchor = None;
                        }
                        Key::End => state.cursor_to_end(),
                        Key::Backspace => {
                            state.backspace();
                            edited = true;
                        }
                        Key::Delete => {
                            state.delete();
                            edited = true;
                        }
                        Key::Tab => state.next_slot(),
                        Key::Enter => out.enter = true,
                        Key::Escape => ui.memory_mut(|m| m.surrender_focus(id)),
                        Key::A if modifiers.command => state.select_all(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if edited {
            out.changed = state.store(text);
        }
    }

    // 2) Composición.
    let painter = ui.painter().clone();
    let ts = Typesetter { comp: Composer { painter: &painter, color } };
    let boxed = ts.row(&state.root, &mut Vec::new(), size, false);
    let Boxed { lay, rows } = boxed;
    state.geo = rows;

    let pad = vec2(6.0, 5.0);
    let avail = ui.available_width();
    let h = (lay.height() + 2.0 * pad.y).max(size * 1.3 + 8.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(avail, h), Sense::click_and_drag());
    let resp = ui.interact(rect, id, Sense::click_and_drag()).union(resp);
    let inner_w = (rect.width() - 2.0 * pad.x).max(10.0);

    // El cursor siempre a la vista.
    if lay.w <= inner_w {
        state.scroll = 0.0;
    } else if focused {
        let cx = state.caret_x();
        if cx - state.scroll > inner_w - 4.0 {
            state.scroll = cx - inner_w + 4.0;
        } else if cx - state.scroll < 4.0 {
            state.scroll = (cx - 4.0).max(0.0);
        }
        state.scroll = state.scroll.min(lay.w - inner_w + 4.0);
    }
    let baseline = rect.center().y + (lay.asc - lay.desc) / 2.0;
    let origin = pos2(rect.left() + pad.x - state.scroll, baseline);

    // 3) Ratón: clic coloca el cursor; arrastrar selecciona (dentro de una misma fila).
    if let Some(p) = resp.interact_pointer_pos() {
        let rel = p - origin;
        if resp.drag_started() || resp.clicked() || (resp.is_pointer_button_down_on() && state.press.is_none()) {
            if !focused {
                resp.request_focus();
            }
            let c = state.hit(rel);
            state.cur = c.clone();
            state.anchor = None;
            state.press = Some(c);
        } else if resp.dragged() {
            if let Some(start) = state.press.clone() {
                let c = state.hit(rel);
                if c.path == start.path {
                    state.anchor = Some(start.pos);
                    state.cur = c;
                }
            }
        }
    }
    if !resp.is_pointer_button_down_on() {
        state.press = None;
    }
    if resp.clicked() && !focused {
        resp.request_focus();
    }
    // Clic fuera: se deja de editar (el cursor se recuerda para la próxima vez).
    if focused && resp.clicked_elsewhere() {
        ui.memory_mut(|m| m.surrender_focus(id));
    }

    // 4) Dibujo.
    let visuals = ui.visuals();
    ui.painter().rect_filled(rect, 4.0, visuals.text_edit_bg_color());
    let stroke = if focused {
        visuals.selection.stroke
    } else if resp.hovered() {
        visuals.widgets.hovered.bg_stroke
    } else {
        Stroke::NONE
    };
    ui.painter().rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
    let clip = ui.painter_at(rect.shrink(1.0));
    if state.root.is_empty() && !focused {
        clip.text(pos2(rect.left() + pad.x, rect.center().y), egui::Align2::LEFT_CENTER, hint, egui::FontId::proportional(size * 0.9), visuals.weak_text_color());
    }
    if focused {
        if let (Some((lo, hi)), Some(g)) = (state.selection(), state.geo_of(&state.cur.path)) {
            let r = Rect::from_min_max(origin + vec2(g.xs[lo], g.y - g.asc), origin + vec2(g.xs[hi], g.y + g.desc));
            clip.rect_filled(r, 2.0, visuals.selection.bg_fill.gamma_multiply(0.6));
        }
    }
    lay.paint(&clip, origin, color);
    if focused {
        if let Some(g) = state.geo_of(&state.cur.path) {
            let x = g.xs.get(state.cur.pos).copied().unwrap_or(0.0);
            let top = origin + vec2(x, g.y - g.asc);
            let bottom = origin + vec2(x, g.y + g.desc);
            clip.line_segment([top, bottom], Stroke::new(1.5, caret_color(ui)));
        }
    }
    (resp.on_hover_cursor(egui::CursorIcon::Text), out)
}

fn caret_color(ui: &egui::Ui) -> Color32 {
    ui.visuals().text_cursor.stroke.color
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn roundtrip(s: &str) -> String {
        to_text(&from_text(s))
    }

    /// El texto releído significa lo mismo que el original.
    fn same_meaning(s: &str) {
        let (f, n) = (HashSet::new(), HashSet::new());
        let back = roundtrip(s);
        // theta y θ son la misma variable.
        let show = |x| format!("{x:?}").replace("\"theta\"", "\"θ\"");
        let a = math::parse_statement(s, &f, &n).map(show);
        let b = math::parse_statement(&back, &f, &n).map(show);
        assert_eq!(a.is_ok(), b.is_ok(), "{s} → {back}");
        if a.is_ok() {
            assert_eq!(a, b, "{s} → {back}");
        }
    }

    #[test]
    fn text_roundtrip() {
        for s in [
            "y = x^2 - 3",
            "y = 1/2x",
            "y = (x+1)/(x-1)",
            "y = 2x/3",
            "a/bc",
            "x^ab",
            "2^-1",
            "x^sin(x)",
            "2^3!",
            "x^2^3",
            "1/2/3",
            "1/x^2",
            "y = sqrt(x) + root(3, x)",
            "sum(n, n^2, 1, 10)",
            "prod(k, 1 - 1/k^2, 2, 50)",
            "int(x, x^2, 0, 1)",
            "y = int(t, sin(t))",
            "y = d/dx(x^3)",
            "y = |x - 2|",
            "abs(x) + |y| = 1",
            "y = a_1 x + a_2",
            "x^2 + y^2 <= 9",
            "y = 2pi x",
            "r = theta",
            "L = [1, 2, 3]",
            "y = x {x > 0}",
            "f(x) = x^2/2 - 1",
            "y = f'(x)",
            "(cos(t), sin(t))",
            "y = sin x^2",
            "y = -x/2",
            "2*3",
            "y = e^(-x^2)",
            "y = (x + 2",
            "A = (1, 2)",
            "[(t, t^2) for t = [1...5]]",
        ] {
            same_meaning(s);
        }
    }

    #[test]
    fn help_examples_roundtrip() {
        let ex = crate::help::examples();
        assert!(ex.len() > 50);
        for s in ex {
            // Los de LaTeX se editan ya traducidos.
            let plain = if crate::latex::is_latex(s) { crate::latex::to_plain(s).unwrap_or_default() } else { s.to_string() };
            same_meaning(&plain);
        }
    }

    #[test]
    fn structures() {
        let r = from_text("sum(n, n^2, 1, 10)");
        assert!(matches!(&r[..], [Atom::Node(Kind::Sum, rows)] if rows[0] == chars("n=1") && rows[1] == chars("10")));
        let r = from_text("(x+1)/2");
        assert!(matches!(&r[..], [Atom::Node(Kind::Frac, rows)] if rows[0] == chars("x+1")));
        assert_eq!(from_text("2pi"), chars("2π"));
        assert_eq!(roundtrip("x^2"), "x^2");
        assert_eq!(roundtrip("y = (x+1)/(x-1)"), "y = (x+1)/(x-1)");
    }

    fn typed(keys: &str) -> State {
        let mut st = State::new("");
        for c in keys.chars() {
            st.insert_char(c);
        }
        st
    }

    #[test]
    fn typing() {
        // Fracción: lo anterior va al numerador y se sigue en el denominador.
        let mut st = typed("2x/3");
        assert_eq!(st.text(), "(2x)/3");
        st.right(false);
        st.insert_char('+');
        st.insert_char('1');
        assert_eq!(st.text(), "(2x)/3+1");
        // Exponente: se sale con →.
        let mut st = typed("x^2");
        st.right(false);
        st.insert_char('+');
        assert_eq!(st.text(), "x^2+");
        // sum crea la sumatoria con el cursor en el límite inferior (tras "n=").
        let mut st = typed("sum1");
        st.right(false);
        for c in "10".chars() {
            st.insert_char(c);
        }
        st.right(false);
        for c in "n^2".chars() {
            st.insert_char(c);
        }
        assert_eq!(st.text(), "sum(n, n^2, 1, 10)");
        assert!(!st.has_empty_slot());
        // Paréntesis: ) sale del paréntesis.
        let st = typed("sin(x)+1");
        assert_eq!(st.text(), "sin(x)+1");
        // <= y pi.
        assert_eq!(typed("y<=2pi").text(), "y≤2π");
        // sqrt.
        assert_eq!(typed("sqrtx").text(), "sqrt(x)");
        // Casillas vacías.
        assert!(typed("1/").has_empty_slot());
    }

    #[test]
    fn deleting() {
        // Borrar tras una fracción entra en ella; al principio de una casilla la deshace.
        let mut st = typed("1/2");
        st.right(false);
        st.backspace(); // entra al denominador
        st.backspace(); // borra el 2
        assert_eq!(st.text(), "1/()");
        st.backspace(); // denominador vacío al principio: se deshace la fracción
        assert_eq!(st.text(), "1");
        // Una estructura vacía se borra entera.
        let mut st = typed("sqrt");
        st.backspace();
        assert_eq!(st.text(), "");
    }

    #[test]
    fn templates() {
        let mut st = State::new("");
        st.insert_template("prod(n, , 1, 10)", 8);
        // El cursor queda en el cuerpo (la casilla vacía).
        st.insert_char('n');
        assert_eq!(st.text(), "prod(n, n, 1, 10)");
        let mut st = State::new("");
        st.insert_template("nCr(, )", 3);
        st.insert_char('5');
        assert_eq!(st.text(), "nCr(5, )");
        let mut st = State::new("y = ");
        st.insert_template("sqrt()", 1);
        st.insert_char('x');
        assert_eq!(st.text(), "y = sqrt(x)");
        let mut st = State::new("x");
        st.insert_template("^2", 0);
        assert_eq!(st.text(), "x^2");
    }

    fn key(k: Key) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// El widget con eventos reales: foco, escritura, flechas y clic dentro de una casilla.
    #[test]
    fn widget_events() {
        let id = egui::Id::new("prueba");
        let mut st = State::new("");
        let mut text = String::new();
        let text_ev = |s: &str| Event::Text(s.into());
        let mut frames = vec![vec![]];
        frames.push(vec![text_ev("y=x^2"), key(Key::ArrowRight), text_ev("+1/2")]);
        frames.push(vec![key(Key::ArrowRight), text_ev("x"), key(Key::ArrowUp)]);
        frames.push(vec![key(Key::ArrowLeft), key(Key::ArrowLeft), key(Key::ArrowUp), text_ev("3")]);
        let mut rect = Rect::NOTHING;
        let ctx = egui::Context::default();
        let mut outs = Vec::new();
        for (k, events) in frames.into_iter().enumerate() {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 200.0))), events, ..Default::default() };
            let mut o = ctx.run_ui(input, |ui| {
                if k == 0 {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                let (r, out) = show(ui, id, &mut st, &mut text, 20.0, "");
                rect = r.rect;
                outs.push(out);
            });
            o.textures_delta.clear();
        }
        // → al final del denominador sale de la fracción; ↑ fuera de toda estructura pide
        // pasar a la línea de arriba; ← ← entra al denominador y ↑ sube al numerador.
        assert!(outs[2].up);
        assert!(!outs[3].up);
        assert_eq!(text, "y=x^2+13/2x");

        // Clic sobre el numerador: el cursor queda ahí y se escribe dentro.
        let num = st.geo.iter().find(|g| g.path.len() == 1 && g.path[0].1 == 0).unwrap().clone();
        // Un clic fuera quita el foco.
        let outside = pos2(rect.center().x, rect.bottom() + 50.0);
        let ev = |pressed| Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for events in [vec![Event::PointerMoved(outside), ev(true)], vec![ev(false)], vec![]] {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 200.0))), events, ..Default::default() };
            let mut o = ctx.run_ui(input, |ui| {
                show(ui, id, &mut st, &mut text, 20.0, "");
            });
            o.textures_delta.clear();
        }
        assert!(!ctx.memory(|m| m.has_focus(id)));
        let c = st.hit(vec2(num.xs[0] + 1.0, num.y - 2.0));
        assert_eq!(c.path, num.path);
    }

    #[test]
    fn latex_input() {
        let st = State::new(r"\frac{x^2}{2}");
        assert!(crate::latex::is_latex(r"\frac{x^2}{2}"), "{:?}", crate::latex::to_plain(r"\frac{x^2}{2}"));
        assert!(matches!(&st.root[..], [Atom::Node(Kind::Frac, _)]), "{:?} {:?}", crate::latex::to_plain(r"\frac{x^2}{2}"), st.root);
        assert!(matches!(&st.root[..], [Atom::Node(Kind::Frac, _)]));
    }
}
