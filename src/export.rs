//! Screen exports retain integrated pixel power and explicit reference/axis metadata.
use crate::{
    screen::{Profile, ScreenAnalysis},
    trace::DetectorMap,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub peak_normalized: bool,
    pub square_root: bool,
    pub maximum_percent: Option<f64>,
}
impl DisplaySettings {
    pub fn maximum_weight(&self, peak: f64, reference: f64) -> f64 {
        if self.peak_normalized {
            peak
        } else {
            self.maximum_percent.map_or(peak, |v| v * reference / 100.0)
        }
    }
}
pub fn rgb(fraction: f64, square_root: bool) -> [u8; 3] {
    let v = if square_root {
        fraction.sqrt()
    } else {
        fraction
    };
    let t = v.clamp(0.0, 1.0) as f32;
    [
        (247.0 - 212.0 * t) as u8,
        (250.0 - 158.0 * t) as u8,
        (254.0 - 78.0 * t) as u8,
    ]
}
fn profile_csv(path: &Path, p: &Profile, reference: f64, peak: f64) -> Result<(), String> {
    let mut out = BufWriter::new(fs::File::create(path).map_err(|e| e.to_string())?);
    writeln!(
        out,
        "coordinate_mm,power_weight,percent_of_reference,peak_normalized"
    )
    .map_err(|e| e.to_string())?;
    for (i, w) in p.weights.iter().enumerate() {
        writeln!(
            out,
            "{:.17e},{:.17e},{:.17e},{:.17e}",
            p.coordinate_mm(i),
            w,
            100.0 * w / reference,
            w / peak
        )
        .map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}
pub fn write_screen(
    dir: &Path,
    map: &DetectorMap,
    reference_plane: &str,
    reference: f64,
    display: DisplaySettings,
) -> Result<(), String> {
    let a = ScreenAnalysis::from_detector(map)?;
    if !reference.is_finite()
        || reference <= 0.0
        || display
            .maximum_percent
            .is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err("出力の基準光量・表示範囲が不正です".into());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut csv =
        BufWriter::new(fs::File::create(dir.join("pixels.csv")).map_err(|e| e.to_string())?);
    writeln!(
        csv,
        "x_mm,y_mm,power_weight,percent_of_reference,percent_of_reference_per_mm2"
    )
    .map_err(|e| e.to_string())?;
    let [dx, dy] = a.pixel_pitch_mm;
    for (i, w) in map.weights.iter().enumerate() {
        let x = -map.half_extent_mm[0] + (i % map.pixels[0]) as f64 * dx + dx * 0.5;
        let y = -map.half_extent_mm[1] + (i / map.pixels[0]) as f64 * dy + dy * 0.5;
        writeln!(
            csv,
            "{x:.17e},{y:.17e},{w:.17e},{:.17e},{:.17e}",
            100.0 * w / reference,
            100.0 * w / reference / (dx * dy)
        )
        .map_err(|e| e.to_string())?;
    }
    csv.flush().map_err(|e| e.to_string())?;
    if let Some(p) = &a.x_profile {
        profile_csv(&dir.join("x_profile.csv"), p, reference, a.peak_weight)?;
    }
    if let Some(p) = &a.y_profile {
        profile_csv(&dir.join("y_profile.csv"), p, reference, a.peak_weight)?;
    }
    let metadata = serde_json::json!({"format":"raytrace-screen-export-v1","software_version":env!("CARGO_PKG_VERSION"),
        "detector":map.name,"reference_plane":reference_plane,"reference_weight":reference,"pixels":map.pixels,"half_extent_mm":map.half_extent_mm,
        "pixel_pitch_mm":a.pixel_pitch_mm,"received_percent":100.0*map.weights.iter().sum::<f64>()/reference,"display":display,
        "fwhm_mm":[a.x_profile.as_ref().and_then(|p|p.half_maximum.width_mm()),a.y_profile.as_ref().and_then(|p|p.half_maximum.width_mm())],
        "axis_definition":"Coordinates are local detector pixel centres in mm. CSV order increases x then y. PNG top row is positive y. Power is integrated per pixel; profiles are slices through the global maximum, without smoothing."});
    fs::write(
        dir.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(dir.join("README.md"),format!("# {}\n\n基準面：{} = 100%（基準光量 {:.17e}）。\n\n`pixels.csv` はスクリーン局所座標の画素中心と元の積算光量、基準光量比、面積で割った光量比を含みます。座標と面積の単位はmm、mm²です。断面は最大画素を通るX/Yの画素列です。\n\n`screen.png` は横軸X・縦軸Yの分布で、上端が+Yです。範囲はX ±{} mm、Y ±{} mm。正規化・表示範囲・半値幅は `metadata.json` に記録しています。\n",map.name,reference_plane,reference,map.half_extent_mm[0],map.half_extent_mm[1])).map_err(|e|e.to_string())?;
    #[cfg(feature = "gui")]
    {
        let maximum = display.maximum_weight(a.peak_weight, reference).max(1e-300);
        let mut pixels = Vec::with_capacity(map.weights.len() * 3);
        for w in map.weights.chunks(map.pixels[0]).rev().flatten() {
            pixels.extend(rgb(*w / maximum, display.square_root));
        }
        image::save_buffer(
            dir.join("screen.png"),
            &pixels,
            map.pixels[0] as u32,
            map.pixels[1] as u32,
            image::ColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Additional selections use the immutable pixel data; the main export retains
/// its original peak slices so downstream tools can always compare both.
pub fn write_postprocessing(
    dir: &Path,
    map: &DetectorMap,
    reference: f64,
    cursor: Option<[f64; 2]>,
    roi: Option<crate::screen::ScreenRoi>,
) -> Result<(), String> {
    if !reference.is_finite() || reference <= 0. {
        return Err("基準光量は正の有限値が必要です".into());
    }
    let a = ScreenAnalysis::at(map, cursor)?;
    let integral = roi.map(|r| r.integrate(map, reference)).transpose()?;
    if cursor.is_some() {
        if let Some(p) = &a.x_profile {
            profile_csv(
                &dir.join("selected_x_profile.csv"),
                p,
                reference,
                a.peak_weight,
            )?;
        }
        if let Some(p) = &a.y_profile {
            profile_csv(
                &dir.join("selected_y_profile.csv"),
                p,
                reference,
                a.peak_weight,
            )?;
        }
    }
    let data = serde_json::json!({"cursor_requested_mm":cursor,"slice_position_mm":a.slice_position_mm,
        "slice_fwhm_mm":[a.x_profile.as_ref().and_then(|p|p.half_maximum.width_mm()),a.y_profile.as_ref().and_then(|p|p.half_maximum.width_mm())],
        "roi":roi,"roi_integral":integral,"reference_weight":reference,
        "method":"Slices select the nearest pixel row/column; half maximum uses each slice's own peak. Rectangular ROI clips to the detector and weights boundary pixels by overlap area, assuming constant density within each pixel. Source pixel data are unchanged."});
    fs::write(
        dir.join("postprocessing.json"),
        serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
