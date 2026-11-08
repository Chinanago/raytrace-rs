//! JSON request/response entry point shared by the CLI and the GUI.
//!
//! Every calculation and file operation is one [`Request`]. The CLI reads requests as
//! JSON (`raytrace-rs run REQUEST.json`) or builds them from its options; the GUI builds
//! the same values in memory. [`execute`] runs a request and [`respond`] wraps the outcome
//! in the [`Response`] envelope printed by the CLI. `raytrace-rs --schema request` and
//! `--schema response` describe both formats.
mod run;

use crate::{convergence, scan};
use raytrace_core::{
    model::{Mode, Ray, Sampling, Scene},
    screen::ScreenRoi,
    trace::TraceControl,
};
use raytrace_io::{
    compatibility::CompatibilityReport,
    export::DisplaySettings,
    project::Project,
    record::{RUN_FORMAT, RunRecord},
    zemax,
};
pub use run::{Benchmark, RunOutput, SurfacePercent};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf};

/// One operation. `command` selects the operation; the other fields depend on it.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    /// Report the software version.
    Version,
    /// Return one JSON Schema.
    Schema { name: SchemaName },
    /// Load a project, ZMX, calculation record or scan directory and return it.
    Open(OpenRequest),
    /// Load a model, report its compatibility and optionally save it as project JSON.
    Inspect(InspectRequest),
    /// Trace a model and return (and optionally save) the calculation record.
    Trace(TraceRequest),
    /// Rerun a calculation record with its recorded optical conditions.
    Replay(ReplayRequest),
    /// Run a parameter scan into a new directory.
    Scan(ScanRequest),
    /// Continue the unfinished cases of a scan directory.
    ResumeScan(ResumeScanRequest),
    /// Trace the model with varied ray counts, seeds, pixels and CAD deflections.
    Convergence(ConvergenceRequest),
    /// Save a project as JSON.
    SaveProject(SaveProjectRequest),
    /// Save a calculation record as JSON.
    SaveRun(SaveRunRequest),
    /// Export a standalone model as ZMX.
    ExportZmx(ExportZmxRequest),
    /// Write CSV/PNG/metadata files for one screen of a calculation record.
    ExportScreen(ExportScreenRequest),
}

impl Request {
    /// The `command` value of this request.
    pub fn command(&self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Schema { .. } => "schema",
            Self::Open(_) => "open",
            Self::Inspect(_) => "inspect",
            Self::Trace(_) => "trace",
            Self::Replay(_) => "replay",
            Self::Scan(_) => "scan",
            Self::ResumeScan(_) => "resume_scan",
            Self::Convergence(_) => "convergence",
            Self::SaveProject(_) => "save_project",
            Self::SaveRun(_) => "save_run",
            Self::ExportZmx(_) => "export_zmx",
            Self::ExportScreen(_) => "export_screen",
        }
    }
}

/// An optical model given by file path or inline JSON. Exactly one key.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelInput {
    /// `.zmx`, project JSON or scene JSON file.
    Path(PathBuf),
    /// Inline project JSON (the `--schema project` format).
    Project(Box<Project>),
    /// Inline scene JSON (the `--schema scene` format, as in `examples/*.json`).
    Scene(Box<Scene>),
}

/// A calculation record given by file path or inline JSON. Exactly one key.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordInput {
    Path(PathBuf),
    Record(Box<RunRecord>),
}

/// Explicit launch rays instead of the model sources. Exactly one key.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RaysInput {
    /// CSV with the header `id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium`.
    Csv(PathBuf),
    /// Inline rays; ids must be unique.
    Rays(Vec<Ray>),
}

/// Changes applied to the loaded model before tracing. Omitted fields keep the model.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    pub mode: Option<Mode>,
    /// Replace every source spectrum with one wavelength in micrometres.
    pub wavelength_um: Option<f64>,
    /// Rays per source.
    pub rays_per_source: Option<usize>,
    /// Upper limit of rays per source (applied after `rays_per_source`).
    pub max_rays_per_source: Option<usize>,
    pub sampling: Option<Sampling>,
    /// Trace coated surfaces as uncoated Fresnel interfaces (changes the physics).
    pub ignore_coatings: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenRequest {
    /// File, or a scan directory containing `scan.json`.
    pub path: PathBuf,
    /// Extra AGF catalog folders for ZMX import.
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    /// For calculation records: retrace up to 64 sample rays to draw light paths.
    #[serde(default)]
    pub viewer_paths: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    pub model: ModelInput,
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    /// Save the imported project JSON here.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Also export the model as ZMX (standalone models only).
    #[serde(default)]
    pub export_zmx: Option<PathBuf>,
    #[serde(default)]
    pub ignore_coatings: bool,
    #[serde(default)]
    pub wavelength_um: Option<f64>,
    /// Return the imported project in the result.
    #[serde(default)]
    pub include_project: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceRequest {
    pub model: ModelInput,
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    /// Explicit launch rays; omitted uses the model sources.
    #[serde(default)]
    pub rays: Option<RaysInput>,
    #[serde(default)]
    pub overrides: Overrides,
    /// Worker threads; omitted uses the runtime default.
    #[serde(default)]
    pub threads: Option<usize>,
    /// Timed repetitions; `benchmark` reports their median.
    #[serde(default = "one")]
    pub repeat: usize,
    /// Run one untimed trace before the timed repetitions.
    #[serde(default)]
    pub warmup: bool,
    /// Save terminal rays in the report.
    #[serde(default)]
    pub terminals: bool,
    /// Save every path segment of the first N launch rays in input order.
    #[serde(default)]
    pub paths: u64,
    /// Replace `report.paths` with up to N sample rays spread over all sources (GUI preview).
    #[serde(default)]
    pub preview_paths: usize,
    /// Reject generated runs with more launch rays than this.
    #[serde(default)]
    pub max_total_rays: Option<usize>,
    /// Also export the effective model as ZMX before tracing.
    #[serde(default)]
    pub export_zmx: Option<PathBuf>,
    /// Save the calculation record (the `--schema run` format) here.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Return the full calculation record in the result.
    #[serde(default)]
    pub include_record: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    pub record: RecordInput,
    /// Worker threads; omitted uses the recorded count.
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default = "one")]
    pub repeat: usize,
    #[serde(default)]
    pub warmup: bool,
    /// Override the recorded terminal-ray setting.
    #[serde(default)]
    pub terminals: Option<bool>,
    /// Override the recorded path count.
    #[serde(default)]
    pub paths: Option<u64>,
    #[serde(default)]
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub include_record: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub model: ModelInput,
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    pub plan: scan::ScanPlan,
    /// New directory for `scan.json` and the case records.
    pub output: PathBuf,
    /// Return the scan manifest in the result.
    #[serde(default)]
    pub include_manifest: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResumeScanRequest {
    pub directory: PathBuf,
    #[serde(default)]
    pub include_manifest: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConvergenceRequest {
    /// Model file; CAD deflection comparisons need the original STEP-referencing ZMX.
    pub model: PathBuf,
    #[serde(default)]
    pub plan: convergence::ConvergencePlan,
    /// Directory for `convergence.json` and the case records.
    pub output: PathBuf,
    #[serde(default)]
    pub include_report: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveProjectRequest {
    pub project: Box<Project>,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveRunRequest {
    pub record: Box<RunRecord>,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportZmxRequest {
    pub model: ModelInput,
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportScreenRequest {
    pub record: RecordInput,
    /// Zero-based index into `report.detectors`.
    #[serde(default)]
    pub detector: usize,
    /// Output directory.
    pub output: PathBuf,
    #[serde(default = "default_display")]
    pub display: DisplaySettings,
    /// Local-mm position of the extra X/Y profiles.
    #[serde(default)]
    pub cursor_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub roi: Option<ScreenRoi>,
}

/// Names accepted by the `schema` command and `raytrace-rs --schema NAME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaName {
    /// Request accepted by `raytrace-rs run`.
    Request,
    /// Response printed by the CLI.
    Response,
    /// Optical model, as in `examples/*.json`.
    Scene,
    /// Project JSON saved by the GUI and `inspect`.
    Project,
    /// Calculation record saved by `trace` and `replay`.
    Run,
    ScanPlan,
    /// `scan.json` in a scan directory.
    Scan,
    ConvergencePlan,
    /// `convergence.json` in a convergence directory.
    Convergence,
}

impl SchemaName {
    pub const ALL: [Self; 9] = [
        Self::Request,
        Self::Response,
        Self::Scene,
        Self::Project,
        Self::Run,
        Self::ScanPlan,
        Self::Scan,
        Self::ConvergencePlan,
        Self::Convergence,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Scene => "scene",
            Self::Project => "project",
            Self::Run => "run",
            Self::ScanPlan => "scan_plan",
            Self::Scan => "scan",
            Self::ConvergencePlan => "convergence_plan",
            Self::Convergence => "convergence",
        }
    }
    /// Accepts `scan_plan` and `scan-plan`.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.replace('-', "_");
        Self::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

fn one() -> usize {
    1
}
fn default_display() -> DisplaySettings {
    DisplaySettings {
        peak_normalized: false,
        square_root: false,
        maximum_percent: None,
    }
}

/// CLI output envelope: `ok`, the request `command`, then `result` and/or `error`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Response {
    pub ok: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Outcome>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct ErrorInfo {
    /// Stable snake_case identifier, e.g. `io`, `invalid_json`, `invalid_request`,
    /// `invalid_scene`, `zmx`, `plan`, `cases_failed`, `cancelled`.
    pub code: String,
    /// Japanese message for people.
    pub message: String,
}

/// Result of one request; the shape follows the request `command`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
    Version {
        version: String,
    },
    Schema {
        schema: schemars::Schema,
    },
    Opened(Document),
    Inspected(InspectResult),
    Traced(TraceResult),
    Scanned(ScanResult),
    Converged(ConvergenceResult),
    /// `save_project`, `save_run`, `export_zmx` and `export_screen`.
    Saved {
        output: PathBuf,
    },
}

/// A document returned by `open`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Document {
    Project { project: Box<Project> },
    Run { record: Box<RunRecord> },
    Scan { manifest: Box<scan::ScanManifest> },
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct InspectResult {
    pub output: Option<PathBuf>,
    pub exported_zmx: Option<PathBuf>,
    pub sources: usize,
    pub surfaces: usize,
    pub materials: usize,
    /// False when an enabled component has a `requires_override` or `unsupported` entry.
    pub traceable: bool,
    pub compatibility: CompatibilityReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<Box<Project>>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TraceResult {
    /// Saved calculation record, if `output` was given.
    pub output: Option<PathBuf>,
    pub exported_zmx: Option<PathBuf>,
    pub reference_plane: String,
    pub launched_rays: usize,
    pub detected_percent_of_reference: f64,
    pub budget_percent_of_reference: BTreeMap<String, f64>,
    pub detector_percent_of_reference: BTreeMap<String, f64>,
    pub relative_budget_closure_error: f64,
    pub median_seconds: f64,
    pub worker_threads: usize,
    /// Unresolved weight and other conditions worth checking in the record.
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<Box<RunOutput>>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ScanResult {
    /// The saved `scan.json`.
    pub output: PathBuf,
    pub directory: PathBuf,
    pub cases: usize,
    pub completed: usize,
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Box<scan::ScanManifest>>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ConvergenceResult {
    /// The saved `convergence.json`.
    pub output: PathBuf,
    pub cases: usize,
    pub comparisons: usize,
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Box<convergence::ConvergenceReport>>,
}

/// Failure of a request, with a stable code and a Japanese message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError {
    pub code: &'static str,
    pub message: String,
}
impl EngineError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn cancelled() -> Self {
        Self::new("cancelled", "計算を中止しました")
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
    fn io(path: &Path, error: std::io::Error) -> Self {
        raytrace_io::Error::io(path, error).into()
    }
}
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for EngineError {}
macro_rules! coded_error {
    ($($t:ty),*) => {$(
        impl From<$t> for EngineError {
            fn from(error: $t) -> Self {
                Self::new(error.code(), error.to_string())
            }
        }
    )*};
}
coded_error!(crate::Error, raytrace_io::Error, raytrace_core::Error);

/// Progress reported while a request runs. Ray progress of single traces is in
/// [`TraceControl::completed`] and [`TraceControl::total`].
#[non_exhaustive]
pub enum Progress<'a> {
    /// The scan manifest after each case changes state.
    Scan(&'a scan::ScanManifest),
}

/// Run one request without cancellation or progress callbacks.
pub fn execute(request: Request) -> Result<Outcome, EngineError> {
    execute_with(request, &TraceControl::default(), &mut |_| {})
}

/// Parse a JSON request, run it and wrap the result in the response envelope.
pub fn execute_json(text: &str) -> Response {
    match serde_json::from_str::<Request>(text) {
        Ok(request) => {
            let command = request.command();
            respond(command, execute(request))
        }
        Err(error) => respond(
            "unknown",
            Err(EngineError::invalid(format!(
                "リクエストJSONが不正です（--schema request を確認してください）: {error}"
            ))),
        ),
    }
}

/// Wrap an outcome in the response envelope. Scans and convergence studies with failed
/// cases keep their result and also report `cases_failed`.
pub fn respond(command: &str, result: Result<Outcome, EngineError>) -> Response {
    let (result, error) = match result {
        Ok(outcome) => {
            let failed = match &outcome {
                Outcome::Scanned(s) if s.failed > 0 => Some(format!(
                    "走査で{}ケースが失敗しました。{}を確認してください",
                    s.failed,
                    s.output.display()
                )),
                Outcome::Converged(c) if c.failed > 0 => Some(format!(
                    "収束確認で{}ケースが失敗しました。{}を確認してください",
                    c.failed,
                    c.output.display()
                )),
                _ => None,
            };
            let error = failed.map(|message| ErrorInfo {
                code: "cases_failed".into(),
                message,
            });
            (Some(outcome), error)
        }
        Err(error) => (
            None,
            Some(ErrorInfo {
                code: error.code.into(),
                message: error.message,
            }),
        ),
    };
    Response {
        ok: error.is_none(),
        command: command.into(),
        error,
        result,
    }
}

/// Run one request. `control` cancels traces and scans; `progress` receives scan updates.
pub fn execute_with(
    request: Request,
    control: &TraceControl,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Outcome, EngineError> {
    match request {
        Request::Version => Ok(Outcome::Version {
            version: env!("CARGO_PKG_VERSION").into(),
        }),
        Request::Schema { name } => Ok(Outcome::Schema {
            schema: schema(name),
        }),
        Request::Open(r) => open(r).map(Outcome::Opened),
        Request::Inspect(r) => inspect(r).map(Outcome::Inspected),
        Request::Trace(r) => run::trace(r, control).map(Outcome::Traced),
        Request::Replay(r) => run::replay(r, control).map(Outcome::Traced),
        Request::Scan(r) => {
            if r.output.join("scan.json").exists() {
                return Err(EngineError::invalid(
                    "走査の出力先は既に存在します。resume_scanまたは新しいディレクトリを指定してください",
                ));
            }
            let project = load(r.model, &r.catalogs)?;
            let manifest = scan::ScanManifest::new(&project, r.plan)?;
            run_scan(manifest, r.output, r.include_manifest, control, progress)
        }
        Request::ResumeScan(r) => {
            let manifest = scan::ScanManifest::open(&r.directory)?;
            run_scan(manifest, r.directory, r.include_manifest, control, progress)
        }
        Request::Convergence(r) => {
            let report = convergence::run(&r.model, &r.plan, &r.output)?;
            Ok(Outcome::Converged(ConvergenceResult {
                output: r.output.join("convergence.json"),
                cases: report.cases.len(),
                comparisons: report.comparisons.len(),
                failed: report.cases.iter().filter(|c| c.error.is_some()).count(),
                report: r.include_report.then(|| Box::new(report)),
            }))
        }
        Request::SaveProject(r) => {
            create_parent(&r.output)?;
            r.project.save(&r.output)?;
            Ok(Outcome::Saved { output: r.output })
        }
        Request::SaveRun(r) => {
            r.record
                .calculation
                .save(&r.record.report, r.record.trace_seconds, &r.output)?;
            Ok(Outcome::Saved { output: r.output })
        }
        Request::ExportZmx(r) => {
            let project = load(r.model, &r.catalogs)?;
            zemax::export(&project, &r.output)?;
            Ok(Outcome::Saved { output: r.output })
        }
        Request::ExportScreen(r) => export_screen(r).map(|output| Outcome::Saved { output }),
    }
}

/// Load a model input as a project.
pub(crate) fn load(model: ModelInput, catalogs: &[PathBuf]) -> Result<Project, EngineError> {
    Ok(match model {
        ModelInput::Path(path) => Project::open(&path, catalogs)?,
        ModelInput::Project(project) => {
            let mut project = *project;
            project.refresh_assembly()?;
            project
        }
        ModelInput::Scene(scene) => Project::new(*scene),
    })
}

fn load_record(record: RecordInput) -> Result<(RunRecord, Option<PathBuf>), EngineError> {
    match record {
        RecordInput::Path(path) => Ok((RunRecord::open(&path)?, Some(path))),
        RecordInput::Record(record) => {
            if record.format != RUN_FORMAT {
                return Err(raytrace_io::Error::Record("未対応の計算記録形式です".into()).into());
            }
            record.calculation.scene.clone().prepare()?;
            Ok((*record, None))
        }
    }
}

fn create_parent(path: &Path) -> Result<(), EngineError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| EngineError::io(parent, e))?;
    }
    Ok(())
}

fn open(r: OpenRequest) -> Result<Document, EngineError> {
    if r.path.is_dir() {
        return Ok(Document::Scan {
            manifest: Box::new(scan::ScanManifest::open(&r.path)?),
        });
    }
    match Project::open(&r.path, &r.catalogs) {
        Ok(project) => Ok(Document::Project {
            project: Box::new(project),
        }),
        // Project::open recognises calculation records and refers them to replay.
        Err(raytrace_io::Error::Project(message)) if message.starts_with("計算記録は") => {
            let mut record = RunRecord::open(&r.path)?;
            if r.viewer_paths {
                record.report.paths = run::viewer_paths(&record)?;
            }
            Ok(Document::Run {
                record: Box::new(record),
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn inspect(r: InspectRequest) -> Result<InspectResult, EngineError> {
    let mut project = load(r.model, &r.catalogs)?;
    project.allow_uncoated |= r.ignore_coatings;
    if let Some(w) = r.wavelength_um {
        project.monochromatic_override_um = Some(w);
    }
    if let Some(path) = &r.export_zmx {
        zemax::export(&project, path)?;
    }
    if let Some(path) = &r.output {
        create_parent(path)?;
        project.save(path)?;
    }
    let compatibility = project.compatibility_report();
    Ok(InspectResult {
        output: r.output,
        exported_zmx: r.export_zmx,
        sources: project.scene.sources.len(),
        surfaces: project.scene.surfaces.len(),
        materials: project.scene.materials.len(),
        traceable: compatibility.traceable,
        compatibility,
        project: r.include_project.then(|| Box::new(project)),
    })
}

fn run_scan(
    mut manifest: scan::ScanManifest,
    directory: PathBuf,
    include_manifest: bool,
    control: &TraceControl,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Outcome, EngineError> {
    manifest.execute(&directory, control, |m| progress(Progress::Scan(m)))?;
    let count = |state: &str| manifest.cases.iter().filter(|c| c.state == state).count();
    Ok(Outcome::Scanned(ScanResult {
        output: directory.join("scan.json"),
        cases: manifest.cases.len(),
        completed: count("completed"),
        failed: count("failed"),
        directory,
        manifest: include_manifest.then(|| Box::new(manifest)),
    }))
}

fn export_screen(r: ExportScreenRequest) -> Result<PathBuf, EngineError> {
    let (record, _) = load_record(r.record)?;
    let map = record
        .report
        .detectors
        .get(r.detector)
        .ok_or_else(|| EngineError::invalid("出力するスクリーンがありません"))?;
    let reference = record.report.budget.launched;
    let calculation = &record.calculation;
    let scene = &calculation.scene;
    fs::create_dir_all(&r.output).map_err(|e| EngineError::io(&r.output, e))?;
    raytrace_io::export::write_screen(
        &r.output,
        map,
        &scene.reference_plane,
        reference,
        r.display,
    )?;
    raytrace_io::export::write_postprocessing(&r.output, map, reference, r.cursor_mm, r.roi)?;
    let conditions = serde_json::json!({
        "calculation_software_version": calculation.software_version, "model": scene.name,
        "wavelengths_um": calculation.launch_wavelengths_um, "input_rays": calculation.input_count(),
        "sources": scene.sources, "pupils": calculation.pupils, "trace_settings": scene.settings,
        "display": r.display,
    });
    let path = r.output.join("conditions.json");
    let bytes = serde_json::to_vec_pretty(&conditions)
        .map_err(|e| raytrace_io::Error::json(Some(path.clone()), e))?;
    fs::write(&path, bytes).map_err(|e| EngineError::io(&path, e))?;
    Ok(r.output)
}

fn schema(name: SchemaName) -> schemars::Schema {
    use schemars::schema_for;
    match name {
        SchemaName::Request => schema_for!(Request),
        SchemaName::Response => schema_for!(Response),
        SchemaName::Scene => schema_for!(Scene),
        SchemaName::Project => schema_for!(Project),
        SchemaName::Run => schema_for!(RunOutput),
        SchemaName::ScanPlan => schema_for!(scan::ScanPlan),
        SchemaName::Scan => schema_for!(scan::ScanManifest),
        SchemaName::ConvergencePlan => schema_for!(convergence::ConvergencePlan),
        SchemaName::Convergence => schema_for!(convergence::ConvergenceReport),
    }
}
