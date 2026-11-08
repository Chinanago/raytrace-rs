//! The command registry and the way a [`Session`] runs a command.
//!
//! A command is a function `fn(&mut Session, In, &mut Context) -> Result<Out, EngineError>`
//! with typed, schema-carrying input and output. [`registry`] lists them all; the JSON entry
//! points ([`Session::execute`], [`Session::execute_request`]) and the typed entry point
//! ([`Session::call`]) run the same function. The request and response schemas, the CLI help
//! and `command.list` are generated from the registry, so a new command needs only an entry in
//! `commands::all`.
use crate::{
    commands,
    engine::{EngineError, ErrorInfo, LogEntry, Progress, Response},
    session::Session,
};
use raytrace_core::trace::TraceControl;
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, value::RawValue};
use std::{any::Any, marker::PhantomData, sync::OnceLock};

/// What a command may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Reads the document or files and never changes the document. Undo is not involved.
    Query,
    /// Changes the document: undoable, bumps the revision, accepts `expected_revision`, and is
    /// recorded in the session log. A mutation whose input has an optional `group` field
    /// lets consecutive changes of one group share an undo step.
    Mutation,
    /// A long calculation. It runs on the calling thread; cancellation and progress come
    /// through the [`Context`].
    Job,
}

/// The previous short command names and the commands that replaced them. This is the only
/// place the old names are mapped; they are deprecated and kept so existing scripts work.
pub const ALIASES: &[(&str, &str)] = &[
    ("version", "app.version"),
    ("schema", "schema.get"),
    ("open", "doc.open"),
    ("inspect", "doc.inspect"),
    ("trace", "analysis.trace"),
    ("replay", "run.replay"),
    ("scan", "study.scan"),
    ("resume_scan", "study.resume"),
    ("convergence", "study.convergence"),
    ("save_design", "doc.save"),
    ("save_run", "run.save"),
    ("export_zmx", "doc.export_zmx"),
    ("export_screen", "run.export_screen"),
];

/// Cancellation and progress of a running command.
pub struct Context<'a> {
    /// Cancels traces and scans; also carries the ray progress of a single trace.
    pub control: &'a TraceControl,
    progress: Option<&'a mut dyn FnMut(Progress<'_>)>,
}

impl<'a> Context<'a> {
    /// A context that only carries the cancellation control.
    pub fn new(control: &'a TraceControl) -> Self {
        Self {
            control,
            progress: None,
        }
    }
    /// A context whose `progress` callback receives the progress reports.
    pub fn with_progress(
        control: &'a TraceControl,
        progress: &'a mut dyn FnMut(Progress<'_>),
    ) -> Self {
        Self {
            control,
            progress: Some(progress),
        }
    }
    /// Report progress to the caller.
    pub fn progress(&mut self, progress: Progress<'_>) {
        if let Some(callback) = &mut self.progress {
            callback(progress);
        }
    }
}

/// A typed handle of a command: its id with the input and output types.
pub struct Cmd<In, Out> {
    pub id: &'static str,
    marker: PhantomData<fn(In) -> Out>,
}
impl<In, Out> Cmd<In, Out> {
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}
impl<In, Out> Clone for Cmd<In, Out> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<In, Out> Copy for Cmd<In, Out> {}

/// The parameters of one call.
pub(crate) enum Params {
    /// The request fields except `command`.
    Json(Value),
    /// The input type itself, from an in-process caller.
    Typed(Box<dyn Any>),
}

type RunFn = Box<
    dyn Fn(&mut Session, Params, &mut Context<'_>) -> Result<Box<dyn Any>, EngineError>
        + Send
        + Sync,
>;
type FailureFn = Box<dyn Fn(&dyn Any) -> Option<String> + Send + Sync>;

/// One registered command.
pub struct CommandSpec {
    /// `<area>.<action>`, lower case.
    pub id: &'static str,
    /// One line for lists and help.
    pub summary: &'static str,
    pub kind: Kind,
    input_schema: fn() -> schemars::Schema,
    output_schema: fn() -> schemars::Schema,
    run: RunFn,
    to_json: fn(&dyn Any) -> Result<Box<RawValue>, EngineError>,
    failure: Option<FailureFn>,
}

impl CommandSpec {
    /// JSON Schema of the parameters (the request fields except `command`).
    pub fn input_schema(&self) -> schemars::Schema {
        (self.input_schema)()
    }
    /// JSON Schema of `result`.
    pub fn output_schema(&self) -> schemars::Schema {
        (self.output_schema)()
    }
    /// Deprecated short names that run this command.
    pub fn aliases(&self) -> impl Iterator<Item = &'static str> {
        let id = self.id;
        ALIASES
            .iter()
            .filter(move |(_, target)| *target == id)
            .map(|(alias, _)| *alias)
    }
    /// A result that is returned but also reported as an error, such as a scan with failed cases.
    pub(crate) fn with_failure<Out: 'static>(
        mut self,
        failure: fn(&Out) -> Option<String>,
    ) -> Self {
        self.failure = Some(Box::new(move |any| failure(any.downcast_ref::<Out>()?)));
        self
    }
}

fn schema_of<T: JsonSchema>() -> schemars::Schema {
    schemars::schema_for!(T)
}

/// The log entry of a change command: its id and its parameters without `expected_revision`,
/// and the `group` they name.
fn log_entry<In: Serialize>(
    id: &str,
    input: &In,
) -> Result<(LogEntry, Option<String>), EngineError> {
    let mut params = serde_json::to_value(input).map_err(|e| {
        EngineError::new(
            "invalid_json",
            format!("{id} の引数をJSONにできません: {e}"),
        )
    })?;
    let mut group = None;
    if let Value::Object(fields) = &mut params {
        fields.remove("expected_revision");
        group = fields
            .get("group")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    Ok((
        LogEntry {
            command: id.to_string(),
            params,
        },
        group,
    ))
}

fn internal(message: &str) -> EngineError {
    EngineError::new("internal", format!("内部エラー: {message}"))
}

/// Register a typed command. The handle fixes the input and output types of the function.
pub(crate) fn command<In, Out>(
    handle: Cmd<In, Out>,
    summary: &'static str,
    kind: Kind,
    run: fn(&mut Session, In, &mut Context<'_>) -> Result<Out, EngineError>,
) -> CommandSpec
where
    In: Serialize + DeserializeOwned + JsonSchema + 'static,
    Out: Serialize + JsonSchema + 'static,
{
    let id = handle.id;
    CommandSpec {
        id,
        summary,
        kind,
        input_schema: schema_of::<In>,
        output_schema: schema_of::<Out>,
        run: Box::new(move |session, params, context| {
            let input = match params {
                Params::Json(value) => serde_json::from_value::<In>(value).map_err(|e| {
                    EngineError::invalid(format!(
                        "{id} の引数が不正です（command.describe id={id} でスキーマを確認してください）: {e}"
                    ))
                })?,
                Params::Typed(input) => *input
                    .downcast::<In>()
                    .map_err(|_| internal("入力の型がコマンドと一致しません"))?,
            };
            if kind == Kind::Mutation {
                let (entry, group) = log_entry(id, &input)?;
                session.begin_command(entry, group);
            }
            let output = run(session, input, context);
            session.end_command();
            Ok(Box::new(output?) as Box<dyn Any>)
        }),
        to_json: |any| {
            let out = any
                .downcast_ref::<Out>()
                .ok_or_else(|| internal("結果の型がコマンドと一致しません"))?;
            serde_json::value::to_raw_value(out).map_err(|e| {
                EngineError::new("invalid_json", format!("結果をJSONにできません: {e}"))
            })
        },
        failure: None,
    }
}

/// All commands of the engine.
pub struct Registry {
    commands: Vec<CommandSpec>,
}

impl Registry {
    pub fn iter(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.iter()
    }
    /// The command with exactly this id.
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }
    /// The command with this id, or the one a deprecated short name stands for.
    pub fn resolve(&self, name: &str) -> Option<&CommandSpec> {
        self.get(name).or_else(|| {
            ALIASES
                .iter()
                .find(|(alias, _)| *alias == name)
                .and_then(|(_, id)| self.get(id))
        })
    }
    /// Ids that resemble an unknown command name: same area, same action or a substring.
    pub fn similar(&self, name: &str) -> Vec<&'static str> {
        let name = name.to_lowercase();
        let (area, action) = name.split_once('.').unwrap_or((&name, &name));
        self.commands
            .iter()
            .filter(|c| {
                let (a, b) = c.id.split_once('.').unwrap_or((c.id, c.id));
                a == area || b == action || c.id.contains(&name) || name.contains(b)
            })
            .map(|c| c.id)
            .take(8)
            .collect()
    }
}

/// The registry, built on first use.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        commands: commands::all(),
    })
}

fn error_response(command: &str, revision: u64, error: EngineError) -> Response {
    Response::failure(command, revision, error)
}

impl Session {
    /// Run the command `command` (an id or a deprecated short name) with its parameters as
    /// a JSON object, and wrap the outcome in the response envelope.
    pub fn execute(&mut self, command: &str, params: Value, context: &mut Context<'_>) -> Response {
        let Some(spec) = registry().resolve(command) else {
            let similar = registry().similar(command);
            let hint = if similar.is_empty() {
                "command.list で一覧を確認してください".to_string()
            } else {
                format!("似たコマンド: {}", similar.join(", "))
            };
            return error_response(
                command,
                self.revision(),
                EngineError::invalid(format!("不明なコマンドです: {command}（{hint}）")),
            );
        };
        let params = if params.is_null() {
            Value::Object(Map::new())
        } else {
            params
        };
        match (spec.run)(self, Params::Json(params), context) {
            Err(error) => error_response(command, self.revision(), error),
            Ok(output) => {
                let failure = spec.failure.as_ref().and_then(|f| f(&*output));
                match (spec.to_json)(&*output) {
                    Err(error) => error_response(command, self.revision(), error),
                    Ok(raw) => Response::new(
                        command,
                        self.revision(),
                        failure.map(|message| ErrorInfo {
                            code: "cases_failed".into(),
                            message,
                        }),
                        Some(raw),
                        Some(output),
                    ),
                }
            }
        }
    }

    /// Run a request object `{"command": ID, ...parameters}`.
    pub fn execute_request(&mut self, request: Value, context: &mut Context<'_>) -> Response {
        let Value::Object(mut fields) = request else {
            return self.invalid_request("リクエストはJSONオブジェクトで指定してください");
        };
        match fields.remove("command") {
            Some(Value::String(command)) => self.execute(&command, Value::Object(fields), context),
            _ => self.invalid_request("command にコマンドIDを文字列で指定してください"),
        }
    }

    /// Parse the text of a JSON request and run it.
    pub fn execute_text(&mut self, text: &str, context: &mut Context<'_>) -> Response {
        match serde_json::from_str::<Value>(text) {
            Ok(request) => self.execute_request(request, context),
            Err(e) => error_response(
                "unknown",
                self.revision(),
                EngineError::new("invalid_json", format!("リクエストのJSONが不正です: {e}")),
            ),
        }
    }

    fn invalid_request(&self, reason: &str) -> Response {
        error_response(
            "unknown",
            self.revision(),
            EngineError::invalid(format!(
                "リクエストが不正です（schema.get name=request を確認してください）: {reason}"
            )),
        )
    }

    /// Run a command with typed input and output, as the GUI does. No JSON is involved.
    pub fn call<In: 'static, Out: 'static>(
        &mut self,
        handle: Cmd<In, Out>,
        input: In,
        context: &mut Context<'_>,
    ) -> Result<Out, EngineError> {
        let spec = registry()
            .get(handle.id)
            .ok_or_else(|| internal("コマンドが登録されていません"))?;
        let output = (spec.run)(self, Params::Typed(Box::new(input)), context)?;
        output
            .downcast::<Out>()
            .map(|out| *out)
            .map_err(|_| internal("結果の型がコマンドと一致しません"))
    }

    /// [`Session::call`] without cancellation or progress.
    pub fn run<In: 'static, Out: 'static>(
        &mut self,
        handle: Cmd<In, Out>,
        input: In,
    ) -> Result<Out, EngineError> {
        let control = TraceControl::default();
        self.call(handle, input, &mut Context::new(&control))
    }
}

/// Run one command in a new session: the one-shot use of the CLI and of tests.
pub fn call_once<In: 'static, Out: 'static>(
    handle: Cmd<In, Out>,
    input: In,
) -> Result<Out, EngineError> {
    Session::new().run(handle, input)
}

// ---- schemas generated from the registry --------------------------------------------------

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Move the `$defs` of a root schema into `defs`; returns the names that already existed with a
/// different definition.
fn take_defs(schema: &mut Value, defs: &mut Map<String, Value>) -> Vec<String> {
    let mut conflicts = Vec::new();
    if let Some(Value::Object(found)) = schema.as_object_mut().and_then(|o| o.remove("$defs")) {
        for (name, definition) in found {
            match defs.get(&name) {
                Some(existing) if *existing != definition => conflicts.push(name),
                Some(_) => {}
                None => {
                    defs.insert(name, definition);
                }
            }
        }
    }
    conflicts
}

fn command_names(spec: &CommandSpec) -> Value {
    let names: Vec<Value> = std::iter::once(spec.id)
        .chain(spec.aliases())
        .map(Value::from)
        .collect();
    serde_json::json!({ "enum": names })
}

fn alias_note(spec: &CommandSpec) -> String {
    let aliases: Vec<_> = spec.aliases().collect();
    if aliases.is_empty() {
        spec.summary.to_string()
    } else {
        format!("{}（旧名: {}）", spec.summary, aliases.join(", "))
    }
}

/// The request schema and the names whose `$defs` differ between two commands (none expected).
fn build_request_schema() -> (Value, Vec<String>) {
    let mut defs = Map::new();
    let mut conflicts = Vec::new();
    let mut variants = Vec::new();
    for spec in registry().iter() {
        let mut input = spec.input_schema().to_value();
        conflicts.extend(take_defs(&mut input, &mut defs));
        let mut properties = Map::new();
        properties.insert("command".into(), command_names(spec));
        if let Some(Value::Object(fields)) = input.get_mut("properties").map(Value::take) {
            properties.extend(fields);
        }
        let mut required = vec![Value::from("command")];
        if let Some(Value::Array(fields)) = input.get_mut("required").map(Value::take) {
            required.extend(fields);
        }
        variants.push(serde_json::json!({
            "type": "object",
            "title": spec.id,
            "description": alias_note(spec),
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }));
    }
    let schema = serde_json::json!({
        "$schema": DRAFT,
        "title": "Request",
        "description": "One command: `command` selects it, the other fields are its parameters (see command.describe).",
        "oneOf": variants,
        "$defs": defs,
    });
    (schema, conflicts)
}

/// `schema.get name=request`: one entry per command with its parameters.
pub fn request_schema() -> schemars::Schema {
    schemars::Schema::try_from(build_request_schema().0).unwrap_or_default()
}

/// `schema.get name=response`: the envelope, with the result of each command.
pub fn response_schema() -> schemars::Schema {
    let mut defs = Map::new();
    let mut error = schemars::schema_for!(ErrorInfo).to_value();
    take_defs(&mut error, &mut defs);
    if let Some(object) = error.as_object_mut() {
        object.remove("$schema");
    }
    defs.insert("ErrorInfo".into(), error);
    let mut variants = Vec::new();
    for spec in registry().iter() {
        let mut output = spec.output_schema().to_value();
        take_defs(&mut output, &mut defs);
        if let Some(object) = output.as_object_mut() {
            object.remove("$schema");
        }
        variants.push(serde_json::json!({
            "type": "object",
            "title": spec.id,
            "properties": {
                "ok": {"type": "boolean"},
                "command": command_names(spec),
                "revision": {"type": "integer", "minimum": 0, "description": "Revision of the session after the command."},
                "error": {"$ref": "#/$defs/ErrorInfo"},
                "result": output,
            },
            "required": ["ok", "command", "revision"],
        }));
    }
    let schema = serde_json::json!({
        "$schema": DRAFT,
        "title": "Response",
        "description": "One line printed by the CLI per request. `result` follows the command.",
        "oneOf": variants,
        "$defs": defs,
    });
    schemars::Schema::try_from(schema).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_schemas_share_one_definition_per_name() {
        let (_, conflicts) = build_request_schema();
        assert!(conflicts.is_empty(), "{conflicts:?}");
    }

    #[test]
    fn similar_ids_come_from_the_same_area_or_action() {
        let registry = registry();
        assert!(registry.similar("doc.opne").contains(&"doc.open"));
        assert!(
            registry
                .similar("analysis.trac")
                .contains(&"analysis.trace")
        );
        assert!(registry.similar("zzz.zzz").is_empty());
    }
}
