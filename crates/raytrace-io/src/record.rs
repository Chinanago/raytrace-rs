//! Self-contained calculation conditions and their validation, separate from the editable design.
//! Tracing a record is a pipeline stage in `raytrace_engine::pipeline`.
use crate::Result;
use crate::{compatibility::CompatibilityReport, design::Design, legacy};
use raytrace_core::{
    model::{PreparedScene, Ray, Scene},
    trace::{Report, TraceOptions},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub const RUN_FORMAT: &str = "raytrace-run-v1";
const SAMPLING_REVISION: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SavedPupil {
    pub source_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<raytrace_core::illumination::PupilGeometry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchInput {
    Generated {
        sampling_revision: u32,
        count: usize,
    },
    Explicit {
        rays: Vec<Ray>,
    },
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RunContext {
    pub software_version: String,
    pub created_unix_ms: u64,
    pub worker_threads: usize,
    /// The editor state, including original documents, parts, placements and overrides.
    /// Records saved by earlier builds hold this as `project` and are migrated on load.
    pub design: Design,
    /// Exact effective scene after CLI overrides or GUI preview limits.
    pub scene: Scene,
    pub input: LaunchInput,
    pub trace_options: TraceOptions,
    pub launch_wavelengths_um: Vec<f64>,
    pub compatibility: CompatibilityReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pupils: Vec<SavedPupil>,
}
/// Record layout as stored; `project` is the field of builds before the design document.
#[derive(Deserialize)]
struct RunContextWire {
    software_version: String,
    created_unix_ms: u64,
    worker_threads: usize,
    #[serde(default)]
    design: Option<Design>,
    #[serde(default)]
    project: Option<serde_json::Value>,
    scene: Scene,
    input: LaunchInput,
    trace_options: TraceOptions,
    launch_wavelengths_um: Vec<f64>,
    compatibility: serde_json::Value,
    #[serde(default)]
    pupils: Vec<SavedPupil>,
}
impl<'de> Deserialize<'de> for RunContext {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let wire = RunContextWire::deserialize(deserializer)?;
        let (design, mut compatibility) = match (wire.design, wire.project) {
            (Some(design), None) => (design, wire.compatibility),
            (None, Some(project)) => {
                let design = legacy::migrate_embedded(project).map_err(D::Error::custom)?;
                let mut compatibility = wire.compatibility;
                legacy::migrate_compatibility(&mut compatibility, &design);
                (design, compatibility)
            }
            _ => {
                return Err(D::Error::custom(
                    "計算記録にはdesign（旧版ではproject）を1つだけ指定してください",
                ));
            }
        };
        let compatibility =
            serde_json::from_value(std::mem::take(&mut compatibility)).map_err(D::Error::custom)?;
        Ok(Self {
            software_version: wire.software_version,
            created_unix_ms: wire.created_unix_ms,
            worker_threads: wire.worker_threads,
            design,
            scene: wire.scene,
            input: wire.input,
            trace_options: wire.trace_options,
            launch_wavelengths_um: wire.launch_wavelengths_um,
            compatibility,
            pupils: wire.pupils,
        })
    }
}
impl RunContext {
    pub fn input_count(&self) -> usize {
        match &self.input {
            LaunchInput::Generated { count, .. } => *count,
            LaunchInput::Explicit { rays } => rays.len(),
        }
    }
    pub fn capture(
        design: &Design,
        scene: &Scene,
        explicit: Option<&[Ray]>,
        worker_threads: usize,
        trace_options: TraceOptions,
    ) -> Self {
        let mut waves: Vec<_> = match explicit {
            Some(rays) => rays.iter().map(|r| r.wavelength_um).collect(),
            None => scene.sources.iter().flat_map(|s| s.wavelengths()).collect(),
        };
        waves.sort_by(f64::total_cmp);
        waves.dedup();
        let mut compatibility = design.compatibility_report_at_wavelengths(&waves);
        if explicit.is_some() {
            for component in &mut compatibility.components {
                if component.sources > 0 {
                    component.effective_wavelengths_um = waves.clone();
                }
                for entry in &mut component.entries {
                    if entry.code == "spectrum" {
                        entry.status = crate::compatibility::CompatibilityStatus::PreservedOnly;
                        entry.message = format!(
                            "明示入力光線の波長を採用。元の光源スペクトルを原文に保持：{}",
                            entry.message
                        );
                    }
                }
            }
            compatibility.traceable = compatibility
                .components
                .iter()
                .filter(|c| c.enabled)
                .all(|c| c.entries.iter().all(|e| !e.status.blocks()));
        }
        Self {
            software_version: env!("CARGO_PKG_VERSION").into(),
            created_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            worker_threads,
            design: design.clone(),
            scene: scene.clone(),
            input: explicit.map_or_else(
                || LaunchInput::Generated {
                    sampling_revision: SAMPLING_REVISION,
                    count: scene.sources.iter().map(|s| s.count).sum(),
                },
                |rays| LaunchInput::Explicit {
                    rays: rays.to_vec(),
                },
            ),
            trace_options,
            compatibility,
            launch_wavelengths_um: waves,
            pupils: scene
                .sources
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    if explicit.is_some() {
                        return None;
                    }
                    s.pupil.as_ref().and_then(|p| p.optical.as_ref())?;
                    Some(
                        match raytrace_core::illumination::pupil_geometry_from_scene(scene, i) {
                            Ok(geometry) => SavedPupil {
                                source_index: i,
                                geometry,
                                error: None,
                            },
                            Err(error) => SavedPupil {
                                source_index: i,
                                geometry: None,
                                error: Some(error.to_string()),
                            },
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn validate_generated(&self, prepared: &PreparedScene) -> Result<()> {
        if self.worker_threads == 0 {
            return Err(crate::Error::Record(
                "計算記録のスレッド数が不正です".into(),
            ));
        }
        if let LaunchInput::Generated {
            sampling_revision,
            count,
        } = self.input
        {
            let legacy = self
                .scene
                .sources
                .iter()
                .all(|s| !s.extended_illumination());
            if sampling_revision != SAMPLING_REVISION
                && !(sampling_revision == 2 && legacy)
                && !(sampling_revision == 1
                    && legacy
                    && self
                        .scene
                        .sources
                        .iter()
                        .all(|s| s.spectrum.is_none() && s.pupil.is_none()))
            {
                return Err(crate::Error::Record(
                    "この計算記録のサンプリング版は再生に対応していません".into(),
                ));
            }
            if prepared.ray_generator()?.len() != count {
                return Err(crate::Error::Record(
                    "計算記録の光線数が一致しません".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn rays(&self) -> Result<Vec<Ray>> {
        if self.worker_threads == 0 {
            return Err(crate::Error::Record(
                "計算記録のスレッド数が不正です".into(),
            ));
        }
        let prepared = self.scene.clone().prepare()?;
        let rays = match &self.input {
            LaunchInput::Generated { .. } => {
                self.validate_generated(&prepared)?;
                prepared.generate_rays()?
            }
            LaunchInput::Explicit { rays } => rays.clone(),
        };
        let mut ids = std::collections::HashSet::new();
        for ray in &rays {
            if !ids.insert(ray.id) {
                return Err(crate::Error::Record(
                    "計算記録の光線IDが重複しています".into(),
                ));
            }
            let mut checked = *ray;
            checked.validate(&prepared)?;
            if (ray.direction.norm() - 1.0).abs() > 1e-12 {
                return Err(crate::Error::Record(
                    "計算記録の光線方向が正規化されていません".into(),
                ));
            }
        }
        if rays.is_empty()
            || !rays.iter().map(|r| r.weight).sum::<f64>().is_finite()
            || rays.iter().all(|r| r.weight == 0.0)
        {
            return Err(crate::Error::Record("計算記録の発射光量が不正です".into()));
        }
        Ok(rays)
    }
    pub fn save(&self, report: &Report, trace_seconds: f64, path: &Path) -> Result<()> {
        #[derive(Serialize)]
        struct BorrowedRun<'a> {
            format: &'static str,
            calculation: &'a RunContext,
            trace_seconds: f64,
            report: &'a Report,
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| crate::Error::io(parent, e))?;
        }
        let bytes = serde_json::to_vec_pretty(&BorrowedRun {
            format: RUN_FORMAT,
            calculation: self,
            trace_seconds,
            report,
        })
        .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
        fs::write(path, bytes).map_err(|e| crate::Error::io(path, e))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RunRecord {
    pub format: String,
    pub calculation: RunContext,
    #[serde(default)]
    pub trace_seconds: f64,
    pub report: Report,
}
impl RunRecord {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|e| crate::Error::io(path, e))?;
        let record: Self = serde_json::from_slice(&bytes)
            .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
        if record.format != RUN_FORMAT {
            return Err(crate::Error::Record("未対応の計算記録形式です".into()));
        }
        // Geometry/material/source validation is mandatory even for explicit rays.
        record.calculation.scene.clone().prepare()?;
        Ok(record)
    }
}
