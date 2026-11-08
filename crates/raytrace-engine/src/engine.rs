//! Shared types of the commands: inputs, results, errors and the response envelope.
//!
//! Every calculation and file operation is a command of the registry ([`crate::command`])
//! run by a [`Session`](crate::Session). The input types here are the typed parameters of
//! those commands: the CLI builds the same values from `FIELD=text` arguments (or
//! `{"command": ID, ...}` JSON) and the GUI builds them in memory. `schema.get name=request`
//! describes all commands, and `command.describe` one of them.
pub(crate) mod run;

use crate::{convergence, scan};
use raytrace_core::{
    model::{Material, Mode, Ray, Sampling, Scene},
    screen::ScreenRoi,
};
use raytrace_doc::PartKind;
use raytrace_io::{
    compatibility::CompatibilityReport,
    design::{self, Design},
    export::DisplaySettings,
    record::{RUN_FORMAT, RunRecord},
};
pub use run::{Benchmark, RunOutput, SurfacePercent};
use serde::{Deserialize, Serialize};
use std::{any::Any, collections::BTreeMap, fs, path::Path, path::PathBuf, sync::Arc};

/// An optical model given by file path or inline JSON. Exactly one key.
///
/// Commands that work on a document accept it as the optional `model`. Without it they use
/// the document of the session. With it they work on that model only and leave the session
/// unchanged (one-shot use).
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelInput {
    /// `.zmx`, design JSON, legacy project JSON or scene JSON file.
    Path(PathBuf),
    /// Inline design JSON (the `schema.get name=design` format).
    Design(Box<Design>),
    /// Inline scene JSON (the `schema.get name=scene` format, as in `examples/*.json`).
    Scene(Box<Scene>),
}

/// A calculation record given by file path or inline JSON. Exactly one key.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordInput {
    Path(PathBuf),
    Record(Box<RunRecord>),
    /// A record that the caller already holds (in-process callers only; never in JSON).
    /// It is used as it is, without copying or validating it again.
    #[serde(skip)]
    #[schemars(skip)]
    Shared(Arc<RunRecord>),
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

/// Parameters of a command that takes none.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

// ---- app, schema, command -------------------------------------------------------------

/// `app.version`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct VersionResult {
    pub version: String,
}

/// `name` alone returns that schema; with `output` it is written to that file.
/// `output` alone writes every schema as `<name>.schema.json` into that directory.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaRequest {
    #[serde(default)]
    pub name: Option<SchemaName>,
    #[serde(default)]
    pub output: Option<PathBuf>,
}

/// The schema itself, or the file or directory it was written to.
#[derive(Debug, Default, Serialize, schemars::JsonSchema)]
pub struct SchemaResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<schemars::Schema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
}

/// Names accepted by `schema.get`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaName {
    /// Request accepted by `raytrace-rs run`: one entry per command.
    Request,
    /// Response printed by the CLI: the result of each command.
    Response,
    /// Optical model, as in `examples/*.json`.
    Scene,
    /// Design JSON saved by the GUI and `doc.save`.
    Design,
    /// Calculation record saved by `analysis.trace` and `run.replay`.
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
        Self::Design,
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
            Self::Design => "design",
            Self::Run => "run",
            Self::ScanPlan => "scan_plan",
            Self::Scan => "scan",
            Self::ConvergencePlan => "convergence_plan",
            Self::Convergence => "convergence",
        }
    }
}

/// `command.describe`.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeRequest {
    /// Command id such as `analysis.trace` (a deprecated alias such as `trace` also works).
    pub id: String,
}

/// One entry of `command.list`.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct CommandSummary {
    pub id: String,
    pub summary: String,
    pub kind: crate::command::Kind,
}

/// `command.list`: every registered command and the deprecated short names.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CommandList {
    pub commands: Vec<CommandSummary>,
    /// Deprecated short names of the previous interface and the command each one runs.
    pub aliases: BTreeMap<String, String>,
}

/// `command.describe`: a command with its input and output JSON Schemas.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CommandDescription {
    pub id: String,
    pub summary: String,
    pub kind: crate::command::Kind,
    /// Deprecated short names that run this command.
    pub aliases: Vec<String>,
    /// Schema of the parameters (the request fields except `command`).
    pub input_schema: schemars::Schema,
    /// Schema of `result`.
    pub output_schema: schemars::Schema,
}

// ---- documents --------------------------------------------------------------------------

/// What a document looks like, without its content.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct DocSummary {
    pub name: String,
    pub parts: usize,
    pub occurrences: usize,
    /// False for a single unplaced system, which is traced verbatim.
    pub assembly: bool,
}

impl DocSummary {
    pub fn of(design: &Design) -> Self {
        Self {
            name: design.display_name().to_string(),
            parts: design.parts.len(),
            occurrences: design.occurrences.len(),
            assembly: design.is_assembly(),
        }
    }
}

/// `doc.new`: start a new document, an empty assembly or the given design, with a new history.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DocNewRequest {
    /// Start from this design instead of an empty assembly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design: Option<Box<Design>>,
    /// Document name; omitted keeps the default (or the name of `design`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Extra AGF catalog folders for later ZMX imports in this session.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<PathBuf>,
    /// Reject the command unless the session is at this revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// `doc.open`: open a ZMX, design JSON, legacy project JSON or scene JSON as the session document.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocOpenRequest {
    pub path: PathBuf,
    /// Extra AGF catalog folders for ZMX import; they stay with the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<PathBuf>,
    /// Also return the opened design in the result.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_design: bool,
    /// Reject the command unless the session is at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// Result of `doc.open`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DocOpened {
    pub path: PathBuf,
    #[serde(flatten)]
    pub summary: DocSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design: Option<Box<Design>>,
}

/// `doc.get`: a JSON Pointer into the design JSON (the whole design by default).
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DocGetRequest {
    /// JSON Pointer such as `/occurrences/@rod-4/pose`; elements of arrays can be given by
    /// index or as `@<id>`.
    pub path: String,
    /// Read this model instead of the session document.
    pub model: Option<ModelInput>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DocGetResult {
    pub path: String,
    pub value: serde_json::Value,
}

/// `doc.replace`: replace the whole document (undoable).
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocReplaceRequest {
    pub design: Box<Design>,
    /// Edits with the same `group` in a row form one undo step (see [`DocSetRequest::group`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Reject the command unless the session is at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// `doc.set`: replace one place of the document, found by JSON Pointer (undoable).
///
/// The edited design must read back as a design and compile; otherwise nothing changes.
/// `id`, `format` and `next_id` cannot be changed, and parts and occurrences cannot be added or
/// removed this way (use the `component.*` commands).
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocSetRequest {
    /// JSON Pointer such as `/occurrences/@rod-4/pose/translation_mm/2`. An array element is
    /// addressed by index or as `@<id>`, matching the `id` field of the element.
    pub path: String,
    /// The new JSON value.
    pub value: serde_json::Value,
    /// Name of an editing gesture. A change whose `group` equals that of the change right
    /// before it (with no undo, redo or open in between) joins the same undo step, so a drag
    /// or the typing in one field can be undone at once. Omitted: a step of its own. Every
    /// change still raises the revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Reject the command unless the session is at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DocSetResult {
    /// The pointer as applied, with `@<id>` replaced by indices.
    pub path: String,
    #[serde(flatten)]
    pub summary: DocSummary,
}

/// `doc.save`: write a design as JSON.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocSaveRequest {
    /// Design to save; omitted saves the session document.
    #[serde(default)]
    pub design: Option<Box<Design>>,
    /// Destination; omitted overwrites the file the session document was opened from or
    /// saved to.
    #[serde(default)]
    pub output: Option<PathBuf>,
}

/// `doc.inspect`: report the compatibility of a model.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    /// Omitted inspects the session document.
    #[serde(default)]
    pub model: Option<ModelInput>,
    /// Extra AGF catalog folders for ZMX import.
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    /// Save the inspected design JSON here.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Also export the model as ZMX (standalone models only).
    #[serde(default)]
    pub export_zmx: Option<PathBuf>,
    #[serde(default)]
    pub ignore_coatings: bool,
    #[serde(default)]
    pub wavelength_um: Option<f64>,
    /// Return the inspected design in the result.
    #[serde(default)]
    pub include_design: bool,
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
    pub design: Option<Box<Design>>,
}

/// `doc.export_zmx`.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportZmxRequest {
    /// Omitted exports the session document.
    #[serde(default)]
    pub model: Option<ModelInput>,
    /// Extra AGF catalog folders for ZMX import.
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    pub output: PathBuf,
}

/// Result of commands that write one file or directory.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Saved {
    pub output: PathBuf,
}

// ---- components -----------------------------------------------------------------------

/// `component.add`: import a file as a part, or add a generated rod or screen (undoable).
///
/// With `path`, the file is imported as a `source`, `optic` (default) or `system` part. Without
/// it, `kind` must be `rod` or `detector`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ComponentAddRequest {
    /// ZMX, design JSON or scene JSON to import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// `source`, `optic` or `system` for a file; `rod` or `detector` without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<PartKind>,
    /// Extra AGF catalog folders for ZMX import (default: those of the session).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<PathBuf>,
    /// Name of the new placement; omitted uses the name of the part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Material of a rod; omitted uses fused silica.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Material>,
    /// Display name of the material of a rod.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_name: Option<String>,
    /// Edits with the same `group` in a row form one undo step (see [`DocSetRequest::group`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Reject the command unless the session is at this revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ComponentAdded {
    pub part: String,
    pub occurrence: String,
    #[serde(flatten)]
    pub summary: DocSummary,
}

/// `component.remove` and `component.duplicate`: one occurrence by id.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentRef {
    /// Occurrence id such as `rod-4`.
    pub occurrence: String,
    /// Edits with the same `group` in a row form one undo step (see [`DocSetRequest::group`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Reject the command unless the session is at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ComponentChanged {
    /// The occurrence that was removed, or the new copy.
    pub occurrence: String,
    #[serde(flatten)]
    pub summary: DocSummary,
}

// ---- session ----------------------------------------------------------------------------

/// `session.undo` and `session.redo`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryRequest {
    /// Reject the command unless the session is at this revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// `session.status`: the state of the session without the document.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionStatus {
    pub revision: u64,
    pub has_document: bool,
    /// File the document was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// The document has changes that are not saved.
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_depth: usize,
    pub redo_depth: usize,
    /// Estimated memory of the undo history in bytes. The session keeps at most 64 undo
    /// steps and about 128 MiB, dropping the oldest steps first.
    pub undo_bytes: usize,
    pub catalogs: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<DocSummary>,
    /// The latest calculation record, which the `analysis.*` commands use when `run` is
    /// omitted. Absent until `analysis.trace`, `run.replay` or `run.open` has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<RunStatus>,
}

/// The latest calculation record of a session (`session.status`).
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct RunStatus {
    /// Revision of the document the record was computed from. Absent for a record that was
    /// opened from a file, replayed, or computed from a `model` given to the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// The document has changed since the record was computed.
    pub stale: bool,
    /// Number of screens (detectors) in the record.
    pub detectors: usize,
}

/// One committed change of the session: the command and the parameters it ran with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LogEntry {
    /// Command id such as `doc.set`.
    pub command: String,
    /// The parameters as executed, without `expected_revision`.
    pub params: serde_json::Value,
}

/// `session.log`: the changes that lead from the opened document to the current one.
///
/// Running `base` and then every entry in a new session gives the same document. Undone
/// changes are not listed; a change after an undo replaces the undone ones. Consecutive
/// `doc.set` of one `group` on one `path` are kept as the last one.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionLog {
    /// Revision of the session.
    pub revision: u64,
    /// The `doc.open` or `doc.new` that started the document; absent when the session
    /// started with `doc.replace`.
    pub base: Option<LogEntry>,
    /// The committed changes after `base`, oldest first.
    pub entries: Vec<LogEntry>,
}

// ---- calculations -----------------------------------------------------------------------

/// `analysis.trace`: trace a model and return (and optionally save) the calculation record.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceRequest {
    /// Omitted traces the session document.
    #[serde(default)]
    pub model: Option<ModelInput>,
    /// Extra AGF catalog folders for ZMX import.
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
    /// Save the calculation record (the `schema.get name=run` format) here.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Return the full calculation record in the result.
    #[serde(default)]
    pub include_record: bool,
}

/// `run.replay`: rerun a calculation record with its recorded optical conditions.
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

/// `run.open`: load a calculation record.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunOpenRequest {
    pub path: PathBuf,
    /// Retrace up to 64 sample rays to draw light paths.
    #[serde(default)]
    pub viewer_paths: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RunOpened {
    pub record: Box<RunRecord>,
}

/// `run.save`.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveRunRequest {
    pub record: Box<RunRecord>,
    pub output: PathBuf,
}

/// `run.export_screen`: write CSV/PNG/metadata files for one screen of a calculation record.
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

// ---- studies ----------------------------------------------------------------------------

/// `study.scan`: run a parameter scan into a new directory.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    /// Omitted scans the session document.
    #[serde(default)]
    pub model: Option<ModelInput>,
    /// Extra AGF catalog folders for ZMX import.
    #[serde(default)]
    pub catalogs: Vec<PathBuf>,
    pub plan: scan::ScanPlan,
    /// New directory for `scan.json` and the case records.
    pub output: PathBuf,
    /// Return the scan manifest in the result.
    #[serde(default)]
    pub include_manifest: bool,
}

/// `study.resume`: continue the unfinished cases of a scan directory.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResumeScanRequest {
    pub directory: PathBuf,
    #[serde(default)]
    pub include_manifest: bool,
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

/// `study.convergence`: trace the model with varied ray counts, seeds, pixels and CAD deflections.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConvergenceRequest {
    /// Model file (`model.path`); CAD deflection comparisons need the original
    /// STEP-referencing ZMX, because each case re-reads it. Omitted uses the file of the
    /// session document, which must have no unsaved changes.
    #[serde(default)]
    pub model: Option<ModelInput>,
    #[serde(default)]
    pub plan: convergence::ConvergencePlan,
    /// Directory for `convergence.json` and the case records.
    pub output: PathBuf,
    #[serde(default)]
    pub include_report: bool,
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

/// `study.open`: load the manifest of a scan directory.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StudyOpenRequest {
    /// Directory containing `scan.json`.
    pub path: PathBuf,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StudyOpened {
    pub manifest: Box<scan::ScanManifest>,
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

// ---- envelope, errors, progress ---------------------------------------------------------

/// CLI output envelope: `ok`, the request `command`, the session `revision`, then `error`
/// and/or `result`.
#[derive(Serialize)]
pub struct Response {
    pub ok: bool,
    /// The command as written in the request (a deprecated alias is kept as given).
    pub command: String,
    /// Revision of the session after the command.
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Box<serde_json::value::RawValue>>,
    /// The typed result, for in-process callers such as the CLI summary.
    #[serde(skip)]
    typed: Option<Box<dyn Any>>,
}

impl Response {
    pub(crate) fn new(
        command: &str,
        revision: u64,
        error: Option<ErrorInfo>,
        result: Option<Box<serde_json::value::RawValue>>,
        typed: Option<Box<dyn Any>>,
    ) -> Self {
        Self {
            ok: error.is_none(),
            command: command.into(),
            revision,
            error,
            result,
            typed,
        }
    }
    /// A failed command: no result, the error code and message.
    pub fn failure(command: &str, revision: u64, error: EngineError) -> Self {
        Self::new(
            command,
            revision,
            Some(ErrorInfo {
                code: error.code.into(),
                message: error.message,
            }),
            None,
            None,
        )
    }
    /// The result as its Rust type, e.g. [`TraceResult`] for `analysis.trace`.
    pub fn typed<T: 'static>(&self) -> Option<&T> {
        self.typed.as_deref()?.downcast_ref()
    }
    /// The result as a JSON value (`null` when there is none).
    pub fn result_value(&self) -> serde_json::Value {
        self.result
            .as_ref()
            .and_then(|raw| serde_json::from_str(raw.get()).ok())
            .unwrap_or(serde_json::Value::Null)
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("ok", &self.ok)
            .field("command", &self.command)
            .field("revision", &self.revision)
            .field("error", &self.error)
            .field("result", &self.result)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct ErrorInfo {
    /// Stable snake_case identifier, e.g. `io`, `invalid_json`, `invalid_request`,
    /// `invalid_scene`, `design`, `zmx`, `plan`, `cases_failed`, `cancelled`, `revision`,
    /// `no_document`.
    pub code: String,
    /// Japanese message for people.
    pub message: String,
}

/// Failure of a command, with a stable code and a Japanese message.
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
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
    /// The command needs a document and the session has none.
    pub fn no_document() -> Self {
        Self::new(
            "no_document",
            "文書が開かれていません。doc.open か doc.new で文書を開くか、model を指定してください",
        )
    }
    pub(crate) fn io(path: &Path, error: std::io::Error) -> Self {
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
coded_error!(
    crate::Error,
    raytrace_io::Error,
    raytrace_doc::Error,
    raytrace_core::Error
);

/// Progress reported while a command runs. Ray progress of single traces is in
/// [`TraceControl::completed`](raytrace_core::trace::TraceControl) and `total`.
#[non_exhaustive]
pub enum Progress<'a> {
    /// The scan manifest after each case changes state.
    Scan(&'a scan::ScanManifest),
}

// ---- helpers shared by the commands -----------------------------------------------------

/// Load a model input as a design.
pub(crate) fn load(model: ModelInput, catalogs: &[PathBuf]) -> Result<Design, EngineError> {
    Ok(match model {
        ModelInput::Path(path) => design::open(&path, catalogs)?,
        ModelInput::Design(design) => {
            design::check(&design)?;
            *design
        }
        ModelInput::Scene(scene) => Design::from_scene(*scene),
    })
}

pub(crate) fn load_record(
    record: RecordInput,
) -> Result<(RunRecord, Option<PathBuf>), EngineError> {
    let (record, path) = load_shared_record(record)?;
    Ok((Arc::unwrap_or_clone(record), path))
}

/// [`load_record`] that keeps a record shared: a `Shared` input is not copied.
pub(crate) fn load_shared_record(
    record: RecordInput,
) -> Result<(Arc<RunRecord>, Option<PathBuf>), EngineError> {
    match record {
        RecordInput::Path(path) => Ok((Arc::new(RunRecord::open(&path)?), Some(path))),
        RecordInput::Record(record) => {
            if record.format != RUN_FORMAT {
                return Err(raytrace_io::Error::Record("未対応の計算記録形式です".into()).into());
            }
            record.calculation.scene.clone().prepare()?;
            Ok((Arc::new(*record), None))
        }
        RecordInput::Shared(record) => Ok((record, None)),
    }
}

pub(crate) fn create_parent(path: &Path) -> Result<(), EngineError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| EngineError::io(parent, e))?;
    }
    Ok(())
}
