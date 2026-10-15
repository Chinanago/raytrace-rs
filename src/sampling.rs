//! Six-dimensional Sobol points with a seeded linear matrix scramble and digital shift.
//! Positions, angles, and diode-array indices each use fixed, separate dimensions.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sampling {
    #[default]
    Sobol,
    Pseudorandom,
}

const DIMS: usize = 6;
const BITS: usize = 52;
const SCALE: f64 = 1.0 / (1u64 << BITS) as f64;

pub(crate) fn random_bits(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

pub(crate) fn uniform(state: &mut u64) -> f64 {
    (random_bits(state) >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

fn directions() -> [[u64; BITS]; DIMS] {
    let mut result = [[0; BITS]; DIMS];
    for (j, value) in result[0].iter_mut().enumerate() {
        *value = 1 << (63 - j);
    }
    // First six Joe-Kuo dimensions, also used by SciPy's unscrambled Sobol engine.
    // https://web.maths.unsw.edu.au/~fkuo/sobol/
    let parameters = [
        (1, 0, [1, 0, 0, 0]),
        (2, 1, [1, 3, 0, 0]),
        (3, 1, [1, 3, 1, 0]),
        (3, 2, [1, 1, 1, 0]),
        (4, 1, [1, 1, 3, 3]),
    ];
    for (dim, (degree, polynomial, initial)) in parameters.into_iter().enumerate() {
        let v = &mut result[dim + 1];
        for j in 0..degree {
            v[j] = (initial[j] as u64) << (63 - j);
        }
        for j in degree..BITS {
            let mut value = v[j - degree] ^ (v[j - degree] >> degree);
            for k in 1..degree {
                if (polynomial >> (degree - 1 - k)) & 1 != 0 {
                    value ^= v[j - k];
                }
            }
            v[j] = value;
        }
    }
    result
}

pub(crate) struct Sobol {
    directions: [[u64; BITS]; DIMS],
    state: [u64; DIMS],
    index: u64,
}

impl Sobol {
    pub(crate) fn new(seed: u64) -> Self {
        let mut rng = seed;
        let mut directions = directions();
        let mut state = [0; DIMS];
        for dim in 0..DIMS {
            let rows: [u64; BITS] = std::array::from_fn(|row| {
                let above_diagonal = if row == 0 { 0 } else { u64::MAX << (64 - row) };
                (random_bits(&mut rng) & above_diagonal) | (1 << (63 - row))
            });
            for value in &mut directions[dim] {
                *value = rows.iter().enumerate().fold(0, |out, (row, mask)| {
                    out | (((*value & mask).count_ones() as u64 & 1) << (63 - row))
                });
            }
            state[dim] = random_bits(&mut rng) & (u64::MAX << (64 - BITS));
        }
        Self {
            directions,
            state,
            index: 0,
        }
    }

    pub(crate) fn next(&mut self) -> [f64; DIMS] {
        assert!(self.index < 1 << BITS, "Sobol sequence exhausted");
        // Midpoints of the 52-bit intervals keep inverse CDF inputs strictly in (0,1).
        let point = std::array::from_fn(|d| ((self.state[d] >> (64 - BITS)) as f64 + 0.5) * SCALE);
        self.index += 1;
        if self.index < 1 << BITS {
            let column = self.index.trailing_zeros() as usize;
            for d in 0..DIMS {
                self.state[d] ^= self.directions[d][column];
            }
        }
        point
    }
}

/// Inverse-CDF sampling of a centred Gaussian truncated to [-half_width, half_width].
pub(crate) fn gaussian_quantile(u: f64, sigma: f64, half_width: f64) -> f64 {
    if sigma == 0.0 || half_width == 0.0 {
        return 0.0;
    }
    let a = half_width / sigma;
    let signed = 2.0 * u - 1.0;
    if a < 1e-3 {
        // Invert the central CDF series without subtracting nearly equal tail probabilities.
        return half_width * (signed + a * a * (signed.powi(3) - signed) / 6.0);
    }
    let half_mass = 0.5 * libm::erf(a / std::f64::consts::SQRT_2);
    let tail = 0.5 - signed.abs() * half_mass;
    let value = -normal_quantile_lower(tail) * sigma * signed.signum();
    value.clamp(-half_width, half_width)
}

fn normal_quantile_lower(p: f64) -> f64 {
    // Acklam's rational approximation, refined with the Gaussian CDF (Halley's method).
    // The symmetry used by gaussian_quantile keeps p in (0, 0.5].
    let mut x = if p < 0.02425 {
        let q = (-2.0 * p.ln()).sqrt();
        (((((-7.784894002430293e-3 * q - 0.3223964580411365) * q - 2.400758277161838) * q
            - 2.549732539343734)
            * q
            + 4.374664141464968)
            * q
            + 2.938163982698783)
            / ((((7.784695709041462e-3 * q + 0.3224671290700398) * q + 2.445134137142996) * q
                + 3.754408661907416)
                * q
                + 1.0)
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((-39.69683028665376 * r + 220.9460984245205) * r - 275.9285104469687) * r
            + 138.357751867269)
            * r
            - 30.66479806614716)
            * r
            + 2.506628277459239)
            * q
            / (((((-54.47609879822406 * r + 161.5858368580409) * r - 155.6989798598866) * r
                + 66.80131188771972)
                * r
                - 13.28068155288572)
                * r
                + 1.0)
    };
    let error = 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2) - p;
    let correction = error * (2.0 * std::f64::consts::PI).sqrt() * (0.5 * x * x).exp();
    x -= correction / (1.0 + 0.5 * x * correction);
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sobol_direction_numbers_match_six_dimensional_reference() {
        // Independent reference: scipy.stats.qmc.Sobol(6, scramble=False).random_base2(3).
        let expected = [
            [0, 0, 0, 0, 0, 0],
            [4, 4, 4, 4, 4, 4],
            [6, 2, 2, 2, 6, 6],
            [2, 6, 6, 6, 2, 2],
            [3, 3, 5, 7, 3, 1],
            [7, 7, 1, 3, 7, 5],
            [5, 1, 7, 5, 5, 7],
            [1, 5, 3, 1, 1, 3],
        ];
        let directions = directions();
        for (i, point) in expected.into_iter().enumerate() {
            let gray = i ^ (i >> 1);
            for d in 0..DIMS {
                let bits = (0..3)
                    .filter(|j| gray & (1 << j) != 0)
                    .fold(0, |bits, j| bits ^ directions[d][j]);
                assert_eq!(bits >> 61, point[d]);
            }
        }
    }

    #[test]
    fn gaussian_inverse_matches_cdf_across_truncation_scales() {
        for a in [1e-8, 1e-4, 0.001, 0.5, 1.0, 4.0, 50.0] {
            for u in [1e-12, 0.0001, 0.1, 0.25, 0.5, 0.9, 1.0 - 1e-12] {
                let x = gaussian_quantile(u, 1.0, a);
                let actual = 0.5
                    + 0.5 * libm::erf(x / std::f64::consts::SQRT_2)
                        / libm::erf(a / std::f64::consts::SQRT_2);
                assert!((actual - u).abs() < 2e-12, "a={a}, u={u}, actual={actual}");
                assert!(x.is_finite() && x.abs() <= a);
            }
        }
    }
}
