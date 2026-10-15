//! Editable project plus the original Zemax document and compatibility diagnostics.
use crate::model::Scene;
use crate::zemax::{self, ZmxDocument};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub code: String,
    pub message: String,
    /// An unsupported physical definition prevents tracing.
    pub blocking: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
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
}

impl Project {
    pub fn new(scene: Scene) -> Self {
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
            format: "raytrace-project-v1".into(),
            scene,
            material_names,
            issues: Vec::new(),
            zemax: None,
            allow_uncoated: false,
            monochromatic_override_um: None,
        }
    }
    pub fn open(path: &Path, catalogs: &[PathBuf]) -> Result<Self, String> {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zmx"))
        {
            return zemax::read_zmx(path, catalogs);
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if value.get("format").is_some() {
            let project: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
            if project.format != "raytrace-project-v1" {
                return Err("未対応のプロジェクト形式です".into());
            }
            Ok(project)
        } else {
            Ok(Self::new(
                serde_json::from_value(value).map_err(|e| e.to_string())?,
            ))
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        fs::write(
            path,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn trace_scene(&self) -> Result<Scene, String> {
        let blocked: Vec<_> = self
            .issues
            .iter()
            .filter(|i| {
                i.blocking
                    && !(i.code == "coating" && self.allow_uncoated
                        || i.code == "spectrum" && self.monochromatic_override_um.is_some())
            })
            .map(|i| i.message.as_str())
            .collect();
        if !blocked.is_empty() {
            return Err(blocked.join("\n"));
        }
        let mut scene = self.scene.clone();
        if let Some(wavelength) = self.monochromatic_override_um {
            for source in &mut scene.sources {
                source.wavelength_um = wavelength;
            }
        }
        Ok(scene)
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
