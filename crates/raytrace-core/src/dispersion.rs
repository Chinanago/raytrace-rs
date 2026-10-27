//! The thirteen AGF dispersion formulas, with wavelength measured in micrometres.
pub fn coefficient_count(formula: usize) -> Option<usize> {
    Some(match formula {
        1..=3 => 6,
        4 | 9 => 5,
        5 => 3,
        6 | 10 | 12 => 8,
        7 | 8 => 4,
        11 => 10,
        13 => 9,
        _ => return None,
    })
}
pub fn index(formula: usize, a: &[f64], wavelength: f64) -> Option<f64> {
    let count = coefficient_count(formula)?;
    if a.len() < count
        || !a.iter().all(|v| v.is_finite())
        || !wavelength.is_finite()
        || wavelength <= 0.
    {
        return None;
    }
    let l = wavelength;
    let l2 = l * l;
    let n = match formula {
        1 | 10 | 12 | 13 => {
            let exponents: &[i32] = match formula {
                1 => &[0, 2, -2, -4, -6, -8],
                10 => &[0, 2, -2, -4, -6, -8, -10, -12],
                12 => &[0, 2, -2, -4, -6, -8, 4, 6],
                _ => &[0, 2, 4, -2, -4, -6, -8, -10, -12],
            };
            a.iter()
                .zip(exponents)
                .map(|(a, e)| a * l.powi(*e))
                .sum::<f64>()
                .sqrt()
        }
        2 | 6 | 11 => (1.
            + a[..count]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| s[0] * l2 / (l2 - s[1]))
                .sum::<f64>())
        .sqrt(),
        3 => {
            let h = 1. / (l2 - 0.028);
            a[0] + a[1] * h + a[2] * h * h + a[3] * l2 + a[4] * l2 * l2 + a[5] * l2 * l2 * l2
        }
        4 => (1. + a[0] + a[1] * l2 / (l2 - a[2] * a[2]) + a[3] / (l2 - a[4] * a[4])).sqrt(),
        5 => a[0] + a[1] / l + a[2] / l.powf(3.5),
        7 => (a[0] + a[1] / (l2 - a[2]) - a[3] * l2).sqrt(),
        8 => (a[0] + a[1] * l2 / (l2 - a[2]) - a[3] * l2).sqrt(),
        9 => (a[0] + a[1] * l2 / (l2 - a[2]) + a[3] * l2 / (l2 - a[4])).sqrt(),
        _ => return None,
    };
    (n.is_finite() && n > 0.).then_some(n)
}
