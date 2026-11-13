//! The commands of the engine, one function each, and the registry that lists them.
//!
//! Ids follow `<area>.<action>` (docs/DESIGN.md §7). To add a command: write the input and
//! output types in `engine.rs` (`Deserialize` + `JsonSchema` with `deny_unknown_fields`, and
//! `Serialize` + `JsonSchema`), write the function here, give it a `Cmd` handle and add it to
//! [`all`]. The CLI, `command.list`, `command.describe` and the schemas pick it up.
mod analysis;
mod component;
mod doc;
mod history;
mod study;

use crate::{
    command::{
        Cmd, CommandSpec, Context, Kind, command, registry, request_schema, response_schema,
    },
    convergence,
    engine::{
        CommandDescription, CommandList, CommandSummary, DescribeRequest, Empty, EngineError,
        ModelInput, SchemaName, SchemaRequest, SchemaResult, VersionResult, create_parent, load,
    },
    scan,
    session::Session,
};
use raytrace_core::model::Scene;
use raytrace_io::design::Design;
use std::{fs, path::PathBuf, sync::Arc};

pub use crate::analysis::{ANALYSIS_BUDGET, ANALYSIS_SCREEN};
pub use analysis::{ANALYSIS_TRACE, RUN_EXPORT_SCREEN, RUN_OPEN, RUN_REPLAY, RUN_SAVE};
pub use component::{COMPONENT_ADD, COMPONENT_DUPLICATE, COMPONENT_REMOVE};
pub use doc::{
    DOC_EXPORT_ZMX, DOC_GET, DOC_INSPECT, DOC_NEW, DOC_OPEN, DOC_REPLACE, DOC_SAVE, DOC_SET,
};
pub use history::{SESSION_LOG, SESSION_REDO, SESSION_STATUS, SESSION_UNDO};
pub use study::{STUDY_CONVERGENCE, STUDY_OPEN, STUDY_RESUME, STUDY_SCAN};

pub const APP_VERSION: Cmd<Empty, VersionResult> = Cmd::new("app.version");
pub const SCHEMA_GET: Cmd<SchemaRequest, SchemaResult> = Cmd::new("schema.get");
pub const COMMAND_LIST: Cmd<Empty, CommandList> = Cmd::new("command.list");
pub const COMMAND_DESCRIBE: Cmd<DescribeRequest, CommandDescription> = Cmd::new("command.describe");

/// Every command, in the order of `command.list`.
pub(crate) fn all() -> Vec<CommandSpec> {
    let mut commands = vec![
        command(
            APP_VERSION,
            "ソフトウェアの版を返す",
            Kind::Query,
            app_version,
        ),
        command(
            COMMAND_LIST,
            "コマンドの一覧を返す",
            Kind::Query,
            command_list,
        ),
        command(
            COMMAND_DESCRIBE,
            "コマンドの入力・出力スキーマを返す",
            Kind::Query,
            command_describe,
        ),
        command(
            SCHEMA_GET,
            "JSONスキーマを返す。output を指定するとファイルに書き出す",
            Kind::Query,
            schema_get,
        ),
    ];
    commands.extend(doc::specs());
    commands.extend(component::specs());
    commands.extend(analysis::specs());
    commands.extend(crate::analysis::specs());
    commands.extend(study::specs());
    commands.extend(history::specs());
    commands
}

/// The model of a command: its own `model` (one-shot, the session stays as it is), or the
/// document of the session.
pub(crate) struct Model {
    pub design: Arc<Design>,
    /// The file the design was read from.
    pub path: Option<PathBuf>,
}

pub(crate) fn resolve_model(
    session: &Session,
    model: Option<ModelInput>,
    catalogs: &[PathBuf],
) -> Result<Model, EngineError> {
    match model {
        None => Ok(Model {
            design: session.require_design()?.clone(),
            path: session.path().map(PathBuf::from),
        }),
        Some(model) => {
            let path = match &model {
                ModelInput::Path(path) => Some(path.clone()),
                _ => None,
            };
            let catalogs = if catalogs.is_empty() {
                session.catalogs()
            } else {
                catalogs
            };
            Ok(Model {
                design: Arc::new(load(model, catalogs)?),
                path,
            })
        }
    }
}

fn app_version(
    _: &mut Session,
    _: Empty,
    _: &mut Context<'_>,
) -> Result<VersionResult, EngineError> {
    Ok(VersionResult {
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

fn command_list(
    _: &mut Session,
    _: Empty,
    _: &mut Context<'_>,
) -> Result<CommandList, EngineError> {
    Ok(CommandList {
        commands: registry()
            .iter()
            .map(|c| CommandSummary {
                id: c.id.into(),
                summary: c.summary.into(),
                kind: c.kind,
            })
            .collect(),
    })
}

fn command_describe(
    _: &mut Session,
    r: DescribeRequest,
    _: &mut Context<'_>,
) -> Result<CommandDescription, EngineError> {
    let Some(spec) = registry().get(&r.id) else {
        let similar = registry().similar(&r.id);
        return Err(EngineError::invalid(if similar.is_empty() {
            format!(
                "不明なコマンドです: {}（command.list で一覧を確認してください）",
                r.id
            )
        } else {
            format!(
                "不明なコマンドです: {}（似たコマンド: {}）",
                r.id,
                similar.join(", ")
            )
        }));
    };
    Ok(CommandDescription {
        id: spec.id.into(),
        summary: spec.summary.into(),
        kind: spec.kind,
        input_schema: spec.input_schema(),
        output_schema: spec.output_schema(),
    })
}

fn schema_get(
    _: &mut Session,
    r: SchemaRequest,
    _: &mut Context<'_>,
) -> Result<SchemaResult, EngineError> {
    let render = |name: SchemaName| -> Result<String, EngineError> {
        serde_json::to_string_pretty(&schema(name))
            .map(|text| text + "\n")
            .map_err(|e| {
                EngineError::new("invalid_json", format!("スキーマをJSONにできません: {e}"))
            })
    };
    match (r.name, r.output) {
        (Some(name), None) => Ok(SchemaResult {
            schema: Some(schema(name)),
            output: None,
        }),
        (Some(name), Some(output)) => {
            create_parent(&output)?;
            fs::write(&output, render(name)?).map_err(|e| EngineError::io(&output, e))?;
            Ok(SchemaResult {
                schema: None,
                output: Some(output),
            })
        }
        (None, Some(output)) => {
            fs::create_dir_all(&output).map_err(|e| EngineError::io(&output, e))?;
            for name in SchemaName::ALL {
                let path = output.join(format!("{}.schema.json", name.as_str()));
                fs::write(&path, render(name)?).map_err(|e| EngineError::io(&path, e))?;
            }
            Ok(SchemaResult {
                schema: None,
                output: Some(output),
            })
        }
        (None, None) => Err(EngineError::invalid(format!(
            "name か output を指定してください。name: {}",
            SchemaName::ALL.map(SchemaName::as_str).join(", ")
        ))),
    }
}

fn schema(name: SchemaName) -> schemars::Schema {
    use schemars::schema_for;
    match name {
        SchemaName::Request => request_schema(),
        SchemaName::Response => response_schema(),
        SchemaName::Scene => schema_for!(Scene),
        SchemaName::Design => schema_for!(Design),
        SchemaName::Run => schema_for!(crate::engine::RunOutput),
        SchemaName::ScanPlan => schema_for!(scan::ScanPlan),
        SchemaName::Scan => schema_for!(scan::ScanManifest),
        SchemaName::ConvergencePlan => schema_for!(convergence::ConvergencePlan),
        SchemaName::Convergence => schema_for!(convergence::ConvergenceReport),
        SchemaName::Connection => schema_for!(crate::remote::ConnectionInfo),
    }
}
