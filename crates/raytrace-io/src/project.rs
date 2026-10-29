//! Editable project plus the original Zemax document and compatibility diagnostics.
use crate::Result;
use crate::assembly::{Assembly, Component, ComponentKind, empty_scene};
use crate::zemax::{self, ZmxDocument};
use raytrace_core::model::Scene;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub use raytrace_formats::Issue;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<Assembly>,
    pub format: String,
    pub scene: Scene,
    pub material_names: Vec<String>,
    #[serde(default)]
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub zemax: Option<ZmxDocument>,
    #[serde(default)]
    pub allow_uncoated: bool,
    #[serde(default)]
    pub monochromatic_override_um: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cad_deflection_mm: Option<f64>,
}

impl Project {
    pub fn new(scene: Scene) -> Self {
        let format = if scene.sources.iter().any(|s| s.extended_illumination()) {
            "raytrace-project-v3"
        } else {
            "raytrace-project-v1"
        }
        .into();
        let material_names = (0..scene.materials.len())
            .map(|i| {
                if i == 0 {
                    "AIR".into()
                } else {
                    format!("RT_M{i}")
                }
            })
            .collect();
        Self {
            assembly: None,
            format,
            scene,
            material_names,
            issues: Vec::new(),
            zemax: None,
            allow_uncoated: false,
            monochromatic_override_um: None,
            cad_deflection_mm: None,
        }
    }
    pub fn open(path: &Path, catalogs: &[PathBuf]) -> Result<Self> {
        Self::open_with_cad_tolerance(path, catalogs, 0.02)
    }
    pub fn open_with_cad_tolerance(
        path: &Path,
        catalogs: &[PathBuf],
        deflection_mm: f64,
    ) -> Result<Self> {
        if !deflection_mm.is_finite() || deflection_mm <= 0.0 {
            return Err(crate::Error::Project(
                "CAD分割精度は正の有限値で指定してください".into(),
            ));
        }
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zmx"))
        {
            return zemax::read_zmx_with_cad_tolerance(path, catalogs, deflection_mm);
        }
        let bytes = fs::read(path).map_err(|e| crate::Error::io(path, e))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
        if value.get("format").is_some() {
            if value["format"] == crate::record::RUN_FORMAT {
                return Err(crate::Error::Project("計算記録は raytrace-rs --replay で再計算できます。配置の編集にはプロジェクトJSONを開いてください".into()));
            }
            let mut project: Self = serde_json::from_value(value)
                .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
            if ![
                "raytrace-project-v1",
                "raytrace-project-v2",
                "raytrace-project-v3",
            ]
            .contains(&project.format.as_str())
                || project.format == "raytrace-project-v2" && project.assembly.is_none()
            {
                return Err(crate::Error::Project("未対応のプロジェクト形式です".into()));
            }
            project.refresh_assembly()?;
            Ok(project)
        } else {
            Ok(Self::new(serde_json::from_value(value).map_err(|e| {
                crate::Error::json(Some(path.to_path_buf()), e)
            })?))
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut snapshot = self.clone();
        snapshot.refresh_assembly()?;
        fs::write(
            path,
            serde_json::to_vec_pretty(&snapshot)
                .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?,
        )
        .map_err(|e| crate::Error::io(path, e))
    }
    pub fn trace_scene(&self) -> Result<Scene> {
        let (mut scene, issues) = if let Some(a) = &self.assembly {
            let (scene, _, issues) = a.compose(&self.scene)?;
            (scene, issues)
        } else {
            (self.scene.clone(), self.issues.clone())
        };
        let blocked: Vec<_> = self
            .blocking_issues(&issues)
            .iter()
            .map(|i| i.message.as_str())
            .collect();
        if !blocked.is_empty() {
            return Err(crate::Error::Project(blocked.join("\n")));
        }
        if self.allow_uncoated {
            for surface in &mut scene.surfaces {
                surface.coating = None;
            }
        }
        if let Some(wavelength) = self.monochromatic_override_um {
            for source in &mut scene.sources {
                source.wavelength_um = wavelength;
                source.spectrum = None;
                if let Some(o) = source.pupil.as_mut().and_then(|p| p.optical.as_mut()) {
                    o.primary_wavelength_um = wavelength;
                }
            }
        }
        Ok(scene)
    }
    fn blocking_issues<'a>(&self, issues: &'a [Issue]) -> Vec<&'a Issue> {
        issues
            .iter()
            .filter(|i| {
                i.blocking
                    && !(i.code == "coating" && self.allow_uncoated
                        || i.code == "spectrum" && self.monochromatic_override_um.is_some())
            })
            .collect()
    }
    pub fn new_assembly() -> Self {
        let mut p = Self::new(empty_scene("組立光学系"));
        p.format = "raytrace-project-v2".into();
        p.assembly = Some(Assembly::default());
        p
    }
    pub fn refresh_assembly(&mut self) -> Result<()> {
        if let Some(a) = &self.assembly {
            let (scene, names, issues) = a.compose(&self.scene)?;
            self.scene = scene;
            self.material_names = names;
            self.issues = issues;
        }
        self.format = if self.requires_extended_illumination() {
            "raytrace-project-v3"
        } else if self.assembly.is_some() {
            "raytrace-project-v2"
        } else {
            "raytrace-project-v1"
        }
        .into();
        Ok(())
    }
    fn requires_extended_illumination(&self) -> bool {
        self.scene.sources.iter().any(|s| s.extended_illumination())
            || self.assembly.as_ref().is_some_and(|a| {
                a.components
                    .iter()
                    .any(|c| c.project.requires_extended_illumination())
            })
    }
    pub fn add_component(&mut self, component: Component) -> Result<()> {
        let mut candidate = self.clone();
        if candidate.assembly.is_none() {
            let current = Component::imported(candidate.clone(), ComponentKind::System)?;
            candidate = Self::new_assembly();
            candidate
                .assembly
                .as_mut()
                .unwrap()
                .components
                .push(current);
            candidate.allow_uncoated = self.allow_uncoated;
            candidate.monochromatic_override_um = self.monochromatic_override_um;
        }
        candidate
            .assembly
            .as_mut()
            .unwrap()
            .components
            .push(component);
        candidate.refresh_assembly()?;
        *self = candidate;
        Ok(())
    }
    pub fn issue(&mut self, code: &str, message: impl Into<String>, blocking: bool) {
        let message = message.into();
        if !self.issues.iter().any(|i| i.message == message) {
            self.issues.push(Issue {
                code: code.into(),
                message,
                blocking,
            });
        }
    }
}
