use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Material {
    Catalog {
        formula: usize,
        coefficients: Vec<f64>,
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    WithAbsorption {
        dispersion: Box<Material>,
        samples: Vec<[f64; 2]>,
    },
    Constant {
        n: f64,
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// n² = 1 + sum(B_i * lambda² / (lambda² - C_i)). C_i has units um².
    Sellmeier {
        b: [f64; 3],
        c_um2: [f64; 3],
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// n² = A0 + A1*l² + A2/l² + A3/l⁴ + A4/l⁶ + A5/l⁸ (AGF formula 1).
    Schott {
        coefficients: [f64; 6],
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// Sorted [wavelength_um, n] pairs. Linear interpolation, no extrapolation.
    Tabulated {
        samples: Vec<[f64; 2]>,
        #[serde(default)]
        absorption_per_mm: f64,
    },
}

impl Material {
    pub fn index(&self, wavelength_um: f64) -> Option<f64> {
        if !wavelength_um.is_finite() || wavelength_um <= 0.0 {
            return None;
        }
        let n = match self {
            Self::Catalog {
                formula,
                coefficients,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                crate::dispersion::index(*formula, coefficients, wavelength_um)?
            }
            Self::WithAbsorption { dispersion, .. } => dispersion.index(wavelength_um)?,
            Self::Constant { n, .. } => *n,
            Self::Sellmeier {
                b,
                c_um2,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                let l2 = wavelength_um * wavelength_um;
                (1.0 + (0..3).map(|i| b[i] * l2 / (l2 - c_um2[i])).sum::<f64>()).sqrt()
            }
            Self::Schott {
                coefficients: a,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                let l2 = wavelength_um * wavelength_um;
                (a[0]
                    + a[1] * l2
                    + a[2] / l2
                    + a[3] / l2.powi(2)
                    + a[4] / l2.powi(3)
                    + a[5] / l2.powi(4))
                .sqrt()
            }
            Self::Tabulated { samples, .. } => {
                if samples.len() < 2
                    || wavelength_um < samples.first()?[0]
                    || wavelength_um > samples.last()?[0]
                {
                    return None;
                }
                let i = samples.partition_point(|s| s[0] < wavelength_um);
                if i == 0 {
                    samples[0][1]
                } else {
                    let [x0, y0] = samples[i - 1];
                    let [x1, y1] = samples[i];
                    y0 + (y1 - y0) * (wavelength_um - x0) / (x1 - x0)
                }
            }
        };
        (n.is_finite() && n > 0.0).then_some(n)
    }
    pub fn absorption(&self) -> f64 {
        match self {
            Self::WithAbsorption { .. } => 0.0,
            Self::Constant {
                absorption_per_mm, ..
            }
            | Self::Catalog {
                absorption_per_mm, ..
            }
            | Self::Sellmeier {
                absorption_per_mm, ..
            }
            | Self::Schott {
                absorption_per_mm, ..
            }
            | Self::Tabulated {
                absorption_per_mm, ..
            } => *absorption_per_mm,
        }
    }
    pub fn absorption_at(&self, wavelength_um: f64) -> Option<f64> {
        if let Self::WithAbsorption { samples, .. } = self {
            if samples.is_empty()
                || wavelength_um < samples.first()?[0]
                || wavelength_um > samples.last()?[0]
            {
                return None;
            }
            let i = samples.partition_point(|v| v[0] < wavelength_um);
            if i == 0 {
                return Some(samples[0][1]);
            }
            let [x0, y0] = samples[i - 1];
            let [x1, y1] = samples[i];
            Some(y0 + (y1 - y0) * (wavelength_um - x0) / (x1 - x0))
        } else {
            Some(self.absorption())
        }
    }
}
