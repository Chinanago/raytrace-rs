//! Discrete wavelengths with relative radiometric power, including Zemax SPCD files.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spectrum {
    /// Ascending [wavelength_um, relative_power] pairs, sampled without interpolation.
    pub samples: Vec<[f64; 2]>,
}
impl Spectrum {
    pub fn validate(&self) -> Result<(), String> {
        if self.samples.is_empty()
            || self.samples.len() > 200
            || !self
                .samples
                .iter()
                .all(|v| v[0].is_finite() && v[0] > 0. && v[1].is_finite() && v[1] >= 0.)
            || !self.samples.windows(2).all(|w| w[0][0] < w[1][0])
            || !self.samples.iter().map(|v| v[1]).sum::<f64>().is_finite()
            || self.samples.iter().map(|v| v[1]).sum::<f64>() <= 0.
        {
            return Err("スペクトルは昇順の波長と非負の光量重みで指定し、合計重みを正にしてください（最大200点）".into());
        }
        Ok(())
    }
    pub fn parse_spcd(text: &str) -> Result<Self, String> {
        let mut samples = Vec::new();
        for line in text.lines() {
            let body = line.split(['#', '!']).next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let values: Vec<f64> = body
                .split_whitespace()
                .map(|s| {
                    s.replace(['D', 'd'], "E")
                        .parse::<f64>()
                        .map_err(|_| format!("SPCD数値を読めません: {s}"))
                })
                .collect::<Result<_, _>>()?;
            if values.len() != 2 {
                return Err("SPCDは波長[um]と相対光量の2列で指定してください".into());
            }
            samples.push([values[0], values[1]]);
        }
        if samples.len() < 3 {
            return Err("SPCDには3〜200点の波長と重みが必要です".into());
        }
        let spectrum = Self { samples };
        spectrum.validate()?;
        Ok(spectrum)
    }
    pub fn sample(&self, u: f64) -> f64 {
        let total = self.samples.iter().map(|v| v[1]).sum::<f64>();
        self.sample_with_total(u, total)
    }
    /// Preserve the subtraction order used by saved sampling revision 2.
    pub(crate) fn sample_with_total(&self, u: f64, total: f64) -> f64 {
        let mut target = u * total;
        for &[wave, weight] in &self.samples {
            if target < weight {
                return wave;
            }
            target -= weight;
        }
        self.samples.iter().rfind(|v| v[1] > 0.).unwrap()[0]
    }
}
