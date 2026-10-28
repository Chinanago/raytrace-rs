use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    /// Reject edits if somebody else has changed this session.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Source,
    Optic,
    System,
}
impl From<Role> for raytrace_io::assembly::ComponentKind {
    fn from(r: Role) -> Self {
        match r {
            Role::Source => Self::Source,
            Role::Optic => Self::Optic,
            Role::System => Self::System,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialInput {
    /// Keep all AGF dispersion and absorption definitions.
    Catalog {
        directory: PathBuf,
        catalog: String,
        name: String,
    },
    /// Reuse a material already present in this project by its exact name.
    Existing { name: String },
    /// An explicitly constant-index material.
    Constant {
        n: f64,
        #[serde(default)]
        absorption_per_mm: f64,
    },
}

/// The property selects its value's Rust type and JSON schema.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "property",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PropertyChange {
    Name(String),
    Enabled(bool),
    /// Component/surface position in its parent's coordinates; mm.
    PositionMm([f64; 3]),
    /// Component/surface rotation in the existing raytrace Pose convention; degrees.
    RotationDeg([f64; 3]),
    RodSizeMm([f64; 3]),
    DetectorSizeMm([f64; 2]),
    Pixels([usize; 2]),
    Rays(usize),
    Seed(u64),
    WavelengthUm(f64),
    TotalWeight(f64),
    DiskRadiusMm(f64),
    ConeHalfAngleDeg(f64),
    Direction([f64; 3]),
    Sampling(SamplingMode),
    MonochromaticOverrideUm(Option<f64>),
    AllowUncoated(bool),
    Spectrum(Option<Vec<[f64; 2]>>),
    DiodeDivergenceDeg([f64; 2]),
    DiodeArray([usize; 2]),
    DiodeSpacingMm([f64; 2]),
    DiodeHalfWidthMm([f64; 2]),
    DiodeGaussianWidthMm([f64; 2]),
    DiodeAngularSupergaussian([f64; 2]),
    DiodeSpatialSupergaussian([f64; 2]),
    DiodeAstigmatismMm(f64),
    DiodeRotationDeg([f64; 3]),
    RadiusMm(f64),
    Conic(f64),
    SurfaceMedia([usize; 2]),
    MaxDepth(usize),
    MaxEventsPerLaunch(usize),
    MinRelativeBranchWeight(f64),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SamplingMode {
    Sobol,
    Pseudorandom,
}
impl From<SamplingMode> for raytrace_core::model::Sampling {
    fn from(s: SamplingMode) -> Self {
        match s {
            SamplingMode::Sobol => Self::Sobol,
            SamplingMode::Pseudorandom => Self::Pseudorandom,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    NewAssembly {
        name: String,
    },
    Open {
        path: PathBuf,
        #[serde(default)]
        catalogs: Vec<PathBuf>,
        #[serde(default)]
        cad_deflection_mm: Option<f64>,
    },
    AddComponent {
        path: PathBuf,
        kind: Role,
        #[serde(default)]
        alias: Option<String>,
        #[serde(default)]
        catalogs: Vec<PathBuf>,
        #[serde(default)]
        cad_deflection_mm: Option<f64>,
    },
    AddRod {
        material: MaterialInput,
        size_mm: [f64; 3],
        #[serde(default)]
        alias: Option<String>,
    },
    AddDetector {
        size_mm: [f64; 2],
        pixels: [usize; 2],
        #[serde(default)]
        alias: Option<String>,
    },
    DuplicateComponent {
        target: String,
        #[serde(default)]
        alias: Option<String>,
    },
    RemoveComponent {
        target: String,
    },
    SetProperty {
        target: String,
        change: PropertyChange,
    },
    SetRodMaterial {
        target: String,
        material: MaterialInput,
    },
    /// Every member must be an edit. Commit all members as one undo unit.
    Transaction {
        label: String,
        edits: Vec<Command>,
    },
    Undo {},
    Redo {},
    Save {
        path: PathBuf,
    },
    ExportZmx {
        path: PathBuf,
    },
    Trace {
        #[serde(flatten)]
        options: TraceSettings,
    },
    Scan {
        plan: PathBuf,
        output: PathBuf,
        #[serde(default)]
        alias: Option<String>,
    },
    Convergence {
        plan: PathBuf,
        output: PathBuf,
        #[serde(default)]
        alias: Option<String>,
    },
    Wait {
        job: String,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    Cancel {
        job: String,
    },
    Query {
        query: Query,
    },
}
pub fn default_threads() -> usize {
    4
}
pub fn default_timeout() -> u64 {
    60_000
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TraceSettings {
    pub alias: Option<String>,
    pub threads: usize,
    pub rays_per_source: Option<usize>,
    /// Cap each source count while retaining smaller original source counts.
    pub preview_limit: Option<usize>,
    pub paths: usize,
    pub terminals: bool,
    pub output: Option<PathBuf>,
}
impl Default for TraceSettings {
    fn default() -> Self {
        Self {
            alias: None,
            threads: default_threads(),
            rays_per_source: None,
            preview_limit: None,
            paths: 0,
            terminals: false,
            output: None,
        }
    }
}

impl Command {
    pub fn needs_preparation(&self) -> bool {
        matches!(
            self,
            Self::Open { .. }
                | Self::AddComponent { .. }
                | Self::AddRod {
                    material: MaterialInput::Catalog { .. },
                    ..
                }
                | Self::SetRodMaterial {
                    material: MaterialInput::Catalog { .. },
                    ..
                }
        ) || matches!(self,Self::Transaction { edits,.. } if edits.iter().any(Self::needs_preparation))
    }
    pub fn is_edit(&self) -> bool {
        matches!(
            self,
            Self::AddComponent { .. }
                | Self::AddRod { .. }
                | Self::AddDetector { .. }
                | Self::DuplicateComponent { .. }
                | Self::RemoveComponent { .. }
                | Self::SetProperty { .. }
                | Self::SetRodMaterial { .. }
                | Self::Transaction { .. }
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    Project {},
    Components {},
    Bindings {},
    Schema {},
    History {},
    Materials {
        #[serde(default)]
        target: Option<String>,
    },
    Properties {
        target: String,
    },
    Job {
        job: String,
    },
    /// Potentially large. Use job summaries for routine progress checks.
    Report {
        job: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub revision: u64,
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<String>,
    pub data: serde_json::Value,
}
