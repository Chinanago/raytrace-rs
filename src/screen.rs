//! Screen post-processing. Detector weights remain integrated power per pixel.
use crate::trace::DetectorMap;

#[derive(Clone, Debug)]
pub struct HalfMaximum {
    pub lower_mm: Option<f64>,
    pub upper_mm: Option<f64>,
    /// More than one disconnected sampled region reaches half of the peak.
    pub multiple_lobes: bool,
}
impl HalfMaximum {
    pub fn width_mm(&self) -> Option<f64> {
        Some(self.upper_mm? - self.lower_mm?)
    }
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub weights: Vec<f64>,
    pub first_center_mm: f64,
    pub pitch_mm: f64,
    pub half_maximum: HalfMaximum,
}
impl Profile {
    pub fn peak_weight(&self) -> f64 {
        self.weights.iter().copied().fold(0., f64::max)
    }
    pub fn coordinate_mm(&self, index: usize) -> f64 {
        self.first_center_mm + index as f64 * self.pitch_mm
    }
    fn new(weights: Vec<f64>, half_extent_mm: f64, peak_index: usize, peak: f64) -> Self {
        let pitch_mm = 2.0 * half_extent_mm / weights.len() as f64;
        let first_center_mm = -half_extent_mm + 0.5 * pitch_mm;
        let half = peak * 0.5;
        let crossing = |outer: usize, inner: usize| {
            let fraction = (half - weights[inner]) / (weights[outer] - weights[inner]);
            first_center_mm + (inner as f64 + fraction * (outer as f64 - inner as f64)) * pitch_mm
        };
        let lower_mm = (0..peak_index)
            .rev()
            .find(|&i| weights[i] <= half)
            .map(|i| crossing(i, i + 1));
        let upper_mm = (peak_index + 1..weights.len())
            .find(|&i| weights[i] <= half)
            .map(|i| crossing(i, i - 1));
        let mut previous = false;
        let mut lobes = 0;
        for &weight in &weights {
            let above = weight >= half;
            if above && !previous {
                lobes += 1;
            }
            previous = above;
        }
        Self {
            weights,
            first_center_mm,
            pitch_mm,
            half_maximum: HalfMaximum {
                lower_mm,
                upper_mm,
                multiple_lobes: lobes > 1,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScreenAnalysis {
    pub peak_weight: f64,
    pub peak_pixel: Option<[usize; 2]>,
    pub peak_position_mm: Option<[f64; 2]>,
    pub slice_position_mm: Option<[f64; 2]>,
    pub pixel_pitch_mm: [f64; 2],
    pub x_profile: Option<Profile>,
    pub y_profile: Option<Profile>,
}
impl ScreenAnalysis {
    /// FWHM uses the X/Y slices through the first global-maximum pixel (row-major).
    /// Crossings are linearly interpolated between pixel centres around that lobe.
    /// A missing crossing stays unknown; the detector edge is never a substitute.
    pub fn from_detector(map: &DetectorMap) -> Result<Self, String> {
        Self::at(map, None)
    }
    /// Optional local-mm cursor selects the nearest pixel row and column.
    /// Half-maximum is relative to each selected slice's own maximum.
    pub fn at(map: &DetectorMap, cursor: Option<[f64; 2]>) -> Result<Self, String> {
        let [nx, ny] = map.pixels;
        if nx == 0
            || ny == 0
            || nx.checked_mul(ny) != Some(map.weights.len())
            || map
                .half_extent_mm
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
            || map.weights.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("スクリーンの画素データが不正です".into());
        }
        let pixel_pitch_mm = [
            2.0 * map.half_extent_mm[0] / nx as f64,
            2.0 * map.half_extent_mm[1] / ny as f64,
        ];
        let mut peak_weight = 0.0;
        let mut peak_pixel = None;
        for (i, &weight) in map.weights.iter().enumerate() {
            if weight > peak_weight {
                peak_weight = weight;
                peak_pixel = Some([i % nx, i / nx]);
            }
        }
        let selected = if let Some(c) = cursor {
            if c.iter()
                .enumerate()
                .any(|(a, v)| !v.is_finite() || v.abs() > map.half_extent_mm[a])
            {
                return Err("断面位置はスクリーンの範囲内で指定してください".into());
            }
            Some(std::array::from_fn(|a| {
                (((c[a] + map.half_extent_mm[a]) / pixel_pitch_mm[a]).floor() as usize)
                    .min(map.pixels[a] - 1)
            }))
        } else {
            peak_pixel
        };
        let profile = |weights: Vec<f64>, half: f64| {
            let (i, &peak) = weights
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(&a.0)))
                .unwrap();
            (peak > 0.).then(|| Profile::new(weights, half, i, peak))
        };
        let (x_profile, y_profile) = if let Some([x, y]) = selected {
            (
                profile(
                    map.weights[y * nx..(y + 1) * nx].to_vec(),
                    map.half_extent_mm[0],
                ),
                profile(
                    (0..ny).map(|row| map.weights[row * nx + x]).collect(),
                    map.half_extent_mm[1],
                ),
            )
        } else {
            (None, None)
        };
        let peak_position_mm = peak_pixel.map(|[x, y]| {
            [
                -map.half_extent_mm[0] + (x as f64 + 0.5) * pixel_pitch_mm[0],
                -map.half_extent_mm[1] + (y as f64 + 0.5) * pixel_pitch_mm[1],
            ]
        });
        Ok(Self {
            peak_weight,
            peak_pixel,
            peak_position_mm,
            slice_position_mm: selected.map(|[x, y]| {
                [
                    -map.half_extent_mm[0] + (x as f64 + 0.5) * pixel_pitch_mm[0],
                    -map.half_extent_mm[1] + (y as f64 + 0.5) * pixel_pitch_mm[1],
                ]
            }),
            pixel_pitch_mm,
            x_profile,
            y_profile,
        })
    }
}
/// Rectangular ROI integral: partially covered pixels contribute by area overlap.
/// The interpolation assumes constant power density within each recorded pixel.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenRoi {
    pub minimum_mm: [f64; 2],
    pub maximum_mm: [f64; 2],
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct RoiIntegral {
    pub percent_of_reference: f64,
    pub power_weight: f64,
    pub area_mm2: f64,
    pub percent_of_reference_per_mm2: f64,
}
impl ScreenRoi {
    pub fn integrate(&self, map: &DetectorMap, reference: f64) -> Result<RoiIntegral, String> {
        let a = ScreenAnalysis::from_detector(map)?;
        if !reference.is_finite()
            || reference <= 0.
            || (0..2).any(|i| {
                !self.minimum_mm[i].is_finite()
                    || !self.maximum_mm[i].is_finite()
                    || self.minimum_mm[i] >= self.maximum_mm[i]
            })
        {
            return Err("ROIの範囲・基準光量が不正です".into());
        }
        let low =
            std::array::from_fn::<_, 2, _>(|i| self.minimum_mm[i].max(-map.half_extent_mm[i]));
        let high =
            std::array::from_fn::<_, 2, _>(|i| self.maximum_mm[i].min(map.half_extent_mm[i]));
        let area = (high[0] - low[0]).max(0.) * (high[1] - low[1]).max(0.);
        let [dx, dy] = a.pixel_pitch_mm;
        let mut power = 0.;
        for (i, w) in map.weights.iter().enumerate() {
            let x = -map.half_extent_mm[0] + (i % map.pixels[0]) as f64 * dx;
            let y = -map.half_extent_mm[1] + (i / map.pixels[0]) as f64 * dy;
            power += w * ((x + dx).min(high[0]) - x.max(low[0])).max(0.) / dx
                * ((y + dy).min(high[1]) - y.max(low[1])).max(0.)
                / dy;
        }
        let percent = 100. * power / reference;
        Ok(RoiIntegral {
            percent_of_reference: percent,
            power_weight: power,
            area_mm2: area,
            percent_of_reference_per_mm2: if area > 0. { percent / area } else { 0. },
        })
    }
}
