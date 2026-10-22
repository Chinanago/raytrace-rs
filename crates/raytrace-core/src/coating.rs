//! Scalar power coatings. TABLE interpolation follows the documented Zemax convention.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coating {
    pub name: String,
    /// Incidence from the minus side of the surface normal.
    pub front: CoatingModel,
    /// Incidence from the plus side; omission uses the front model on both sides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<CoatingModel>,
    /// Original named coating block, including S/P and phase data for lossless exchange.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zemax_definition: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoatingModel {
    Ideal {
        reflectance: f64,
        transmittance: f64,
        #[serde(default = "one")]
        tir_reflectance: f64,
    },
    Table {
        wavelengths_um: Vec<f64>,
        angles_deg: Vec<f64>,
        /// Angle-major, then wavelength; each element is [R, T] in power units.
        coefficients: Vec<[f64; 2]>,
        /// Zemax TABLE angles refer to the lower-index medium on either traversal.
        #[serde(default)]
        low_index_angle: bool,
        #[serde(default = "one")]
        tir_reflectance: f64,
    },
}
fn one() -> f64 {
    1.0
}
fn fraction(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}
fn power(v: [f64; 2]) -> bool {
    v.iter().all(|v| fraction(*v)) && v[0] + v[1] <= 1.0 + 1e-12
}

impl CoatingModel {
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Ideal {
                reflectance,
                transmittance,
                tir_reflectance,
            } => power([*reflectance, *transmittance]) && fraction(*tir_reflectance),
            Self::Table {
                wavelengths_um,
                angles_deg,
                coefficients,
                tir_reflectance,
                ..
            } => {
                !wavelengths_um.is_empty()
                    && !angles_deg.is_empty()
                    && wavelengths_um.iter().all(|w| w.is_finite() && *w > 0.0)
                    && wavelengths_um.windows(2).all(|w| w[0] < w[1])
                    && angles_deg
                        .iter()
                        .all(|a| a.is_finite() && (0.0..=90.0).contains(a))
                    && angles_deg.windows(2).all(|a| a[0] < a[1])
                    && angles_deg.len().checked_mul(wavelengths_um.len())
                        == Some(coefficients.len())
                    && coefficients.iter().all(|v| power(*v))
                    && fraction(*tir_reflectance)
            }
        };
        valid
            .then_some(())
            .ok_or_else(|| "コーティングの波長・角度・R/T収支が不正です".into())
    }
    pub fn covers(&self, wavelength: f64) -> bool {
        match self {
            Self::Ideal { .. } => true,
            Self::Table { wavelengths_um, .. } => wavelengths_um
                .first()
                .zip(wavelengths_um.last())
                .is_some_and(|(lo, hi)| (*lo..=*hi).contains(&wavelength)),
        }
    }
    pub fn opaque(&self) -> bool {
        match self {
            Self::Ideal { transmittance, .. } => *transmittance == 0.0,
            Self::Table { coefficients, .. } => coefficients.iter().all(|v| v[1] == 0.0),
        }
    }
    pub fn evaluate(
        &self,
        wavelength: f64,
        cos_incident: f64,
        n_before: f64,
        n_after: f64,
        tir: bool,
    ) -> [f64; 2] {
        match self {
            Self::Ideal {
                reflectance,
                transmittance,
                tir_reflectance,
            } => {
                if tir {
                    [*tir_reflectance, 0.0]
                } else {
                    [*reflectance, transmittance.min(1.0 - reflectance)]
                }
            }
            Self::Table {
                wavelengths_um: waves,
                angles_deg: angles,
                coefficients: data,
                low_index_angle,
                tir_reflectance,
            } => {
                if tir {
                    return [*tir_reflectance, 0.0];
                }
                let cosine = if *low_index_angle && n_before > n_after {
                    (1.0 - (n_before / n_after).powi(2) * (1.0 - cos_incident.powi(2)))
                        .max(0.0)
                        .sqrt()
                } else {
                    cos_incident.clamp(0.0, 1.0)
                };
                let angle = cosine.acos().to_degrees();
                let ai = angles.partition_point(|a| *a < angle).min(angles.len() - 1);
                let a0 = ai.saturating_sub(1);
                let w = waves
                    .partition_point(|w| *w < wavelength)
                    .min(waves.len() - 1);
                let w0 = w.saturating_sub(1);
                let wt = if w == w0 {
                    0.0
                } else {
                    (wavelength - waves[w0]) / (waves[w] - waves[w0])
                };
                let at = if ai == a0 {
                    0.0
                } else {
                    ((cosine - angles[a0].to_radians().cos())
                        / (angles[ai].to_radians().cos() - angles[a0].to_radians().cos()))
                    .clamp(0.0, 1.0)
                };
                let mut rt = std::array::from_fn(|k| {
                    let row = |a: usize| {
                        data[a * waves.len() + w0][k] * (1.0 - wt)
                            + data[a * waves.len() + w][k] * wt
                    };
                    row(a0) * (1.0 - at) + row(ai) * at
                });
                rt[1] = rt[1].min(1.0 - rt[0]);
                rt
            }
        }
    }
}
impl Coating {
    pub fn to_zemax(&self) -> Result<String, String> {
        self.validate()?;
        let same = |a: &CoatingModel, b: &CoatingModel| {
            serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
        };
        if self.back.as_ref().is_some_and(|b| !same(b, &self.front)) {
            return Err("表裏で異なるコーティングは単一のZemax IDEAL/TABLEで表現できません".into());
        }
        if let Some(original) = &self.zemax_definition {
            if parse_zemax(original, &self.name)?.is_some_and(|c| same(&c.front, &self.front)) {
                return Ok(original.clone());
            }
        }
        if self.name.is_empty()
            || self.name.len() > 32
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err("Zemaxのコーティング名は32文字以内の英数字・下線で指定してください".into());
        }
        let mut text = String::new();
        match &self.front {
            CoatingModel::Ideal {
                reflectance,
                transmittance,
                tir_reflectance,
            } => text.push_str(&format!(
                "IDEAL {} {:.17e} {:.17e} {:.17e}\n",
                self.name, transmittance, reflectance, tir_reflectance
            )),
            CoatingModel::Table {
                wavelengths_um,
                angles_deg,
                coefficients,
                low_index_angle: true,
                tir_reflectance: 1.0,
            } => {
                text.push_str(&format!("TABLE {}\n", self.name));
                for (a, angle) in angles_deg.iter().enumerate() {
                    text.push_str(&format!("ANGL {angle:.17e}\n"));
                    for (w, wave) in wavelengths_um.iter().enumerate() {
                        let [r, t] = coefficients[a * wavelengths_um.len() + w];
                        text.push_str(&format!(
                            "WAVE {wave:.17e} {r:.17e} {r:.17e} {t:.17e} {t:.17e} 0 0 0 0\n"
                        ));
                    }
                }
            }
            _ => {
                return Err(
                    "この角度基準・全反射係数の表はZemax TABLEで表現できません。JSONへ保存できます"
                        .into(),
                )
            }
        }
        Ok(text)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.front.validate()?;
        if let Some(back) = &self.back {
            back.validate()?;
        }
        Ok(())
    }
    pub fn side(&self, from_minus: bool) -> &CoatingModel {
        if from_minus {
            &self.front
        } else {
            self.back.as_ref().unwrap_or(&self.front)
        }
    }
    pub fn covers(&self, wavelength: f64) -> bool {
        self.front.covers(wavelength) && self.back.as_ref().is_none_or(|b| b.covers(wavelength))
    }
}

fn number(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .replace(['d', 'D'], "E")
        .parse()
        .map_err(|_| format!("コーティング数値が不正です: {s}"))?;
    if !v.is_finite() {
        return Err("コーティングに非有限値があります".into());
    }
    Ok(v)
}

/// Read a named IDEAL or TABLE coating; all other prescriptions remain unsupported.
pub fn parse_zemax(text: &str, name: &str) -> Result<Option<Coating>, String> {
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| {
            l.split('!')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect()
        })
        .collect();
    let Some((start, header)) = rows.iter().enumerate().find(|(_, w)| {
        w.get(1).is_some_and(|n| n.eq_ignore_ascii_case(name))
            && w.first()
                .is_some_and(|k| ["IDEAL", "TABLE", "COAT", "IDEAL2"].contains(k))
    }) else {
        return Ok(None);
    };
    let front = match header[0] {
        "IDEAL" => {
            if !(4..=5).contains(&header.len()) {
                return Err("IDEALにはT・R・任意のTIR係数が必要です".into());
            }
            let mut t = number(header[2])?;
            let mut r = number(header[3])?;
            if t < 0.0 || r < 0.0 {
                return Err("IDEALのT/Rは非負で指定してください".into());
            }
            if t + r > 1.0 {
                let sum = t + r;
                if !sum.is_finite() {
                    return Err("IDEAL係数の合計が大きすぎます".into());
                }
                t /= sum;
                r /= sum;
            }
            CoatingModel::Ideal {
                reflectance: r,
                transmittance: t,
                tir_reflectance: header.get(4).map(|s| number(s)).transpose()?.unwrap_or(1.0),
            }
        }
        "TABLE" => {
            let mut angles = Vec::new();
            let mut grid: Vec<Vec<f64>> = Vec::new();
            let mut data = Vec::new();
            for w in rows.iter().skip(start + 1).filter(|w| !w.is_empty()) {
                if [
                    "COAT",
                    "TABLE",
                    "IDEAL",
                    "IDEAL2",
                    "MATE",
                    "TAPR",
                    "ENCRYPTED",
                ]
                .contains(&w[0])
                {
                    break;
                }
                match w[0] {
                    "ANGL" if w.len() == 2 => {
                        angles.push(number(w[1])?);
                        grid.push(Vec::new());
                    }
                    "WAVE" if [6, 10].contains(&w.len()) && !grid.is_empty() => {
                        let v: Vec<_> = w
                            .iter()
                            .skip(1)
                            .map(|s| number(s))
                            .collect::<Result<_, _>>()?;
                        if !power([v[1], v[3]]) || !power([v[2], v[4]]) {
                            return Err("TABLEのS/PごとのR+Tが1を超えています".into());
                        }
                        grid.last_mut().unwrap().push(v[0]);
                        data.push([(v[1] + v[2]) / 2.0, (v[3] + v[4]) / 2.0]);
                    }
                    _ => {
                        return Err(format!(
                            "TABLEのレコードが不正または未対応です: {}",
                            w.join(" ")
                        ))
                    }
                }
            }
            let wavelengths_um = grid.first().ok_or("TABLEにANGLがありません")?.clone();
            if grid.iter().any(|g| *g != wavelengths_um) {
                return Err("TABLEの波長表は全角度で揃えてください".into());
            }
            CoatingModel::Table {
                wavelengths_um,
                angles_deg: angles,
                coefficients: data,
                low_index_angle: true,
                tir_reflectance: 1.0,
            }
        }
        _ => return Err(
            "このコーティングは多層膜・複素振幅の計算が必要です。現在はIDEAL/TABLEに対応しています"
                .into(),
        ),
    };
    let source_lines: Vec<_> = text.lines().collect();
    let end = rows
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, w)| {
            w.first().is_some_and(|k| {
                [
                    "COAT",
                    "TABLE",
                    "IDEAL",
                    "IDEAL2",
                    "MATE",
                    "TAPR",
                    "ENCRYPTED",
                ]
                .contains(k)
            })
        })
        .map_or(rows.len(), |(i, _)| i);
    let coating = Coating {
        name: name.into(),
        front,
        back: None,
        zemax_definition: Some(source_lines[start..end].join("\n") + "\n"),
    };
    coating.validate()?;
    Ok(Some(coating))
}
