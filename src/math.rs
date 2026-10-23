//! Motor matemático: lexer, parser y evaluador de expresiones.
//!
//! Flujo: texto -> tokens -> `Expr` (AST con nombres) -> `Compiled`
//! (árbol resuelto, sin nombres: constantes sustituidas y funciones de
//! usuario expandidas en línea), que es lo que se evalúa miles de veces
//! por fotograma.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::special;

pub use num_complex::Complex64 as C64;

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
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Un único punto decimal, y solo si le sigue un dígito: así "1...5" es 1 … 5.
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v = s.parse::<f64>().map_err(|_| MathError(format!("número inválido: {s}")))?;
            out.push(Tok::Num(v));
        } else if chars[i..].starts_with(&['d', '/', 'd', 'x']) && !chars.get(i + 4).is_some_and(|c| c.is_alphanumeric()) {
            // d/dx(…): derivada respecto de x.
            out.push(Tok::Ident("deriv".into()));
            i += 4;
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
                '√' | '∑' | '∏' | '∫' | '∞' => {
                    let name = match c {
                        '√' => "sqrt",
                        '∑' => "sum",
                        '∏' => "prod",
                        '∫' => "int",
                        _ => "inf",
                    };
                    out.push(Tok::Ident(name.into()));
                    i += 1;
                    continue;
                }
                '\'' | '′' => '\'',
                '.' if chars[i..].starts_with(&['.', '.', '.']) => {
                    out.push(Tok::Op('…'));
                    i += 3;
                    continue;
                }
                // Campo de un punto con nombre: A.x, A.y
                '.' if chars.get(i + 1).is_some_and(|d| d.is_alphabetic()) => '.',
                '+' | '-' | '*' | '/' | '^' | '(' | ')' | ',' | '|' | '=' | '!' | '[' | ']' | '…' | ';' => c,
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
    Asec, Acsc, Acot,
    Asinh, Acosh, Atanh, Sech, Csch, Coth,
    Gamma, LnGamma, Erf, Erfc, LambertW, Trunc,
    /// Parte real, parte imaginaria, argumento y conjugado de un complejo.
    Re, Im, Arg, Conj,
    /// Módulo de un complejo (|z|); el compilador lo usa en vez de Abs si hace falta.
    CAbs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Func2 {
    Add, Sub, Mul, Div, Pow,
    Min, Max, Mod, Atan2, LogBase, Root,
    NCr, NPr, Gcd, Lcm, Beta,
    /// W(k, x): rama k (0 o −1) de la función W de Lambert.
    LambertWk,
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
    ("asec", Func1::Asec), ("arcsec", Func1::Asec), ("acsc", Func1::Acsc), ("arccsc", Func1::Acsc),
    ("acot", Func1::Acot), ("arccot", Func1::Acot),
    ("asinh", Func1::Asinh), ("arcsinh", Func1::Asinh), ("arsinh", Func1::Asinh),
    ("acosh", Func1::Acosh), ("arccosh", Func1::Acosh), ("arcosh", Func1::Acosh),
    ("atanh", Func1::Atanh), ("arctanh", Func1::Atanh), ("artanh", Func1::Atanh),
    ("sech", Func1::Sech), ("csch", Func1::Csch), ("coth", Func1::Coth),
    ("gamma", Func1::Gamma), ("Γ", Func1::Gamma), ("lgamma", Func1::LnGamma), ("lngamma", Func1::LnGamma),
    ("erf", Func1::Erf), ("erfc", Func1::Erfc), ("lambertw", Func1::LambertW), ("trunc", Func1::Trunc),
    ("re", Func1::Re), ("real", Func1::Re), ("im", Func1::Im), ("imag", Func1::Im),
    ("arg", Func1::Arg), ("conj", Func1::Conj),
];

const FUNC2_NAMES: &[(&str, Func2)] = &[
    ("min", Func2::Min), ("max", Func2::Max), ("mod", Func2::Mod),
    ("atan2", Func2::Atan2), ("logb", Func2::LogBase), ("root", Func2::Root),
    ("nCr", Func2::NCr), ("comb", Func2::NCr), ("binom", Func2::NCr), ("nPr", Func2::NPr), ("perm", Func2::NPr),
    ("gcd", Func2::Gcd), ("mcd", Func2::Gcd), ("lcm", Func2::Lcm), ("mcm", Func2::Lcm), ("beta", Func2::Beta),
];

const CONSTANTS: &[(&str, f64)] = &[
    ("pi", std::f64::consts::PI),
    ("π", std::f64::consts::PI),
    ("tau", std::f64::consts::TAU),
    ("τ", std::f64::consts::TAU),
    ("e", std::f64::consts::E),
    ("inf", f64::INFINITY),
];

/// Funciones con sintaxis propia (no van en las tablas de arriba).
const SPECIAL_FUNCS: &[&str] = &["sum", "prod", "int", "integral", "deriv", "diff"];

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
    /// Expresión con dominio restringido: `x^2 {x > 0}` (en definiciones de funciones).
    Restrict(Box<Expr>, Vec<Restriction>),
    /// Lista `[a, b, c]`.
    List(Vec<Expr>),
    /// Rango `[inicio ... fin]` o `[inicio, segundo ... fin]` (el paso es segundo − inicio).
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    /// `cuerpo for var = lista`: el cuerpo evaluado para cada valor de la lista.
    For { body: Box<Expr>, var: String, list: Box<Expr> },
    /// Sumatoria, productoria o integral de `body` con `var` entre `from` y `to`.
    Big { op: BigOp, var: String, from: Box<Expr>, to: Box<Expr>, body: Box<Expr> },
    /// Derivada de `body` respecto de x, de orden `order`: d/dx(…).
    Deriv { body: Box<Expr>, order: u32 },
    /// f'(arg), f''(arg)…: derivada de una función de usuario evaluada en `arg`.
    Prime { name: String, order: u32, arg: Box<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigOp {
    Sum,
    Prod,
    Int,
}

/// Condición `lhs <op> rhs` de una restricción `{…}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Restriction {
    pub lhs: Expr,
    pub rhs: Expr,
    pub cmp: Comparison,
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
        loop {
            if self.eat('!') {
                e = Expr::F1(Func1::Fact, Box::new(e));
            } else if self.eat('.') {
                // A.x / A.y: coordenada de un punto con nombre.
                let (Expr::Var(name), Some(Tok::Ident(field))) = (&e, self.next()) else {
                    return err("usa .x o .y después del nombre de un punto");
                };
                if field != "x" && field != "y" {
                    return err(format!("'.{field}' no existe: usa .x o .y"));
                }
                e = Expr::Var(format!("{name}.{field}"));
            } else {
                return Ok(e);
            }
        }
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
        if self.peek().is_none() {
            return err("falta cerrar el paréntesis de la función: ')'");
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
                // Un paréntesis de agrupación sin cerrar al final se cierra solo: "(x + 2".
                if self.peek().is_some() {
                    self.expect(')')?;
                }
                Ok(if items.len() == 1 { items.pop().unwrap() } else { Expr::Tuple(items) })
            }
            Some(Tok::Op('|')) => {
                let inner = self.parse_expr()?;
                self.expect('|')?;
                Ok(Expr::F1(Func1::Abs, Box::new(inner)))
            }
            Some(Tok::Op('[')) => {
                let mut items = vec![self.parse_expr()?];
                while self.eat(',') {
                    items.push(self.parse_expr()?);
                }
                if self.eat('…') {
                    let end = Box::new(self.parse_expr()?);
                    self.expect(']')?;
                    let mut it = items.into_iter();
                    return match (it.next(), it.next(), it.next()) {
                        (Some(a), None, _) => Ok(Expr::Range(Box::new(a), None, end)),
                        (Some(a), Some(b), None) => Ok(Expr::Range(Box::new(a), Some(Box::new(b)), end)),
                        _ => err("un rango es [inicio ... fin] o [inicio, segundo ... fin]"),
                    };
                }
                self.expect(']')?;
                Ok(Expr::List(items))
            }
            Some(Tok::Ident(name)) => self.parse_ident(name),
            Some(Tok::Op(c)) => err(format!("'{c}' inesperado")),
            None => err("expresión incompleta"),
        }
    }

    /// sum(n, n^2, 1, 10), prod(…), int(u, u^2, 0, x): variable, expresión, desde, hasta.
    /// int(x, f) (sin límites) es la primitiva ∫₀ˣ f, con constante 0.
    fn parse_big(&mut self, op: BigOp, name: &str) -> Result<Expr, MathError> {
        let usage = match op {
            BigOp::Int => "int(variable, función, desde, hasta), p. ej. int(x, x^2, 0, 1); o int(x, f) para la primitiva".to_string(),
            _ => format!("{name}(variable, expresión, desde, hasta), p. ej. {name}(n, n^2, 1, 10)"),
        };
        if !self.eat('(') {
            return err(usage);
        }
        let Some(Tok::Ident(var)) = self.next() else { return err(usage) };
        if !self.eat(',') {
            return err(usage);
        }
        let body = self.parse_big_body(&var)?;
        // Primitiva: int(x, f) = ∫₀ˣ f(u) du (el límite superior es la x de fuera).
        if op == BigOp::Int && matches!(self.peek(), Some(Tok::Op(')')) | None) {
            if self.peek().is_none() {
                return err("falta cerrar el paréntesis de int: ')'");
            }
            self.expect(')')?;
            let to = Expr::Var(var.clone());
            return Ok(Expr::Big { op, var, from: Box::new(Expr::Num(0.0)), to: Box::new(to), body: Box::new(body) });
        }
        if !self.eat(',') {
            return err(usage);
        }
        let from = self.parse_expr()?;
        if !self.eat(',') {
            return err(usage);
        }
        let to = self.parse_expr()?;
        if self.peek().is_none() {
            return err(format!("falta cerrar el paréntesis de {name}: ')'"));
        }
        self.expect(')')?;
        Ok(Expr::Big { op, var, from: Box::new(from), to: Box::new(to), body: Box::new(body) })
    }

    /// Expresión de una suma/integral. Un nombre de función solo (f, sin…) se aplica a la
    /// variable: int(x, f) = int(x, f(x)).
    fn parse_big_body(&mut self, var: &str) -> Result<Expr, MathError> {
        if let (Some(Tok::Ident(name)), Some(Tok::Op(',' | ')')) | None) = (self.toks.get(self.pos).cloned(), self.toks.get(self.pos + 1)) {
            let arg = || Box::new(Expr::Var(var.to_string()));
            if self.user_funcs.contains(&name) {
                self.pos += 1;
                return Ok(Expr::Call(name, vec![Expr::Var(var.to_string())]));
            }
            if let Some(&(_, f)) = FUNC1_NAMES.iter().find(|(n, _)| *n == name) {
                self.pos += 1;
                return Ok(Expr::F1(f, arg()));
            }
        }
        self.parse_expr()
    }

    fn parse_ident(&mut self, name: String) -> Result<Expr, MathError> {
        let has_paren = self.peek() == Some(&Tok::Op('('));

        match name.as_str() {
            "sum" => return self.parse_big(BigOp::Sum, "sum"),
            "prod" => return self.parse_big(BigOp::Prod, "prod"),
            "int" | "integral" => return self.parse_big(BigOp::Int, "int"),
            "deriv" | "diff" => {
                let body = if has_paren {
                    let mut args = self.parse_args()?;
                    if args.len() != 1 {
                        return err("d/dx(expresión): derivada respecto de x");
                    }
                    args.pop().unwrap()
                } else {
                    self.parse_bare_arg()?
                };
                return Ok(Expr::Deriv { body: Box::new(body), order: 1 });
            }
            // W(x) y W(k, x): función W de Lambert (si W no es un nombre del usuario).
            "W" if has_paren && !self.user_names.contains("W") => {
                let mut args = self.parse_args()?;
                return match args.len() {
                    1 => Ok(Expr::F1(Func1::LambertW, Box::new(args.pop().unwrap()))),
                    2 => {
                        let x = args.pop().unwrap();
                        let k = args.pop().unwrap();
                        Ok(Expr::bin(Func2::LambertWk, k, x))
                    }
                    _ => err("W(x) o W(k, x)"),
                };
            }
            _ => {}
        }
        // f'(x), f''(x)…
        if self.user_funcs.contains(&name) && self.peek() == Some(&Tok::Op('\'')) {
            let mut order = 0;
            while self.eat('\'') {
                order += 1;
            }
            let mut args = self.parse_args()?;
            if args.len() != 1 {
                return err(format!("{name}' solo existe para funciones de una variable"));
            }
            return Ok(Expr::Prime { name, order, arg: Box::new(args.pop().unwrap()) });
        }

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
    FUNC1_NAMES.iter().any(|(n, _)| *n == name) || FUNC2_NAMES.iter().any(|(n, _)| *n == name) || SPECIAL_FUNCS.contains(&name)
}

/// Variables reservadas además de x e y: ángulo `θ` y radio `r` (de las polares).
/// `t` no lo está: es el parámetro de las paramétricas salvo que el usuario lo defina.
const RESERVED_VARS: &[&str] = &["θ", "theta", "r"];

fn is_builtin_name(name: &str) -> bool {
    is_builtin_func(name) || CONSTANTS.iter().any(|(n, _)| *n == name) || name == "theta"
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

impl Comparison {
    /// ¿Se cumple `lhs <op> rhs`, dado `v = lhs - rhs`?
    #[inline]
    pub fn holds(self, v: f64) -> bool {
        match (self.less, self.strict) {
            (true, true) => v < 0.0,
            (true, false) => v <= 0.0,
            (false, true) => v > 0.0,
            (false, false) => v >= 0.0,
        }
    }
}

/// Divide `a < b <= c …` en los operandos y las comparaciones entre ellos.
fn split_comparisons(src: &str) -> (Vec<&str>, Vec<Comparison>) {
    const OPS: [(&str, bool, bool); 6] = [
        ("<=", true, false), (">=", false, false), ("≤", true, false), ("≥", false, false), ("<", true, true), (">", false, true),
    ];
    let mut parts = Vec::new();
    let mut cmps = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < src.len() {
        // Ante "<=" y "<" en la misma posición gana el más largo (está antes en OPS).
        if let Some(&(op, less, strict)) = OPS.iter().find(|(op, ..)| src[i..].starts_with(op)) {
            parts.push(&src[start..i]);
            cmps.push(Comparison { less, strict });
            i += op.len();
            start = i;
        } else {
            i += src[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    parts.push(&src[start..]);
    (parts, cmps)
}

/// Separa `cuerpo for var = lista` en (cuerpo, var, lista).
fn split_for(src: &str) -> Option<(&str, &str, &str)> {
    let pos = src.find(" for ")?;
    let (body, rest) = (&src[..pos], &src[pos + 5..]);
    let (var, list) = rest.split_once('=')?;
    let var = var.trim();
    (var.chars().all(|c| c.is_alphanumeric() || c == '_') && !var.is_empty()).then_some((body, var, list))
}

/// Separa `expr {cond; cond} {cond}` en la expresión y las condiciones (como texto).
fn split_restrictions(src: &str) -> Result<(&str, Vec<&str>), MathError> {
    let Some(open) = src.find('{') else {
        if src.contains('}') {
            return err("'}' sin '{'");
        }
        return Ok((src, Vec::new()));
    };
    let mut conds = Vec::new();
    let mut rest = &src[open..];
    while let Some(after) = rest.strip_prefix('{') {
        let Some(close) = after.find('}') else { return err("falta '}'") };
        let inner = &after[..close];
        if inner.contains('{') {
            return err("llaves anidadas");
        }
        conds.extend(inner.split(';').map(str::trim).filter(|c| !c.is_empty()));
        rest = after[close + 1..].trim_start();
    }
    if !rest.is_empty() {
        return err("después de las restricciones '{…}' no puede haber más texto");
    }
    if conds.is_empty() {
        return err("restricción vacía");
    }
    Ok((&src[..open], conds))
}

fn parse_restrictions(conds: &[&str], user_funcs: &HashSet<String>, user_names: &HashSet<String>) -> Result<Vec<Restriction>, MathError> {
    let mut out = Vec::new();
    for c in conds {
        let (parts, cmps) = split_comparisons(c);
        if cmps.is_empty() {
            return err(format!("la restricción '{c}' necesita <, >, ≤ o ≥"));
        }
        if c.contains('=') && !c.contains("<=") && !c.contains(">=") {
            return err("en las restricciones usa <, >, <= o >=");
        }
        let exprs = parts.iter().map(|p| parse_expr(p, user_funcs, user_names)).collect::<Result<Vec<_>, _>>()?;
        // a < x < b  →  a < x  y  x < b
        for (k, cmp) in cmps.into_iter().enumerate() {
            out.push(Restriction { lhs: exprs[k].clone(), rhs: exprs[k + 1].clone(), cmp });
        }
    }
    Ok(out)
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
    if !valid || name == "x" || name == "y" || RESERVED_VARS.contains(&name) || is_builtin_name(name) {
        return None;
    }
    Some((name.to_string(), is_func))
}

/// Parsea una línea completa. Devuelve la sentencia y sus restricciones `{…}`; en las
/// definiciones de funciones las restricciones pasan a formar parte del cuerpo.
pub fn parse_statement(
    src: &str,
    user_funcs: &HashSet<String>,
    user_names: &HashSet<String>,
) -> Result<(Statement, Vec<Restriction>), MathError> {
    let (main, conds) = split_restrictions(src)?;
    let restrictions = parse_restrictions(&conds, user_funcs, user_names)?;
    let st = match split_for(main) {
        // cuerpo for n = lista
        Some((body_src, var, list_src)) => {
            let list = Box::new(parse_expr(list_src, user_funcs, user_names)?);
            // El nombre de la variable debe reconocerse entero (p. ej. "k" en "2k").
            let mut names = user_names.clone();
            names.insert(var.to_string());
            let wrap = |body: Expr| Expr::For { body: Box::new(body), var: var.to_string(), list: list.clone() };
            match parse_main(body_src, user_funcs, &names)? {
                Statement::Constant { name, expr } => Statement::Constant { name, expr: wrap(expr) },
                Statement::Bare(e) => Statement::Bare(wrap(e)),
                _ => return err("'for' solo sirve para crear listas: (n, n^2) for n = [1...5]"),
            }
        }
        None => parse_main(main, user_funcs, user_names)?,
    };
    Ok(match st {
        Statement::FuncDef { name, params, body } if !restrictions.is_empty() => {
            (Statement::FuncDef { name, params, body: Expr::Restrict(Box::new(body), restrictions) }, Vec::new())
        }
        st => (st, restrictions),
    })
}

fn parse_main(src: &str, user_funcs: &HashSet<String>, user_names: &HashSet<String>) -> Result<Statement, MathError> {
    let (parts, cmps) = split_comparisons(src);
    match cmps.len() {
        0 => {}
        1 => {
            if parts[1].contains('=') {
                return err("una desigualdad no puede llevar además '='");
            }
            let lhs = parse_expr(parts[0], user_funcs, user_names)?;
            let rhs = parse_expr(parts[1], user_funcs, user_names)?;
            return Ok(Statement::Inequality { lhs, rhs, cmp: cmps[0] });
        }
        _ => return err("solo se admite una comparación por línea (usa {…} para restringir)"),
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
#[derive(Debug, Clone, PartialEq)]
pub enum Compiled {
    Num(f64),
    /// Constante compleja (parte real, parte imaginaria), con parte imaginaria ≠ 0.
    CNum(f64, f64),
    X,
    Y,
    /// Parámetro `t` de una curva paramétrica (se sustituye antes de evaluar).
    T,
    /// Ángulo `θ` en coordenadas polares (se sustituye antes de evaluar).
    Theta,
    /// Radio `r` en coordenadas polares (se sustituye antes de evaluar).
    R,
    Neg(Box<Compiled>),
    F1(Func1, Box<Compiled>),
    F2(Func2, Box<Compiled>, Box<Compiled>),
    /// Variable ligada de una suma, integral o derivada: posición en la pila de evaluación.
    Bound(usize),
    /// Suma / producto / integral de `body` con `Bound(idx)` entre `from` y `to`.
    Big { op: BigOp, idx: usize, from: Box<Compiled>, to: Box<Compiled>, body: Box<Compiled> },
    /// Derivada de orden 1 o 2 de `body` respecto de `Bound(idx)`, evaluada en `at`.
    Deriv { idx: usize, order: u32, body: Box<Compiled>, at: Box<Compiled> },
    /// `value` si se cumplen las condiciones (cada una como `lhs - rhs`), si no NaN.
    /// `sub` indica en qué punto se evalúan las condiciones.
    Cond { value: Box<Compiled>, conds: Vec<(Compiled, Comparison)>, sub: CondPoint },
}

/// Punto en el que se comprueban las condiciones de `Compiled::Cond`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CondPoint {
    /// En (x, y) tal cual.
    Xy,
    /// El valor es la y de la curva: se comprueba en (x, valor). Para y = f(x).
    ValueIsY,
    /// El valor es la x de la curva: se comprueba en (valor, y). Para x = g(y).
    ValueIsX,
}

/// Envuelve `value` con condiciones (si las hay).
pub fn restrict(value: Compiled, conds: Vec<(Compiled, Comparison)>, sub: CondPoint) -> Compiled {
    if conds.is_empty() { value } else { Compiled::Cond { value: Box::new(value), conds, sub } }
}

pub fn conds_hold(conds: &[(Compiled, Comparison)], x: f64, y: f64) -> bool {
    conds.iter().all(|(c, cmp)| cmp.holds(c.eval(x, y)))
}

/// Rango entero [lo, hi] de una suma/producto (con límites redondeados). None si está
/// vacío, no es finito o tiene más de un millón de términos.
fn int_range(a: f64, b: f64) -> Option<(i64, i64)> {
    if !(a.is_finite() && b.is_finite()) {
        return None;
    }
    let (lo, hi) = (a.round(), b.round());
    (hi >= lo && hi - lo <= 1e6).then_some((lo as i64, hi as i64))
}

/// Máximo de variables ligadas anidadas (sumas dentro de sumas…).
const MAX_BOUND: usize = 16;
type Stack = [f64; MAX_BOUND];

/// Paso para derivar numéricamente en `a` (fórmulas de 5 puntos).
fn deriv_step(a: f64) -> f64 {
    1e-3 * a.abs().max(1.0)
}

/// Derivada numérica de orden 1 o 2 con diferencias centradas de 5 puntos (error O(h⁴)).
fn finite_diff<T>(f: &mut dyn FnMut(f64) -> T, a: f64, order: u32) -> T
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<f64, Output = T>,
{
    let h = deriv_step(a);
    let (f2p, f1p, f1m, f2m) = (f(a + 2.0 * h), f(a + h), f(a - h), f(a - 2.0 * h));
    if order == 1 {
        (f1p - f1m) * (8.0 / (12.0 * h)) - (f2p - f2m) * (1.0 / (12.0 * h))
    } else {
        let f0 = f(a);
        ((f1p + f1m) * 16.0 - (f2p + f2m) - f0 * 30.0) * (1.0 / (12.0 * h * h))
    }
}

impl Compiled {
    #[inline]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        self.ev(x, y, &mut [0.0; MAX_BOUND])
    }

    fn ev(&self, x: f64, y: f64, s: &mut Stack) -> f64 {
        match self {
            Compiled::Num(v) => *v,
            Compiled::CNum(re, im) => if *im == 0.0 { *re } else { f64::NAN },
            Compiled::X => x,
            Compiled::Y => y,
            Compiled::Bound(i) => s[*i],
            Compiled::T | Compiled::Theta | Compiled::R => f64::NAN,
            Compiled::Neg(a) => -a.ev(x, y, s),
            // Funciones que extraen un real de un complejo: el argumento se evalúa en complejo.
            Compiled::F1(f @ (Func1::Re | Func1::Im | Func1::Arg | Func1::Conj | Func1::CAbs), a) => {
                let z = apply1_c(*f, a.ev_c(x, y, s));
                if z.im == 0.0 { z.re } else { f64::NAN }
            }
            Compiled::F1(f, a) => apply1(*f, a.ev(x, y, s)),
            Compiled::F2(f, a, b) => apply2(*f, a.ev(x, y, s), b.ev(x, y, s)),
            Compiled::Big { op, idx, from, to, body } => {
                let (a, b) = (from.ev(x, y, s), to.ev(x, y, s));
                let saved = s[*idx];
                let v = match op {
                    BigOp::Sum | BigOp::Prod => match int_range(a, b) {
                        Some((lo, hi)) => {
                            let mut acc = if *op == BigOp::Sum { 0.0 } else { 1.0 };
                            for k in lo..=hi {
                                s[*idx] = k as f64;
                                let t = body.ev(x, y, s);
                                if *op == BigOp::Sum { acc += t } else { acc *= t }
                            }
                            acc
                        }
                        None if a.is_finite() && b.is_finite() => if *op == BigOp::Sum { 0.0 } else { 1.0 },
                        None => f64::NAN,
                    },
                    BigOp::Int => special::integrate(
                        &mut |u| {
                            s[*idx] = u;
                            body.ev(x, y, s)
                        },
                        a,
                        b,
                    ),
                };
                s[*idx] = saved;
                v
            }
            Compiled::Deriv { idx, order, body, at } => {
                let a = at.ev(x, y, s);
                let saved = s[*idx];
                let v = finite_diff(
                    &mut |u| {
                        s[*idx] = u;
                        body.ev(x, y, s)
                    },
                    a,
                    *order,
                );
                s[*idx] = saved;
                v
            }
            Compiled::Cond { value, conds, sub } => {
                let v = value.ev(x, y, s);
                let (px, py) = match sub {
                    CondPoint::Xy => (x, y),
                    CondPoint::ValueIsY => (x, v),
                    CondPoint::ValueIsX => (v, y),
                };
                if conds_hold(conds, px, py) { v } else { f64::NAN }
            }
        }
    }

    /// Evaluación en números complejos (las condiciones se comprueban con la parte real).
    pub fn eval_c(&self, x: f64, y: f64) -> C64 {
        self.ev_c(x, y, &mut [0.0; MAX_BOUND])
    }

    fn ev_c(&self, x: f64, y: f64, s: &mut Stack) -> C64 {
        match self {
            Compiled::Num(v) => C64::new(*v, 0.0),
            Compiled::CNum(re, im) => C64::new(*re, *im),
            Compiled::X => C64::new(x, 0.0),
            Compiled::Y => C64::new(y, 0.0),
            Compiled::Bound(i) => C64::new(s[*i], 0.0),
            Compiled::T | Compiled::Theta | Compiled::R => C64::new(f64::NAN, 0.0),
            Compiled::Neg(a) => -a.ev_c(x, y, s),
            Compiled::F1(f, a) => apply1_c(*f, a.ev_c(x, y, s)),
            Compiled::F2(f, a, b) => apply2_c(*f, a.ev_c(x, y, s), b.ev_c(x, y, s)),
            Compiled::Big { op, idx, from, to, body } => {
                // Los límites son reales; el cuerpo puede ser complejo.
                let (a, b) = (from.ev(x, y, s), to.ev(x, y, s));
                let saved = s[*idx];
                let v = match op {
                    BigOp::Sum | BigOp::Prod => match int_range(a, b) {
                        Some((lo, hi)) => {
                            let mut acc = C64::new(if *op == BigOp::Sum { 0.0 } else { 1.0 }, 0.0);
                            for k in lo..=hi {
                                s[*idx] = k as f64;
                                let t = body.ev_c(x, y, s);
                                if *op == BigOp::Sum { acc += t } else { acc *= t }
                            }
                            acc
                        }
                        None if a.is_finite() && b.is_finite() => C64::new(if *op == BigOp::Sum { 0.0 } else { 1.0 }, 0.0),
                        None => C64::new(f64::NAN, 0.0),
                    },
                    BigOp::Int => {
                        let re = special::integrate(
                            &mut |u| {
                                s[*idx] = u;
                                body.ev_c(x, y, s).re
                            },
                            a,
                            b,
                        );
                        let im = special::integrate(
                            &mut |u| {
                                s[*idx] = u;
                                body.ev_c(x, y, s).im
                            },
                            a,
                            b,
                        );
                        C64::new(re, im)
                    }
                };
                s[*idx] = saved;
                v
            }
            Compiled::Deriv { idx, order, body, at } => {
                let a = at.ev(x, y, s);
                let saved = s[*idx];
                let v = finite_diff(
                    &mut |u| {
                        s[*idx] = u;
                        body.ev_c(x, y, s)
                    },
                    a,
                    *order,
                );
                s[*idx] = saved;
                v
            }
            Compiled::Cond { value, conds, sub } => {
                let v = value.ev_c(x, y, s);
                let (px, py) = match sub {
                    CondPoint::Xy => (x, y),
                    CondPoint::ValueIsY => (x, v.re),
                    CondPoint::ValueIsX => (v.re, y),
                };
                if conds_hold(conds, px, py) { v } else { C64::new(f64::NAN, 0.0) }
            }
        }
    }

    /// ¿Contiene complejos (constantes complejas o funciones de complejos)?
    pub fn needs_complex(&self) -> bool {
        self.any(&|c| {
            matches!(c, Compiled::CNum(..) | Compiled::F1(Func1::Re | Func1::Im | Func1::Arg | Func1::Conj | Func1::CAbs, _))
        })
    }

    /// ¿Algún valor complejo llega al resultado sin pasar por re, im, arg o |…|?
    /// (Una gráfica real no puede dibujar algo así.)
    pub fn leaks_complex(&self) -> bool {
        match self {
            Compiled::CNum(..) => true,
            Compiled::F1(Func1::Re | Func1::Im | Func1::Arg | Func1::CAbs, _) => false,
            Compiled::Neg(a) | Compiled::F1(_, a) => a.leaks_complex(),
            Compiled::F2(_, a, b) => a.leaks_complex() || b.leaks_complex(),
            Compiled::Cond { value, conds, .. } => value.leaks_complex() || conds.iter().any(|(c, _)| c.leaks_complex()),
            Compiled::Big { from, to, body, .. } => from.leaks_complex() || to.leaks_complex() || body.leaks_complex(),
            Compiled::Deriv { body, at, .. } => body.leaks_complex() || at.leaks_complex(),
            _ => false,
        }
    }

    pub fn uses_x(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::X))
    }

    pub fn uses_y(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::Y))
    }

    pub fn uses_t(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::T))
    }

    pub fn uses_theta(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::Theta))
    }

    pub fn uses_r(&self) -> bool {
        self.any(&|c| matches!(c, Compiled::R))
    }

    /// Copia del árbol sustituyendo las hojas para las que `f` devuelve algo.
    pub fn subst(&self, f: &dyn Fn(&Compiled) -> Option<Compiled>) -> Compiled {
        if let Some(c) = f(self) {
            return c;
        }
        let c = match self {
            Compiled::Neg(a) => Compiled::Neg(Box::new(a.subst(f))),
            Compiled::F1(op, a) => Compiled::F1(*op, Box::new(a.subst(f))),
            Compiled::F2(op, a, b) => Compiled::F2(*op, Box::new(a.subst(f)), Box::new(b.subst(f))),
            Compiled::Cond { value, conds, sub } => Compiled::Cond {
                value: Box::new(value.subst(f)),
                conds: conds.iter().map(|(c, cmp)| (c.subst(f), *cmp)).collect(),
                sub: *sub,
            },
            Compiled::Big { op, idx, from, to, body } => Compiled::Big {
                op: *op,
                idx: *idx,
                from: Box::new(from.subst(f)),
                to: Box::new(to.subst(f)),
                body: Box::new(body.subst(f)),
            },
            Compiled::Deriv { idx, order, body, at } => {
                Compiled::Deriv { idx: *idx, order: *order, body: Box::new(body.subst(f)), at: Box::new(at.subst(f)) }
            }
            other => other.clone(),
        };
        fold(c)
    }
    fn any(&self, pred: &dyn Fn(&Compiled) -> bool) -> bool {
        pred(self)
            || match self {
                Compiled::Neg(a) | Compiled::F1(_, a) => a.any(pred),
                Compiled::F2(_, a, b) => a.any(pred) || b.any(pred),
                Compiled::Cond { value, conds, .. } => value.any(pred) || conds.iter().any(|(c, _)| c.any(pred)),
                Compiled::Big { from, to, body, .. } => from.any(pred) || to.any(pred) || body.any(pred),
                Compiled::Deriv { body, at, .. } => body.any(pred) || at.any(pred),
                _ => false,
            }
    }

    /// Mayor índice de variable ligada que aparece en el árbol.
    fn max_bound(&self) -> Option<usize> {
        let own = if let Compiled::Bound(i) = self { Some(*i) } else { None };
        let kids: Vec<Option<usize>> = match self {
            Compiled::Neg(a) | Compiled::F1(_, a) => vec![a.max_bound()],
            Compiled::F2(_, a, b) => vec![a.max_bound(), b.max_bound()],
            Compiled::Cond { value, conds, .. } => {
                std::iter::once(value.max_bound()).chain(conds.iter().map(|(c, _)| c.max_bound())).collect()
            }
            Compiled::Big { from, to, body, .. } => vec![from.max_bound(), to.max_bound(), body.max_bound()],
            Compiled::Deriv { body, at, .. } => vec![body.max_bound(), at.max_bound()],
            _ => Vec::new(),
        };
        own.into_iter().chain(kids.into_iter().flatten()).max()
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
        Func1::Fact => libm::tgamma(v + 1.0),
        Func1::Asec => (1.0 / v).acos(),
        Func1::Acsc => (1.0 / v).asin(),
        Func1::Acot => if v == 0.0 { std::f64::consts::FRAC_PI_2 } else { (1.0 / v).atan() },
        Func1::Asinh => v.asinh(),
        Func1::Acosh => v.acosh(),
        Func1::Atanh => v.atanh(),
        Func1::Sech => 1.0 / v.cosh(),
        Func1::Csch => 1.0 / v.sinh(),
        Func1::Coth => 1.0 / v.tanh(),
        Func1::Gamma => libm::tgamma(v),
        Func1::LnGamma => libm::lgamma(v),
        Func1::Erf => libm::erf(v),
        Func1::Erfc => libm::erfc(v),
        Func1::LambertW => special::lambert_w(v, 0),
        Func1::Trunc => v.trunc(),
        Func1::Re | Func1::Conj => v,
        Func1::Im => if v.is_nan() { v } else { 0.0 },
        Func1::Arg => if v < 0.0 { std::f64::consts::PI } else if v.is_nan() { v } else { 0.0 },
        Func1::CAbs => v.abs(),
    }
}

/// Valor complejo de un resultado: si la evaluación real da un número finito y la
/// expresión no tiene complejos, ese; si no (p. ej. sqrt(-1)), la evaluación compleja.
/// Así (-8)^(1/3) sigue siendo -2 y sqrt(-4) es 2i.
pub fn value_of(c: &Compiled) -> C64 {
    let v = c.eval(0.0, 0.0);
    if v.is_finite() && !c.needs_complex() {
        return C64::new(v, 0.0);
    }
    snap(c.eval_c(0.0, 0.0))
}

/// Anula partes residuales por redondeo: e^(iπ) = -1 + 1.2e-16 i  →  -1.
pub fn snap(z: C64) -> C64 {
    let scale = z.re.abs().max(z.im.abs()).max(1.0);
    let clean = |v: f64| if v.abs() <= 1e-13 * scale { 0.0 } else { v };
    C64::new(clean(z.re), clean(z.im))
}

fn cnan() -> C64 {
    C64::new(f64::NAN, f64::NAN)
}

/// Aplica una función real solo si el argumento es real.
fn real_only(z: C64, f: impl Fn(f64) -> f64) -> C64 {
    if z.im == 0.0 { C64::new(f(z.re), 0.0) } else { cnan() }
}

fn apply1_c(f: Func1, z: C64) -> C64 {
    let one = C64::new(1.0, 0.0);
    match f {
        Func1::Sin => z.sin(),
        Func1::Cos => z.cos(),
        Func1::Tan => z.tan(),
        Func1::Sec => one / z.cos(),
        Func1::Csc => one / z.sin(),
        Func1::Cot => one / z.tan(),
        Func1::Asin => z.asin(),
        Func1::Acos => z.acos(),
        Func1::Atan => z.atan(),
        Func1::Sinh => z.sinh(),
        Func1::Cosh => z.cosh(),
        Func1::Tanh => z.tanh(),
        Func1::Sqrt => z.sqrt(),
        // Raíz cúbica real para reales negativos (como en la parte real de la app).
        Func1::Cbrt => if z.im == 0.0 { C64::new(z.re.cbrt(), 0.0) } else { z.cbrt() },
        Func1::Abs | Func1::CAbs => C64::new(z.norm(), 0.0),
        Func1::Ln => z.ln(),
        Func1::Log => z.log10(),
        Func1::Exp => z.exp(),
        Func1::Floor => real_only(z, f64::floor),
        Func1::Ceil => real_only(z, f64::ceil),
        Func1::Round => real_only(z, f64::round),
        // sign(z) = z / |z|
        Func1::Sign => if z == C64::new(0.0, 0.0) { z } else { z / z.norm() },
        Func1::Fact => real_only(z, |v| libm::tgamma(v + 1.0)),
        Func1::Asec => (one / z).acos(),
        Func1::Acsc => (one / z).asin(),
        Func1::Acot => if z == C64::new(0.0, 0.0) { C64::new(std::f64::consts::FRAC_PI_2, 0.0) } else { (one / z).atan() },
        Func1::Asinh => z.asinh(),
        Func1::Acosh => if z.im == 0.0 && z.re >= 1.0 { C64::new(z.re.acosh(), 0.0) } else { z.acosh() },
        Func1::Atanh => z.atanh(),
        Func1::Sech => one / z.cosh(),
        Func1::Csch => one / z.sinh(),
        Func1::Coth => one / z.tanh(),
        Func1::Gamma => real_only(z, libm::tgamma),
        Func1::LnGamma => real_only(z, libm::lgamma),
        Func1::Erf => real_only(z, libm::erf),
        Func1::Erfc => real_only(z, libm::erfc),
        Func1::LambertW => real_only(z, |v| special::lambert_w(v, 0)),
        Func1::Trunc => real_only(z, f64::trunc),
        Func1::Re => C64::new(z.re, 0.0),
        Func1::Im => C64::new(z.im, 0.0),
        Func1::Arg => C64::new(z.arg(), 0.0),
        Func1::Conj => z.conj(),
    }
}

fn apply2_c(f: Func2, a: C64, b: C64) -> C64 {
    let real2 = |g: fn(f64, f64) -> f64| if a.im == 0.0 && b.im == 0.0 { C64::new(g(a.re, b.re), 0.0) } else { cnan() };
    match f {
        Func2::Add => a + b,
        Func2::Sub => a - b,
        Func2::Mul => a * b,
        Func2::Div => a / b,
        Func2::Pow => cpow(a, b),
        Func2::Min => real2(f64::min),
        Func2::Max => real2(f64::max),
        Func2::Mod => real2(f64::rem_euclid),
        Func2::Atan2 => real2(f64::atan2),
        Func2::LogBase => b.ln() / a.ln(),
        Func2::Root => cpow(b, C64::new(1.0, 0.0) / a),
        Func2::NCr => real2(special::ncr),
        Func2::NPr => real2(special::npr),
        Func2::Gcd => real2(special::gcd),
        Func2::Lcm => real2(special::lcm),
        Func2::Beta => real2(special::beta),
        Func2::LambertWk => real2(|k, x| if k == 0.0 || k == -1.0 { special::lambert_w(x, k as i32) } else { f64::NAN }),
    }
}

/// Potencia compleja (rama principal). Exponentes enteros pequeños se calculan por
/// multiplicación para que i^2 dé exactamente -1.
fn cpow(a: C64, b: C64) -> C64 {
    if b.im == 0.0 && b.re.fract() == 0.0 && b.re.abs() <= 64.0 {
        return a.powi(b.re as i32);
    }
    if a == C64::new(0.0, 0.0) {
        return if b.re > 0.0 { a } else { cnan() };
    }
    a.powc(b)
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
        Func2::NCr => special::ncr(a, b),
        Func2::NPr => special::npr(a, b),
        Func2::Gcd => special::gcd(a, b),
        Func2::Lcm => special::lcm(a, b),
        Func2::Beta => special::beta(a, b),
        Func2::LambertWk => if a == 0.0 || a == -1.0 { special::lambert_w(b, a as i32) } else { f64::NAN },
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

/// Entorno de definiciones globales usado al compilar.
#[derive(Default, Clone)]
pub struct Env {
    pub constants: HashMap<String, f64>,
    pub funcs: HashMap<String, (Vec<String>, Expr)>,
    /// Nombres de puntos y listas definidos (`A = (1, 2)`), para dar mejores errores.
    pub points: HashSet<String>,
    /// Constantes complejas definidas (`z = 1 + 2i`).
    pub complex: HashMap<String, C64>,
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
            } else if name == "t" && !env.constants.contains_key("t") && !env.complex.contains_key("t") {
                // t es el parámetro de las paramétricas, salvo que el usuario lo defina.
                Compiled::T
            } else if name == "θ" || name == "theta" {
                Compiled::Theta
            } else if name == "r" {
                Compiled::R
            } else if let Some(v) = env.constants.get(name) {
                Compiled::Num(*v)
            } else if let Some(z) = env.complex.get(name) {
                Compiled::CNum(z.re, z.im)
            } else if env.funcs.contains_key(name) {
                return err(format!("'{name}' es una función: usa {name}(…)"));
            } else if env.points.contains(name) {
                return err(format!("'{name}' es un punto o una lista: usa {name}.x o {name}.y"));
            } else if name == "i" {
                // Unidad imaginaria (si no hay nada definido con ese nombre).
                Compiled::CNum(0.0, 1.0)
            } else {
                return err(format!("'{name}' no está definido"));
            }
        }
        Expr::Neg(a) => Compiled::Neg(Box::new(compile_scoped(a, env, locals, depth)?)),
        Expr::F1(f, a) => {
            let a = compile_scoped(a, env, locals, depth)?;
            // |z| de un complejo es su módulo.
            let f = if *f == Func1::Abs && a.needs_complex() { Func1::CAbs } else { *f };
            Compiled::F1(f, Box::new(a))
        }
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
        Expr::List(_) | Expr::Range(..) | Expr::For { .. } => return err("las listas solo pueden ir solas en una línea"),
        Expr::Big { op, var, from, to, body } => {
            let from = compile_scoped(from, env, locals, depth)?;
            let to = compile_scoped(to, env, locals, depth)?;
            let (idx, inner) = bind(locals, var)?;
            let body = compile_scoped(body, env, &inner, depth)?;
            Compiled::Big { op: *op, idx, from: Box::new(from), to: Box::new(to), body: Box::new(body) }
        }
        // d/dx(cuerpo): el cuerpo se compila con x como variable ligada y se evalúa en x.
        Expr::Deriv { body, order } => {
            let (idx, inner) = bind(locals, "x")?;
            let body = compile_scoped(body, env, &inner, depth)?;
            let at = locals.get("x").cloned().unwrap_or(Compiled::X);
            nest_deriv(idx, *order, body, at)
        }
        // f'(arg): el cuerpo de f con su parámetro como variable ligada, evaluado en arg.
        Expr::Prime { name, order, arg } => {
            let Some((params, body)) = env.funcs.get(name) else {
                return err(format!("función '{name}' no definida"));
            };
            if params.len() != 1 {
                return err(format!("{name}' solo existe para funciones de una variable"));
            }
            let at = compile_scoped(arg, env, locals, depth)?;
            let idx = next_bound(locals)?;
            let inner = HashMap::from([(params[0].clone(), Compiled::Bound(idx))]);
            let body = compile_scoped(body, env, &inner, depth + 1)?;
            nest_deriv(idx, *order, body, at)
        }
        Expr::Restrict(body, rs) => {
            let value = compile_scoped(body, env, locals, depth)?;
            let conds = rs
                .iter()
                .map(|r| {
                    let l = compile_scoped(&r.lhs, env, locals, depth)?;
                    let rr = compile_scoped(&r.rhs, env, locals, depth)?;
                    Ok((fold(Compiled::F2(Func2::Sub, Box::new(l), Box::new(rr))), r.cmp))
                })
                .collect::<Result<Vec<_>, MathError>>()?;
            restrict(value, conds, CondPoint::Xy)
        }
    };
    Ok(fold(c))
}

/// Siguiente posición libre de la pila de variables ligadas.
fn next_bound(locals: &HashMap<String, Compiled>) -> Result<usize, MathError> {
    let idx = locals.values().filter_map(Compiled::max_bound).max().map_or(0, |m| m + 1);
    if idx >= MAX_BOUND {
        return err("demasiadas sumas, integrales o derivadas anidadas");
    }
    Ok(idx)
}

/// Nuevo ámbito con `var` como variable ligada.
fn bind(locals: &HashMap<String, Compiled>, var: &str) -> Result<(usize, HashMap<String, Compiled>), MathError> {
    let idx = next_bound(locals)?;
    let mut inner = locals.clone();
    inner.insert(var.to_string(), Compiled::Bound(idx));
    Ok((idx, inner))
}

/// Derivada de orden n: se encadenan derivadas de orden 2 y 1 (respecto de la misma variable).
fn nest_deriv(idx: usize, order: u32, body: Compiled, at: Compiled) -> Compiled {
    match order {
        0 => body.subst(&|c| (*c == Compiled::Bound(idx)).then(|| at.clone())),
        1 | 2 => Compiled::Deriv { idx, order, body: Box::new(body), at: Box::new(at) },
        n => {
            // f⁽ⁿ⁾(a) = (f⁽ⁿ⁻²⁾)''(a): la interior se evalúa en la variable ligada.
            let inner = nest_deriv(idx, n - 2, body, Compiled::Bound(idx));
            Compiled::Deriv { idx, order: 2, body: Box::new(inner), at: Box::new(at) }
        }
    }
}

/// Plegado de constantes: evalúa subárboles que no dependen de x ni y.
fn fold(c: Compiled) -> Compiled {
    let is_const = |c: &Compiled| matches!(c, Compiled::Num(_) | Compiled::CNum(..));
    let constant = match &c {
        Compiled::Neg(a) | Compiled::F1(_, a) => is_const(a),
        Compiled::F2(_, a, b) => is_const(a) && is_const(b),
        _ => false,
    };
    if !constant {
        return c;
    }
    if !c.needs_complex() {
        let v = c.eval(0.0, 0.0);
        if !v.is_nan() {
            return Compiled::Num(v);
        }
        // sqrt(-1), ln(-1)…: sin valor real, pero sí complejo.
    }
    // Con complejos: si el resultado es real (p. ej. i·i o e^(iπ)), queda un número real.
    let z = snap(c.eval_c(0.0, 0.0));
    if z.im == 0.0 { Compiled::Num(z.re) } else { Compiled::CNum(z.re, z.im) }
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
            let (Statement::Inequality { cmp, .. }, _) = parse_statement(src, &none, &none).unwrap() else { panic!("{src}") };
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
        let (Statement::FuncDef { name, params, body }, _) = parse_statement("f(t) = a t^2", &funcs, &names).unwrap() else {
            panic!()
        };
        env.funcs.insert(name, (params, body));
        let e = parse_expr("f(x+1)", &funcs, &names).unwrap();
        assert_eq!(compile(&e, &env).unwrap().eval(1.0, 0.0), 8.0);
    }

    #[test]
    fn restrictions() {
        let none = HashSet::new();
        let (st, rs) = parse_statement("y = x^2 {x > 2 ; y < 12}", &none, &none).unwrap();
        assert!(matches!(st, Statement::Equation { .. }));
        assert_eq!(rs.len(), 2);
        // Varios grupos y desigualdad doble.
        let (_, rs) = parse_statement("y = x {-1 <= x < 3}{y > 0}", &none, &none).unwrap();
        assert_eq!(rs.len(), 3);
        assert_eq!(rs[0].cmp, Comparison { less: true, strict: false });
        assert!(parse_statement("y = x {x > 1", &none, &none).is_err());
        assert!(parse_statement("y = x {x}", &none, &none).is_err());
        assert!(parse_statement("y = x {x>1} + 2", &none, &none).is_err());

        // y = x^2 restringida a x > 2, y < 12 (evaluada como curva y = f(x)).
        let env = Env::default();
        let e = parse_expr("x^2", &none, &none).unwrap();
        let conds = rs_compiled(&["x > 2", "y < 12"], &env);
        let f = restrict(compile(&e, &env).unwrap(), conds, CondPoint::ValueIsY);
        assert!(f.eval(1.0, 0.0).is_nan());
        assert_eq!(f.eval(3.0, 0.0), 9.0);
        assert!(f.eval(3.5, 0.0).is_nan()); // 12.25 > 12

        // Restricción dentro de una función: f(t) = t {t > 0}.
        let funcs: HashSet<String> = ["f".to_string()].into();
        let (Statement::FuncDef { name, params, body }, rest) = parse_statement("f(t) = t {t > 0}", &funcs, &funcs).unwrap() else {
            panic!()
        };
        assert!(rest.is_empty());
        let mut env = Env::default();
        env.funcs.insert(name, (params, body));
        let c = compile(&parse_expr("f(x - 1)", &funcs, &funcs).unwrap(), &env).unwrap();
        assert!(c.eval(0.5, 0.0).is_nan());
        assert_eq!(c.eval(3.0, 0.0), 2.0);
    }

    fn rs_compiled(srcs: &[&str], env: &Env) -> Vec<(Compiled, Comparison)> {
        let none = HashSet::new();
        srcs.iter()
            .flat_map(|s| parse_restrictions(&[s], &none, &none).unwrap())
            .map(|r| {
                let l = compile(&r.lhs, env).unwrap();
                let rr = compile(&r.rhs, env).unwrap();
                (Compiled::F2(Func2::Sub, Box::new(l), Box::new(rr)), r.cmp)
            })
            .collect()
    }

    #[test]
    fn polar_and_param_vars() {
        let none = HashSet::new();
        let c = compile(&parse_expr("1 + cos(theta) + t r", &none, &none).unwrap(), &Env::default()).unwrap();
        assert!(c.uses_theta() && c.uses_t() && c.uses_r());
        // θ -> x: se evalúa como función de x.
        let d = c.subst(&|c| match c {
            Compiled::Theta => Some(Compiled::X),
            Compiled::T | Compiled::R => Some(Compiled::Num(0.0)),
            _ => None,
        });
        assert!((d.eval(0.0, 0.0) - 2.0).abs() < 1e-12);
        assert!(scan_definition("r = 2").is_none());
        // t se puede definir (deslizador); entonces deja de ser el parámetro.
        assert_eq!(scan_definition("t = 2"), Some(("t".to_string(), false)));
        let mut env = Env::default();
        env.constants.insert("t".into(), 2.0);
        let c = compile(&parse_expr("3t", &none, &none).unwrap(), &env).unwrap();
        assert!(!c.uses_t());
        assert_eq!(c.eval(0.0, 0.0), 6.0);
    }

    #[test]
    fn complex_numbers() {
        let none = HashSet::new();
        let val = |src: &str| value_of(&compile(&parse_expr(src, &none, &none).unwrap(), &Env::default()).unwrap());
        let close = |z: C64, re: f64, im: f64| (z.re - re).abs() < 1e-12 && (z.im - im).abs() < 1e-12;
        assert!(close(val("sqrt(-1)"), 0.0, 1.0));
        assert!(close(val("i^2"), -1.0, 0.0));
        assert!(close(val("e^(i pi)"), -1.0, 0.0));
        assert!(close(val("(1 + 2i)(3 - i)"), 5.0, 5.0));
        assert!(close(val("(1 + i)/(1 - i)"), 0.0, 1.0));
        assert!(close(val("ln(-1)"), 0.0, std::f64::consts::PI));
        assert!(close(val("|3 + 4i|"), 5.0, 0.0));
        assert!(close(val("arg(i)"), std::f64::consts::FRAC_PI_2, 0.0));
        assert!(close(val("conj(2 - 3i)"), 2.0, 3.0));
        assert!(close(val("re(2 - 3i) + im(2 - 3i)"), -1.0, 0.0));
        assert!(close(val("i^i"), (-std::f64::consts::FRAC_PI_2).exp(), 0.0));
        // Lo real sigue siendo real.
        assert!(close(val("(-8)^(1/3)"), -2.0, 0.0));
        // re(√x) para x < 0 vale 0, no "indefinido".
        let c = compile(&parse_expr("re(sqrt(x))", &none, &none).unwrap(), &Env::default()).unwrap();
        assert_eq!(c.eval(-4.0, 0.0), 0.0);
        assert!(!c.leaks_complex());
        let c = compile(&parse_expr("(1 + i) x", &none, &none).unwrap(), &Env::default()).unwrap();
        assert!(c.leaks_complex());
        // Una variable "i" definida por el usuario tiene prioridad.
        let mut env = Env::default();
        env.constants.insert("i".into(), 3.0);
        assert_eq!(value_of(&compile(&parse_expr("2i", &none, &none).unwrap(), &env).unwrap()), C64::new(6.0, 0.0));
    }
    #[test]
    fn calculus_and_special() {
        let funcs: HashSet<String> = ["f".to_string()].into();
        let mut env = Env::default();
        let (Statement::FuncDef { name, params, body }, _) = parse_statement("f(u) = u^3", &funcs, &funcs).unwrap() else { panic!() };
        env.funcs.insert(name, (params, body));
        let at = |src: &str, x: f64| compile(&parse_expr(src, &funcs, &funcs).unwrap(), &env).unwrap().eval(x, 0.0);
        let close = |a: f64, b: f64, tol: f64| (a - b).abs() <= tol * (1.0 + b.abs());

        // Sumatorias y productorias.
        assert_eq!(at("sum(n, n, 1, 100)", 0.0), 5050.0);
        assert_eq!(at("∑(k, k^2, 1, 4)", 0.0), 30.0);
        assert_eq!(at("prod(n, n, 1, 5)", 0.0), 120.0);
        assert_eq!(at("sum(n, n, 1, x)", 4.0), 10.0);
        assert_eq!(at("sum(i, sum(j, j, 1, i), 1, 3)", 0.0), 10.0); // sumas anidadas
        assert!(close(at("sum(n, x^n / n!, 0, 20)", 1.0), std::f64::consts::E, 1e-12));
        // Integrales.
        assert!(close(at("int(u, u^2, 0, 3)", 0.0), 9.0, 1e-10));
        assert!(close(at("int(u, cos(u), 0, x)", 1.2), 1.2f64.sin(), 1e-10));
        assert!(close(at("∫(u, e^(-u), 0, ∞)", 0.0), 1.0, 1e-8));
        // Derivadas.
        assert!(close(at("d/dx(x^3)", 2.0), 12.0, 1e-8));
        assert!(close(at("deriv(sin(x))", 0.5), 0.5f64.cos(), 1e-8));
        assert!(close(at("f'(x)", 2.0), 12.0, 1e-8));
        assert!(close(at("f''(x)", 2.0), 12.0, 1e-6));
        assert!(close(at("f'''(x)", 2.0), 6.0, 1e-4));
        assert!(close(at("f'(2)", 0.0), 12.0, 1e-8)); // valor constante
        assert!(close(at("d/dx(d/dx(x^4))", 1.0), 12.0, 1e-5));
        // Funciones especiales.
        assert_eq!(at("nCr(5, 2) + nPr(5, 2)", 0.0), 30.0);
        assert!(close(at("gamma(5)", 0.0), 24.0, 1e-12));
        assert!(close(at("Γ(0.5)^2", 0.0), std::f64::consts::PI, 1e-12));
        assert!(close(at("W(1)", 0.0), 0.567_143_290_409_783_8, 1e-13));
        assert!(close(at("W(-1, -0.2)", 0.0), -2.542_641_357_773_527, 1e-12));
        assert!(close(at("erf(1)", 0.0), 0.842_700_792_949_714_9, 1e-12));
        assert_eq!(at("floor(2.7) + ceil(2.1) + mod(7, 3) + gcd(12, 18) + lcm(4, 6)", 0.0), 2.0 + 3.0 + 1.0 + 6.0 + 12.0);
        assert!(close(
            at("asinh(1) + acosh(2) + atanh(0.5) + coth(1) + sech(0) + acot(1)", 0.0),
            1f64.asinh() + 2f64.acosh() + 0.5f64.atanh() + 1.0 / 1f64.tanh() + 1.0 + std::f64::consts::FRAC_PI_4,
            1e-12
        ));
        assert!(close(at("log(2, 8) + ln(e^2)", 0.0), 5.0, 1e-12));
        // Sumas con complejos.
        let z = value_of(&compile(&parse_expr("sum(k, e^(i 2pi k/6), 0, 5)", &funcs, &funcs).unwrap(), &env).unwrap());
        assert!(z.norm() < 1e-12);
        // Primitiva con constante 0: int(x, f) y int(x, expresión).
        assert!(close(at("int(x, f)", 2.0), 4.0, 1e-10)); // ∫₀² u³ du = 4
        assert!(close(at("int(x, x^2)", 3.0), 9.0, 1e-10));
        assert!(close(at("int(x, cos)", 1.0), 1f64.sin(), 1e-10));
        assert!(close(at("int(x, f, 0, 1)", 0.0), 0.25, 1e-10));
        assert!(close(at("sum(n, f, 1, 3)", 0.0), 36.0, 1e-12)); // 1 + 8 + 27
        // Paréntesis de agrupación sin cerrar: se cierran solos; los de funciones no.
        assert_eq!(at("(x + 2", 1.0), 3.0);
        assert_eq!(at("2(x + (1", 1.0), 4.0);
        assert!(parse_expr("sin(x", &funcs, &funcs).is_err());
        assert!(parse_expr("int(x, f", &funcs, &funcs).is_err());
        // Errores útiles.
        assert!(parse_expr("sum(n, n^2, 1)", &funcs, &funcs).is_err());
    }
}
