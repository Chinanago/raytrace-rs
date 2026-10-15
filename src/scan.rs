//! One-variable parameter studies with immutable inputs and resumable completed cases.
use crate::{
    model::Interaction,
    project::Project,
    record::{RunContext, RunRecord},
    screen::ScreenAnalysis,
    trace::{trace_controlled, TraceControl, TraceOptions},
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
    fn slot<'a>(&self, p: &'a mut Project) -> Result<&'a mut f64, String> {
        let bad = || "走査対象の部品・面・軸が見つかりません".to_string();
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
                    return Err("組立光学系は部品の位置・角度を指定してください".into());
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
                    .ok_or("光源がありません")?;
                Ok(p.monochromatic_override_um.get_or_insert(wave))
            }
        }
    }
    pub fn value(&self, p: &Project) -> Result<f64, String> {
        self.slot(&mut p.clone()).copied()
    }
    pub fn apply(&self, p: &mut Project, value: f64) -> Result<(), String> {
        self.validate_value(value)?;
        *self.slot(p)? = value;
        p.refresh_assembly()
    }
    fn validate_value(&self, value: f64) -> Result<(), String> {
        if !value.is_finite()
            || value.abs() > 1e12
            || (matches!(self, Self::RodSize { .. } | Self::Wavelength) && value <= 0.0)
        {
            return Err("走査値の単位・範囲を確認してください".into());
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
        }
    }
}

pub fn linear_values(start: f64, end: f64, count: usize) -> Result<Vec<f64>, String> {
    if !start.is_finite()
        || !end.is_finite()
        || !(1..=256).contains(&count)
        || (count == 1 && start != end)
    {
        return Err("範囲は有限値、点数は2〜256で指定してください".into());
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
    pub threads: usize,
    pub rays_per_source: Option<usize>,
}
impl Default for ScanPlan {
    fn default() -> Self {
        Self {
            parameter: Parameter::Wavelength,
            values: vec![0.45, 0.55, 0.635],
            threads: 4,
            rays_per_source: None,
        }
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
    pub fn new(base: &Project, plan: ScanPlan) -> Result<Self, String> {
        if plan.values.is_empty()
            || plan.values.len() > 256
            || !(1..=64).contains(&plan.threads)
            || plan
                .rays_per_source
                .is_some_and(|n| n == 0 || n > 2_000_000)
        {
            return Err("走査の点数・光線数・スレッド数が不正です".into());
        }
        plan.parameter.value(base)?;
        for (i, &v) in plan.values.iter().enumerate() {
            plan.parameter.validate_value(v)?;
            if !v.is_finite() || plan.values[..i].contains(&v) {
                return Err("走査値に無効な値・重複があります".into());
            }
        }
        let cases = plan
            .values
            .iter()
            .map(|&value| ScanCase {
                value,
                state: "pending".into(),
                error: None,
                run_file: None,
                detected_percent: None,
                detectors: Vec::new(),
                trace_seconds: None,
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
    pub fn open(dir: &Path) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_slice(&fs::read(dir.join("scan.json")).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if manifest.format != "raytrace-scan-v1"
            || manifest.software_version != env!("CARGO_PKG_VERSION")
        {
            return Err("走査記録は同じソフトの版で再開してください".into());
        }
        Self::new(&manifest.base, manifest.plan.clone())?;
        if manifest.cases.len() != manifest.plan.values.len()
            || manifest
                .cases
                .iter()
                .zip(&manifest.plan.values)
                .any(|(c, v)| {
                    c.value != *v
                        || !["pending", "running", "completed", "failed"]
                            .contains(&c.state.as_str())
                })
        {
            return Err("走査記録のケースが一致しません".into());
        }
        for (i, c) in manifest
            .cases
            .iter()
            .enumerate()
            .filter(|(_, c)| c.state == "completed")
        {
            let expected = format!("case_{i:04}.raytrace-run.json");
            if c.run_file.as_deref() != Some(&expected) {
                return Err("走査結果のファイル名が不正です".into());
            }
            let record = RunRecord::open(&dir.join(expected))?;
            if record.calculation.software_version != manifest.software_version
                || manifest.plan.parameter.value(&record.calculation.project)? != c.value
            {
                return Err("走査記録とケースの計算条件が一致しません".into());
            }
        }
        Ok(manifest)
    }
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("scan.json.tmp"),
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(dir.join("scan.json.tmp"), dir.join("scan.json")).map_err(|e| e.to_string())
    }
    pub fn execute(
        &mut self,
        dir: &Path,
        control: &TraceControl,
        mut progress: impl FnMut(&Self),
    ) -> Result<(), String> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.plan.threads)
            .build()
            .map_err(|e| e.to_string())?;
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
            let outcome = (|| -> Result<Option<ScanCase>, String> {
                let mut project = self.base.clone();
                self.plan
                    .parameter
                    .apply(&mut project, self.cases[i].value)?;
                let mut scene = project.trace_scene()?;
                if let Some(n) = self.plan.rays_per_source {
                    for s in &mut scene.sources {
                        s.count = n;
                    }
                }
                let total: u128 = scene.sources.iter().map(|s| s.count as u128).sum();
                if total == 0 || total > 2_000_000 {
                    return Err("各ケースの光線数合計は1〜2,000,000本で指定してください".into());
                }
                if scene.surfaces.iter().any(|s| matches!(s.interaction,Interaction::Detector{pixels} if pixels[0].saturating_mul(pixels[1])>1_000_000)) {return Err("スクリーンは100万画素までです".into());}
                let prepared = scene.clone().prepare()?;
                let rays = prepared.generate_rays()?;
                let power = rays.iter().map(|r| r.weight).sum::<f64>();
                if !power.is_finite() || power <= 0.0 {
                    return Err("発射基準光量は正の有限値で指定してください".into());
                }
                let start = Instant::now();
                let Some(report) = pool.install(|| {
                    trace_controlled(&prepared, &rays, TraceOptions::default(), control)
                }) else {
                    return Ok(None);
                };
                let seconds = start.elapsed().as_secs_f64();
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
                    self.cases[i].error = Some(error);
                }
            }
            self.save(dir)?;
            progress(self);
        }
        Ok(())
    }
}
