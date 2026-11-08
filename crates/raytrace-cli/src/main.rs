//! Command-line front end of the JSON engine.
//!
//! The CLI has no options of its own. Every argument sets one field of an engine request,
//! so the request schema (`raytrace-rs schema name=request`) is the complete interface and
//! `raytrace-rs COMMAND --help` is generated from it:
//!
//! ```text
//! raytrace-rs COMMAND [FIELD=TEXT | FIELD:=JSON | FIELD:=@FILE.json]...
//! raytrace-rs run REQUEST.json|-
//! ```
//!
//! stdout carries one JSON response per request; people read stderr.
use raytrace_core::trace::TraceControl;
use raytrace_engine::engine::{
    self, EngineError, Outcome, Progress, Request, SchemaName, SchemaRequest,
};
use serde_json::{Map, Value};
use std::{
    env, fs,
    io::{Read, Write},
};

fn usage(message: impl Into<String>) -> EngineError {
    EngineError::new("usage", message)
}

/// Build `{"command": COMMAND, ...}` from `a.b=text`, `a.b:=json` and `a.b:=@file.json`.
fn build(command: &str, fields: &[String]) -> Result<Value, EngineError> {
    let mut root = Map::new();
    root.insert("command".into(), Value::String(command.into()));
    for field in fields {
        let (key, value) = field.split_once('=').ok_or_else(|| {
            usage(format!(
                "引数 {field} は FIELD=文字列、FIELD:=JSON、FIELD:=@FILE.json の形で指定してください（raytrace-rs {command} --help）"
            ))
        })?;
        let (key, value) = match key.strip_suffix(':') {
            Some(key) => (key, parse_json(value)?),
            None => (key, Value::String(value.into())),
        };
        let path: Vec<&str> = key.split('.').collect();
        if path.iter().any(|p| p.is_empty()) || key == "command" {
            return Err(usage(format!("フィールド名が不正です: {key}")));
        }
        insert(&mut root, &path, value).map_err(|()| usage(format!("{key} が重複しています")))?;
    }
    Ok(Value::Object(root))
}

fn parse_json(text: &str) -> Result<Value, EngineError> {
    match text.strip_prefix('@') {
        Some(path) => {
            let bytes = fs::read(path).map_err(|e| raytrace_io::Error::io(path, e))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| raytrace_io::Error::json(Some(path.into()), e).into())
        }
        None => serde_json::from_str(text)
            .map_err(|e| usage(format!("JSONの値が不正です（{text}）: {e}"))),
    }
}

fn insert(object: &mut Map<String, Value>, path: &[&str], value: Value) -> Result<(), ()> {
    match path {
        [last] => match object.insert((*last).into(), value) {
            None => Ok(()),
            Some(_) => Err(()),
        },
        [first, rest @ ..] => match object
            .entry(*first)
            .or_insert_with(|| Value::Object(Map::new()))
        {
            Value::Object(inner) => insert(inner, rest, value),
            _ => Err(()),
        },
        [] => Err(()),
    }
}

/// Japanese lines for stderr.
fn describe(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Version { version } => vec![format!("raytrace-rs {version}")],
        Outcome::Schema { .. } => Vec::new(),
        Outcome::Opened(_) => vec!["読込完了".into()],
        Outcome::Inspected(r) => {
            let mut lines = vec![format!(
                "読込結果: 光源{}個、面{}個、材料{}個",
                r.sources, r.surfaces, r.materials
            )];
            for component in &r.compatibility.components {
                lines.push(format!(
                    "{}{}",
                    component.name,
                    if component.enabled { "" } else { " [無効]" }
                ));
                for entry in &component.entries {
                    lines.push(format!("  {}: {}", entry.status.label(), entry.message));
                }
            }
            lines
        }
        Outcome::Traced(r) => {
            let mut lines = vec![
                format!("基準: {} = 100%", r.reference_plane),
                format!(
                    "発射光線数: {}; 検出光量: {:.6}%",
                    r.launched_rays, r.detected_percent_of_reference
                ),
                format!(
                    "追跡時間の中央値: {:.3} ms; {:.3}百万光線/秒; {}スレッド",
                    r.median_seconds * 1000.0,
                    r.launched_rays as f64 / r.median_seconds / 1e6,
                    r.worker_threads
                ),
                format!(
                    "光量収支の誤差: {:.3e}; 計算記録: {}",
                    r.relative_budget_closure_error,
                    r.output.as_ref().map_or_else(
                        || "保存なし（output で指定）".into(),
                        |p| p.display().to_string()
                    )
                ),
            ];
            lines.extend(r.warnings.iter().cloned());
            lines
        }
        Outcome::Scanned(r) => vec![format!(
            "走査を保存しました: {}（{}ケース、完了{}、失敗{}）",
            r.output.display(),
            r.cases,
            r.completed,
            r.failed
        )],
        Outcome::Converged(r) => vec![format!(
            "収束確認: {}ケース、{}比較、{}失敗; {}",
            r.cases,
            r.comparisons,
            r.failed,
            r.output.display()
        )],
        Outcome::Saved { output } => vec![format!("保存しました: {}", output.display())],
    }
}

fn print(stdout: &mut impl Write, response: &engine::Response) {
    let line = serde_json::to_string(response).expect("responses serialize");
    let _ = writeln!(stdout, "{line}");
}

fn fail(stdout: &mut impl Write, command: &str, error: EngineError) -> bool {
    eprintln!("エラー: {error}");
    print(stdout, &engine::respond(command, Err(error)));
    false
}

/// Validate one request value, run it and print its response line. Returns success.
fn perform(value: Value, stdout: &mut impl Write) -> bool {
    let request = match serde_json::from_value::<Request>(value) {
        Ok(request) => request,
        Err(e) => {
            return fail(
                stdout,
                "unknown",
                EngineError::new(
                    "invalid_request",
                    format!("リクエストが不正です（schema name=request を確認してください）: {e}"),
                ),
            );
        }
    };
    let command = request.command();
    let mut progress = |p: Progress<'_>| {
        if let Progress::Scan(m) = p {
            let done = m
                .cases
                .iter()
                .filter(|c| c.state == "completed" || c.state == "failed")
                .count();
            eprintln!("走査: {done}/{}ケース", m.cases.len());
        }
    };
    let result = engine::execute_with(request, &TraceControl::default(), &mut progress);
    if let Ok(outcome) = &result {
        for line in describe(outcome) {
            eprintln!("{line}");
        }
    }
    let response = engine::respond(command, result);
    if let Some(error) = &response.error {
        eprintln!("エラー: {}", error.message);
    }
    print(stdout, &response);
    response.ok
}

/// `run FILE|-`: a stream of JSON requests, one response line each.
fn run_requests(source: Option<&String>, stdout: &mut impl Write) -> bool {
    let text = match source.map(String::as_str) {
        None => {
            return fail(
                stdout,
                "run",
                usage("リクエストJSONのファイルか - を指定してください"),
            );
        }
        Some("-") => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                return fail(stdout, "run", raytrace_io::Error::io("-", e).into());
            }
            text
        }
        Some(path) => match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => return fail(stdout, "run", raytrace_io::Error::io(path, e).into()),
        },
    };
    let mut ok = true;
    let mut any = false;
    for value in serde_json::Deserializer::from_str(&text).into_iter::<Value>() {
        any = true;
        ok &= match value {
            Ok(value) => perform(value, stdout),
            Err(e) => fail(
                stdout,
                "unknown",
                EngineError::new("invalid_json", format!("リクエストのJSONが不正です: {e}")),
            ),
        };
    }
    if !any {
        return fail(stdout, "run", usage("リクエストJSONが空です"));
    }
    ok
}

/// The request schema as JSON, for generated help.
fn request_schema() -> Value {
    let request = Request::Schema(SchemaRequest {
        name: Some(SchemaName::Request),
        output: None,
    });
    match engine::execute(request) {
        Ok(Outcome::Schema { schema }) => schema.to_value(),
        _ => unreachable!("the request schema is built in"),
    }
}

/// `(command, description, variant schema)` for every request command.
fn commands(schema: &Value) -> Vec<(String, String, Value)> {
    schema["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            let name = variant["properties"]["command"]["const"].as_str()?;
            let text = variant["description"].as_str().unwrap_or_default();
            Some((name.to_string(), text.to_string(), variant.clone()))
        })
        .collect()
}

/// Follow `$ref` and `anyOf: [X, null]` to the schema that describes a value.
fn resolve<'a>(schema: &'a Value, value: &'a Value) -> &'a Value {
    if let Some(name) = value["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/$defs/"))
    {
        return resolve(schema, &schema["$defs"][name]);
    }
    if let Some([only]) = value["anyOf"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter(|o| o["type"] != "null")
                .collect::<Vec<_>>()
        })
        .as_deref()
    {
        return resolve(schema, only);
    }
    value
}

/// Short type label: `string`, `integer`, `sequential|non_sequential`, `[string]`, `Design`.
fn label(schema: &Value, value: &Value) -> String {
    let target = resolve(schema, value);
    let consts: Vec<&str> = target["oneOf"]
        .as_array()
        .or_else(|| target["enum"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v["const"].as_str().or_else(|| v.as_str()))
        .collect();
    if !consts.is_empty() {
        return consts.join("|");
    }
    let types: Vec<&str> = match &target["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(list) => list
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .collect(),
        _ => Vec::new(),
    };
    match types.as_slice() {
        ["array"] => format!("[{}]", label(schema, &target["items"])),
        ["object"] | [] => value["anyOf"]
            .as_array()
            .into_iter()
            .flatten()
            .chain([value])
            .find_map(|v| v["$ref"].as_str())
            .and_then(|r| r.rsplit('/').next())
            .or_else(|| target["title"].as_str())
            .unwrap_or("object")
            .to_string(),
        list => list.join("|"),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Field list of one command, with one level of nested fields and alternatives.
fn command_help(schema: &Value, name: &str, text: &str, variant: &Value) -> String {
    let required: Vec<&str> = variant["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out = format!(
        "raytrace-rs {name} [FIELD=文字列 | FIELD:=JSON | FIELD:=@FILE.json]...\n{text}\n\nフィールド:\n"
    );
    let empty = Map::new();
    let properties = variant["properties"].as_object().unwrap_or(&empty);
    for (field, value) in properties.iter().filter(|(f, _)| *f != "command") {
        let mark = if required.contains(&field.as_str()) {
            "（必須）"
        } else {
            ""
        };
        let default = match &value["default"] {
            Value::Null | Value::Object(_) => String::new(),
            v => format!(" 既定: {v}"),
        };
        let description = value["description"]
            .as_str()
            .or_else(|| resolve(schema, value)["description"].as_str())
            .map(first_line)
            .unwrap_or_default();
        out += &format!(
            "  {field}{mark}: {}{default}\n      {description}\n",
            label(schema, value)
        );
        let target = resolve(schema, value);
        // Exactly-one-key inputs such as model: {path} | {design} | {scene}.
        for option in target["oneOf"].as_array().into_iter().flatten() {
            if let (Some(keys), Some(_)) = (
                option["required"].as_array(),
                option["properties"].as_object(),
            ) && let [key] = keys.as_slice()
                && let Some(key) = key.as_str()
            {
                let inner = &option["properties"][key];
                let description = option["description"]
                    .as_str()
                    .or_else(|| inner["description"].as_str())
                    .map(first_line)
                    .unwrap_or_default();
                out += &format!(
                    "    {field}.{key}: {}  {description}\n",
                    label(schema, inner)
                );
            }
        }
        if target["type"] == "object" && target["oneOf"].is_null() && value["$ref"].is_string()
            || value["anyOf"].is_array() && target["type"] == "object"
        {
            for (key, inner) in target["properties"].as_object().into_iter().flatten() {
                let description = inner["description"]
                    .as_str()
                    .map(first_line)
                    .unwrap_or_default();
                out += &format!(
                    "    {field}.{key}: {}  {description}\n",
                    label(schema, inner)
                );
            }
        }
    }
    out
}

fn overview(schema: &Value) -> String {
    let mut out = String::from(
        "raytrace-rs COMMAND [FIELD=文字列 | FIELD:=JSON | FIELD:=@FILE.json]...\n\
         raytrace-rs run REQUEST.json|-\n\
         raytrace-rs COMMAND --help\n\
         raytrace-rs --version\n\n\
         引数はリクエストJSONのフィールドをそのまま指定します（a.b=文字列、a.b:=数値や真偽値などのJSON、a.b:=@ファイル）。\n\
         run はJSONリクエストを実行し、- は標準入力から複数のリクエストを続けて読みます。\n\
         標準出力: リクエストごとに1行のJSON（schema name=response）。成功時 \"ok\": true と result、失敗時 \"ok\": false と error.code・error.message（終了コード1）。人向けの表示は標準エラーへ出力します。\n\
         単位: mm、um、任意の光量重み。発射光線の重み合計を100%の基準とします。\n\nコマンド:\n",
    );
    for (name, text, _) in commands(schema) {
        out += &format!("  {name:<14} {}\n", first_line(&text));
    }
    out += "\n例:\n  raytrace-rs trace model.path=examples/sequential_lens.json overrides.rays_per_source:=1024 output=results/lens.json\n  raytrace-rs inspect model.path=lens.zmx\n  raytrace-rs scan model.path=examples/sequential_lens.json plan:=@scan_plan.json output=results/scan\n  raytrace-rs schema name=scene\n  echo '{\"command\":\"version\"}' | raytrace-rs run -";
    out
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    let ok = match args.first().map(String::as_str) {
        None | Some("--help" | "-h" | "help") => {
            let _ = writeln!(stdout, "{}", overview(&request_schema()));
            true
        }
        // Plain text: packaging scripts compare it with the crate version.
        Some("--version" | "-V") => {
            let _ = writeln!(stdout, "raytrace-rs {}", env!("CARGO_PKG_VERSION"));
            true
        }
        Some("run") => run_requests(args.get(1), &mut stdout),
        Some(command) => {
            let schema = request_schema();
            let known = commands(&schema);
            match known.iter().find(|(name, _, _)| name == command) {
                None => fail(
                    &mut stdout,
                    "usage",
                    usage(format!(
                        "不明なコマンドです: {command}（{}）",
                        known
                            .iter()
                            .map(|(n, _, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                ),
                Some((name, text, variant))
                    if args[1..].iter().any(|a| a == "--help" || a == "-h") =>
                {
                    let _ = write!(stdout, "{}", command_help(&schema, name, text, variant));
                    true
                }
                Some(_) => match build(command, &args[1..]) {
                    Ok(value) => perform(value, &mut stdout),
                    Err(error) => fail(&mut stdout, command, error),
                },
            }
        }
    };
    let _ = stdout.flush();
    if !ok {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments_become_request_fields() {
        let fields = [
            "model.path=a b.zmx",
            "overrides.rays_per_source:=1024",
            "overrides.ignore_coatings:=true",
            "output=1024",
        ]
        .map(String::from);
        assert_eq!(
            build("trace", &fields).unwrap(),
            json!({
                "command": "trace",
                "model": {"path": "a b.zmx"},
                "overrides": {"rays_per_source": 1024, "ignore_coatings": true},
                "output": "1024",
            })
        );
        for bad in ["model.path", "model..path=x", "command=x", "n:=nope"] {
            assert_eq!(
                build("trace", &[bad.into()]).unwrap_err().code,
                "usage",
                "{bad}"
            );
        }
        let twice = ["output=a", "output=b"].map(String::from);
        assert_eq!(build("trace", &twice).unwrap_err().code, "usage");
    }

    #[test]
    fn help_lists_every_command_and_nested_fields() {
        let schema = request_schema();
        let names: Vec<_> = commands(&schema).into_iter().map(|(n, _, _)| n).collect();
        assert!(
            names.contains(&"trace".to_string()) && names.contains(&"export_screen".to_string())
        );
        let (name, text, variant) = commands(&schema)
            .into_iter()
            .find(|(n, _, _)| n == "trace")
            .unwrap();
        let help = command_help(&schema, &name, &text, &variant);
        for expected in [
            "model（必須）",
            "model.path",
            "model.scene",
            "overrides.rays_per_source",
            "rays.csv",
            "sequential|non_sequential",
        ] {
            assert!(help.contains(expected), "{expected} missing from:\n{help}");
        }
    }
}
