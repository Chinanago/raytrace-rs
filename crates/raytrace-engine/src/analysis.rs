//! The analysis registry: post-processing of a calculation record, one command each.
//!
//! An analysis is a type that implements [`Analysis`]: an id, a one-line summary, typed
//! settings, a typed result and a function `run(&RunView, settings) -> result`. Registering it
//! in [`specs`] turns it into the query command `analysis.<id>`, so it appears in
//! `command.list`, `command.describe`, the CLI, the MCP tools and the generated request and
//! response schemas without any other change (docs/DESIGN.md §9).
//!
//! The registry only adapts. The numerical work stays in pure functions of the lower layers
//! (`raytrace_core::screen`, [`crate::pipeline`]); an analysis calls them on the record and
//! shapes the result.
//!
//! Every analysis command takes an optional `run` (a calculation record, as a file path or
//! inline) next to its own settings. Without it the command uses the latest record of the
//! session (`analysis.trace`, `run.replay` or `run.open`), or fails with `no_run`. Every
//! result carries `run_revision` and `stale`: the revision of the session document the record
//! was computed from, and whether the document has changed since.
//!
//! To add an analysis: write a module here with the settings and result types and an
//! `impl Analysis`, give it a [`Cmd`] handle for typed callers, and list it in [`specs`].
mod budget;
mod screen;

pub use budget::{
    ANALYSIS_BUDGET, Budget, BudgetEntry, BudgetOutput, BudgetSettings, DetectorReceived,
};
pub use screen::{ANALYSIS_SCREEN, DetectorRef, Screen, ScreenOutput, ScreenSettings, ScreenSlice};

use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    engine::{EngineError, RecordInput, load_shared_record},
    pipeline,
    session::Session,
};
use raytrace_core::{model::Scene, trace::Report};
use raytrace_io::record::{RunContext, RunRecord};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::{borrow::Cow, sync::Arc};

/// One analysis of a calculation record.
pub trait Analysis: 'static {
    /// The id in lower case. The command is `analysis.<ID>`.
    const ID: &'static str;
    /// One line for lists and help.
    const SUMMARY: &'static str;
    /// The parameters of the analysis besides `run`. Use `deny_unknown_fields`.
    type Settings: Serialize + DeserializeOwned + JsonSchema + 'static;
    /// The result, an object. `run_revision` and `stale` are added next to its fields.
    type Output: Serialize + JsonSchema + 'static;
    /// Analyse the record. A pure function of the record and the settings.
    fn run(view: &RunView<'_>, settings: Self::Settings) -> Result<Self::Output, EngineError>;
}

/// Read access to the calculation record an analysis works on.
pub struct RunView<'a> {
    record: &'a RunRecord,
    reference_weight: f64,
    revision: Option<u64>,
    stale: bool,
}

impl<'a> RunView<'a> {
    /// A view of `record`. Fails when the launched weight, which every percentage divides by,
    /// is not a positive finite number.
    pub fn new(
        record: &'a RunRecord,
        revision: Option<u64>,
        stale: bool,
    ) -> Result<Self, EngineError> {
        let reference_weight = pipeline::launched_power(&record.report).ok_or_else(|| {
            EngineError::new(
                "invalid_ray",
                "発射光線の重み合計は正の有限値にしてください",
            )
        })?;
        Ok(Self {
            record,
            reference_weight,
            revision,
            stale,
        })
    }
    pub fn record(&self) -> &RunRecord {
        self.record
    }
    /// The trace report: budget, detector pixel weights, surface totals and diagnostics.
    pub fn report(&self) -> &Report {
        &self.record.report
    }
    /// The effective scene that was traced (after overrides and preview limits).
    pub fn scene(&self) -> &Scene {
        &self.record.calculation.scene
    }
    /// The frozen design, scene, launch input and software version of the calculation.
    pub fn calculation(&self) -> &RunContext {
        &self.record.calculation
    }
    /// The launched weight; every `*_percent_of_reference` divides by this value.
    pub fn reference_weight(&self) -> f64 {
        self.reference_weight
    }
    /// The revision of the session document the record was computed from, if it was.
    pub fn revision(&self) -> Option<u64> {
        self.revision
    }
    /// The session document has changed since the record was computed.
    pub fn stale(&self) -> bool {
        self.stale
    }
}

/// The parameters of an `analysis.*` command: the optional `run` and the settings of the
/// analysis, flat in one object.
#[derive(Clone, Debug, Default)]
pub struct AnalysisRequest<S> {
    /// The calculation record to analyse; omitted uses the latest record of the session.
    pub run: Option<RecordInput>,
    pub settings: S,
}

impl<S> AnalysisRequest<S> {
    /// The settings on the latest record of the session.
    pub fn new(settings: S) -> Self {
        Self {
            run: None,
            settings,
        }
    }
    /// The settings on the given record.
    pub fn on(run: RecordInput, settings: S) -> Self {
        Self {
            run: Some(run),
            settings,
        }
    }
}

impl<S: Serialize> Serialize for AnalysisRequest<S> {
    fn serialize<T: serde::Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        #[derive(Serialize)]
        struct Wire<'a, S> {
            #[serde(skip_serializing_if = "Option::is_none")]
            run: &'a Option<RecordInput>,
            #[serde(flatten)]
            settings: &'a S,
        }
        Wire {
            run: &self.run,
            settings: &self.settings,
        }
        .serialize(serializer)
    }
}

impl<'de, S: DeserializeOwned> Deserialize<'de> for AnalysisRequest<S> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let run = match fields.remove("run") {
            None | Some(Value::Null) => None,
            Some(run) => Some(serde_json::from_value(run).map_err(Error::custom)?),
        };
        // The settings reject unknown fields themselves.
        let settings = serde_json::from_value(Value::Object(fields)).map_err(Error::custom)?;
        Ok(Self { run, settings })
    }
}

impl<S: JsonSchema> JsonSchema for AnalysisRequest<S> {
    fn schema_name() -> Cow<'static, str> {
        format!("AnalysisRequest_for_{}", S::schema_name()).into()
    }
    fn schema_id() -> Cow<'static, str> {
        format!("{}::AnalysisRequest<{}>", module_path!(), S::schema_id()).into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut run = generator.subschema_for::<Option<RecordInput>>();
        run.ensure_object().insert(
            "description".into(),
            "計算記録（ファイルのパスかインライン）。省略するとセッションの直近の計算結果を使う"
                .into(),
        );
        let mut schema = S::json_schema(generator);
        let properties = schema
            .ensure_object()
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(properties) = properties.as_object_mut() {
            properties.insert("run".into(), run.to_value());
        }
        schema
    }
}

/// The result of an `analysis.*` command: the fields of the analysis, then the record it
/// used.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AnalysisResponse<O> {
    /// The revision of the session document the record was computed from. Absent when the
    /// record does not come from the session document (a record given as `run`, opened from
    /// a file, replayed, or computed from a `model` given to the command).
    pub run_revision: Option<u64>,
    /// The session document has changed since the record was computed, so the numbers
    /// describe an earlier version of the design.
    pub stale: bool,
    #[serde(flatten)]
    pub result: O,
}

/// The error of an analysis command without a record to work on.
pub fn no_run() -> EngineError {
    EngineError::new(
        "no_run",
        "解析する計算結果がありません。先に analysis.trace を実行するか、run.open で計算記録を開くか、run に計算記録を指定してください",
    )
}

fn execute<A: Analysis>(
    session: &mut Session,
    request: AnalysisRequest<A::Settings>,
    _: &mut Context<'_>,
) -> Result<AnalysisResponse<A::Output>, EngineError> {
    let AnalysisRequest { run, settings } = request;
    let (record, revision) = match run {
        // A record given to the command is not the one the session computed.
        Some(input) => (load_shared_record(input)?.0, None),
        None => {
            let latest = session.latest_run().ok_or_else(no_run)?;
            (Arc::clone(&latest.record), latest.revision)
        }
    };
    let stale = revision.is_some_and(|revision| revision != session.revision());
    let view = RunView::new(&record, revision, stale)?;
    let result = A::run(&view, settings)?;
    Ok(AnalysisResponse {
        run_revision: revision,
        stale,
        result,
    })
}

/// The command of an analysis. The id `analysis.<ID>` is built once, when the registry is.
fn register<A: Analysis>() -> CommandSpec {
    let id: &'static str = Box::leak(format!("analysis.{}", A::ID).into_boxed_str());
    command(
        Cmd::<AnalysisRequest<A::Settings>, AnalysisResponse<A::Output>>::new(id),
        A::SUMMARY,
        Kind::Query,
        execute::<A>,
    )
}

/// Every analysis, in the order of `command.list`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![register::<Budget>(), register::<Screen>()]
}
