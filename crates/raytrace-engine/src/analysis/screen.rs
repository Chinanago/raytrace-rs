//! `analysis.screen`: the light distribution on one screen (detector) of the record.
use super::{Analysis, AnalysisRequest, AnalysisResponse, RunView};
use crate::{command::Cmd, engine::EngineError, pipeline};
use raytrace_core::screen::{Profile, RoiIntegral, ScreenAnalysis, ScreenRoi};
use serde::{Deserialize, Serialize};

pub const ANALYSIS_SCREEN: Cmd<AnalysisRequest<ScreenSettings>, AnalysisResponse<ScreenOutput>> =
    Cmd::new("analysis.screen");

pub struct Screen;

/// Which screen of the record: its zero-based index in `report.detectors`, or its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum DetectorRef {
    Index(usize),
    /// A name as `analysis.budget` lists it. Text that is no name but a whole number counts
    /// as an index, so `detector=1` works on the command line.
    Name(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ScreenSettings {
    /// The screen to analyse; omitted uses the first one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detector: Option<DetectorRef>,
    /// Local-mm position of the X/Y slices (nearest pixel row and column); omitted: through the brightest pixel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_mm: Option<[f64; 2]>,
    /// Rectangle to integrate (local mm); edge pixels count by the area they share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roi: Option<ScreenRoi>,
    /// Also return the pixel values along the X and Y slices.
    pub include_profiles: bool,
}

/// One slice (a pixel row for X, a pixel column for Y) with its half-maximum width.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ScreenSlice {
    /// The largest pixel weight of the slice; the half maximum is half of it.
    pub peak_weight: f64,
    /// Full width at half maximum in mm, or null when a crossing is not on the screen.
    pub fwhm_mm: Option<f64>,
    /// The lower half-maximum crossing in local mm (null when not found).
    pub lower_mm: Option<f64>,
    /// The upper half-maximum crossing in local mm (null when not found).
    pub upper_mm: Option<f64>,
    /// More than one separate region of the slice reaches half of the peak; the width is
    /// that of the lobe around the main peak.
    pub multiple_lobes: bool,
    /// Local-mm position of the centre of the first pixel of the slice.
    pub first_center_mm: f64,
    /// Distance between the pixel centres of the slice.
    pub pitch_mm: f64,
    /// Pixel weights along the slice, with `include_profiles`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<f64>>,
}

impl ScreenSlice {
    /// Local-mm position of the centre of pixel `index` of the slice.
    pub fn coordinate_mm(&self, index: usize) -> f64 {
        self.first_center_mm + index as f64 * self.pitch_mm
    }

    fn of(profile: &Profile, include_profiles: bool) -> Self {
        Self {
            peak_weight: profile.peak_weight(),
            fwhm_mm: profile.half_maximum.width_mm(),
            lower_mm: profile.half_maximum.lower_mm,
            upper_mm: profile.half_maximum.upper_mm,
            multiple_lobes: profile.half_maximum.multiple_lobes,
            first_center_mm: profile.first_center_mm,
            pitch_mm: profile.pitch_mm,
            weights: include_profiles.then(|| profile.weights.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ScreenOutput {
    /// Zero-based index of the screen in `report.detectors`.
    pub detector: usize,
    pub name: String,
    /// Pixel counts in X and Y.
    pub pixels: [usize; 2],
    pub pixel_pitch_mm: [f64; 2],
    /// Half of the screen size in X and Y.
    pub half_extent_mm: [f64; 2],
    /// The launched weight every percentage divides by.
    pub reference_weight: f64,
    /// The integrated power of the screen.
    pub received_weight: f64,
    pub received_percent_of_reference: f64,
    /// The largest pixel weight (0 when no light arrived).
    pub peak_weight: f64,
    pub peak_percent_of_reference: f64,
    /// Zero-based pixel (x, y) of the first brightest pixel in row-major order.
    pub peak_pixel: Option<[usize; 2]>,
    pub peak_position_mm: Option<[f64; 2]>,
    /// Local-mm centre of the pixel the slices pass through.
    pub slice_position_mm: Option<[f64; 2]>,
    /// The slice along X (a pixel row); null when no light arrived.
    pub x_slice: Option<ScreenSlice>,
    /// The slice along Y (a pixel column); null when no light arrived.
    pub y_slice: Option<ScreenSlice>,
    /// The integral over `roi`, when one was given.
    pub roi: Option<RoiIntegral>,
}

impl Analysis for Screen {
    const ID: &'static str = "screen";
    const SUMMARY: &'static str = "スクリーンの受光・ピーク・X/Y断面と半値幅・ROIの積分を返す（設定: detector, cursor_mm, roi, include_profiles）";
    type Settings = ScreenSettings;
    type Output = ScreenOutput;

    fn run(view: &RunView<'_>, settings: ScreenSettings) -> Result<ScreenOutput, EngineError> {
        let report = view.report();
        let reference = view.reference_weight();
        let index = find_detector(view, settings.detector.as_ref())?;
        let map = &report.detectors[index];
        let analysis = ScreenAnalysis::at(map, settings.cursor_mm)?;
        let roi = settings
            .roi
            .map(|roi| roi.integrate(map, reference))
            .transpose()?;
        let slice = |profile: &Option<Profile>| {
            profile
                .as_ref()
                .map(|p| ScreenSlice::of(p, settings.include_profiles))
        };
        Ok(ScreenOutput {
            detector: index,
            name: map.name.clone(),
            pixels: map.pixels,
            pixel_pitch_mm: analysis.pixel_pitch_mm,
            half_extent_mm: map.half_extent_mm,
            reference_weight: reference,
            received_weight: pipeline::received_weight(map),
            received_percent_of_reference: pipeline::received_percent(map, reference),
            peak_weight: analysis.peak_weight,
            peak_percent_of_reference: 100.0 * analysis.peak_weight / reference,
            peak_pixel: analysis.peak_pixel,
            peak_position_mm: analysis.peak_position_mm,
            slice_position_mm: analysis.slice_position_mm,
            x_slice: slice(&analysis.x_profile),
            y_slice: slice(&analysis.y_profile),
            roi,
        })
    }
}

fn find_detector(view: &RunView<'_>, wanted: Option<&DetectorRef>) -> Result<usize, EngineError> {
    let detectors = &view.report().detectors;
    if detectors.is_empty() {
        return Err(EngineError::invalid(
            "計算記録にスクリーンがありません。光学系にスクリーンを追加して計算してください",
        ));
    }
    let names = || {
        detectors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match wanted {
        None => Ok(0),
        Some(DetectorRef::Index(index)) if *index < detectors.len() => Ok(*index),
        Some(DetectorRef::Index(index)) => Err(EngineError::invalid(format!(
            "スクリーン {index} はありません（0〜{} を指定してください。名前: {}）",
            detectors.len() - 1,
            names()
        ))),
        Some(DetectorRef::Name(name)) => detectors
            .iter()
            .position(|d| d.name == *name)
            .or_else(|| {
                name.trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|index| *index < detectors.len())
            })
            .ok_or_else(|| {
                EngineError::invalid(format!(
                    "スクリーン {name} はありません（名前: {}）",
                    names()
                ))
            }),
    }
}
