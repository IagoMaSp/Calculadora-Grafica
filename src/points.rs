//! Puntos con nombre, listas (de números o de puntos) y puntos arrastrables.

use std::collections::{HashMap, HashSet};

use crate::math::{self, C64, Env, Expr, MathError};

/// Máximo de elementos de una lista (evita bloquear la app con [1...1e9]).
const MAX_LEN: usize = 10_000;

/// Valor de una definición de lista: `L = [1, 2, 3]` o `P = [(1, 2), (3, 4)]`.
#[derive(Clone, Debug)]
pub enum ListValue {
    Numbers(Vec<f64>),
    Points(Vec<(f64, f64)>),
}

/// Listas y puntos con nombre ya evaluados.
#[derive(Default)]
pub struct Lists {
    pub numbers: HashMap<String, Vec<f64>>,
    pub points: HashMap<String, Vec<(f64, f64)>>,
}

fn err<T>(msg: impl Into<String>) -> Result<T, MathError> {
    Err(MathError(msg.into()))
}

/// ¿La expresión es sintácticamente una lista, un punto o una comprensión?
pub fn is_list_syntax(e: &Expr) -> bool {
    matches!(e, Expr::Tuple(_) | Expr::List(_) | Expr::Range(..) | Expr::For { .. })
}

/// ¿La expresión produce puntos (y no números)?
pub fn is_pointish(e: &Expr, lists: &Lists) -> bool {
    match e {
        Expr::Tuple(_) => true,
        Expr::List(items) => items.first().is_some_and(|i| is_pointish(i, lists)),
        Expr::For { body, .. } => is_pointish(body, lists),
        Expr::Var(n) => lists.points.contains_key(n),
        _ => false,
    }
}

/// ¿La expresión produce una lista de números?
pub fn is_number_list(e: &Expr, lists: &Lists) -> bool {
    match e {
        Expr::List(_) | Expr::Range(..) | Expr::For { .. } => !is_pointish(e, lists),
        _ => {
            let mut vars = HashSet::new();
            collect_vars(e, &mut vars);
            vars.iter().any(|v| lists.numbers.contains_key(v))
        }
    }
}

fn collect_vars(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Num(_) => {}
        Expr::Var(v) => {
            out.insert(v.clone());
        }
        Expr::Neg(a) | Expr::F1(_, a) => collect_vars(a, out),
        Expr::F2(_, a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        Expr::Call(_, args) | Expr::Tuple(args) | Expr::List(args) => args.iter().for_each(|a| collect_vars(a, out)),
        Expr::Restrict(body, rs) => {
            collect_vars(body, out);
            for r in rs {
                collect_vars(&r.lhs, out);
                collect_vars(&r.rhs, out);
            }
        }
        Expr::Range(a, b, c) => {
            collect_vars(a, out);
            if let Some(b) = b {
                collect_vars(b, out);
            }
            collect_vars(c, out);
        }
        Expr::For { body, list, var } => {
            let mut inner = HashSet::new();
            collect_vars(body, &mut inner);
            inner.remove(var);
            out.extend(inner);
            collect_vars(list, out);
        }
    }
}

/// Valor (posiblemente complejo) de una expresión constante.
fn scalar_c(e: &Expr, env: &Env) -> Result<C64, MathError> {
    let c = math::compile(e, env)?;
    if c.uses_x() || c.uses_y() {
        return err("los elementos de una lista no pueden depender de x ni y");
    }
    if c.uses_t() || c.uses_theta() || c.uses_r() {
        return err("los elementos de una lista no pueden usar t, θ ni r");
    }
    Ok(math::value_of(&c))
}

fn scalar(e: &Expr, env: &Env) -> Result<f64, MathError> {
    let z = scalar_c(e, env)?;
    if z.im != 0.0 {
        return err("aquí se necesita un número real, no un complejo");
    }
    Ok(z.re)
}

/// Lista de números reales, o de puntos del plano complejo si algún elemento es complejo.
pub fn eval_number_list(e: &Expr, env: &Env, lists: &Lists) -> Result<ListValue, MathError> {
    let values = eval_values(e, env, lists)?;
    Ok(if values.iter().all(|z| z.im == 0.0) {
        ListValue::Numbers(values.into_iter().map(|z| z.re).collect())
    } else {
        ListValue::Points(values.into_iter().map(|z| (z.re, z.im)).collect())
    })
}

/// Evalúa una lista de números reales; si algún elemento es complejo, error.
pub fn eval_numbers(e: &Expr, env: &Env, lists: &Lists) -> Result<Vec<f64>, MathError> {
    let values = eval_values(e, env, lists)?;
    if values.iter().any(|z| z.im != 0.0) {
        return err("la lista tiene números complejos");
    }
    Ok(values.into_iter().map(|z| z.re).collect())
}

/// Evalúa una expresión como lista de números, reales o complejos (un escalar da una
/// lista de 1 elemento). Si usa listas con nombre, se evalúa elemento a elemento:
/// (L, L^2) con L = [1, 2, 3].
pub fn eval_values(e: &Expr, env: &Env, lists: &Lists) -> Result<Vec<C64>, MathError> {
    let out: Vec<C64> = match e {
        Expr::List(items) => {
            let mut out = Vec::new();
            for it in items {
                out.extend(eval_values(it, env, lists)?);
            }
            out
        }
        Expr::Range(a, b, c) => {
            let start = scalar(a, env)?;
            let end = scalar(c, env)?;
            let step = match b {
                Some(b) => scalar(b, env)? - start,
                None => if end >= start { 1.0 } else { -1.0 },
            };
            if step == 0.0 || !step.is_finite() || (end - start) * step < 0.0 {
                return err("rango inválido");
            }
            let n = ((end - start) / step + 1e-9).floor() as usize + 1;
            if n > MAX_LEN {
                return err(format!("la lista tiene más de {MAX_LEN} elementos"));
            }
            (0..n).map(|k| C64::new(start + step * k as f64, 0.0)).collect()
        }
        Expr::For { body, var, list } => {
            let values = eval_numbers(list, env, lists)?;
            let mut env2 = env.clone();
            let mut out = Vec::new();
            for v in values {
                env2.constants.insert(var.clone(), v);
                out.extend(eval_values(body, &env2, lists)?);
            }
            out
        }
        Expr::Tuple(_) => return err("se esperaba un número, no un punto"),
        _ => {
            // Listas con nombre que aparecen en la expresión: evaluación elemento a elemento.
            let mut vars = HashSet::new();
            collect_vars(e, &mut vars);
            let used: Vec<&String> = vars.iter().filter(|v| lists.numbers.contains_key(*v)).collect();
            if used.is_empty() {
                vec![scalar_c(e, env)?]
            } else {
                let n = used.iter().map(|v| lists.numbers[*v].len()).min().unwrap_or(0);
                let mut env2 = env.clone();
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    for v in &used {
                        env2.constants.insert((*v).clone(), lists.numbers[*v][i]);
                    }
                    out.push(scalar_c(e, &env2)?);
                }
                out
            }
        }
    };
    if out.len() > MAX_LEN {
        return err(format!("la lista tiene más de {MAX_LEN} elementos"));
    }
    Ok(out)
}

/// Evalúa una expresión como lista de puntos.
pub fn eval_points(e: &Expr, env: &Env, lists: &Lists) -> Result<Vec<(f64, f64)>, MathError> {
    let out = match e {
        Expr::Tuple(items) => {
            if items.len() != 2 {
                return err("un punto tiene 2 coordenadas");
            }
            // Si una coordenada usa una lista con nombre, ambas se evalúan juntas por índice.
            let mut vars = HashSet::new();
            collect_vars(e, &mut vars);
            let used: Vec<&String> = vars.iter().filter(|v| lists.numbers.contains_key(*v)).collect();
            let (xs, ys) = if used.is_empty() {
                (eval_numbers(&items[0], env, lists)?, eval_numbers(&items[1], env, lists)?)
            } else {
                let n = used.iter().map(|v| lists.numbers[*v].len()).min().unwrap_or(0);
                let mut env2 = env.clone();
                let (mut xs, mut ys) = (Vec::with_capacity(n), Vec::with_capacity(n));
                for i in 0..n {
                    for v in &used {
                        env2.constants.insert((*v).clone(), lists.numbers[*v][i]);
                    }
                    xs.push(scalar(&items[0], &env2)?);
                    ys.push(scalar(&items[1], &env2)?);
                }
                (xs, ys)
            };
            // Una coordenada escalar se repite: ([1, 2, 3], 0) son tres puntos sobre el eje x.
            match (xs.len(), ys.len()) {
                (1, n) => ys.into_iter().map(|y| (xs[0], y)).take(n).collect(),
                (n, 1) => xs.into_iter().map(|x| (x, ys[0])).take(n).collect(),
                _ => xs.into_iter().zip(ys).collect(),
            }
        }
        Expr::List(items) => {
            let mut out = Vec::new();
            for it in items {
                out.extend(eval_points(it, env, lists)?);
            }
            out
        }
        Expr::Var(n) if lists.points.contains_key(n) => lists.points[n].clone(),
        Expr::For { body, var, list } => {
            let values = eval_numbers(list, env, lists)?;
            let mut env2 = env.clone();
            let mut out = Vec::new();
            for v in values {
                env2.constants.insert(var.clone(), v);
                out.extend(eval_points(body, &env2, lists)?);
            }
            out
        }
        _ => return err("se esperaba un punto (x, y)"),
    };
    if out.len() > MAX_LEN {
        return err(format!("la lista tiene más de {MAX_LEN} puntos"));
    }
    Ok(out)
}

// ───────────────────────────── Puntos arrastrables ─────────────────────────────

/// Cómo se mueve cada coordenada de un punto al arrastrarlo.
#[derive(Clone, Debug, PartialEq)]
pub enum Coord {
    /// Número escrito en el texto: se reescribe.
    Literal,
    /// Un deslizador: se mueve el deslizador.
    Slider(String),
    /// Expresión: esa coordenada no se puede mover.
    Fixed,
}

/// Qué coordenadas de `(a, b)` se pueden arrastrar. `None` si ninguna.
pub fn drag_spec(e: &Expr, sliders: &HashSet<String>) -> Option<[Coord; 2]> {
    let Expr::Tuple(items) = e else { return None };
    if items.len() != 2 {
        return None;
    }
    let coord = |e: &Expr| match e {
        Expr::Num(_) => Coord::Literal,
        Expr::Neg(inner) if matches!(**inner, Expr::Num(_)) => Coord::Literal,
        Expr::Var(n) if sliders.contains(n) => Coord::Slider(n.clone()),
        _ => Coord::Fixed,
    };
    let spec = [coord(&items[0]), coord(&items[1])];
    (spec != [Coord::Fixed, Coord::Fixed]).then_some(spec)
}

/// Reescribe las coordenadas de un punto en el texto, conservando el nombre (`A = `)
/// y las restricciones. `None` en una coordenada la deja como estaba.
pub fn rewrite_point_text(text: &str, new_x: Option<String>, new_y: Option<String>) -> Option<String> {
    let (main, suffix) = match text.find('{') {
        Some(i) => (&text[..i], text[i..].trim()),
        None => (text, ""),
    };
    let (prefix, tuple) = match main.find('=') {
        Some(i) => (format!("{} = ", main[..i].trim()), main[i + 1..].trim()),
        None => (String::new(), main.trim()),
    };
    let inner = tuple.strip_prefix('(')?.strip_suffix(')')?;
    // Coma de primer nivel (fuera de paréntesis y corchetes).
    let mut depth = 0i32;
    let comma = inner.char_indices().find(|&(_, c)| {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        c == ',' && depth == 0
    })?;
    let (xs, ys) = (inner[..comma.0].trim(), inner[comma.0 + 1..].trim());
    let x = new_x.unwrap_or_else(|| xs.to_string());
    let y = new_y.unwrap_or_else(|| ys.to_string());
    let suffix = if suffix.is_empty() { String::new() } else { format!(" {suffix}") };
    Some(format!("{prefix}({x}, {y}){suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Expr {
        let names: HashSet<String> = ["L", "n", "k"].iter().map(|s| s.to_string()).collect();
        let st = math::parse_statement(src, &HashSet::new(), &names).unwrap().0;
        match st {
            math::Statement::Bare(e) | math::Statement::Constant { expr: e, .. } => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lists() {
        let env = Env::default();
        let mut lists = Lists::default();
        assert_eq!(eval_numbers(&parse("[1...5]"), &env, &lists).unwrap(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(eval_numbers(&parse("[0, 0.5 ... 2]"), &env, &lists).unwrap(), vec![0.0, 0.5, 1.0, 1.5, 2.0]);
        assert_eq!(eval_numbers(&parse("[3...1]"), &env, &lists).unwrap(), vec![3.0, 2.0, 1.0]);
        let pts = eval_points(&parse("[(1, 2), (3, 4)]"), &env, &lists).unwrap();
        assert_eq!(pts, vec![(1.0, 2.0), (3.0, 4.0)]);
        let pts = eval_points(&parse("(n, n^2) for n = [1...3]"), &env, &lists).unwrap();
        assert_eq!(pts, vec![(1.0, 1.0), (2.0, 4.0), (3.0, 9.0)]);
        lists.numbers.insert("L".into(), vec![1.0, 2.0, 3.0]);
        let pts = eval_points(&parse("(L, 2L)"), &env, &lists).unwrap();
        assert_eq!(pts, vec![(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]);
        let pts = eval_points(&parse("([1, 2], 0)"), &env, &lists).unwrap();
        assert_eq!(pts, vec![(1.0, 0.0), (2.0, 0.0)]);
        assert!(eval_numbers(&parse("[1...1000000]"), &env, &lists).is_err());
        let z = eval_values(&parse("[1 + i, 2i]"), &env, &lists).unwrap();
        assert_eq!(z, vec![C64::new(1.0, 1.0), C64::new(0.0, 2.0)]);
        assert!(eval_numbers(&parse("[1 + i, 2i]"), &env, &lists).is_err());
    }

    #[test]
    fn rewrite() {
        assert_eq!(rewrite_point_text("A = (1, 2)", Some("3".into()), None).unwrap(), "A = (3, 2)");
        assert_eq!(rewrite_point_text("(a, -1) {x > 0}", None, Some("5".into())).unwrap(), "(a, 5) {x > 0}");
        assert_eq!(rewrite_point_text("P=(f(1,2), 3)", None, Some("4".into())).unwrap(), "P = (f(1,2), 4)");
    }
}
