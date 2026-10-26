//! Funciones especiales e integración numérica.

/// Función W de Lambert: solución de w·e^w = x. `branch` 0 = rama principal (x ≥ −1/e),
/// −1 = rama inferior (−1/e ≤ x < 0).
pub fn lambert_w(x: f64, branch: i32) -> f64 {
    const INV_E: f64 = 1.0 / std::f64::consts::E;
    if !x.is_finite() {
        return if branch == 0 && x == f64::INFINITY { x } else { f64::NAN };
    }
    if x < -INV_E - 1e-15 || (branch == -1 && x >= 0.0) || (branch != 0 && branch != -1) {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    let x = x.max(-INV_E);
    // Estimación inicial.
    let p = (2.0 * (std::f64::consts::E * x + 1.0)).max(0.0).sqrt();
    let mut w = if branch == 0 {
        if x < -0.25 {
            -1.0 + p - p * p / 3.0 + 11.0 / 72.0 * p * p * p
        } else if x < 3.0 {
            x / (1.0 + x).sqrt().max(1e-300) * 0.9
        } else {
            let l = x.ln();
            l - l.ln()
        }
    } else if x < -0.25 {
        -1.0 - p - p * p / 3.0 - 11.0 / 72.0 * p * p * p
    } else {
        let l = (-x).ln();
        l - (-l).ln()
    };
    // Iteración de Halley.
    for _ in 0..64 {
        let ew = w.exp();
        let f = w * ew - x;
        let wp1 = w + 1.0;
        if wp1 == 0.0 {
            break;
        }
        let dw = f / (ew * wp1 - (w + 2.0) * f / (2.0 * wp1));
        w -= dw;
        if dw.abs() <= 1e-15 * (1.0 + w.abs()) {
            break;
        }
    }
    w
}

fn is_int(v: f64) -> bool {
    v.is_finite() && v.fract() == 0.0
}

/// Combinaciones C(n, r). Con enteros es exacto; si no, se usa la función gamma.
pub fn ncr(n: f64, r: f64) -> f64 {
    if is_int(n) && is_int(r) && n >= 0.0 {
        if r < 0.0 || r > n {
            return 0.0;
        }
        let r = r.min(n - r);
        let mut acc = 1.0;
        for k in 1..=(r as u64) {
            acc = acc * (n - r + k as f64) / k as f64;
        }
        return acc.round();
    }
    libm::tgamma(n + 1.0) / (libm::tgamma(r + 1.0) * libm::tgamma(n - r + 1.0))
}

/// Permutaciones (variaciones) P(n, r) = n! / (n − r)!.
pub fn npr(n: f64, r: f64) -> f64 {
    if is_int(n) && is_int(r) && n >= 0.0 {
        if r < 0.0 || r > n {
            return 0.0;
        }
        let mut acc = 1.0;
        for k in 0..(r as u64) {
            acc *= n - k as f64;
        }
        return acc;
    }
    libm::tgamma(n + 1.0) / libm::tgamma(n - r + 1.0)
}

/// Máximo común divisor (solo enteros).
pub fn gcd(a: f64, b: f64) -> f64 {
    if !is_int(a) || !is_int(b) {
        return f64::NAN;
    }
    let (mut a, mut b) = (a.abs(), b.abs());
    while b > 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Mínimo común múltiplo (solo enteros).
pub fn lcm(a: f64, b: f64) -> f64 {
    let g = gcd(a, b);
    if g.is_nan() {
        return g;
    }
    if g == 0.0 { 0.0 } else { (a / g * b).abs() }
}

/// Función beta B(a, b) = Γ(a)Γ(b) / Γ(a + b).
pub fn beta(a: f64, b: f64) -> f64 {
    if a > 0.0 && b > 0.0 {
        (libm::lgamma(a) + libm::lgamma(b) - libm::lgamma(a + b)).exp()
    } else {
        libm::tgamma(a) * libm::tgamma(b) / libm::tgamma(a + b)
    }
}

// ───────────────────────────── Integración ─────────────────────────────

/// Nodos y pesos de Gauss–Kronrod 15 (con el Gauss de 7 puntos embebido), de QUADPACK.
const XGK: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_4,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_5,
    0.0,
];
const WGK: [f64; 8] = [
    0.022_935_322_010_529_22,
    0.063_092_092_629_978_55,
    0.104_790_010_322_250_2,
    0.140_653_259_715_525_9,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_4,
    0.204_432_940_075_298_9,
    0.209_482_141_084_727_8,
];
const WG: [f64; 4] = [0.129_484_966_168_869_7, 0.279_705_391_489_276_7, 0.381_830_050_505_118_9, 0.417_959_183_673_469_4];

/// Gauss–Kronrod en [a, b]: (integral, estimación del error).
fn gk15(f: &mut dyn FnMut(f64) -> f64, a: f64, b: f64) -> (f64, f64) {
    let c = 0.5 * (a + b);
    let h = 0.5 * (b - a);
    let fc = f(c);
    let mut kronrod = fc * WGK[7];
    let mut gauss = fc * WG[3];
    for j in 0..7 {
        let dx = h * XGK[j];
        let s = f(c - dx) + f(c + dx);
        kronrod += WGK[j] * s;
        if j % 2 == 1 {
            gauss += WG[j / 2] * s;
        }
    }
    (kronrod * h, ((kronrod - gauss) * h).abs())
}

fn adaptive(f: &mut dyn FnMut(f64) -> f64, a: f64, b: f64, tol: f64, depth: u32, evals: &mut u32) -> f64 {
    let (k, e) = gk15(f, a, b);
    *evals += 15;
    if !k.is_finite() || e <= tol.max(1e-15 * k.abs()) || depth == 0 || *evals > 40_000 {
        return k;
    }
    let m = 0.5 * (a + b);
    adaptive(f, a, m, tol * 0.5, depth - 1, evals) + adaptive(f, m, b, tol * 0.5, depth - 1, evals)
}

/// ∫ₐᵇ f(u) du, con a y b finitos o infinitos (se cambia de variable para llevarlos a
/// un intervalo finito; Gauss–Kronrod no evalúa en los extremos).
pub fn integrate(f: &mut dyn FnMut(f64) -> f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return 0.0;
    }
    if a > b {
        return -integrate(f, b, a);
    }
    let mut evals = 0;
    let tol = 1e-11;
    match (a.is_finite(), b.is_finite()) {
        (true, true) => {
            // Tolerancia relativa a una primera estimación.
            let (rough, _) = gk15(f, a, b);
            adaptive(f, a, b, tol * rough.abs().max(1e-3), 50, &mut evals)
        }
        // u = a + t/(1−t), t ∈ [0, 1)
        (true, false) => {
            let mut g = |t: f64| {
                let s = 1.0 - t;
                f(a + t / s) / (s * s)
            };
            adaptive(&mut g, 0.0, 1.0, tol, 50, &mut evals)
        }
        // u = b − t/(1−t)
        (false, true) => {
            let mut g = |t: f64| {
                let s = 1.0 - t;
                f(b - t / s) / (s * s)
            };
            adaptive(&mut g, 0.0, 1.0, tol, 50, &mut evals)
        }
        // u = t/(1−t²), t ∈ (−1, 1)
        (false, false) => {
            let mut g = |t: f64| {
                let s = 1.0 - t * t;
                f(t / s) * (1.0 + t * t) / (s * s)
            };
            adaptive(&mut g, -1.0, 1.0, tol, 50, &mut evals)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + b.abs())
    }

    #[test]
    fn lambert() {
        assert!(close(lambert_w(1.0, 0), 0.567_143_290_409_783_8, 1e-14));
        assert!(close(lambert_w(10.0, 0), 1.745_528_002_740_699_4, 1e-14));
        assert!(close(lambert_w(-0.2, 0), -0.259_171_101_819_073_7, 1e-13));
        assert!(close(lambert_w(-0.2, -1), -2.542_641_357_773_527, 1e-13));
        assert!(close(lambert_w(-1.0 / std::f64::consts::E, 0), -1.0, 1e-6));
        assert!(lambert_w(-1.0, 0).is_nan());
        for x in [0.001, 0.5, 2.0, 100.0, 1e6] {
            let w = lambert_w(x, 0);
            assert!(close(w * w.exp(), x, 1e-13), "W({x})");
        }
    }

    #[test]
    fn combinatorics() {
        assert_eq!(ncr(5.0, 2.0), 10.0);
        assert_eq!(ncr(52.0, 5.0), 2_598_960.0);
        assert_eq!(npr(5.0, 2.0), 20.0);
        assert_eq!(ncr(3.0, 5.0), 0.0);
        assert!(close(ncr(4.5, 2.0), 7.875, 1e-12));
        assert_eq!(gcd(12.0, 18.0), 6.0);
        assert_eq!(lcm(4.0, 6.0), 12.0);
        assert!(gcd(1.5, 2.0).is_nan());
        assert!(close(beta(2.0, 3.0), 1.0 / 12.0, 1e-12));
    }

    #[test]
    fn integration() {
        let pi = std::f64::consts::PI;
        assert!(close(integrate(&mut |x| x * x, 0.0, 3.0), 9.0, 1e-12));
        assert!(close(integrate(&mut f64::sin, 0.0, pi), 2.0, 1e-12));
        assert!(close(integrate(&mut |x| (-x).exp(), 0.0, f64::INFINITY), 1.0, 1e-9));
        assert!(close(integrate(&mut |x| (-x * x).exp(), f64::NEG_INFINITY, f64::INFINITY), pi.sqrt(), 1e-9));
        assert!(close(integrate(&mut |x| 1.0 / (1.0 + x * x), 0.0, f64::INFINITY), pi / 2.0, 1e-9));
        assert!(close(integrate(&mut |x| x.abs(), -1.0, 2.0), 2.5, 1e-9));
        assert!(close(integrate(&mut |x| x * x, 3.0, 0.0), -9.0, 1e-12));
        // Singularidad integrable en el extremo.
        assert!(close(integrate(&mut |x| 1.0 / x.sqrt(), 0.0, 1.0), 2.0, 1e-6));
    }
}
