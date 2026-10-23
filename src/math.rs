//! Motor matemático: lexer, parser y evaluador de expresiones.
//!
//! Flujo: texto -> tokens -> `Expr` (AST con nombres) -> `Compiled`
//! (árbol resuelto, sin nombres: constantes sustituidas y funciones de
//! usuario expandidas en línea), que es lo que se evalúa miles de veces
//! por fotograma.

use std::collections::{HashMap, HashSet};
use std::fmt;

// ───────────────────────────── Errores ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct MathError(pub String);

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, MathError> {
    Err(MathError(msg.into()))
}

// ───────────────────────────── Lexer ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, MathError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let v = s.parse::<f64>().map_err(|_| MathError(format!("número inválido: {s}")))?;
            out.push(Tok::Num(v));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                // Los dígitos solo forman parte del nombre tras un '_' (a_1), así "2x3" o "x2" se leen como productos.
                if chars[i].is_ascii_digit() && !chars[start..i].contains(&'_') {
                    break;
                }
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            let op = match c {
                '·' | '×' => '*',
                '÷' => '/',
                '−' => '-',
                '√' => {
                    out.push(Tok::Ident("sqrt".into()));
                    i += 1;
                    continue;
                }
                '+' | '-' | '*' | '/' | '^' | '(' | ')' | ',' | '|' | '=' | '!' => c,
                _ => return err(format!("carácter inesperado '{c}'")),
            };
            out.push(Tok::Op(op));
            i += 1;
        }
    }
    Ok(out)
}

// ───────────────────────────── AST ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Func1 {
    Sin, Cos, Tan, Sec, Csc, Cot,
    Asin, Acos, Atan,
    Sinh, Cosh, Tanh,
    Sqrt, Cbrt, Abs, Ln, Log, Exp,
    Floor, Ceil, Round, Sign, Fact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Func2 {
    Add, Sub, Mul, Div, Pow,
    Min, Max, Mod, Atan2, LogBase, Root,
}

const FUNC1_NAMES: &[(&str, Func1)] = &[
    ("sin", Func1::Sin), ("sen", Func1::Sin), ("cos", Func1::Cos), ("tan", Func1::Tan),
    ("sec", Func1::Sec), ("csc", Func1::Csc), ("cot", Func1::Cot),
    ("asin", Func1::Asin), ("arcsin", Func1::Asin), ("acos", Func1::Acos), ("arccos", Func1::Acos),
    ("atan", Func1::Atan), ("arctan", Func1::Atan),
    ("sinh", Func1::Sinh), ("cosh", Func1::Cosh), ("tanh", Func1::Tanh),
    ("sqrt", Func1::Sqrt), ("cbrt", Func1::Cbrt), ("abs", Func1::Abs),
    ("ln", Func1::Ln), ("log", Func1::Log), ("exp", Func1::Exp),
    ("floor", Func1::Floor), ("ceil", Func1::Ceil), ("round", Func1::Round), ("sign", Func1::Sign),
];

const FUNC2_NAMES: &[(&str, Func2)] = &[
    ("min", Func2::Min), ("max", Func2::Max), ("mod", Func2::Mod),
    ("atan2", Func2::Atan2), ("logb", Func2::LogBase), ("root", Func2::Root),
];

const CONSTANTS: &[(&str, f64)] = &[
    ("pi", std::f64::consts::PI),
    ("π", std::f64::consts::PI),
    ("tau", std::f64::consts::TAU),
    ("τ", std::f64::consts::TAU),
    ("e", std::f64::consts::E),
];

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    F1(Func1, Box<Expr>),
    F2(Func2, Box<Expr>, Box<Expr>),
    /// Llamada a una función definida por el usuario: f(x+1).
    Call(String, Vec<Expr>),
    /// Par ordenado (punto).
    Tuple(Vec<Expr>),
}

impl Expr {
    fn bin(op: Func2, a: Expr, b: Expr) -> Expr {
        Expr::F2(op, Box::new(a), Box::new(b))
    }
}

// ───────────────────────────── Parser ─────────────────────────────

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    /// Nombres de funciones de usuario conocidas (para distinguir f(x) de f·(x)).
    user_funcs: &'a HashSet<String>,
    /// Nombres multiletra conocidos (constantes / funciones de usuario).
    user_names: &'a HashSet<String>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: char) -> Result<(), MathError> {
        if self.eat(op) {
            Ok(())
        } else {
            err(format!("se esperaba '{op}'"))
        }
    }

    /// Divide identificadores como "ax" o "xsin" en piezas conocidas,
    /// al estilo Desmos: se toma el nombre conocido más largo y, si no hay,
    /// una sola letra.
    fn split_idents(&mut self) {
        let mut out = Vec::with_capacity(self.toks.len());
        for t in self.toks.drain(..) {
            let Tok::Ident(word) = t else {
                out.push(t);
                continue;
            };
            if word.contains('_') || self.user_names.contains(&word) || is_builtin_name(&word) {
                out.push(Tok::Ident(word));
                continue;
            }
            let chars: Vec<char> = word.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let mut taken = 1;
                for len in (2..=chars.len() - i).rev() {
                    let piece: String = chars[i..i + len].iter().collect();
                    if self.user_names.contains(&piece) || is_builtin_name(&piece) {
                        taken = len;
                        break;
                    }
                }
                out.push(Tok::Ident(chars[i..i + taken].iter().collect()));
                i += taken;
            }
        }
        self.toks = out;
    }

    fn parse_expr(&mut self) -> Result<Expr, MathError> {
        let mut lhs = self.parse_term()?;
        loop {
            if self.eat('+') {
                lhs = Expr::bin(Func2::Add, lhs, self.parse_term()?);
            } else if self.eat('-') {
                lhs = Expr::bin(Func2::Sub, lhs, self.parse_term()?);
            } else {
                return Ok(lhs);
            }
        }
    }

    /// ¿El siguiente token puede empezar un factor (para multiplicación implícita)?
    fn starts_factor(&self) -> bool {
        matches!(self.peek(), Some(Tok::Num(_)) | Some(Tok::Ident(_)) | Some(Tok::Op('(')))
    }

    fn parse_term(&mut self) -> Result<Expr, MathError> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.eat('*') {
                lhs = Expr::bin(Func2::Mul, lhs, self.parse_unary()?);
            } else if self.eat('/') {
                lhs = Expr::bin(Func2::Div, lhs, self.parse_unary()?);
            } else if self.starts_factor() {
                lhs = Expr::bin(Func2::Mul, lhs, self.parse_power()?);
            } else {
                return Ok(lhs);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, MathError> {
        if self.eat('-') {
            Ok(Expr::Neg(Box::new(self.parse_unary()?)))
        } else if self.eat('+') {
            self.parse_unary()
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Result<Expr, MathError> {
        let base = self.parse_postfix()?;
        if self.eat('^') {
            // Asociativa por la derecha y admite signo: 2^-x, 2^3^2.
            let exp = self.parse_unary()?;
            Ok(Expr::bin(Func2::Pow, base, exp))
        } else {
            Ok(base)
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, MathError> {
        let mut e = self.parse_primary()?;
        while self.eat('!') {
            e = Expr::F1(Func1::Fact, Box::new(e));
        }
        Ok(e)
    }

    /// Argumento de una función escrita sin paréntesis: "sin 2x" = sin(2x).
    fn parse_bare_arg(&mut self) -> Result<Expr, MathError> {
        let mut e = self.parse_power()?;
        while matches!(self.peek(), Some(Tok::Num(_)) | Some(Tok::Ident(_))) {
            if let Some(Tok::Ident(name)) = self.peek() {
                if is_builtin_func(name) {
                    break;
                }
            }
            e = Expr::bin(Func2::Mul, e, self.parse_power()?);
        }
        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, MathError> {
        self.expect('(')?;
        let mut args = vec![self.parse_expr()?];
        while self.eat(',') {
            args.push(self.parse_expr()?);
        }
        self.expect(')')?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, MathError> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(Expr::Num(v)),
            Some(Tok::Op('(')) => {
                let mut items = vec![self.parse_expr()?];
                while self.eat(',') {
                    items.push(self.parse_expr()?);
                }
                self.expect(')')?;
                Ok(if items.len() == 1 { items.pop().unwrap() } else { Expr::Tuple(items) })
            }
            Some(Tok::Op('|')) => {
                let inner = self.parse_expr()?;
                self.expect('|')?;
                Ok(Expr::F1(Func1::Abs, Box::new(inner)))
            }
            Some(Tok::Ident(name)) => self.parse_ident(name),
            Some(Tok::Op(c)) => err(format!("'{c}' inesperado")),
            None => err("expresión incompleta"),
        }
    }

    fn parse_ident(&mut self, name: String) -> Result<Expr, MathError> {
        let has_paren = self.peek() == Some(&Tok::Op('('));

        if let Some(&(_, f)) = FUNC1_NAMES.iter().find(|(n, _)| *n == name) {
            // sin^2(x) = (sin x)^2
            let power = if self.eat('^') { Some(self.parse_postfix()?) } else { None };
            let arg = if self.peek() == Some(&Tok::Op('(')) {
                let mut args = self.parse_args()?;
                match (f, args.len()) {
                    (_, 1) => args.pop().unwrap(),
                    // log(b, x): logaritmo en base b
                    (Func1::Log, 2) => {
                        let x = args.pop().unwrap();
                        let b = args.pop().unwrap();
                        let e = Expr::bin(Func2::LogBase, b, x);
                        return Ok(apply_power(e, power));
                    }
                    _ => return err(format!("{name} recibe 1 argumento")),
                }
            } else {
                self.parse_bare_arg()?
            };
            return Ok(apply_power(Expr::F1(f, Box::new(arg)), power));
        }

        if let Some(&(_, f)) = FUNC2_NAMES.iter().find(|(n, _)| *n == name) {
            let args = self.parse_args()?;
            if args.len() != 2 {
                return err(format!("{name} recibe 2 argumentos"));
            }
            let mut it = args.into_iter();
            return Ok(Expr::F2(f, Box::new(it.next().unwrap()), Box::new(it.next().unwrap())));
        }

        if has_paren && self.user_funcs.contains(&name) {
            let args = self.parse_args()?;
            return Ok(Expr::Call(name, args));
        }

        if let Some(&(_, v)) = CONSTANTS.iter().find(|(n, _)| *n == name) {
            if !self.user_names.contains(&name) {
                return Ok(Expr::Num(v));
            }
        }

        Ok(Expr::Var(name))
    }
}

fn apply_power(e: Expr, power: Option<Expr>) -> Expr {
    match power {
        Some(p) => Expr::bin(Func2::Pow, e, p),
        None => e,
    }
}

fn is_builtin_func(name: &str) -> bool {
    FUNC1_NAMES.iter().any(|(n, _)| *n == name) || FUNC2_NAMES.iter().any(|(n, _)| *n == name)
}

fn is_builtin_name(name: &str) -> bool {
    is_builtin_func(name) || CONSTANTS.iter().any(|(n, _)| *n == name)
}

pub fn parse_expr(src: &str, user_funcs: &HashSet<String>, user_names: &HashSet<String>) -> Result<Expr, MathError> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return err("expresión vacía");
    }
    let mut p = Parser { toks, pos: 0, user_funcs, user_names };
    p.split_idents();
    let e = p.parse_expr()?;
    match p.peek() {
        None => Ok(e),
        Some(Tok::Op(')')) => err("paréntesis ')' de más"),
        Some(t) => err(format!("no se esperaba {t:?}")),
    }
}

// ───────────────────────────── Sentencias ─────────────────────────────

/// Lo que representa una línea del panel de expresiones.
#[derive(Debug, Clone)]
pub enum Statement {
    /// `nombre = expr` sin x ni y: constante (o deslizador si es un número).
    Constant { name: String, expr: Expr },
    /// `f(a, b) = expr`
    FuncDef { name: String, params: Vec<String>, body: Expr },
    /// `lhs = rhs` (incluye y = f(x), x = g(y) e implícitas).
    Equation { lhs: Expr, rhs: Expr },
    /// `lhs < rhs`, `lhs >= rhs`, … (región sombreada).
    Inequality { lhs: Expr, rhs: Expr, cmp: Comparison },
    /// Expresión suelta: si depende de x se grafica como y = expr; si no, se evalúa.
    Bare(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// `lhs < rhs` o `lhs <= rhs` (si es falso: `>` o `>=`).
    pub less: bool,
    /// `<` / `>` (sin igualdad): el borde se dibuja discontinuo.
    pub strict: bool,
}

/// Separa `lhs <op> rhs` si el texto contiene un operador de comparación.
fn split_comparison(src: &str) -> Option<(&str, &str, Comparison)> {
    let ops: [(&str, bool, bool); 6] = [
        ("<=", true, false), (">=", false, false), ("≤", true, false), ("≥", false, false), ("<", true, true), (">", false, true),
    ];
    let (pos, op, less, strict) = ops
        .iter()
        .filter_map(|&(op, less, strict)| src.find(op).map(|p| (p, op, less, strict)))
        // El primero en el texto; ante empate ("<=" vs "<") gana el más largo.
        .min_by_key(|&(p, op, ..)| (p, usize::MAX - op.len()))?;
    Some((&src[..pos], &src[pos + op.len()..], Comparison { less, strict }))
}

/// Primera pasada sobre el texto: detecta qué nombres se definen en `nombre = …`
/// o `nombre(…) = …`, para que el parser los reconozca en las demás líneas.
pub fn scan_definition(src: &str) -> Option<(String, bool)> {
    let (lhs, _) = src.split_once('=')?;
    let lhs = lhs.trim();
    let (name, is_func) = match lhs.find('(') {
        Some(i) if lhs.ends_with(')') => (lhs[..i].trim(), true),
        Some(_) => return None,
        None => (lhs, false),
    };
    let valid = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_alphabetic())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !valid || name == "x" || name == "y" || is_builtin_name(name) {
        return None;
    }
    Some((name.to_string(), is_func))
}

pub fn parse_statement(src: &str, user_funcs: &HashSet<String>, user_names: &HashSet<String>) -> Result<Statement, MathError> {
    if let Some((l, r, cmp)) = split_comparison(src) {
        if r.contains(['<', '>', '≤', '≥', '=']) {
            return err("solo se admite una comparación por línea");
        }
        let lhs = parse_expr(l, user_funcs, user_names)?;
        let rhs = parse_expr(r, user_funcs, user_names)?;
        return Ok(Statement::Inequality { lhs, rhs, cmp });
    }
    let parts: Vec<&str> = src.split('=').collect();
    match parts.len() {
        1 => Ok(Statement::Bare(parse_expr(src, user_funcs, user_names)?)),
        2 => {
            if let Some((name, is_func)) = scan_definition(src) {
                let body = parse_expr(parts[1], user_funcs, user_names)?;
                if is_func {
                    let lhs = parts[0].trim();
                    let inner = &lhs[lhs.find('(').unwrap() + 1..lhs.len() - 1];
                    let params: Vec<String> = inner.split(',').map(|s| s.trim().to_string()).collect();
                    if params.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_alphanumeric() || c == '_')) {
                        return err("parámetros inválidos");
                    }
                    return Ok(Statement::FuncDef { name, params, body });
                }
                return Ok(Statement::Constant { name, expr: body });
            }
            let lhs = parse_expr(parts[0], user_funcs, user_names)?;
            let rhs = parse_expr(parts[1], user_funcs, user_names)?;
            Ok(Statement::Equation { lhs, rhs })
        }
        _ => err("demasiados '='"),
    }
}

// ───────────────────────────── Compilación ─────────────────────────────

/// Árbol listo para evaluar: solo depende de x e y.
#[derive(Debug, Clone)]
pub enum Compiled {
    Num(f64),
    X,
    Y,
    Neg(Box<Compiled>),
    F1(Func1, Box<Compiled>),
    F2(Func2, Box<Compiled>, Box<Compiled>),
}

impl Compiled {
    #[inline]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        match self {
            Compiled::Num(v) => *v,
            Compiled::X => x,
            Compiled::Y => y,
            Compiled::Neg(a) => -a.eval(x, y),
            Compiled::F1(f, a) => apply1(*f, a.eval(x, y)),
            Compiled::F2(f, a, b) => apply2(*f, a.eval(x, y), b.eval(x, y)),
        }
    }

    pub fn uses_x(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::X))
    }

    pub fn uses_y(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::Y))
    }

    fn any(&self, pred: &dyn Fn(&Compiled) -> bool) -> bool {
        pred(self)
            || match self {
                Compiled::Neg(a) | Compiled::F1(_, a) => a.any(pred),
                Compiled::F2(_, a, b) => a.any(pred) || b.any(pred),
                _ => false,
            }
    }
}

#[inline]
fn apply1(f: Func1, v: f64) -> f64 {
    match f {
        Func1::Sin => v.sin(),
        Func1::Cos => v.cos(),
        Func1::Tan => v.tan(),
        Func1::Sec => 1.0 / v.cos(),
        Func1::Csc => 1.0 / v.sin(),
        Func1::Cot => 1.0 / v.tan(),
        Func1::Asin => v.asin(),
        Func1::Acos => v.acos(),
        Func1::Atan => v.atan(),
        Func1::Sinh => v.sinh(),
        Func1::Cosh => v.cosh(),
        Func1::Tanh => v.tanh(),
        Func1::Sqrt => v.sqrt(),
        Func1::Cbrt => v.cbrt(),
        Func1::Abs => v.abs(),
        Func1::Ln => v.ln(),
        Func1::Log => v.log10(),
        Func1::Exp => v.exp(),
        Func1::Floor => v.floor(),
        Func1::Ceil => v.ceil(),
        Func1::Round => v.round(),
        Func1::Sign => {
            if v == 0.0 { 0.0 } else { v.signum() }
        }
        Func1::Fact => gamma(v + 1.0),
    }
}

#[inline]
fn apply2(f: Func2, a: f64, b: f64) -> f64 {
    match f {
        Func2::Add => a + b,
        Func2::Sub => a - b,
        Func2::Mul => a * b,
        Func2::Div => a / b,
        Func2::Pow => pow(a, b),
        Func2::Min => a.min(b),
        Func2::Max => a.max(b),
        Func2::Mod => a.rem_euclid(b),
        Func2::Atan2 => a.atan2(b),
        Func2::LogBase => b.ln() / a.ln(),
        Func2::Root => pow(b, 1.0 / a),
    }
}

/// Potencia que admite bases negativas con exponentes racionales de denominador
/// impar, p. ej. x^(1/3) para x < 0.
fn pow(a: f64, b: f64) -> f64 {
    if a >= 0.0 || b.fract() == 0.0 {
        return a.powf(b);
    }
    for den in [3.0, 5.0, 7.0, 9.0] {
        let num = b * den;
        if (num - num.round()).abs() < 1e-9 {
            let r = (-a).powf(b);
            return if (num.round() as i64) % 2 == 0 { r } else { -r };
        }
    }
    f64::NAN
}

/// Función gamma (aproximación de Lanczos), para el factorial.
fn gamma(x: f64) -> f64 {
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x));
    }
    const G: [f64; 9] = [
        0.999_999_999_999_809_9, 676.520_368_121_885_1, -1_259.139_216_722_402_8,
        771.323_428_777_653_1, -176.615_029_162_140_6, 12.507_343_278_686_905,
        -0.138_571_095_265_720_12, 9.984_369_578_019_572e-6, 1.505_632_735_149_311_6e-7,
    ];
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

/// Entorno de definiciones globales usado al compilar.
#[derive(Default)]
pub struct Env {
    pub constants: HashMap<String, f64>,
    pub funcs: HashMap<String, (Vec<String>, Expr)>,
}

pub fn compile(e: &Expr, env: &Env) -> Result<Compiled, MathError> {
    compile_scoped(e, env, &HashMap::new(), 0)
}

fn compile_scoped(e: &Expr, env: &Env, locals: &HashMap<String, Compiled>, depth: u32) -> Result<Compiled, MathError> {
    if depth > 64 {
        return err("definición recursiva");
    }
    let c = match e {
        Expr::Num(v) => Compiled::Num(*v),
        Expr::Var(name) => {
            if let Some(c) = locals.get(name) {
                c.clone()
            } else if name == "x" {
                Compiled::X
            } else if name == "y" {
                Compiled::Y
            } else if let Some(v) = env.constants.get(name) {
                Compiled::Num(*v)
            } else if env.funcs.contains_key(name) {
                return err(format!("'{name}' es una función: usa {name}(…)"));
            } else {
                return err(format!("'{name}' no está definido"));
            }
        }
        Expr::Neg(a) => Compiled::Neg(Box::new(compile_scoped(a, env, locals, depth)?)),
        Expr::F1(f, a) => Compiled::F1(*f, Box::new(compile_scoped(a, env, locals, depth)?)),
        Expr::F2(f, a, b) => Compiled::F2(
            *f,
            Box::new(compile_scoped(a, env, locals, depth)?),
            Box::new(compile_scoped(b, env, locals, depth)?),
        ),
        Expr::Call(name, args) => {
            let Some((params, body)) = env.funcs.get(name) else {
                return err(format!("función '{name}' no definida"));
            };
            if params.len() != args.len() {
                return err(format!("{name} espera {} argumento(s)", params.len()));
            }
            let mut inner = HashMap::new();
            for (p, a) in params.iter().zip(args) {
                inner.insert(p.clone(), compile_scoped(a, env, locals, depth)?);
            }
            compile_scoped(body, env, &inner, depth + 1)?
        }
        Expr::Tuple(_) => return err("los puntos (a, b) solo pueden ir solos en una línea"),
    };
    Ok(fold(c))
}

/// Plegado de constantes: evalúa subárboles que no dependen de x ni y.
fn fold(c: Compiled) -> Compiled {
    let constant = match &c {
        Compiled::Neg(a) | Compiled::F1(_, a) => matches!(**a, Compiled::Num(_)),
        Compiled::F2(_, a, b) => matches!(**a, Compiled::Num(_)) && matches!(**b, Compiled::Num(_)),
        _ => false,
    };
    if constant { Compiled::Num(c.eval(0.0, 0.0)) } else { c }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(src: &str, x: f64) -> f64 {
        let none = HashSet::new();
        let e = parse_expr(src, &none, &none).unwrap();
        compile(&e, &Env::default()).unwrap().eval(x, 0.0)
    }

    #[test]
    fn precedence_and_implicit_mult() {
        assert_eq!(ev("1 + 2*3", 0.0), 7.0);
        assert_eq!(ev("-x^2", 3.0), -9.0);
        assert_eq!(ev("2x", 4.0), 8.0);
        assert_eq!(ev("2^3^2", 0.0), 512.0);
        assert_eq!(ev("(x+1)(x-1)", 3.0), 8.0);
        assert_eq!(ev("2^-1", 0.0), 0.5);
        assert_eq!(ev("|x - 5|", 2.0), 3.0);
        assert_eq!(ev("3!", 0.0).round(), 6.0);
        assert!((ev("2pi", 0.0) - std::f64::consts::TAU).abs() < 1e-12);
        assert!((ev("sin 2x", 1.0) - 2f64.sin()).abs() < 1e-12);
        assert!((ev("sin^2(x) + cos^2(x)", 0.7) - 1.0).abs() < 1e-12);
        assert!((ev("x^(1/3)", -8.0) + 2.0).abs() < 1e-12);
        assert!((ev("log(2, 8)", 0.0) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn inequalities() {
        let none = HashSet::new();
        for (src, less, strict) in [("y < x^2", true, true), ("y >= 2", false, false), ("x^2+y^2 ≤ 4", true, false), ("y>x", false, true)] {
            let Statement::Inequality { cmp, .. } = parse_statement(src, &none, &none).unwrap() else { panic!("{src}") };
            assert_eq!(cmp, Comparison { less, strict }, "{src}");
        }
        assert!(parse_statement("1 < x < 3", &none, &none).is_err());
    }

    #[test]
    fn user_definitions() {
        let funcs: HashSet<String> = ["f".to_string()].into();
        let names: HashSet<String> = ["f".to_string(), "a".to_string()].into();
        let mut env = Env::default();
        env.constants.insert("a".into(), 2.0);
        let Statement::FuncDef { name, params, body } = parse_statement("f(t) = a t^2", &funcs, &names).unwrap() else {
            panic!()
        };
        env.funcs.insert(name, (params, body));
        let e = parse_expr("f(x+1)", &funcs, &names).unwrap();
        assert_eq!(compile(&e, &env).unwrap().eval(1.0, 0.0), 8.0);
    }
}
