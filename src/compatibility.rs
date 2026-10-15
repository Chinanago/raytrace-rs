//! Effective import conditions, grouped by the original optical component.
use crate::{
    model::Interaction,
    project::{Issue, Project},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Computed,
    PreservedOnly,
    RequiresOverride,
    OverrideApplied,
    Unsupported,
}
impl CompatibilityStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Computed => "計算対応",
            Self::PreservedOnly => "原文のみ保持",
            Self::RequiresOverride => "条件指定待ち",
            Self::OverrideApplied => "条件指定済み",
            Self::Unsupported => "未対応",
        }
    }
    pub fn blocks(self) -> bool {
        matches!(self, Self::RequiresOverride | Self::Unsupported)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatibilityEntry {
    pub code: String,
    pub message: String,
    pub status: CompatibilityStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpticalSample {
    pub wavelength_um: f64,
    pub refractive_index: Option<f64>,
    pub absorption_per_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialUse {
    pub index: usize,
    pub name: String,
    pub samples: Vec<OpticalSample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentCompatibility {
    /// Zero-based index in the assembly; None denotes a standalone project.
    pub component: Option<usize>,
    pub name: String,
    pub enabled: bool,
    pub source_format: String,
    pub sources: usize,
    pub surfaces: usize,
    pub effective_wavelengths_um: Vec<f64>,
    pub materials: Vec<MaterialUse>,
    pub entries: Vec<CompatibilityEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub traceable: bool,
    pub components: Vec<ComponentCompatibility>,
}

fn classify(issue: &Issue, uncoated: bool, mono: Option<f64>) -> CompatibilityEntry {
    let replacement = match issue.code.as_str() {
        "coating" if uncoated => Some("未コートFresnelを使用".to_string()),
        "spectrum" => mono.map(|w| format!("単色 {:.3} nm を使用", w * 1000.0)),
        _ => None,
    };
    let status = if replacement.is_some() {
        CompatibilityStatus::OverrideApplied
    } else if issue.blocking && ["coating", "spectrum"].contains(&issue.code.as_str()) {
        CompatibilityStatus::RequiresOverride
    } else if issue.blocking {
        CompatibilityStatus::Unsupported
    } else if issue.code == "air_cad" {
        CompatibilityStatus::PreservedOnly
    } else {
        CompatibilityStatus::Computed
    };
    CompatibilityEntry {
        code: issue.code.clone(),
        message: replacement.map_or_else(
            || issue.message.clone(),
            |r| format!("{} / {r}", issue.message),
        ),
        status,
    }
}

fn component_report(
    project: &Project,
    index: Option<usize>,
    name: &str,
    enabled: bool,
    uncoated: bool,
    mono: Option<f64>,
    global_waves: &[f64],
) -> ComponentCompatibility {
    let mut used = vec![0];
    for surface in &project.scene.surfaces {
        if let Interaction::Dielectric {
            minus_medium,
            plus_medium,
            ..
        } = surface.interaction
        {
            used.extend([minus_medium, plus_medium]);
        }
    }
    // Generated rods keep their material in slot 1 until their faces are composed.
    if project.scene.surfaces.is_empty()
        && project.scene.materials.len() == 2
        && project.scene.sources.is_empty()
    {
        used.push(1);
    }
    used.extend(project.scene.sources.iter().map(|s| s.medium));
    used.sort_unstable();
    used.dedup();
    let materials = used
        .into_iter()
        .filter_map(|i| {
            let material = project.scene.materials.get(i)?;
            Some(MaterialUse {
                index: i,
                name: project
                    .material_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("M{i}")),
                samples: global_waves
                    .iter()
                    .map(|&w| OpticalSample {
                        wavelength_um: w,
                        refractive_index: material.index(w),
                        absorption_per_mm: material.absorption_at(w),
                    })
                    .collect(),
            })
        })
        .collect();
    let mut entries = vec![CompatibilityEntry {
        code: "model".into(),
        message: format!(
            "{} 光源・{} 面を計算モデルに読込",
            project.scene.sources.len(),
            project.scene.surfaces.len()
        ),
        status: CompatibilityStatus::Computed,
    }];
    entries.extend(project.issues.iter().map(|i| classify(i, uncoated, mono)));
    ComponentCompatibility {
        component: index,
        name: name.into(),
        enabled,
        source_format: if project.zemax.is_some() {
            "zmx"
        } else {
            "json"
        }
        .into(),
        sources: project.scene.sources.len(),
        surfaces: project.scene.surfaces.len(),
        effective_wavelengths_um: unique_waves(
            project
                .scene
                .sources
                .iter()
                .map(|s| mono.unwrap_or(s.wavelength_um)),
        ),
        materials,
        entries,
    }
}

fn unique_waves(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut waves: Vec<_> = values.collect();
    waves.sort_by(f64::total_cmp);
    waves.dedup();
    waves
}

impl Project {
    pub fn compatibility_report(&self) -> CompatibilityReport {
        let waves = if let Some(a) = &self.assembly {
            unique_waves(a.components.iter().filter(|c| c.enabled).flat_map(|c| {
                c.project.scene.sources.iter().map(move |s| {
                    self.monochromatic_override_um
                        .or(c.project.monochromatic_override_um)
                        .unwrap_or(s.wavelength_um)
                })
            }))
        } else {
            unique_waves(
                self.scene
                    .sources
                    .iter()
                    .map(|s| self.monochromatic_override_um.unwrap_or(s.wavelength_um)),
            )
        };
        self.compatibility_report_at_wavelengths(&waves)
    }
    pub fn compatibility_report_at_wavelengths(&self, wavelengths: &[f64]) -> CompatibilityReport {
        let waves = unique_waves(wavelengths.iter().copied());
        let components = if let Some(a) = &self.assembly {
            a.components
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut report = component_report(
                        &c.project,
                        Some(i),
                        &c.name,
                        c.enabled,
                        self.allow_uncoated || c.project.allow_uncoated,
                        self.monochromatic_override_um
                            .or(c.project.monochromatic_override_um),
                        &waves,
                    );
                    if c.rod.is_some() {
                        report.source_format = "generated".into();
                        report.surfaces = 6;
                        report.entries[0].message = "指定寸法と材料からロッドの6面を構成".into();
                    }
                    report
                })
                .collect()
        } else {
            vec![component_report(
                self,
                None,
                &self.scene.name,
                true,
                self.allow_uncoated,
                self.monochromatic_override_um,
                &waves,
            )]
        };
        CompatibilityReport {
            traceable: components
                .iter()
                .filter(|c| c.enabled)
                .all(|c| c.entries.iter().all(|e| !e.status.blocks())),
            components,
        }
    }
}
