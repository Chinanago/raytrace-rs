//! Native ZMX/AGF parsing. Unknown data stays in the original document.
use crate::Result;
mod exchange;
mod illumination;
use crate::project::Project;
use raytrace_core::{
    math::{Pose, V3},
    model::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

mod materials;
mod nsc;
mod sequential;
mod text;
mod write;

use materials::{catalogs, material_id, Glass};
use nsc::read_nsc;
use sequential::read_sequential;
use text::{checked_parameters, encode, field, number, parameters, pose_from, scalar, words};
pub use text::{decode, read_spcd, TextEncoding};
pub use write::export;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZmxDocument {
    pub text: String,
    pub encoding: TextEncoding,
    pub source_path: PathBuf,
    pub baseline_scene: Scene,
    /// One source-document surface number per editable sequential surface.
    pub surface_numbers: Vec<usize>,
    pub glass_records: BTreeMap<String, String>,
}
pub fn read_zmx(path: &Path, extra: &[PathBuf]) -> Result<Project> {
    read_zmx_with_cad_tolerance(path, extra, 0.02)
}
pub fn read_zmx_with_cad_tolerance(
    path: &Path,
    extra: &[PathBuf],
    deflection_mm: f64,
) -> Result<Project> {
    if !deflection_mm.is_finite() || deflection_mm <= 0.0 {
        return Err(crate::Error::zmx("CAD分割精度が不正です".into()));
    }
    let (text, encoding) = decode(&fs::read(path).map_err(|e| crate::Error::io(path, e))?);
    let all: Vec<String> = text.lines().map(str::to_owned).collect();
    if field(&all, "UNIT").is_some_and(|s| s.split_whitespace().next() != Some("MM")) {
        return Err(crate::Error::zmx(
            "現在はmm単位のZMXに対応しています".into(),
        ));
    }
    let mode = field(&all, "MODE").unwrap_or("SEQ");
    if !["SEQ", "NSC"].contains(&mode) {
        return Err(crate::Error::zmx(format!(
            "未対応のZMX追跡方式です: {mode}"
        )));
    }
    let wavelengths = checked_parameters(&all, "WAVM")?;
    for line in &all {
        let w = words(line);
        if w.first() == Some(&"WAVM")
            && (w.len() < 3
                || w[1].parse::<usize>().ok().is_none_or(|i| i == 0)
                || number(w[2]).ok().is_none_or(|v| !v.is_finite() || v <= 0.0))
        {
            return Err(crate::Error::zmx(format!(
                "ZMXの波長定義が不正です: {}",
                line.trim()
            )));
        }
    }
    let primary_value = scalar(&all, "PWAV", 1.0)?;
    if !primary_value.is_finite() || primary_value < 1.0 || primary_value.fract() != 0.0 {
        return Err(crate::Error::zmx("ZMXの主波長番号が不正です".into()));
    }
    let primary = primary_value as usize;
    let wavelength = *wavelengths
        .get(&primary)
        .or_else(|| wavelengths.values().next())
        .unwrap_or(&0.55);
    let mut project = Project::new(Scene {
        name: field(&all, "NAME")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("Zemax"))
            .into(),
        reference_plane: "発射面の光線重み合計".into(),
        mode: if mode.starts_with("NSC") {
            Mode::NonSequential
        } else {
            Mode::Sequential
        },
        materials: vec![Material::Constant {
            n: 1.0,
            absorption_per_mm: 0.0,
        }],
        surfaces: Vec::new(),
        sources: Vec::new(),
        settings: Settings::default(),
    });
    let declared: Vec<String> = field(&all, "GCAT")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if !wavelengths.contains_key(&primary) {
        project.issue(
            "spectrum",
            "主波長が波長表にありません。計算波長を明示してください",
            true,
        );
    }
    if let Some(environment) = field(&all, "ENVD") {
        let values: Vec<_> = environment
            .split_whitespace()
            .map(number)
            .collect::<Result<_, _>>()?;
        if values.len() < 2
            || (values[0] - 20.0).abs() > 1e-12
            || (values[1] - 1.0).abs() > 1e-12
            || values.iter().any(|v| !v.is_finite())
            || values.iter().skip(2).any(|v| *v != 0.0)
        {
            project.issue(
                "environment",
                format!("環境条件ENVD {environment}の温度・圧力による屈折率補正は未対応です"),
                true,
            );
        }
    }
    let glasses = catalogs(path, extra, &declared);
    let mut records = BTreeMap::new();
    let mut bindings = Vec::new();
    if mode.starts_with("NSC") {
        read_nsc(
            &all,
            path,
            wavelength,
            &mut project,
            &glasses,
            &mut records,
            deflection_mm,
        )?;
    } else {
        read_sequential(
            &all,
            (path, extra),
            wavelength,
            &mut project,
            &glasses,
            &mut records,
            &mut bindings,
        )?;
    }
    project.zemax = Some(ZmxDocument {
        text,
        encoding,
        source_path: path.into(),
        baseline_scene: project.scene.clone(),
        surface_numbers: bindings,
        glass_records: records,
    });
    project.refresh_assembly()?;
    Ok(project)
}
