//! Notación LaTeX: traduce \frac{x}{2}, \sqrt{x}, \sum_{n=1}^{10}, \int_0^1 … dx, etc. a la
//! sintaxis propia de la app, para que el resto (parser, gráficas, escritura) no cambie.

use crate::math::MathError;

fn err<T>(msg: impl Into<String>) -> Result<T, MathError> {
    Err(MathError(msg.into()))
}

/// ¿El texto usa notación LaTeX?
pub fn is_latex(src: &str) -> bool {
    src.contains('\\')
}

/// Traduce una línea en LaTeX a la sintaxis de la app.
pub fn to_plain(src: &str) -> Result<String, MathError> {
    let mut c = Conv { chars: src.chars().collect(), pos: 0 };
    let out = c.convert_until(None)?;
    Ok(out.split_whitespace().collect::<Vec<_>>().join(" "))
}

struct Conv {
    chars: Vec<char>,
    pos: usize,
}

const GREEK: &[(&str, &str)] = &[
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"), ("varepsilon", "ε"),
    ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("vartheta", "θ"), ("iota", "ι"), ("kappa", "κ"),
    ("lambda", "λ"), ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"), ("pi", "π"), ("rho", "ρ"), ("sigma", "σ"),
    ("tau", "τ"), ("upsilon", "υ"), ("phi", "φ"), ("varphi", "φ"), ("chi", "χ"), ("psi", "ψ"), ("omega", "ω"),
    ("Gamma", "Γ"), ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"), ("Sigma", "Σ"),
    ("Upsilon", "Υ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"),
];

/// Comandos que son nombres de funciones: se quita la barra.
const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "sec", "csc", "cot", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth",
    "sech", "csch", "ln", "exp", "min", "max", "gcd", "lcm", "arg",
];

/// Comandos que se sustituyen por un texto fijo.
const SYMBOLS: &[(&str, &str)] = &[
    ("cdot", " * "), ("times", " * "), ("div", " / "), ("infty", " inf "), ("le", " <= "), ("leq", " <= "),
    ("ge", " >= "), ("geq", " >= "), ("lt", " < "), ("gt", " > "), ("ldots", "..."), ("dots", "..."),
    ("cdots", "..."), ("lfloor", " floor("), ("rfloor", ")"), ("lceil", " ceil("), ("rceil", ")"),
    ("Re", " re"), ("Im", " im"), ("prime", "'"),
];

/// Espacios y comandos que no aportan nada al cálculo.
const IGNORED: &[&str] = &["left", "right", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "displaystyle", "quad", "qquad"];

impl Conv {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    /// Lee el nombre de un comando tras la barra: letras, o un único símbolo (\{, \,).
    fn command(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start {
            if let Some(c) = self.peek() {
                self.pos += 1;
                return c.to_string();
            }
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// Contenido de un grupo {…} ya traducido (sin las llaves).
    fn group(&mut self) -> Result<String, MathError> {
        self.skip_spaces();
        if self.peek() != Some('{') {
            return err("se esperaba '{'");
        }
        self.pos += 1;
        self.convert_until(Some('}'))
    }

    /// Un argumento: un grupo {…}, un comando (\pi) o un solo carácter (\frac12).
    fn argument(&mut self) -> Result<String, MathError> {
        self.skip_spaces();
        match self.peek() {
            Some('{') => self.group(),
            Some('\\') => {
                self.pos += 1;
                let name = self.command();
                self.command_text(&name)
            }
            Some(c) => {
                self.pos += 1;
                Ok(c.to_string())
            }
            None => err("falta un argumento"),
        }
    }

    /// Argumento de una función con base (\log_2 x, \log_2(x+1)): un paréntesis
    /// completo, un grupo o un símbolo.
    fn function_argument(&mut self) -> Result<String, MathError> {
        self.skip_spaces();
        if self.peek() == Some('(') {
            self.pos += 1;
            let inner = self.convert_until(Some(')'))?;
            return Ok(inner);
        }
        // Una variable o número entero (varias letras/cifras seguidas).
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '.') {
            self.pos += 1;
        }
        if self.pos > start {
            return Ok(self.chars[start..self.pos].iter().collect());
        }
        self.argument()
    }

    /// Subíndice y superíndice opcionales (en cualquier orden): _{a}^{b}.
    fn limits(&mut self) -> Result<(Option<String>, Option<String>), MathError> {
        let (mut lo, mut hi) = (None, None);
        loop {
            self.skip_spaces();
            match self.peek() {
                Some('_') if lo.is_none() => {
                    self.pos += 1;
                    lo = Some(self.argument()?);
                }
                Some('^') if hi.is_none() => {
                    self.pos += 1;
                    hi = Some(self.argument()?);
                }
                _ => return Ok((lo, hi)),
            }
        }
    }

    /// Traducción de un comando simple (sin la barra).
    fn command_text(&mut self, name: &str) -> Result<String, MathError> {
        if let Some((_, g)) = GREEK.iter().find(|(n, _)| *n == name) {
            return Ok(g.to_string());
        }
        if FUNCTIONS.contains(&name) {
            return Ok(format!(" {name} "));
        }
        if let Some((_, s)) = SYMBOLS.iter().find(|(n, _)| *n == name) {
            return Ok(s.to_string());
        }
        if IGNORED.contains(&name) {
            return Ok(" ".into());
        }
        match name {
            "," | ";" | "!" | ":" | " " => Ok(" ".into()),
            "{" => Ok("{".into()),
            "}" => Ok("}".into()),
            "|" => Ok("|".into()),
            "frac" | "dfrac" | "tfrac" => {
                let (num, den) = (self.argument()?, self.argument()?);
                // \frac{d}{dx}: derivada.
                if num.trim() == "d" && den.trim().starts_with('d') && den.trim().len() == 2 {
                    return Ok(" d/dx ".into());
                }
                Ok(format!("(({num})/({den}))"))
            }
            "binom" => {
                let (n, k) = (self.argument()?, self.argument()?);
                Ok(format!(" nCr({n}, {k}) "))
            }
            "sqrt" => {
                self.skip_spaces();
                if self.peek() == Some('[') {
                    self.pos += 1;
                    let index = self.convert_until(Some(']'))?;
                    let radicand = self.argument()?;
                    return Ok(format!(" root({index}, {radicand}) "));
                }
                Ok(format!(" sqrt({}) ", self.argument()?))
            }
            "log" => {
                self.skip_spaces();
                if self.peek() == Some('_') {
                    self.pos += 1;
                    let base = self.argument()?;
                    let arg = self.function_argument()?;
                    return Ok(format!(" log({base}, {arg}) "));
                }
                Ok(" log ".into())
            }
            "operatorname" | "mathrm" | "text" | "mathit" | "mathbf" => {
                // Contenido literal: \operatorname{nCr}, \mathrm{e}.
                let inner = self.group()?;
                Ok(format!(" {} ", inner.trim()))
            }
            "sum" | "prod" | "int" => err("internal"),
            other => err(format!("comando LaTeX no admitido: \\{other}")),
        }
    }

    /// Traduce hasta el carácter de cierre `close` (que se consume) o hasta el final.
    fn convert_until(&mut self, close: Option<char>) -> Result<String, MathError> {
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return match close {
                    Some(c) => err(format!("falta '{c}'")),
                    None => Ok(out),
                };
            };
            if Some(c) == close {
                self.pos += 1;
                return Ok(out);
            }
            match c {
                '\\' => {
                    self.pos += 1;
                    let name = self.command();
                    match name.as_str() {
                        "sum" | "prod" => {
                            let (lo, hi) = self.limits()?;
                            let rest = self.convert_until(close)?;
                            out.push_str(&big_operator(&name, lo, hi, &rest)?);
                            return Ok(out);
                        }
                        "int" => {
                            let (lo, hi) = self.limits()?;
                            let rest = self.convert_until(close)?;
                            out.push_str(&integral(lo, hi, &rest)?);
                            return Ok(out);
                        }
                        _ => out.push_str(&self.command_text(&name)?),
                    }
                }
                '{' => {
                    self.pos += 1;
                    let inner = self.convert_until(Some('}'))?;
                    // Unas llaves con una comparación son una restricción; si no, agrupan.
                    if has_comparison(&inner) {
                        out.push_str(&format!(" {{{inner}}} "));
                    } else {
                        out.push_str(&format!("({inner})"));
                    }
                }
                '_' => {
                    // Subíndice de un nombre: a_{12} → a_12.
                    self.pos += 1;
                    let sub = self.argument()?;
                    out.push('_');
                    out.push_str(sub.trim());
                }
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }
}

fn has_comparison(s: &str) -> bool {
    s.contains(['<', '>', '≤', '≥'])
}

/// Posición del primer + o − de primer nivel (fin del término al que afecta ∑ o ∏).
fn term_end(s: &str) -> usize {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '+' | '-' | '=' | '<' | '>' | ',' | ';' if depth == 0 && !s[..i].trim().is_empty() => return i,
            _ => {}
        }
    }
    s.len()
}

/// \sum_{n=1}^{10} cuerpo → sum(n, cuerpo, 1, 10). El cuerpo llega hasta el siguiente
/// + o − de primer nivel, como en la notación habitual.
fn big_operator(name: &str, lo: Option<String>, hi: Option<String>, rest: &str) -> Result<String, MathError> {
    let usage = format!("\\{name} necesita límites: \\{name}_{{n=1}}^{{10}}");
    let (Some(lo), Some(hi)) = (lo, hi) else { return err(usage) };
    let Some((var, from)) = lo.split_once('=') else { return err(usage) };
    let end = term_end(rest);
    let (body, after) = rest.split_at(end);
    if body.trim().is_empty() {
        return err(format!("falta la expresión de \\{name}"));
    }
    Ok(format!(" {name}({}, {}, {}, {}) {after}", var.trim(), body.trim(), from.trim(), hi.trim()))
}

/// \int_a^b cuerpo dx → int(x, cuerpo, a, b); sin límites, la primitiva int(x, cuerpo).
fn integral(lo: Option<String>, hi: Option<String>, rest: &str) -> Result<String, MathError> {
    // Busca el diferencial "dx" (o dt, du…) de primer nivel.
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let mut depth = 0i32;
    let mut found = None;
    for (k, &(i, c)) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            'd' if depth == 0 => {
                let prev_ok = k == 0 || !chars[k - 1].1.is_alphabetic();
                let var = chars.get(k + 1).map(|p| p.1);
                let next_ok = chars.get(k + 2).is_none_or(|p| !p.1.is_alphanumeric());
                if let (true, Some(v), true) = (prev_ok, var, next_ok) {
                    if v.is_alphabetic() {
                        found = Some((i, v));
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let (body, var, after) = match found {
        Some((i, v)) => (&rest[..i], v.to_string(), &rest[i + 1 + v.len_utf8()..]),
        None => (rest, "x".to_string(), ""),
    };
    if body.trim().is_empty() {
        return err("falta la función que se integra");
    }
    Ok(match (lo, hi) {
        (Some(a), Some(b)) => format!(" int({var}, {}, {}, {}) {after}", body.trim(), a.trim(), b.trim()),
        (None, None) => format!(" int({var}, {}) {after}", body.trim()),
        _ => return err("una integral definida necesita los dos límites: \\int_a^b"),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::math::{self, Env};

    fn plain(s: &str) -> String {
        to_plain(s).unwrap()
    }

    /// Valor numérico de una expresión LaTeX en x.
    fn at(s: &str, x: f64) -> f64 {
        let none = HashSet::new();
        let e = math::parse_expr(&plain(s), &none, &none).unwrap_or_else(|e| panic!("{s} → {}: {e}", plain(s)));
        math::compile(&e, &Env::default()).unwrap().eval(x, 0.0)
    }

    #[test]
    fn translation() {
        assert_eq!(plain(r"\frac{x}{2}"), "((x)/(2))");
        assert_eq!(plain(r"y=\sqrt{x^{2}+1}"), "y= sqrt(x^(2)+1)");
        assert_eq!(plain(r"a_{12}+1"), "a_12+1");
        assert_eq!(plain(r"y=x \{x>0\}"), "y=x {x>0}");
        assert_eq!(plain(r"\left(x+1\right)^{2}"), "(x+1 )^(2)");
    }

    #[test]
    fn evaluation() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * (1.0 + b.abs());
        assert!(close(at(r"\frac{x}{2}", 3.0), 1.5));
        assert!(close(at(r"\frac12 + \dfrac{1}{4}", 0.0), 0.75));
        assert!(close(at(r"\sqrt[3]{27} + \sqrt{16}", 0.0), 7.0));
        assert!(close(at(r"2\pi", 0.0), std::f64::consts::TAU));
        assert!(close(at(r"\sin^{2}(x) + \cos^2 x", 0.7), 1.0));
        assert!(close(at(r"x \cdot 3 \times 2", 1.0), 6.0));
        assert!(close(at(r"\log_{2}(8) + \log_2 4 + \ln e", 0.0), 6.0));
        assert!(close(at(r"\sum_{n=1}^{100} n", 0.0), 5050.0));
        assert!(close(at(r"\sum_{n=1}^{3} n^2 + 1", 0.0), 15.0)); // el +1 queda fuera
        assert!(close(at(r"\prod_{k=1}^{5} k", 0.0), 120.0));
        assert!(close(at(r"\int_{0}^{1} x^2 \, dx", 0.0), 1.0 / 3.0));
        assert!(close(at(r"\int_0^{\pi} \sin(t)\,dt", 0.0), 2.0));
        assert!(close(at(r"\int x^2\,dx", 3.0), 9.0)); // primitiva
        assert!(close(at(r"\frac{d}{dx}(x^3)", 2.0), 12.0));
        assert!(close(at(r"\binom{5}{2} + \lfloor 2.7 \rfloor + \left|-3\right|", 0.0), 15.0));
        assert!(close(at(r"e^{\frac{1}{2}}", 0.0), 0.5f64.exp()));
        assert!(close(at(r"\int_{-\infty}^{\infty} e^{-x^2} dx", 0.0), std::f64::consts::PI.sqrt()));
        assert!(to_plain(r"\frac{1}{2").is_err());
        assert!(to_plain(r"\foo x").is_err());
    }
}
