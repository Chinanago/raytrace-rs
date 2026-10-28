use crate::{
    Command, Error, MaterialInput, Outcome, Query, Request, Result,
    properties::{self, Target},
};
use raytrace_core::{
    model::Material,
    trace::{TraceControl, TraceOptions, trace_generated_controlled},
};
use raytrace_io::{
    assembly::Component,
    identity::Bindings,
    project::Project,
    record::{RUN_FORMAT, RunContext, RunRecord},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Snapshot {
    project: Project,
    aliases: HashMap<String, String>,
    revision: u64,
    label: String,
}
/// An isolated editor used to load assets without blocking the interactive session.
pub struct EditPreparation {
    session: Session,
    request: Request,
    generation: u64,
    reset: bool,
}
pub struct PreparedEdit {
    project: Project,
    aliases: HashMap<String, String>,
    generation: u64,
    reset: bool,
    created: Vec<String>,
    label: String,
}
impl EditPreparation {
    pub fn run(mut self) -> Result<PreparedEdit> {
        let label = if let Command::Transaction { label, .. } = &self.request.command {
            label.clone()
        } else {
            "読込と編集".into()
        };
        let outcome = self.session.apply(self.request)?;
        Ok(PreparedEdit {
            project: self.session.project.clone(),
            aliases: self.session.aliases.clone(),
            generation: self.generation,
            reset: self.reset,
            created: outcome.created,
            label,
        })
    }
}
enum JobProduct {
    Trace(Box<RunRecord>),
    Study(Value),
}
struct Job {
    kind: &'static str,
    revision: u64,
    control: Arc<TraceControl>,
    rx: Receiver<Result<Option<JobProduct>>>,
    record: Option<Box<RunRecord>>,
    study: Option<Value>,
    error: Option<Error>,
    finished: bool,
    bindings: Bindings,
}
impl Job {
    fn poll(&mut self) {
        if self.finished {
            return;
        }
        match self.rx.try_recv() {
            Ok(Ok(product)) => {
                match product {
                    Some(JobProduct::Trace(record)) => self.record = Some(record),
                    Some(JobProduct::Study(value)) => self.study = Some(value),
                    None => (),
                }
                self.finished = true;
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.finished = true;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some(Error::new("worker_failed", "計算ワーカーが終了しました"));
                self.finished = true;
            }
            Err(mpsc::TryRecvError::Empty) => (),
        }
    }
    fn summary(&mut self, revision: u64) -> Value {
        self.poll();
        let status = if !self.finished {
            "running"
        } else if self.error.is_some() {
            "failed"
        } else if self.record.is_some() || self.study.is_some() {
            "completed"
        } else {
            "cancelled"
        };
        json!({"kind":self.kind,"status":status,"revision":self.revision,"stale":self.revision != revision,"progress_scope":if self.kind=="trace" { "launch_rays" } else { "current_case_rays" },
            "completed":self.control.completed.load(Ordering::Relaxed),"total":self.control.total.load(Ordering::Relaxed),
            "error":self.error,"budget":self.record.as_ref().map(|r| &r.report.budget),"study":self.study,"bindings":self.bindings})
    }
}

/// Owns mutable editor state. Tracing jobs hold immutable snapshots of that state.
pub struct Session {
    project: Project,
    revision: u64,
    next_revision: u64,
    id_high_water: u64,
    aliases: HashMap<String, String>,
    past: VecDeque<Snapshot>,
    future: Vec<Snapshot>,
    coalesce: Option<String>,
    jobs: HashMap<String, Job>,
    job_aliases: HashMap<String, String>,
    next_job: u64,
    base: PathBuf,
    catalogs: Vec<PathBuf>,
    cad_deflection_mm: f64,
}
impl Drop for Session {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            if !job.finished {
                job.control.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}
impl Session {
    pub fn new(mut project: Project) -> Result<Self> {
        project.refresh_assembly()?;
        project.validate_identity()?;
        let id_high_water = project.identity.next_id;
        Ok(Self {
            project,
            revision: 0,
            next_revision: 1,
            id_high_water,
            aliases: HashMap::new(),
            past: VecDeque::new(),
            future: Vec::new(),
            coalesce: None,
            jobs: HashMap::new(),
            job_aliases: HashMap::new(),
            next_job: 1,
            base: std::env::current_dir()?,
            catalogs: Vec::new(),
            cad_deflection_mm: 0.02,
        })
    }
    pub fn project(&self) -> &Project {
        &self.project
    }
    /// Carry the non-undoable allocation watermark into a frontend's edit buffer.
    pub fn prepare_draft(&self, project: &mut Project) {
        project.identity.next_id = project.identity.next_id.max(self.id_high_water);
    }
    fn frozen_project(&self) -> Project {
        let mut project = self.project.clone();
        self.prepare_draft(&mut project);
        project
    }
    /// Replace a GUI model while keeping revisions unique within the live session.
    pub fn reset_project(&mut self, project: Project) -> Result<()> {
        self.replace(project)
    }
    pub fn prepare_edit(&self, request: Request) -> Result<EditPreparation> {
        self.check_revision(request.expected_revision)?;
        if !request.command.is_edit() && !matches!(request.command, Command::Open { .. }) {
            return Err(Error::new("not_an_edit", "編集コマンドを指定してください"));
        }
        let reset = matches!(request.command, Command::Open { .. });
        let mut fork = Self::new(self.project.clone())?;
        fork.revision = self.revision;
        fork.next_revision = self.next_revision;
        fork.id_high_water = self.id_high_water;
        fork.aliases = self.aliases.clone();
        fork.base = self.base.clone();
        fork.catalogs = self.catalogs.clone();
        fork.cad_deflection_mm = self.cad_deflection_mm;
        Ok(EditPreparation {
            session: fork,
            request,
            generation: self.next_revision,
            reset,
        })
    }
    pub fn apply_prepared(&mut self, prepared: PreparedEdit) -> Result<Outcome> {
        if self.next_revision != prepared.generation {
            return Err(Error::new(
                "revision_conflict",
                format!(
                    "読込中に光学系が変更されました（現在のrevision={}）",
                    self.revision
                ),
            ));
        }
        if prepared.reset {
            self.replace(prepared.project)?;
            return Ok(self.outcome(true, prepared.created, Value::Null));
        }
        self.commit(
            prepared.project,
            prepared.aliases,
            &prepared.label,
            None,
            prepared.created,
        )
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn job_control(&self, id: &str) -> Result<Arc<TraceControl>> {
        self.jobs
            .get(id)
            .map(|j| j.control.clone())
            .ok_or_else(|| Error::new("not_found", id))
    }
    pub fn completed_record(&mut self, id: &str) -> Result<Option<(u64, Box<RunRecord>)>> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| Error::new("not_found", id))?;
        job.poll();
        Ok(job.record.as_ref().map(|r| (job.revision, r.clone())))
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    pub fn set_base_directory(&mut self, base: impl Into<PathBuf>) {
        self.base = base.into();
    }
    pub fn set_catalogs(&mut self, catalogs: Vec<PathBuf>) {
        self.catalogs = catalogs;
    }
    pub fn set_cad_deflection_mm(&mut self, value: f64) -> Result<()> {
        properties::dimensions(&[value])?;
        self.cad_deflection_mm = value;
        Ok(())
    }
    pub fn end_gesture(&mut self) {
        self.coalesce = None;
    }
    pub(crate) fn require_finished_jobs(&mut self) -> Result<()> {
        for (id, j) in &mut self.jobs {
            j.poll();
            if !j.finished {
                return Err(Error::new(
                    "unfinished_jobs",
                    format!("{id} の完了をwaitで確認してください"),
                ));
            }
            if let Some(error) = &j.error {
                return Err(error.clone());
            }
        }
        Ok(())
    }
    fn path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.base.join(path)
        }
    }
    fn open_project(
        &self,
        path: &Path,
        catalogs: &[PathBuf],
        tolerance: Option<f64>,
    ) -> Result<Project> {
        let mut dirs = self.catalogs.clone();
        dirs.extend(catalogs.iter().map(|p| self.path(p)));
        Ok(Project::open_with_cad_tolerance(
            &self.path(path),
            &dirs,
            tolerance.unwrap_or(self.cad_deflection_mm),
        )?)
    }
    fn check_revision(&self, expected: Option<u64>) -> Result<()> {
        if expected.is_some_and(|r| r != self.revision) {
            Err(Error::new(
                "revision_conflict",
                format!("現在のrevisionは{}です", self.revision),
            ))
        } else {
            Ok(())
        }
    }
    fn snapshot(&self, label: String) -> Snapshot {
        Snapshot {
            project: self.project.clone(),
            aliases: self.aliases.clone(),
            revision: self.revision,
            label,
        }
    }
    fn outcome(&self, changed: bool, created: Vec<String>, data: Value) -> Outcome {
        Outcome {
            revision: self.revision,
            changed,
            created,
            data,
        }
    }
    fn resolve(&self, target: &str) -> String {
        if let Some((parent, local)) = target.split_once('/') {
            format!(
                "{}/{}",
                self.aliases.get(parent).map_or(parent, String::as_str),
                local
            )
        } else {
            self.aliases
                .get(target)
                .cloned()
                .unwrap_or_else(|| target.into())
        }
    }
    fn alias(aliases: &mut HashMap<String, String>, name: &Option<String>, id: &str) -> Result<()> {
        if let Some(name) = name {
            if name.is_empty()
                || name == "project"
                || name.contains('/')
                || name.starts_with('c') && name[1..].parse::<u64>().is_ok()
                || aliases.contains_key(name)
            {
                return Err(Error::new(
                    "invalid_alias",
                    format!("別の部品名を指定してください: {name}"),
                ));
            }
            aliases.insert(name.clone(), id.into());
        }
        Ok(())
    }
    fn check_job_alias(&self, alias: &Option<String>) -> Result<()> {
        if let Some(name) = alias
            && (name.is_empty()
                || self.job_aliases.contains_key(name)
                || name.starts_with('j') && name[1..].parse::<u64>().is_ok())
        {
            return Err(Error::new("invalid_alias", name));
        }
        Ok(())
    }
    fn material(&self, input: &MaterialInput, p: &Project) -> Result<(Material, String)> {
        Ok(match input {
            MaterialInput::Catalog {
                directory,
                catalog,
                name,
            } => (
                raytrace_io::zemax::read_catalog_material(&self.path(directory), catalog, name)?,
                name.clone(),
            ),
            MaterialInput::Existing { name } => {
                let i = p
                    .material_names
                    .iter()
                    .position(|n| n == name)
                    .ok_or_else(|| Error::new("not_found", format!("材料 {name} がありません")))?;
                (p.scene.materials[i].clone(), name.clone())
            }
            MaterialInput::Constant {
                n,
                absorption_per_mm,
            } => {
                properties::dimensions(&[*n])?;
                if !absorption_per_mm.is_finite() || *absorption_per_mm < 0. {
                    return Err(Error::new(
                        "invalid_value",
                        "吸収係数は非負の有限値で指定してください",
                    ));
                }
                (
                    Material::Constant {
                        n: *n,
                        absorption_per_mm: *absorption_per_mm,
                    },
                    format!("n={n}"),
                )
            }
        })
    }
    fn edit(
        &self,
        p: &mut Project,
        aliases: &mut HashMap<String, String>,
        command: &Command,
        created: &mut Vec<String>,
        depth: usize,
    ) -> Result<()> {
        if depth > 16 {
            return Err(Error::new(
                "transaction_depth",
                "一括編集の入れ子は16段までです",
            ));
        }
        let resolve = |target: &str| {
            if let Some((parent, local)) = target.split_once('/') {
                format!(
                    "{}/{}",
                    aliases.get(parent).map_or(parent, String::as_str),
                    local
                )
            } else {
                aliases
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| target.into())
            }
        };
        let added = match command {
            Command::AddComponent {
                path,
                kind,
                alias,
                catalogs,
                cad_deflection_mm,
            } => {
                let source = self.open_project(path, catalogs, *cad_deflection_mm)?;
                Some((Component::imported(source, (*kind).into())?, alias))
            }
            Command::AddRod {
                material,
                size_mm,
                alias,
            } => {
                properties::dimensions(size_mm)?;
                let (m, name) = self.material(material, p)?;
                let mut c = Component::rod(m, name);
                let r = c.rod.as_mut().unwrap();
                r.width_mm = size_mm[0];
                r.height_mm = size_mm[1];
                r.length_mm = size_mm[2];
                Some((c, alias))
            }
            Command::AddDetector {
                size_mm,
                pixels,
                alias,
            } => {
                let mut c = Component::detector();
                properties::dimensions(size_mm)?;
                properties::pixels(*pixels)?;
                let s = &mut c.project.scene.surfaces[0];
                s.aperture = raytrace_core::model::Aperture::Rectangle {
                    half_width_mm: size_mm[0] / 2.,
                    half_height_mm: size_mm[1] / 2.,
                };
                s.interaction = raytrace_core::model::Interaction::Detector { pixels: *pixels };
                Some((c, alias))
            }
            Command::DuplicateComponent { target, alias } => {
                let Target::Component(i) = properties::locate(p, &resolve(target))? else {
                    return Err(Error::new("invalid_target", "部品のIDを指定してください"));
                };
                let mut c = p.assembly.as_ref().unwrap().components[i].clone();
                c.id = Default::default();
                c.name.push_str(" コピー");
                Some((c, alias))
            }
            Command::RemoveComponent { target } => {
                let id = resolve(target);
                let Target::Component(i) = properties::locate(p, &id)? else {
                    return Err(Error::new("invalid_target", "部品のIDを指定してください"));
                };
                p.assembly.as_mut().unwrap().components.remove(i);
                aliases.retain(|_, v| *v != id);
                None
            }
            Command::SetProperty { target, change } => {
                properties::set(p, &resolve(target), change)?;
                None
            }
            Command::SetRodMaterial { target, material } => {
                let Target::Component(i) = properties::locate(p, &resolve(target))? else {
                    return Err(Error::new("invalid_target", "ロッドのIDを指定してください"));
                };
                let (material, name) = self.material(material, p)?;
                let c = &mut p.assembly.as_mut().unwrap().components[i];
                if c.rod.is_none() {
                    return Err(Error::new("invalid_target", "ロッドのIDを指定してください"));
                }
                let slot = c.project.scene.materials.get_mut(1).ok_or_else(|| {
                    Error::new("invalid_material", "ロッドの材料定義が不足しています")
                })?;
                *slot = material;
                c.project
                    .material_names
                    .resize(c.project.scene.materials.len(), String::new());
                c.project.material_names[1] = name;
                None
            }
            Command::Transaction { edits, .. } => {
                if edits.len() > 4096 {
                    return Err(Error::new("transaction_size", "一括編集は4096操作までです"));
                }
                for c in edits {
                    self.edit(p, aliases, c, created, depth + 1)?;
                }
                None
            }
            _ => {
                return Err(Error::new(
                    "not_an_edit",
                    "一括編集には部品の追加・削除・値の変更を指定してください",
                ));
            }
        };
        if let Some((c, alias)) = added {
            p.add_component(c)?;
            let id = p
                .assembly
                .as_ref()
                .unwrap()
                .components
                .last()
                .unwrap()
                .id
                .to_string();
            Self::alias(aliases, alias, &id)?;
            created.push(id);
        }
        p.refresh_assembly()?;
        Ok(())
    }
    /// Shared commit boundary for existing GUI editors during their gradual migration.
    /// Coalescing stores one undo snapshot for a whole drag; `end_gesture` closes it.
    pub fn commit_project(
        &mut self,
        mut candidate: Project,
        label: &str,
        expected: Option<u64>,
        gesture: Option<&str>,
    ) -> Result<Outcome> {
        self.check_revision(expected)?;
        candidate.identity.next_id = candidate.identity.next_id.max(self.id_high_water);
        candidate.refresh_assembly()?;
        candidate.validate_identity()?;
        self.commit(candidate, self.aliases.clone(), label, gesture, Vec::new())
    }
    fn commit(
        &mut self,
        candidate: Project,
        aliases: HashMap<String, String>,
        label: &str,
        gesture: Option<&str>,
        created: Vec<String>,
    ) -> Result<Outcome> {
        // Identity counters are intentionally monotonic even after undo.
        if gesture.is_none() || self.coalesce.as_deref() != gesture {
            self.past.push_back(self.snapshot(label.into()));
        }
        self.coalesce = gesture.map(str::to_owned);
        self.future.clear();
        self.id_high_water = self.id_high_water.max(candidate.identity.next_id);
        self.project = candidate;
        self.aliases = aliases;
        self.revision = self.next_revision;
        self.next_revision += 1;
        while self.past.len() > 64 {
            self.past.pop_front();
        }
        let mut bytes: usize = self.past.iter().map(|s| estimated_bytes(&s.project)).sum();
        while self.past.len() > 1 && bytes > 128 * 1024 * 1024 {
            if let Some(s) = self.past.pop_front() {
                bytes = bytes.saturating_sub(estimated_bytes(&s.project));
            }
        }
        Ok(self.outcome(true, created, Value::Null))
    }
    pub fn apply(&mut self, request: Request) -> Result<Outcome> {
        self.check_revision(request.expected_revision)?;
        let c = request.command;
        if c.is_edit() {
            let mut p = self.project.clone();
            p.identity.next_id = p.identity.next_id.max(self.id_high_water);
            let mut aliases = self.aliases.clone();
            let mut created = Vec::new();
            self.edit(&mut p, &mut aliases, &c, &mut created, 0)?;
            p.validate_identity()?;
            let label = if let Command::Transaction { label, .. } = &c {
                label.as_str()
            } else {
                "編集"
            };
            return self.commit(p, aliases, label, None, created);
        }
        let changed = matches!(c, Command::NewAssembly { .. } | Command::Open { .. });
        let data = match c {
            Command::NewAssembly { name } => {
                let mut p = Project::new_assembly();
                p.scene.name = name;
                self.replace(p)?;
                Value::Null
            }
            Command::Open {
                path,
                catalogs,
                cad_deflection_mm,
            } => {
                let p = self.open_project(&path, &catalogs, cad_deflection_mm)?;
                self.replace(p)?;
                Value::Null
            }
            Command::Undo { .. } | Command::Redo { .. } => {
                self.end_gesture();
                let undo = matches!(c, Command::Undo { .. });
                let previous = if undo {
                    self.past.pop_back()
                } else {
                    self.future.pop()
                }
                .ok_or_else(|| Error::new("history_empty", "復元する編集がありません"))?;
                let current = self.snapshot(previous.label.clone());
                if undo {
                    self.future.push(current);
                } else {
                    self.past.push_back(current);
                }
                self.project = previous.project;
                self.aliases = previous.aliases;
                self.revision = previous.revision;
                return Ok(self.outcome(true, Vec::new(), Value::Null));
            }
            Command::Save { path } => {
                let path = self.path(&path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                self.frozen_project().save(&path)?;
                json!({"path":path})
            }
            Command::ExportZmx { path } => {
                let path = self.path(&path);
                raytrace_io::zemax::export(&self.project, &path)?;
                json!({"path":path})
            }
            Command::Trace { options } => {
                let alias = options.alias.clone();
                self.check_job_alias(&alias)?;
                let id = self.start_trace(options)?;
                if let Some(name) = alias {
                    self.job_aliases.insert(name, id.clone());
                }
                return Ok(self.outcome(false, vec![id.clone()], json!({"job":id})));
            }
            Command::Scan {
                plan,
                output,
                alias,
            } => {
                self.check_job_alias(&alias)?;
                let plan = serde_json::from_slice(&std::fs::read(self.path(&plan))?)?;
                let mut manifest =
                    raytrace_app::scan::ScanManifest::new(&self.frozen_project(), plan)?;
                let output = self.path(&output);
                let id = self.start_study("scan", move |control| {
                    manifest.execute(&output, control, |_| {})?;
                    if control.cancel.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    if manifest.cases.iter().any(|c| c.state == "failed") {
                        return Err(Error::new(
                            "study_failed",
                            format!("走査に失敗したケースがあります: {}", output.display()),
                        ));
                    }
                    Ok(Some(json!({"path":output,"cases":manifest.cases})))
                })?;
                if let Some(name) = alias {
                    self.job_aliases.insert(name, id.clone());
                }
                return Ok(self.outcome(false, vec![id.clone()], json!({"job":id})));
            }
            Command::Convergence {
                plan,
                output,
                alias,
            } => {
                self.check_job_alias(&alias)?;
                let plan = serde_json::from_slice(&std::fs::read(self.path(&plan))?)?;
                let project = self.frozen_project();
                let output = self.path(&output);
                let id = self.start_study("convergence", move |control| {
                    let Some(summary) = raytrace_app::convergence::run_project_controlled(
                        &project, &plan, &output, control,
                    )?
                    else {
                        return Ok(None);
                    };
                    if summary.cases.iter().any(|c| c.error.is_some()) {
                        return Err(Error::new(
                            "study_failed",
                            format!("収束確認に失敗したケースがあります: {}", output.display()),
                        ));
                    }
                    Ok(Some(json!({"path":output,"summary":summary})))
                })?;
                if let Some(name) = alias {
                    self.job_aliases.insert(name, id.clone());
                }
                return Ok(self.outcome(false, vec![id.clone()], json!({"job":id})));
            }
            Command::Wait { job, timeout_ms } => {
                if timeout_ms > 60_000 {
                    return Err(Error::new("invalid_value", "待機は1回60,000 msまでです"));
                }
                let id = self.job_aliases.get(&job).unwrap_or(&job).clone();
                let started = Instant::now();
                loop {
                    let j = self
                        .jobs
                        .get_mut(&id)
                        .ok_or_else(|| Error::new("not_found", &id))?;
                    j.poll();
                    if j.finished || started.elapsed() >= Duration::from_millis(timeout_ms) {
                        break j.summary(self.revision);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Command::Cancel { job } => {
                let id = self.job_aliases.get(&job).unwrap_or(&job);
                let j = self
                    .jobs
                    .get(id)
                    .ok_or_else(|| Error::new("not_found", id))?;
                j.control.cancel.store(true, Ordering::Relaxed);
                json!({"job":id,"cancel_requested":true})
            }
            Command::Query { query } => self.query(query)?,
            _ => unreachable!("edits handled above"),
        };
        Ok(self.outcome(changed, Vec::new(), data))
    }
    fn replace(&mut self, mut project: Project) -> Result<()> {
        project.refresh_assembly()?;
        project.validate_identity()?;
        self.project = project;
        self.id_high_water = self.project.identity.next_id;
        self.aliases.clear();
        self.past.clear();
        self.future.clear();
        self.coalesce = None;
        self.revision = self.next_revision;
        self.next_revision += 1;
        Ok(())
    }
    fn query(&mut self, query: Query) -> Result<Value> {
        Ok(match query {
            Query::Project { .. } => json!({"name":self.project.scene.name,"format":self.project.format,"revision":self.revision,"assembly":self.project.assembly.is_some(),"surfaces":self.project.scene.surfaces.len(),"sources":self.project.scene.sources.len(),"issues":self.project.issues,"aliases":self.aliases,"can_undo":self.can_undo(),"can_redo":self.can_redo()}),
            Query::Components { .. } => Value::Array(self.project.assembly.as_ref().map_or(Vec::new(), |a| a.components.iter().map(|c| json!({"id":c.id,"name":c.name,"kind":c.kind,"enabled":c.enabled,"pose":c.pose,"sources":c.project.identity.sources.iter().map(|i|format!("{}/{}",c.id,i)).collect::<Vec<_>>(),"surfaces":c.project.identity.surfaces.iter().map(|i|format!("{}/{}",c.id,i)).collect::<Vec<_>>()})).collect())),
            Query::Bindings { .. } => serde_json::to_value(self.project.bindings()?)?,
            Query::Schema { .. } => crate::schema(),
            Query::Materials { target } => {
                let target=self.resolve(target.as_deref().unwrap_or("project"));
                let p=match properties::locate(&self.project,&target)? {
                    Target::Project|Target::Surface(None,_)|Target::Source(None,_)=>&self.project,
                    Target::Component(i)|Target::Surface(Some(i),_)|Target::Source(Some(i),_)=>&self.project.assembly.as_ref().unwrap().components[i].project,
                };
                json!({"target":target,"materials":p.material_names.iter().zip(&p.scene.materials).enumerate().map(|(index,(name,definition))|json!({"index":index,"name":name,"definition":definition})).collect::<Vec<_>>()})
            }
            Query::Properties { target } => properties::describe(&self.project,&self.resolve(&target))?,
            Query::History { .. } => json!({"undo":self.past.iter().map(|s|&s.label).collect::<Vec<_>>(),"redo":self.future.iter().map(|s|&s.label).collect::<Vec<_>>(),"snapshot_bytes_estimate":self.past.iter().map(|s|estimated_bytes(&s.project)).sum::<usize>()}),
            Query::Job { job } => { let id=self.job_aliases.get(&job).unwrap_or(&job); self.jobs.get_mut(id).ok_or_else(|| Error::new("not_found",id))?.summary(self.revision) }
            Query::Report { job } => {
                let id=self.job_aliases.get(&job).unwrap_or(&job); let j=self.jobs.get_mut(id).ok_or_else(|| Error::new("not_found",id))?; j.poll();
                let record=j.record.as_ref().ok_or_else(|| j.error.clone().unwrap_or_else(|| Error::new("not_ready","完了した計算結果がありません")))?;
                json!({"revision":j.revision,"stale":j.revision!=self.revision,"bindings":j.bindings,"report":record.report})
            }
        })
    }
    fn start_trace(&mut self, options: crate::TraceSettings) -> Result<String> {
        let crate::TraceSettings {
            threads,
            rays_per_source: rays,
            preview_limit,
            paths,
            terminals,
            output,
            ..
        } = options;
        if !(1..=64).contains(&threads)
            || rays.is_some_and(|n| !(1..=2_000_000).contains(&n))
            || preview_limit.is_some_and(|n| !(1..=2_000_000).contains(&n))
            || paths > 2_000_000
        {
            return Err(Error::new(
                "invalid_value",
                "スレッド数1〜64、光線数1〜2,000,000で指定してください",
            ));
        }
        let project = self.frozen_project();
        let mut scene = project.trace_scene()?;
        if let Some(n) = rays {
            for s in &mut scene.sources {
                s.count = n;
            }
        }
        if let Some(n) = preview_limit {
            for s in &mut scene.sources {
                s.count = s.count.min(n);
            }
        }
        let total: usize = scene
            .sources
            .iter()
            .map(|s| s.count)
            .try_fold(0usize, usize::checked_add)
            .ok_or_else(|| Error::new("invalid_value", "光線数の合計が大きすぎます"))?;
        if !(1..=2_000_000).contains(&total) {
            return Err(Error::new(
                "invalid_value",
                "光線数は合計1〜2,000,000本で指定してください",
            ));
        }
        let bindings = project.bindings()?;
        let output = output.map(|p| self.path(&p));
        self.prune_jobs()?;
        let control = Arc::new(TraceControl::default());
        let worker_control = control.clone();
        let (tx, rx) = mpsc::channel();
        let id = format!("j{}", self.next_job);
        self.next_job += 1;
        std::thread::spawn(move || {
            let result = (|| -> Result<Option<JobProduct>> {
                if worker_control.cancel.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let prepared = scene.clone().prepare()?;
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .map_err(|e| Error::new("threads", e))?;
                let options = TraceOptions {
                    keep_terminals: terminals,
                    path_launch_count: paths as u64,
                };
                let start = Instant::now();
                let Some(report) = pool
                    .install(|| trace_generated_controlled(&prepared, options, &worker_control))?
                else {
                    return Ok(None);
                };
                if !report.budget.launched.is_finite() || report.budget.launched <= 0. {
                    return Err(Error::new(
                        "invalid_power",
                        "発射基準光量は正の有限値で指定してください",
                    ));
                }
                let seconds = start.elapsed().as_secs_f64();
                let calculation = RunContext::capture(&project, &scene, None, threads, options);
                if let Some(path) = output {
                    calculation.save(&report, seconds, &path)?;
                }
                Ok(Some(JobProduct::Trace(Box::new(RunRecord {
                    format: RUN_FORMAT.into(),
                    calculation,
                    trace_seconds: seconds,
                    report,
                }))))
            })();
            let _ = tx.send(result);
        });
        self.jobs.insert(
            id.clone(),
            Job {
                kind: "trace",
                revision: self.revision,
                control,
                rx,
                record: None,
                study: None,
                error: None,
                finished: false,
                bindings,
            },
        );
        Ok(id)
    }
    fn start_study(
        &mut self,
        kind: &'static str,
        worker: impl FnOnce(&TraceControl) -> Result<Option<Value>> + Send + 'static,
    ) -> Result<String> {
        self.prune_jobs()?;
        let bindings = self.project.bindings()?;
        let control = Arc::new(TraceControl::default());
        let worker_control = control.clone();
        let (tx, rx) = mpsc::channel();
        let id = format!("j{}", self.next_job);
        self.next_job += 1;
        std::thread::spawn(move || {
            let result = worker(&worker_control).map(|v| v.map(JobProduct::Study));
            let _ = tx.send(result);
        });
        self.jobs.insert(
            id.clone(),
            Job {
                kind,
                revision: self.revision,
                control,
                rx,
                record: None,
                study: None,
                error: None,
                finished: false,
                bindings,
            },
        );
        Ok(id)
    }
    fn prune_jobs(&mut self) -> Result<()> {
        for job in self.jobs.values_mut() {
            job.poll();
        }
        if self.jobs.values().filter(|j| !j.finished).count() >= 8 {
            return Err(Error::new("job_limit", "同時に実行できるジョブは8件です"));
        }
        let mut completed = self
            .jobs
            .iter()
            .filter(|(_, j)| j.finished)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        completed.sort_by_key(|id| id[1..].parse::<u64>().unwrap_or(u64::MAX));
        let mut bytes: usize = self
            .jobs
            .values()
            .filter_map(|j| j.record.as_ref())
            .map(|r| {
                estimated_bytes(&r.calculation.project)
                    + r.report.terminals.len() * 256
                    + r.report.paths.len() * 256
                    + r.report
                        .detectors
                        .iter()
                        .map(|d| d.weights.len() * 8)
                        .sum::<usize>()
            })
            .sum();
        let mut retained = completed.len();
        for id in completed {
            if retained < 8 && (bytes <= 128 * 1024 * 1024 || retained <= 1) {
                break;
            }
            if let Some(j) = self.jobs.remove(&id) {
                if let Some(r) = j.record {
                    bytes = bytes.saturating_sub(
                        estimated_bytes(&r.calculation.project)
                            + r.report.terminals.len() * 256
                            + r.report.paths.len() * 256
                            + r.report
                                .detectors
                                .iter()
                                .map(|d| d.weights.len() * 8)
                                .sum::<usize>(),
                    );
                }
                self.job_aliases.retain(|_, v| *v != id);
                retained -= 1;
            }
        }
        Ok(())
    }
}

fn estimated_bytes(p: &Project) -> usize {
    fn scene(s: &raytrace_core::model::Scene) -> usize {
        s.surfaces
            .iter()
            .map(|s| match &s.shape {
                raytrace_core::model::Shape::TriangleMesh {
                    vertices,
                    triangles,
                    normals,
                } => (vertices.len() + normals.len()) * 24 + triangles.len() * 12,
                _ => 1024,
            })
            .sum::<usize>()
            + s.materials.len() * 1024
            + s.sources.len() * 4096
    }
    scene(&p.scene)
        + p.zemax
            .as_ref()
            .map_or(0, |z| z.text.len() + scene(&z.baseline_scene))
        + p.assembly.as_ref().map_or(0, |a| {
            a.components
                .iter()
                .map(|c| estimated_bytes(&c.project))
                .sum()
        })
}
