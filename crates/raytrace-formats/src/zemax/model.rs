use super::ZmxDocument;
use crate::Issue;
use raytrace_core::model::Scene;

#[derive(Clone, Debug)]
pub struct ZmxModel {
    pub scene: Scene,
    pub material_names: Vec<String>,
    pub issues: Vec<Issue>,
    pub zemax: Option<ZmxDocument>,
    pub cad_deflection_mm: Option<f64>,
}
impl ZmxModel {
    pub(super) fn new(scene: Scene) -> Self {
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
            scene,
            material_names,
            issues: Vec::new(),
            zemax: None,
            cad_deflection_mm: None,
        }
    }
    pub(super) fn issue(&mut self, code: &str, message: impl Into<String>, blocking: bool) {
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

pub struct ZmxView<'a> {
    pub scene: &'a Scene,
    pub material_names: &'a [String],
    pub issues: &'a [Issue],
    pub zemax: Option<&'a ZmxDocument>,
}
