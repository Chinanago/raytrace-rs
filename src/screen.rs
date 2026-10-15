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
    pub pixel_pitch_mm: [f64; 2],
    pub x_profile: Option<Profile>,
    pub y_profile: Option<Profile>,
}
impl ScreenAnalysis {
    /// FWHM uses the X/Y slices through the first global-maximum pixel (row-major).
    /// Crossings are linearly interpolated between pixel centres around that lobe.
    /// A missing crossing stays unknown; the detector edge is never a substitute.
    pub fn from_detector(map: &DetectorMap) -> Result<Self, String> {
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
        let (x_profile, y_profile) = if let Some([x, y]) = peak_pixel {
            (
                Some(Profile::new(
                    map.weights[y * nx..(y + 1) * nx].to_vec(),
                    map.half_extent_mm[0],
                    x,
                    peak_weight,
                )),
                Some(Profile::new(
                    (0..ny).map(|row| map.weights[row * nx + x]).collect(),
                    map.half_extent_mm[1],
                    y,
                    peak_weight,
                )),
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
            pixel_pitch_mm,
            x_profile,
            y_profile,
        })
    }
}
