//! Effective import conditions, grouped by occurrence.
use crate::{Design, OccurrenceId, Part};
use raytrace_core::{issue::Issue, model::Interaction};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CompatibilityEntry {
    pub code: String,
    pub message: String,
    pub status: CompatibilityStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OpticalSample {
    pub wavelength_um: f64,
    pub refractive_index: Option<f64>,
    pub absorption_per_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MaterialUse {
    pub index: usize,
    pub name: String,
    pub samples: Vec<OpticalSample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ComponentCompatibility {
    /// The occurrence; None denotes a single system traced as it is.
    #[serde(default)]
    pub occurrence: Option<OccurrenceId>,
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

fn part_report(
    project: &Part,
    occurrence: Option<OccurrenceId>,
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
    entries.extend(
        project
            .scene
            .sources
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.spectrum.as_ref().map(|spectrum| CompatibilityEntry {
                    code: "spectrum_model".into(),
                    message: mono.map_or_else(
                        || {
                            format!(
                                "光源{}: {}波長を相対光量でサンプリング。発射光量の合計を維持",
                                i + 1,
                                spectrum.samples.len()
                            )
                        },
                        |w| {
                            format!(
                                "光源{}: スペクトルを保存し、単色{:.3} nmを使用",
                                i + 1,
                                w * 1000.
                            )
                        },
                    ),
                    status: if mono.is_some() {
                        CompatibilityStatus::OverrideApplied
                    } else {
                        CompatibilityStatus::Computed
                    },
                })
            }),
    );
    entries.extend(project.scene.surfaces.iter().filter_map(|s| {
        s.coating.as_ref().map(|c| CompatibilityEntry {
            code: "coating_model".into(),
            message: if uncoated {
                format!("{} / {}: 未コートFresnelを使用", s.name, c.name)
            } else {
                format!(
                    "{} / {}: 波長・角度・表裏に対応するR/T/Aを使用（各界面でS/P強度を平均）",
                    s.name, c.name
                )
            },
            status: if uncoated {
                CompatibilityStatus::OverrideApplied
            } else {
                CompatibilityStatus::Computed
            },
        })
    }));
    ComponentCompatibility {
        occurrence,
        name: name.into(),
        enabled,
        source_format: project
            .provenance
            .as_ref()
            .map_or("json", |p| p.format.as_str())
            .into(),
        sources: project.scene.sources.len(),
        surfaces: project.scene.surfaces.len(),
        effective_wavelengths_um: unique_waves(
            project
                .scene
                .sources
                .iter()
                .flat_map(|s| mono.map_or_else(|| s.wavelengths(), |w| vec![w])),
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

impl Design {
    pub fn compatibility_report(&self) -> CompatibilityReport {
        let waves = if let Some(part) = self.lone_system() {
            let mono = self.effective_conditions().monochromatic_override_um;
            unique_waves(
                part.scene
                    .sources
                    .iter()
                    .flat_map(|s| mono.map_or_else(|| s.wavelengths(), |w| vec![w])),
            )
        } else {
            unique_waves(
                self.occurrences
                    .iter()
                    .filter(|o| o.enabled)
                    .filter_map(|o| self.part(&o.part))
                    .flat_map(|part| {
                        part.scene.sources.iter().flat_map(move |s| {
                            self.conditions
                                .monochromatic_override_um
                                .or(part.conditions.monochromatic_override_um)
                                .map_or_else(|| s.wavelengths(), |w| vec![w])
                        })
                    }),
            )
        };
        self.compatibility_report_at_wavelengths(&waves)
    }
    pub fn compatibility_report_at_wavelengths(&self, wavelengths: &[f64]) -> CompatibilityReport {
        let waves = unique_waves(wavelengths.iter().copied());
        let components = if let Some(part) = self.lone_system() {
            let conditions = self.effective_conditions();
            vec![part_report(
                part,
                None,
                &part.scene.name,
                true,
                conditions.allow_uncoated,
                conditions.monochromatic_override_um,
                &waves,
            )]
        } else {
            self.occurrences
                .iter()
                .filter_map(|o| Some((o, self.part(&o.part)?)))
                .map(|(o, part)| {
                    let conditions = self.conditions.over(&part.conditions);
                    let mut report = part_report(
                        part,
                        Some(o.id.clone()),
                        &o.name,
                        o.enabled,
                        conditions.allow_uncoated,
                        conditions.monochromatic_override_um,
                        &waves,
                    );
                    if let Some(rod) = &part.rod {
                        report.source_format = "generated".into();
                        report.surfaces = 6;
                        report.entries[0].message = "指定寸法と材料からロッドの6面を構成".into();
                        report
                            .entries
                            .extend(rod.coatings.iter().map(|(face, coating)| {
                                CompatibilityEntry {
                                    code: "coating_model".into(),
                                    message: format!(
                                        "{face} / {}: {}",
                                        coating.name,
                                        if conditions.allow_uncoated {
                                            "未コートFresnelを使用"
                                        } else {
                                            "表裏・波長・角度ごとのR/T/Aを使用"
                                        }
                                    ),
                                    status: if conditions.allow_uncoated {
                                        CompatibilityStatus::OverrideApplied
                                    } else {
                                        CompatibilityStatus::Computed
                                    },
                                }
                            }));
                    }
                    report
                })
                .collect()
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
