//! Parameter studies with immutable inputs, affine links and resumable cases.
use crate::{
    Result,
    pipeline::{
        self, Completion, LaunchRequest, LimitViolation, Limits, MAX_CASE_RAYS, MAX_SCREEN_PIXELS,
        SceneOverrides, Schedule,
    },
};
use raytrace_core::trace::{TraceControl, TraceOptions};
use raytrace_io::{
    design::{self, Design, OccurrenceId},
    record::RunRecord,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, sync::atomic::Ordering};

/// Format name of `scan.json`.
const FORMAT: &str = "raytrace-scan-v2";

/// Scanned value. Components are addressed by occurrence id and surfaces by name; axis
/// 0/1/2 is X/Y/Z.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Parameter {
    /// Occurrence position in mm.
    ComponentPosition {
        occurrence: OccurrenceId,
        axis: usize,
    },
    /// Occurrence rotation in degrees.
    ComponentRotation {
        occurrence: OccurrenceId,
        axis: usize,
    },
    /// Rod width X / width Y / length (axis 0/1/2) in mm, of the occurrence's rod part.
    RodSize {
        occurrence: OccurrenceId,
        axis: usize,
    },
    /// Surface position in mm, in the coordinates of its part. Without `occurrence` the
    /// design must be a single unplaced system.
    SurfacePosition {
        #[serde(default)]
        occurrence: Option<OccurrenceId>,
        /// Surface name in the part.
        surface: String,
        axis: usize,
    },
    /// Surface rotation in degrees, in the coordinates of its part.
    SurfaceRotation {
        #[serde(default)]
        occurrence: Option<OccurrenceId>,
        surface: String,
        axis: usize,
    },
    /// Wavelength of every source in micrometres.
    Wavelength,
}
impl Parameter {
    pub fn unit(&self) -> &'static str {
        match self {
            Self::ComponentRotation { .. } | Self::SurfaceRotation { .. } => "deg",
            Self::Wavelength => "um",
            _ => "mm",
        }
    }
    fn slot<'a>(&self, d: &'a mut Design) -> Result<&'a mut f64> {
        let bad = || crate::Error::Plan("走査対象の部品・面・軸が見つかりません".into());
        match self {
            Self::ComponentPosition { occurrence, axis }
            | Self::ComponentRotation { occurrence, axis } => {
                let o = d.occurrence_mut(occurrence).ok_or_else(bad)?;
                let v = if matches!(self, Self::ComponentPosition { .. }) {
                    &mut o.pose.translation_mm.0
                } else {
                    &mut o.pose.rotation_deg.0
                };
                v.get_mut(*axis).ok_or_else(bad)
            }
            Self::RodSize { occurrence, axis } => {
                let index = d.occurrence_index(occurrence).ok_or_else(bad)?;
                let r = d
                    .occurrence_part_mut(index)
                    .and_then(|p| p.rod.as_mut())
                    .ok_or_else(bad)?;
                match axis {
                    0 => Ok(&mut r.width_mm),
                    1 => Ok(&mut r.height_mm),
                    2 => Ok(&mut r.length_mm),
                    _ => Err(bad()),
                }
            }
            Self::SurfacePosition {
                occurrence,
                surface,
                axis,
            }
            | Self::SurfaceRotation {
                occurrence,
                surface,
                axis,
            } => {
                let part = match occurrence {
                    None => {
                        if d.is_assembly() {
                            return Err(crate::Error::Plan(
                                "組立光学系は部品の位置・角度を指定してください".into(),
                            ));
                        }
                        d.lone_system_mut().ok_or_else(bad)?
                    }
                    Some(id) => {
                        let index = d.occurrence_index(id).ok_or_else(bad)?;
                        d.occurrence_part_mut(index).ok_or_else(bad)?
                    }
                };
                let s = part
                    .scene
                    .surfaces
                    .iter_mut()
                    .find(|s| s.name == *surface)
                    .ok_or_else(bad)?;
                let v = if matches!(self, Self::SurfacePosition { .. }) {
                    &mut s.pose.translation_mm.0
                } else {
                    &mut s.pose.rotation_deg.0
                };
                v.get_mut(*axis).ok_or_else(bad)
            }
            Self::Wavelength => {
                if d.conditions.monochromatic_override_um.is_none() {
                    let wave = d
                        .compile()
                        .map_err(raytrace_io::Error::from)?
                        .scene
                        .sources
                        .first()
                        .map(|s| s.wavelength_um)
                        .ok_or_else(|| crate::Error::Plan("光源がありません".into()))?;
                    d.conditions.monochromatic_override_um = Some(wave);
                }
                d.conditions
                    .monochromatic_override_um
                    .as_mut()
                    .ok_or_else(bad)
            }
        }
    }
    pub fn value(&self, d: &Design) -> Result<f64> {
        self.slot(&mut d.clone()).copied()
    }
    pub fn apply(&self, d: &mut Design, value: f64) -> Result<()> {
        self.validate_value(value)?;
        *self.slot(d)? = value;
        design::check(d).map_err(crate::Error::from)
    }
    fn validate_value(&self, value: f64) -> Result<()> {
        if !value.is_finite()
            || value.abs() > 1e12
            || (matches!(self, Self::RodSize { .. } | Self::Wavelength) && value <= 0.0)
        {
            return Err(crate::Error::Plan(
                "走査値の単位・範囲を確認してください".into(),
            ));
        }
        Ok(())
    }
    pub fn label(&self, d: &Design) -> String {
        let axis = |a: usize| ["X", "Y", "Z"].get(a).copied().unwrap_or("?");
        let part = |id: &OccurrenceId| d.occurrence(id).map_or("?", |o| o.name.as_str());
        let surface = |occurrence: &Option<OccurrenceId>, surface: &str| match occurrence {
            Some(id) => format!("{} / {surface}", part(id)),
            None => surface.to_string(),
        };
        match self {
            Self::ComponentPosition {
                occurrence,
                axis: a,
            } => {
                format!("{} · 位置 {}", part(occurrence), axis(*a))
            }
            Self::ComponentRotation {
                occurrence,
                axis: a,
            } => {
                format!("{} · 傾き {}", part(occurrence), axis(*a))
            }
            Self::RodSize {
                occurrence,
                axis: a,
            } => format!(
                "{} · {}",
                part(occurrence),
                ["幅 X", "幅 Y", "長さ"].get(*a).unwrap_or(&"?")
            ),
            Self::SurfacePosition {
                occurrence,
                surface: s,
                axis: a,
            } => format!("{} · 位置 {}", surface(occurrence, s), axis(*a)),
            Self::SurfaceRotation {
                occurrence,
                surface: s,
                axis: a,
            } => format!("{} · 傾き {}", surface(occurrence, s), axis(*a)),
            Self::Wavelength => "全光源 · 計算波長".into(),
        }
    }
}

pub fn linear_values(start: f64, end: f64, count: usize) -> Result<Vec<f64>> {
    if !start.is_finite()
        || !end.is_finite()
        || !(1..=256).contains(&count)
        || (count == 1 && start != end)
    {
        return Err(crate::Error::Plan(
            "範囲は有限値、点数は2〜256で指定してください".into(),
        ));
    }
    if count == 1 {
        return Ok(vec![start]);
    }
    Ok((0..count)
        .map(|i| {
            if i == count - 1 {
                end
            } else {
                start + (end - start) * i as f64 / (count - 1) as f64
            }
        })
        .collect())
}
/// `plan` of the `scan` command. At most 256 cases.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ScanPlan {
    pub parameter: Parameter,
    /// Values of `parameter` in its unit (mm, deg or um).
    pub values: Vec<f64>,
    /// Optional second axis; every combination with `values` is traced.
    pub secondary: Option<ScanAxis>,
    pub links: Vec<ParameterLink>,
    pub threads: usize,
    /// Rays per source for every case; null keeps the model setting.
    pub rays_per_source: Option<usize>,
}
impl Default for ScanPlan {
    fn default() -> Self {
        Self {
            parameter: Parameter::Wavelength,
            values: vec![0.45, 0.55, 0.635],
            secondary: None,
            links: Vec::new(),
            threads: 4,
            rays_per_source: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanAxis {
    pub parameter: Parameter,
    pub values: Vec<f64>,
}
/// target = source * scale + offset; scale is dimensionless, offset uses target units.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParameterLink {
    pub source: Parameter,
    pub target: Parameter,
    pub scale: f64,
    pub offset: f64,
}
impl ScanPlan {
    fn link_order(&self, base: &Design) -> Result<Vec<usize>> {
        if self.links.len() > 64 {
            return Err(crate::Error::Plan("連動条件は64個までです".into()));
        }
        if self
            .secondary
            .as_ref()
            .is_some_and(|s| s.parameter == self.parameter)
        {
            return Err(crate::Error::Plan(
                "2軸には異なる対象を指定してください".into(),
            ));
        }
        for (i, l) in self.links.iter().enumerate() {
            l.source.value(base)?;
            l.target.value(base)?;
            if !l.scale.is_finite()
                || !l.offset.is_finite()
                || l.source.unit() != l.target.unit()
                || l.target == self.parameter
                || self
                    .secondary
                    .as_ref()
                    .is_some_and(|s| s.parameter == l.target)
                || self.links[..i].iter().any(|p| p.target == l.target)
            {
                return Err(crate::Error::Plan(
                    "連動先の重複・走査軸との競合・単位・係数を確認してください".into(),
                ));
            }
        }
        let mut order = Vec::new();
        while order.len() < self.links.len() {
            let next = (0..self.links.len()).find(|i| {
                !order.contains(i)
                    && self
                        .links
                        .iter()
                        .enumerate()
                        .all(|(j, l)| l.target != self.links[*i].source || order.contains(&j))
            });
            order.push(next.ok_or_else(|| crate::Error::Plan("連動条件に循環があります".into()))?);
        }
        Ok(order)
    }
    pub fn case_design(&self, base: &Design, value: f64, secondary: Option<f64>) -> Result<Design> {
        let order = self.link_order(base)?;
        let mut p = base.clone();
        self.parameter.apply(&mut p, value)?;
        match (&self.secondary, secondary) {
            (Some(axis), Some(v)) => axis.parameter.apply(&mut p, v)?,
            (None, None) => (),
            _ => return Err(crate::Error::Plan("第2軸の値が一致しません".into())),
        }
        for i in order {
            let l = &self.links[i];
            let v = l.source.value(&p)? * l.scale + l.offset;
            l.target.apply(&mut p, v)?;
        }
        Ok(p)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DetectorMetrics {
    pub name: String,
    pub received_percent: f64,
    pub fwhm_mm: [Option<f64>; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScanCase {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_value: Option<f64>,
    pub state: String,
    pub error: Option<String>,
    pub run_file: Option<String>,
    pub detected_percent: Option<f64>,
    pub detectors: Vec<DetectorMetrics>,
    pub trace_seconds: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScanManifest {
    pub format: String,
    pub software_version: String,
    pub base: Design,
    pub plan: ScanPlan,
    pub cases: Vec<ScanCase>,
}
impl ScanManifest {
    pub fn new(base: &Design, plan: ScanPlan) -> Result<Self> {
        if plan.values.is_empty()
            || plan.values.len() > 256
            || !(1..=64).contains(&plan.threads)
            || plan
                .rays_per_source
                .is_some_and(|n| n == 0 || n > MAX_CASE_RAYS)
        {
            return Err(crate::Error::Plan(
                "走査の点数・光線数・スレッド数が不正です".into(),
            ));
        }
        plan.link_order(base)?;
        for (parameter, values) in std::iter::once((&plan.parameter, &plan.values))
            .chain(plan.secondary.iter().map(|s| (&s.parameter, &s.values)))
        {
            parameter.value(base)?;
            if values.is_empty() || values.len() > 256 {
                return Err(crate::Error::Plan("各軸の走査点数は1〜256です".into()));
            }
            for (i, &v) in values.iter().enumerate() {
                parameter.validate_value(v)?;
                if values[..i].contains(&v) {
                    return Err(crate::Error::Plan("走査値に重複があります".into()));
                }
            }
        }
        if plan.values.len() * plan.secondary.as_ref().map_or(1, |s| s.values.len()) > 256 {
            return Err(crate::Error::Plan(
                "2軸の組合せは合計256ケースまでです".into(),
            ));
        }
        let cases = plan
            .values
            .iter()
            .flat_map(|&value| {
                plan.secondary
                    .as_ref()
                    .map_or(vec![None], |s| s.values.iter().map(|v| Some(*v)).collect())
                    .into_iter()
                    .map(move |secondary_value| ScanCase {
                        value,
                        secondary_value,
                        state: "pending".into(),
                        error: None,
                        run_file: None,
                        detected_percent: None,
                        detectors: Vec::new(),
                        trace_seconds: None,
                    })
            })
            .collect();
        Ok(Self {
            format: FORMAT.into(),
            software_version: env!("CARGO_PKG_VERSION").into(),
            base: base.clone(),
            plan,
            cases,
        })
    }
    pub fn open(dir: &Path) -> Result<Self> {
        let bytes = fs::read(dir.join("scan.json"))
            .map_err(|e| raytrace_io::Error::io(dir.join("scan.json"), e))?;
        let json_error = |e| raytrace_io::Error::json(Some(dir.join("scan.json")), e);
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(json_error)?;
        if value["format"] == "raytrace-scan-v1" {
            return Err(crate::Error::Plan(
                "この走査記録は設計文書への移行前の形式です。走査をやり直してください".into(),
            ));
        }
        let manifest: Self = serde_json::from_value(value).map_err(json_error)?;
        if manifest.format != FORMAT || manifest.software_version != env!("CARGO_PKG_VERSION") {
            return Err(crate::Error::Plan(
                "走査記録は同じソフトの版で再開してください".into(),
            ));
        }
        let expected = Self::new(&manifest.base, manifest.plan.clone())?;
        if manifest.cases.len() != expected.cases.len()
            || manifest.cases.iter().zip(&expected.cases).any(|(c, v)| {
                c.value != v.value
                    || c.secondary_value != v.secondary_value
                    || !["pending", "running", "completed", "failed"].contains(&c.state.as_str())
            })
        {
            return Err(crate::Error::Plan("走査記録のケースが一致しません".into()));
        }
        for (i, c) in manifest
            .cases
            .iter()
            .enumerate()
            .filter(|(_, c)| c.state == "completed")
        {
            let expected = format!("case_{i:04}.raytrace-run.json");
            if c.run_file.as_deref() != Some(&expected) {
                return Err(crate::Error::Plan("走査結果のファイル名が不正です".into()));
            }
            let record = RunRecord::open(&dir.join(expected))?;
            let intended = manifest
                .plan
                .case_design(&manifest.base, c.value, c.secondary_value)?;
            let parameters = std::iter::once(&manifest.plan.parameter)
                .chain(manifest.plan.secondary.iter().map(|s| &s.parameter))
                .chain(manifest.plan.links.iter().map(|l| &l.target));
            if record.calculation.software_version != manifest.software_version
                || parameters
                    .map(|p| Ok(p.value(&record.calculation.design)? != p.value(&intended)?))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .any(|v| v)
            {
                return Err(crate::Error::Plan(
                    "走査記録とケースの計算条件が一致しません".into(),
                ));
            }
        }
        Ok(manifest)
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).map_err(|e| crate::Error::from(raytrace_io::Error::io(dir, e)))?;
        fs::write(
            dir.join("scan.json.tmp"),
            serde_json::to_vec_pretty(self)
                .map_err(|e| raytrace_io::Error::json(Some(dir.join("scan.json.tmp")), e))?,
        )
        .map_err(|e| crate::Error::from(raytrace_io::Error::io(dir.join("scan.json.tmp"), e)))?;
        fs::rename(dir.join("scan.json.tmp"), dir.join("scan.json"))
            .map_err(|e| crate::Error::from(raytrace_io::Error::io(dir.join("scan.json"), e)))
    }
    pub fn execute(
        &mut self,
        dir: &Path,
        control: &TraceControl,
        mut progress: impl FnMut(&Self),
    ) -> Result<()> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.plan.threads)
            .build()
            .map_err(|e| crate::Error::Plan(format!("スレッドプールを作成できません: {e}")))?;
        self.save(dir)?;
        for i in 0..self.cases.len() {
            if self.cases[i].state == "completed" || self.cases[i].state == "failed" {
                continue;
            }
            if control.cancel.load(Ordering::Relaxed) {
                break;
            }
            self.cases[i].state = "running".into();
            self.cases[i].error = None;
            control.completed.store(0, Ordering::Relaxed);
            control.total.store(0, Ordering::Relaxed);
            self.save(dir)?;
            progress(self);
            let outcome = (|| -> Result<Option<ScanCase>> {
                let design = self.plan.case_design(
                    &self.base,
                    self.cases[i].value,
                    self.cases[i].secondary_value,
                )?;
                let mut scene = design.trace_scene()?;
                pipeline::configure_scene(
                    &mut scene,
                    &SceneOverrides {
                        rays_per_source: self.plan.rays_per_source,
                        ..SceneOverrides::default()
                    },
                );
                pipeline::check_limits(
                    &scene,
                    Limits {
                        max_rays: MAX_CASE_RAYS,
                        max_screen_pixels: Some(MAX_SCREEN_PIXELS),
                    },
                )
                .map_err(|violation| {
                    crate::Error::Plan(
                        match violation {
                            LimitViolation::Rays { .. } => {
                                "各ケースの光線数合計は1〜2,000,000本で指定してください"
                            }
                            LimitViolation::ScreenPixels { .. } => "スクリーンは100万画素までです",
                        }
                        .into(),
                    )
                })?;
                let prepared = scene.clone().prepare()?;
                let launch = pipeline::resolve_launch(&prepared, LaunchRequest::Generated)?;
                let options = TraceOptions::default();
                let Completion::Done(timed) = pipeline::run_timed(
                    &pool,
                    &prepared,
                    &launch,
                    options,
                    control,
                    Schedule::once(),
                )?
                else {
                    return Ok(None);
                };
                let (report, seconds) = (timed.report, timed.timings.median());
                let Some(power) = pipeline::launched_power(&report) else {
                    return Err(crate::Error::Plan(
                        "発射基準光量は正の有限値で指定してください".into(),
                    ));
                };
                let detectors = report
                    .detectors
                    .iter()
                    .map(|d| {
                        let stats = pipeline::detector_stats(d, power)?;
                        Ok(DetectorMetrics {
                            name: stats.name,
                            received_percent: stats.received_percent,
                            fwhm_mm: stats.fwhm_mm,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let file = format!("case_{i:04}.raytrace-run.json");
                pipeline::capture(&design, &scene, &launch, self.plan.threads, options).save(
                    &report,
                    seconds,
                    &dir.join(&file),
                )?;
                Ok(Some(ScanCase {
                    value: self.cases[i].value,
                    secondary_value: self.cases[i].secondary_value,
                    state: "completed".into(),
                    error: None,
                    run_file: Some(file),
                    detected_percent: Some(pipeline::detected_percent(&report, power)),
                    detectors,
                    trace_seconds: Some(seconds),
                }))
            })();
            match outcome {
                Ok(Some(case)) => self.cases[i] = case,
                Ok(None) => {
                    self.cases[i].state = "pending".into();
                    self.save(dir)?;
                    progress(self);
                    break;
                }
                Err(error) => {
                    self.cases[i].state = "failed".into();
                    self.cases[i].error = Some(error.to_string());
                }
            }
            self.save(dir)?;
            progress(self);
        }
        Ok(())
    }
}
