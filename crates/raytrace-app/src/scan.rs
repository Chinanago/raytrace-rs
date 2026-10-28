//! Parameter studies with immutable inputs, affine links and resumable cases.
use crate::Result;
use raytrace_core::{
    model::Interaction,
    screen::ScreenAnalysis,
    trace::{TraceControl, TraceOptions, trace_generated_controlled},
};
use raytrace_io::{
    project::Project,
    record::{RunContext, RunRecord},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, sync::atomic::Ordering, time::Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Parameter {
    ComponentPosition { component: usize, axis: usize },
    ComponentRotation { component: usize, axis: usize },
    RodSize { component: usize, axis: usize },
    SurfacePosition { surface: usize, axis: usize },
    SurfaceRotation { surface: usize, axis: usize },
    ComponentPositionId { component: String, axis: usize },
    ComponentRotationId { component: String, axis: usize },
    RodSizeId { component: String, axis: usize },
    SurfacePositionId { surface: String, axis: usize },
    SurfaceRotationId { surface: String, axis: usize },
    Wavelength,
}
impl Parameter {
    pub fn unit(&self) -> &'static str {
        match self {
            Self::ComponentRotation { .. }
            | Self::SurfaceRotation { .. }
            | Self::ComponentRotationId { .. }
            | Self::SurfaceRotationId { .. } => "deg",
            Self::Wavelength => "um",
            _ => "mm",
        }
    }
    fn slot<'a>(&self, p: &'a mut Project) -> Result<&'a mut f64> {
        if let Some(parameter) = self.resolve_id(p)? {
            return parameter.slot(p);
        }
        let bad = || crate::Error::Plan("走査対象の部品・面・軸が見つかりません".into());
        match *self {
            Self::ComponentPosition { component, axis }
            | Self::ComponentRotation { component, axis } => {
                let c = p
                    .assembly
                    .as_mut()
                    .and_then(|a| a.components.get_mut(component))
                    .ok_or_else(bad)?;
                let v = if matches!(self, Self::ComponentPosition { .. }) {
                    &mut c.pose.translation_mm.0
                } else {
                    &mut c.pose.rotation_deg.0
                };
                v.get_mut(axis).ok_or_else(bad)
            }
            Self::RodSize { component, axis } => {
                let r = p
                    .assembly
                    .as_mut()
                    .and_then(|a| a.components.get_mut(component))
                    .and_then(|c| c.rod.as_mut())
                    .ok_or_else(bad)?;
                match axis {
                    0 => Ok(&mut r.width_mm),
                    1 => Ok(&mut r.height_mm),
                    2 => Ok(&mut r.length_mm),
                    _ => Err(bad()),
                }
            }
            Self::SurfacePosition { surface, axis } | Self::SurfaceRotation { surface, axis } => {
                if p.assembly.is_some() {
                    return Err(crate::Error::Plan(
                        "組立光学系は部品の位置・角度を指定してください".into(),
                    ));
                }
                let s = p.scene.surfaces.get_mut(surface).ok_or_else(bad)?;
                let v = if matches!(self, Self::SurfacePosition { .. }) {
                    &mut s.pose.translation_mm.0
                } else {
                    &mut s.pose.rotation_deg.0
                };
                v.get_mut(axis).ok_or_else(bad)
            }
            Self::Wavelength => {
                let wave = p
                    .monochromatic_override_um
                    .or_else(|| p.scene.sources.first().map(|s| s.wavelength_um))
                    .ok_or_else(|| crate::Error::Plan("光源がありません".into()))?;
                Ok(p.monochromatic_override_um.get_or_insert(wave))
            }
            _ => unreachable!("stable references resolved above"),
        }
    }
    pub fn value(&self, p: &Project) -> Result<f64> {
        if let Some(parameter) = self.resolve_id(p)? {
            return parameter.value(p);
        }
        let bad = || crate::Error::Plan("走査対象の部品・面・軸が見つかりません".into());
        let values = match *self {
            Self::ComponentPosition { component, axis }
            | Self::ComponentRotation { component, axis } => {
                let c = p
                    .assembly
                    .as_ref()
                    .and_then(|a| a.components.get(component))
                    .ok_or_else(bad)?;
                if matches!(self, Self::ComponentPosition { .. }) {
                    c.pose.translation_mm.0.get(axis).copied()
                } else {
                    c.pose.rotation_deg.0.get(axis).copied()
                }
            }
            Self::RodSize { component, axis } => {
                let r = p
                    .assembly
                    .as_ref()
                    .and_then(|a| a.components.get(component))
                    .and_then(|c| c.rod.as_ref())
                    .ok_or_else(bad)?;
                [r.width_mm, r.height_mm, r.length_mm].get(axis).copied()
            }
            Self::SurfacePosition { surface, axis } | Self::SurfaceRotation { surface, axis } => {
                if p.assembly.is_some() {
                    return Err(crate::Error::Plan(
                        "組立光学系は部品の位置・角度を指定してください".into(),
                    ));
                }
                let s = p.scene.surfaces.get(surface).ok_or_else(bad)?;
                if matches!(self, Self::SurfacePosition { .. }) {
                    s.pose.translation_mm.0.get(axis).copied()
                } else {
                    s.pose.rotation_deg.0.get(axis).copied()
                }
            }
            Self::Wavelength => p
                .monochromatic_override_um
                .or_else(|| p.scene.sources.first().map(|s| s.wavelength_um)),
            _ => unreachable!("stable references resolved above"),
        };
        values.ok_or_else(bad)
    }
    pub fn apply(&self, p: &mut Project, value: f64) -> Result<()> {
        self.validate_value(value)?;
        *self.slot(p)? = value;
        p.refresh_assembly().map_err(crate::Error::from)
    }
    fn validate_value(&self, value: f64) -> Result<()> {
        if !value.is_finite()
            || value.abs() > 1e12
            || (matches!(
                self,
                Self::RodSize { .. } | Self::RodSizeId { .. } | Self::Wavelength
            ) && value <= 0.0)
        {
            return Err(crate::Error::Plan(
                "走査値の単位・範囲を確認してください".into(),
            ));
        }
        Ok(())
    }
    pub fn label(&self, p: &Project) -> String {
        let axis = |a: usize| ["X", "Y", "Z"].get(a).copied().unwrap_or("?");
        let part = |i: usize| {
            p.assembly
                .as_ref()
                .and_then(|a| a.components.get(i))
                .map_or("?", |c| c.name.as_str())
        };
        match *self {
            Self::ComponentPosition { component, axis: a } => {
                format!("{} · 位置 {}", part(component), axis(a))
            }
            Self::ComponentRotation { component, axis: a } => {
                format!("{} · 傾き {}", part(component), axis(a))
            }
            Self::RodSize { component, axis: a } => format!(
                "{} · {}",
                part(component),
                ["幅 X", "幅 Y", "長さ"].get(a).unwrap_or(&"?")
            ),
            Self::SurfacePosition { surface, axis: a } => format!(
                "{} · 位置 {}",
                p.scene
                    .surfaces
                    .get(surface)
                    .map_or("?", |s| s.name.as_str()),
                axis(a)
            ),
            Self::SurfaceRotation { surface, axis: a } => format!(
                "{} · 傾き {}",
                p.scene
                    .surfaces
                    .get(surface)
                    .map_or("?", |s| s.name.as_str()),
                axis(a)
            ),
            Self::Wavelength => "全光源 · 計算波長".into(),
            _ => self
                .resolve_id(p)
                .ok()
                .flatten()
                .map_or_else(|| "参照先がありません".into(), |v| v.label(p)),
        }
    }
    fn resolve_id(&self, p: &Project) -> Result<Option<Self>> {
        let missing = || crate::Error::Plan("走査対象の固定IDが見つかりません".into());
        let component = |id: &str| {
            p.assembly
                .as_ref()
                .and_then(|a| a.components.iter().position(|c| c.id.0 == id))
                .ok_or_else(missing)
        };
        let surface = |id: &str| {
            if p.assembly.is_some() {
                return Err(crate::Error::Plan(
                    "組立光学系は部品の位置・角度を指定してください".into(),
                ));
            }
            p.identity
                .surfaces
                .iter()
                .position(|s| s.0 == id)
                .ok_or_else(missing)
        };
        Ok(Some(match self {
            Self::ComponentPositionId {
                component: id,
                axis,
            } => Self::ComponentPosition {
                component: component(id)?,
                axis: *axis,
            },
            Self::ComponentRotationId {
                component: id,
                axis,
            } => Self::ComponentRotation {
                component: component(id)?,
                axis: *axis,
            },
            Self::RodSizeId {
                component: id,
                axis,
            } => Self::RodSize {
                component: component(id)?,
                axis: *axis,
            },
            Self::SurfacePositionId { surface: id, axis } => Self::SurfacePosition {
                surface: surface(id)?,
                axis: *axis,
            },
            Self::SurfaceRotationId { surface: id, axis } => Self::SurfaceRotation {
                surface: surface(id)?,
                axis: *axis,
            },
            _ => return Ok(None),
        }))
    }
    /// Bind old array references once, against the saved base project.
    pub fn bind(&self, p: &Project) -> Result<Self> {
        self.value(p)?;
        let component = |i: usize| {
            p.assembly
                .as_ref()
                .and_then(|a| a.components.get(i))
                .map(|c| c.id.to_string())
                .ok_or_else(|| crate::Error::Plan("部品IDがありません".into()))
        };
        let surface = |i: usize| {
            p.identity
                .surfaces
                .get(i)
                .map(ToString::to_string)
                .ok_or_else(|| crate::Error::Plan("面IDがありません".into()))
        };
        Ok(match *self {
            Self::ComponentPosition { component: i, axis } => Self::ComponentPositionId {
                component: component(i)?,
                axis,
            },
            Self::ComponentRotation { component: i, axis } => Self::ComponentRotationId {
                component: component(i)?,
                axis,
            },
            Self::RodSize { component: i, axis } => Self::RodSizeId {
                component: component(i)?,
                axis,
            },
            Self::SurfacePosition { surface: i, axis } => Self::SurfacePositionId {
                surface: surface(i)?,
                axis,
            },
            Self::SurfaceRotation { surface: i, axis } => Self::SurfaceRotationId {
                surface: surface(i)?,
                axis,
            },
            _ => self.clone(),
        })
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanPlan {
    pub parameter: Parameter,
    pub values: Vec<f64>,
    pub secondary: Option<ScanAxis>,
    pub links: Vec<ParameterLink>,
    pub threads: usize,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanAxis {
    pub parameter: Parameter,
    pub values: Vec<f64>,
}
/// target = source * scale + offset; scale is dimensionless, offset uses target units.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterLink {
    pub source: Parameter,
    pub target: Parameter,
    pub scale: f64,
    pub offset: f64,
}
impl ScanPlan {
    pub fn bind(&self, base: &Project) -> Result<Self> {
        let mut plan = self.clone();
        plan.parameter = plan.parameter.bind(base)?;
        if let Some(s) = &mut plan.secondary {
            s.parameter = s.parameter.bind(base)?;
        }
        for link in &mut plan.links {
            link.source = link.source.bind(base)?;
            link.target = link.target.bind(base)?;
        }
        Ok(plan)
    }
    fn link_order(&self, base: &Project) -> Result<Vec<usize>> {
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
    pub fn case_project(
        &self,
        base: &Project,
        value: f64,
        secondary: Option<f64>,
    ) -> Result<Project> {
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectorMetrics {
    pub name: String,
    pub received_percent: f64,
    pub fwhm_mm: [Option<f64>; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanManifest {
    pub format: String,
    pub software_version: String,
    pub base: Project,
    pub plan: ScanPlan,
    pub cases: Vec<ScanCase>,
}
impl ScanManifest {
    pub fn new(base: &Project, plan: ScanPlan) -> Result<Self> {
        let mut normalized = base.clone();
        normalized.refresh_assembly()?;
        let base = &normalized;
        let plan = plan.bind(base)?;
        if plan.values.is_empty()
            || plan.values.len() > 256
            || !(1..=64).contains(&plan.threads)
            || plan
                .rays_per_source
                .is_some_and(|n| n == 0 || n > 2_000_000)
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
            format: "raytrace-scan-v1".into(),
            software_version: env!("CARGO_PKG_VERSION").into(),
            base: base.clone(),
            plan,
            cases,
        })
    }
    pub fn open(dir: &Path) -> Result<Self> {
        let mut manifest: Self = serde_json::from_slice(
            &fs::read(dir.join("scan.json"))
                .map_err(|e| raytrace_io::Error::io(dir.join("scan.json"), e))?,
        )
        .map_err(|e| raytrace_io::Error::json(Some(dir.join("scan.json")), e))?;
        if manifest.format != "raytrace-scan-v1"
            || manifest.software_version != env!("CARGO_PKG_VERSION")
        {
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
            let mut recorded_project = record.calculation.project.clone();
            recorded_project.refresh_assembly()?;
            let intended =
                manifest
                    .plan
                    .case_project(&manifest.base, c.value, c.secondary_value)?;
            let parameters = std::iter::once(&manifest.plan.parameter)
                .chain(manifest.plan.secondary.iter().map(|s| &s.parameter))
                .chain(manifest.plan.links.iter().map(|l| &l.target));
            if record.calculation.software_version != manifest.software_version
                || parameters
                    .map(|p| Ok(p.value(&recorded_project)? != p.value(&intended)?))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .any(|v| v)
            {
                return Err(crate::Error::Plan(
                    "走査記録とケースの計算条件が一致しません".into(),
                ));
            }
        }
        manifest.base.refresh_assembly()?;
        manifest.plan = manifest.plan.bind(&manifest.base)?;
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
                let project = self.plan.case_project(
                    &self.base,
                    self.cases[i].value,
                    self.cases[i].secondary_value,
                )?;
                let mut scene = project.trace_scene()?;
                if let Some(n) = self.plan.rays_per_source {
                    for s in &mut scene.sources {
                        s.count = n;
                    }
                }
                let total: u128 = scene.sources.iter().map(|s| s.count as u128).sum();
                if total == 0 || total > 2_000_000 {
                    return Err(crate::Error::Plan(
                        "各ケースの光線数合計は1〜2,000,000本で指定してください".into(),
                    ));
                }
                if scene.surfaces.iter().any(|s| matches!(s.interaction,Interaction::Detector{pixels} if pixels[0].saturating_mul(pixels[1])>1_000_000)) {return Err(crate::Error::Plan("スクリーンは100万画素までです".into()));}
                let prepared = scene.clone().prepare()?;
                let start = Instant::now();
                let Some(report) = pool.install(|| {
                    trace_generated_controlled(&prepared, TraceOptions::default(), control)
                })?
                else {
                    return Ok(None);
                };
                let seconds = start.elapsed().as_secs_f64();
                let power = report.budget.launched;
                if !power.is_finite() || power <= 0.0 {
                    return Err(crate::Error::Plan(
                        "発射基準光量は正の有限値で指定してください".into(),
                    ));
                }
                let mut detectors = Vec::new();
                for d in &report.detectors {
                    let a = ScreenAnalysis::from_detector(d)?;
                    detectors.push(DetectorMetrics {
                        name: d.name.clone(),
                        received_percent: 100.0 * d.weights.iter().sum::<f64>()
                            / report.budget.launched,
                        fwhm_mm: [
                            a.x_profile.as_ref().and_then(|p| p.half_maximum.width_mm()),
                            a.y_profile.as_ref().and_then(|p| p.half_maximum.width_mm()),
                        ],
                    });
                }
                let file = format!("case_{i:04}.raytrace-run.json");
                RunContext::capture(
                    &project,
                    &scene,
                    None,
                    self.plan.threads,
                    TraceOptions::default(),
                )
                .save(&report, seconds, &dir.join(&file))?;
                Ok(Some(ScanCase {
                    value: self.cases[i].value,
                    secondary_value: self.cases[i].secondary_value,
                    state: "completed".into(),
                    error: None,
                    run_file: Some(file),
                    detected_percent: Some(100.0 * report.budget.detected / report.budget.launched),
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
