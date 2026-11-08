//! The session and the command registry: revisions, undo and redo, `doc.set`, one-shot use of
//! `model`, schemas from the registry and a stream of requests sharing one session.
#[path = "support/cli.rs"]
mod cli_support;
use raytrace_rs::{
    command::{Context, Kind, registry},
    engine::{EngineError, Response},
    session::Session,
    trace::TraceControl,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

const LENS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/sequential_lens.json");

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/session_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn exec(session: &mut Session, command: &str, params: Value) -> Response {
    let control = TraceControl::default();
    session.execute(command, params, &mut Context::new(&control))
}

/// Run a command that must succeed and return its result.
fn ok(session: &mut Session, command: &str, params: Value) -> Value {
    let response = exec(session, command, params);
    assert!(response.ok, "{command}: {:?}", response.error);
    response.result_value()
}

fn error_code(session: &mut Session, command: &str, params: Value) -> String {
    let response = exec(session, command, params);
    assert!(!response.ok, "{command} should fail");
    response.error.unwrap().code
}

fn document(session: &mut Session) -> Value {
    ok(session, "doc.get", json!({}))["value"].clone()
}

fn open_lens() -> Session {
    let mut session = Session::new();
    ok(&mut session, "doc.open", json!({"path": LENS}));
    session
}

#[test]
fn undo_and_redo_restore_the_design_and_count_revisions() {
    let mut session = open_lens();
    assert_eq!(session.revision(), 1);
    let original = document(&mut session);

    let set = exec(
        &mut session,
        "doc.set",
        json!({"path": "/parts/@system-1/scene/surfaces/2/pose/translation_mm/2", "value": 70.0}),
    );
    assert!(set.ok, "{:?}", set.error);
    assert_eq!(set.revision, 2);
    let edited = document(&mut session);
    assert_ne!(edited, original);
    assert_eq!(
        edited["parts"][0]["scene"]["surfaces"][2]["pose"]["translation_mm"][2],
        70.0
    );

    let status = ok(&mut session, "session.undo", json!({}));
    assert_eq!(status["revision"], 3, "undo is a change of the session");
    assert_eq!(status["can_redo"], true);
    assert_eq!(status["can_undo"], false);
    assert_eq!(document(&mut session), original);

    let status = ok(&mut session, "session.redo", json!({}));
    assert_eq!(status["revision"], 4);
    assert_eq!(status["can_undo"], true);
    assert_eq!(document(&mut session), edited);

    // A new change clears the redo history.
    ok(&mut session, "session.undo", json!({}));
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "renamed"}),
    );
    assert_eq!(
        error_code(&mut session, "session.redo", json!({})),
        "history"
    );
    // Nothing to undo before the first change either.
    let mut fresh = open_lens();
    assert_eq!(error_code(&mut fresh, "session.undo", json!({})), "history");
}

#[test]
fn failed_changes_leave_document_history_and_revision_alone() {
    let mut session = open_lens();
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "first"}),
    );
    let before = document(&mut session);
    let status_before = ok(&mut session, "session.status", json!({}));

    let attempts = [
        // Pointer errors.
        ("doc.set", json!({"path": "name", "value": "x"})),
        (
            "doc.set",
            json!({"path": "/parts/@nope/name", "value": "x"}),
        ),
        ("doc.set", json!({"path": "/parts/7/name", "value": "x"})),
        ("doc.set", json!({"path": "/missing/deeper", "value": 1})),
        ("doc.set", json!({"path": "", "value": {}})),
        // The edited document does not read back as a design.
        ("doc.set", json!({"path": "/mode", "value": "bogus"})),
        ("doc.set", json!({"path": "/parts/0/surprise", "value": 1})),
        // Identity of the document is not editable.
        ("doc.set", json!({"path": "/format", "value": "other"})),
        ("doc.set", json!({"path": "/next_id", "value": 99})),
        (
            "doc.set",
            json!({"path": "/parts/0/id", "value": "system-9"}),
        ),
        (
            "doc.set",
            json!({"path": "/occurrences/@system-2/part", "value": "system-9"}),
        ),
        ("doc.set", json!({"path": "/occurrences", "value": []})),
        ("component.remove", json!({"occurrence": "nope-1"})),
        ("doc.replace", json!({"design": {"format": "x"}})),
    ];
    for (command, params) in attempts {
        let response = exec(&mut session, command, params.clone());
        assert!(!response.ok, "{command} {params}");
        let code = response.error.unwrap().code;
        assert!(
            ["invalid_request", "invalid_json", "design"].contains(&code.as_str()),
            "{command} {params}: {code}"
        );
        assert_eq!(response.revision, status_before["revision"]);
    }
    assert_eq!(document(&mut session), before);
    assert_eq!(ok(&mut session, "session.status", json!({})), status_before);
}

#[test]
fn expected_revision_rejects_stale_edits() {
    let mut session = open_lens();
    let revision = session.revision();
    let stale = exec(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "a", "expected_revision": revision + 5}),
    );
    assert_eq!(stale.error.as_ref().unwrap().code, "revision");
    assert_eq!(stale.revision, revision);
    let fresh = exec(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "a", "expected_revision": revision}),
    );
    assert!(fresh.ok, "{:?}", fresh.error);
    assert_eq!(fresh.revision, revision + 1);
    // The same revision cannot be used twice.
    assert_eq!(
        error_code(
            &mut session,
            "doc.set",
            json!({"path": "/name", "value": "b", "expected_revision": revision})
        ),
        "revision"
    );
    assert_eq!(
        error_code(
            &mut session,
            "session.undo",
            json!({"expected_revision": revision})
        ),
        "revision"
    );
    assert_eq!(
        error_code(
            &mut session,
            "component.add",
            json!({"kind": "detector", "expected_revision": revision})
        ),
        "revision"
    );
}

#[test]
fn array_elements_are_addressed_by_index_or_id() {
    let mut session = Session::new();
    ok(&mut session, "doc.new", json!({}));
    let rod = ok(&mut session, "component.add", json!({"kind": "rod"}));
    let rod_occurrence = rod["occurrence"].as_str().unwrap().to_string();
    let rod_part = rod["part"].as_str().unwrap().to_string();
    assert!(rod_occurrence.starts_with("rod-"), "{rod}");

    let by_id = format!("/occurrences/@{rod_occurrence}/pose/translation_mm/2");
    let by_index = "/occurrences/0/pose/translation_mm/2";
    let set = ok(
        &mut session,
        "doc.set",
        json!({"path": by_id, "value": 12.5}),
    );
    assert_eq!(set["path"], by_index, "ids resolve to indices");
    assert_eq!(
        ok(&mut session, "doc.get", json!({"path": by_index}))["value"],
        12.5
    );
    let length = format!("/parts/@{rod_part}/rod/length_mm");
    ok(
        &mut session,
        "doc.set",
        json!({"path": length, "value": 44.0}),
    );
    assert_eq!(
        ok(
            &mut session,
            "doc.get",
            json!({"path": "/parts/0/rod/length_mm"})
        )["value"],
        44.0
    );
    // Whole subtrees can be replaced too.
    ok(
        &mut session,
        "doc.set",
        json!({"path": format!("/occurrences/@{rod_occurrence}/pose"), "value": {"translation_mm": [1.0, 2.0, 3.0]}}),
    );
    assert_eq!(
        ok(&mut session, "doc.get", json!({"path": by_index}))["value"],
        3.0
    );
}

#[test]
fn components_are_added_duplicated_and_removed_with_undo() {
    let mut session = Session::new();
    ok(&mut session, "doc.new", json!({"name": "テスト"}));
    let rod = ok(
        &mut session,
        "component.add",
        json!({"kind": "rod", "name": "R"}),
    );
    let screen = ok(&mut session, "component.add", json!({"kind": "detector"}));
    assert_eq!(screen["occurrences"], 2);
    let copy = ok(
        &mut session,
        "component.duplicate",
        json!({"occurrence": rod["occurrence"]}),
    );
    assert_eq!(
        (copy["parts"].clone(), copy["occurrences"].clone()),
        (json!(3), json!(3))
    );
    assert_ne!(copy["occurrence"], rod["occurrence"]);
    let removed = ok(
        &mut session,
        "component.remove",
        json!({"occurrence": copy["occurrence"]}),
    );
    assert_eq!(removed["occurrences"], 2);
    assert_eq!(removed["parts"], 2, "the copy's part goes with it");
    let after_remove = document(&mut session);
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(
        ok(&mut session, "session.status", json!({}))["document"]["occurrences"],
        3
    );
    ok(&mut session, "session.redo", json!({}));
    assert_eq!(document(&mut session), after_remove);
    // Files are imported as parts; a source, an optic and a system kind are possible.
    let optic = ok(
        &mut session,
        "component.add",
        json!({"path": LENS, "kind": "optic"}),
    );
    assert!(optic["occurrence"].as_str().unwrap().starts_with("optic-"));
    assert_eq!(
        error_code(
            &mut session,
            "component.add",
            json!({"path": LENS, "kind": "rod"})
        ),
        "invalid_request"
    );
    assert_eq!(
        error_code(&mut session, "component.add", json!({})),
        "invalid_request"
    );
    assert_eq!(
        error_code(&mut Session::new(), "component.add", json!({"kind": "rod"})),
        "no_document"
    );
}

#[test]
fn commands_use_the_session_document_or_a_one_shot_model() {
    let mut empty = Session::new();
    let missing = exec(&mut empty, "analysis.trace", json!({"threads": 1}));
    let error = missing.error.unwrap();
    assert_eq!(error.code, "no_document");
    assert!(
        error.message.contains("文書が開かれていません"),
        "{}",
        error.message
    );
    for (command, params) in [
        ("doc.get", json!({})),
        ("doc.inspect", json!({})),
        ("doc.export_zmx", json!({"output": "x.zmx"})),
        ("doc.set", json!({"path": "/name", "value": "x"})),
        ("session.undo", json!({})),
    ] {
        let code = error_code(&mut empty, command, params);
        assert!(
            ["no_document", "history"].contains(&code.as_str()),
            "{command}: {code}"
        );
    }

    let mut session = open_lens();
    let revision = session.revision();
    let before = document(&mut session);
    // With `model` the command works on that model and the session stays as it is.
    let inspected = ok(
        &mut session,
        "doc.inspect",
        json!({"model": {"path": concat!(env!("CARGO_MANIFEST_DIR"), "/examples/integrator_rod.json")}}),
    );
    assert_eq!(inspected["traceable"], true);
    let traced = ok(
        &mut session,
        "analysis.trace",
        json!({"model": {"path": concat!(env!("CARGO_MANIFEST_DIR"), "/examples/folded_mirror.json")},
               "overrides": {"rays_per_source": 4}, "threads": 1}),
    );
    assert!(traced["launched_rays"].as_u64().unwrap() > 0);
    assert_eq!(session.revision(), revision);
    assert_eq!(document(&mut session), before);
    assert_eq!(
        session.design().unwrap().display_name(),
        "Dispersive silica biconvex lens"
    );
}

#[test]
fn saving_tracks_the_path_and_the_dirty_flag() {
    let dir = folder();
    let mut session = open_lens();
    let status = ok(&mut session, "session.status", json!({}));
    assert_eq!(status["dirty"], false);
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "x"}),
    );
    assert!(session.is_dirty());
    // The file the document came from is a scene, not a design; save elsewhere.
    let saved = dir.join("lens.design.json");
    ok(&mut session, "doc.save", json!({"output": saved}));
    assert!(!session.is_dirty());
    assert_eq!(session.path(), Some(saved.as_path()));
    // Undo goes back to a state that differs from the file.
    ok(&mut session, "session.undo", json!({}));
    assert!(session.is_dirty());
    ok(&mut session, "session.redo", json!({}));
    assert!(!session.is_dirty());
    // Saving again without `output` overwrites the same file.
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "y"}),
    );
    ok(&mut session, "doc.save", json!({}));
    let mut reopened = Session::new();
    ok(&mut reopened, "doc.open", json!({"path": saved}));
    assert_eq!(reopened.design().unwrap().name, "y");
    let mut empty = Session::new();
    assert_eq!(error_code(&mut empty, "doc.save", json!({})), "no_document");
    ok(&mut empty, "doc.new", json!({}));
    assert_eq!(
        error_code(&mut empty, "doc.save", json!({})),
        "invalid_request",
        "a new document has no file yet"
    );
}

#[test]
fn doc_open_refers_records_and_scan_directories_to_their_commands() {
    let dir = folder();
    let mut session = open_lens();
    let record = dir.join("run.json");
    ok(
        &mut session,
        "analysis.trace",
        json!({"overrides": {"rays_per_source": 4}, "threads": 1, "output": record}),
    );
    let revision = session.revision();
    assert_eq!(
        error_code(&mut session, "doc.open", json!({"path": record})),
        "run_record"
    );
    assert_eq!(
        error_code(&mut session, "doc.open", json!({"path": dir})),
        "study_directory"
    );
    assert_eq!(
        session.revision(),
        revision,
        "a failed open keeps the document"
    );
    let opened = ok(&mut session, "run.open", json!({"path": record}));
    assert_eq!(opened["record"]["format"], "raytrace-run-v1");
}

#[test]
fn every_command_has_schemas_and_is_listed() {
    let mut session = Session::new();
    let list = ok(&mut session, "command.list", json!({}));
    let listed: Vec<&str> = list["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    let registered: Vec<&str> = registry().iter().map(|c| c.id).collect();
    assert_eq!(listed, registered);
    assert!(registered.len() >= 25, "{registered:?}");
    // The catalogue of docs/DESIGN.md §7.
    for id in [
        "app.version",
        "command.list",
        "command.describe",
        "schema.get",
        "doc.new",
        "doc.open",
        "doc.save",
        "doc.get",
        "doc.inspect",
        "doc.export_zmx",
        "doc.set",
        "doc.replace",
        "component.add",
        "component.remove",
        "component.duplicate",
        "analysis.trace",
        "study.scan",
        "study.resume",
        "study.convergence",
        "study.open",
        "run.open",
        "run.replay",
        "run.save",
        "run.export_screen",
        "session.undo",
        "session.redo",
        "session.status",
        "session.log",
    ] {
        assert!(registered.contains(&id), "{id} is not registered");
    }
    let request = ok(&mut session, "schema.get", json!({"name": "request"}))["schema"].clone();
    let response = ok(&mut session, "schema.get", json!({"name": "response"}))["schema"].clone();
    assert_eq!(request["oneOf"].as_array().unwrap().len(), registered.len());
    assert_eq!(
        response["oneOf"].as_array().unwrap().len(),
        registered.len()
    );

    for spec in registry().iter() {
        assert_eq!(
            spec.id.split('.').count(),
            2,
            "{} is <area>.<action>",
            spec.id
        );
        assert!(!spec.summary.is_empty(), "{}", spec.id);
        let description = ok(&mut session, "command.describe", json!({"id": spec.id}));
        assert_eq!(description["id"], spec.id);
        for schema in ["input_schema", "output_schema"] {
            assert!(
                description[schema].is_object() && description[schema]["type"] == "object",
                "{} has no {schema}",
                spec.id
            );
        }
        // Inputs refuse fields that are not in their schema.
        assert_eq!(
            description["input_schema"]["additionalProperties"], false,
            "{}",
            spec.id
        );
        let variant = request["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["command"]["enum"][0] == spec.id)
            .unwrap_or_else(|| panic!("{} is missing from the request schema", spec.id));
        assert_eq!(variant["additionalProperties"], false);
        let unknown = exec(&mut session, spec.id, json!({"no_such_field": 1}));
        assert_eq!(
            unknown.error.as_ref().map(|e| e.code.as_str()),
            Some("invalid_request"),
            "{}",
            spec.id
        );
        // Mutations take `expected_revision`.
        assert_eq!(
            spec.kind == Kind::Mutation,
            matches!(
                spec.id,
                "doc.new"
                    | "doc.open"
                    | "doc.set"
                    | "doc.replace"
                    | "component.add"
                    | "component.remove"
                    | "component.duplicate"
                    | "session.undo"
                    | "session.redo"
            ),
            "{}",
            spec.id
        );
        if spec.kind == Kind::Mutation {
            assert!(
                description["input_schema"]["properties"]["expected_revision"].is_object(),
                "{} takes no expected_revision",
                spec.id
            );
            // The commands that make one undoable change can name a group of changes.
            assert_eq!(
                description["input_schema"]["properties"]["group"].is_object(),
                matches!(
                    spec.id,
                    "doc.set"
                        | "doc.replace"
                        | "component.add"
                        | "component.remove"
                        | "component.duplicate"
                ),
                "{}",
                spec.id
            );
        }
    }

    // The previous short names stay as deprecated aliases of the new ids.
    let aliases = list["aliases"].as_object().unwrap();
    assert_eq!(aliases.len(), 13);
    for (alias, id) in aliases {
        assert_eq!(registry().resolve(alias).unwrap().id, id.as_str().unwrap());
        assert!(registry().get(alias).is_none(), "{alias} is not an id");
    }
    let by_alias = exec(&mut session, "version", json!({}));
    assert!(by_alias.ok);
    assert_eq!(
        by_alias.command, "version",
        "the command is echoed as written"
    );
    assert_eq!(
        ok(&mut session, "command.describe", json!({"id": "trace"}))["id"],
        "analysis.trace"
    );
}

#[test]
fn unknown_commands_and_malformed_requests_are_invalid_requests() {
    let mut session = Session::new();
    let unknown = exec(&mut session, "analysis.trac", json!({}));
    let error = unknown.error.unwrap();
    assert_eq!(error.code, "invalid_request");
    assert!(
        error.message.contains("analysis.trace"),
        "{}",
        error.message
    );
    let control = TraceControl::default();
    let mut context = Context::new(&control);
    for request in [json!([]), json!({"model": 1}), json!({"command": 3})] {
        let response = session.execute_request(request, &mut context);
        assert_eq!(response.error.unwrap().code, "invalid_request");
    }
    let response = session.execute_text("{", &mut context);
    assert_eq!(response.error.unwrap().code, "invalid_json");
    // Every response carries the session revision, failed or not.
    assert_eq!(unknown.revision, 0);
}

#[test]
fn typed_calls_run_the_same_commands() {
    use raytrace_rs::{commands, engine::DocSetRequest};
    let mut session = open_lens();
    let result = session
        .run(
            commands::DOC_SET,
            DocSetRequest {
                path: "/name".into(),
                value: json!("typed"),
                group: None,
                expected_revision: Some(1),
            },
        )
        .unwrap();
    assert_eq!(result.path, "/name");
    assert_eq!(session.design().unwrap().name, "typed");
    let error: EngineError = session
        .run(
            commands::DOC_SET,
            DocSetRequest {
                path: "/name".into(),
                value: json!("again"),
                group: None,
                expected_revision: Some(1),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "revision");
}

/// Run the CLI and parse every stdout line as one JSON response.
fn cli(dir: &PathBuf, args: &[&str], stdin: &str) -> (bool, Vec<Value>) {
    let mut child = Command::new(cli_support::executable())
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect();
    (output.status.success(), responses)
}

#[test]
fn a_run_stream_shares_one_session() {
    let dir = folder();
    let trace = json!({"command": "analysis.trace", "overrides": {"rays_per_source": 64},
                       "threads": 1, "include_record": true});
    let requests = [
        json!({"command": "doc.open", "path": LENS}),
        trace.clone(),
        json!({"command": "doc.set", "path": "/parts/@system-1/scene/surfaces/2/pose/translation_mm/2", "value": 70.0}),
        trace.clone(),
        json!({"command": "session.undo"}),
        trace.clone(),
        json!({"command": "session.status"}),
    ]
    .map(|r| r.to_string())
    .join("\n");
    let (ok, responses) = cli(&dir, &["run", "-"], &requests);
    assert!(ok, "{responses:?}");
    assert_eq!(responses.len(), 7);
    let report = |i: usize| responses[i]["result"]["record"]["report"].clone();
    assert!(report(1).is_object());
    assert_ne!(report(1), report(3), "moving the screen changes the report");
    assert_eq!(
        report(1),
        report(5),
        "undo brings back the original calculation"
    );
    assert_eq!(
        responses[1]["result"]["detected_percent_of_reference"],
        responses[5]["result"]["detected_percent_of_reference"]
    );
    let revisions: Vec<_> = responses
        .iter()
        .map(|r| r["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, [1, 1, 2, 2, 3, 3, 3]);
    assert_eq!(responses[6]["result"]["can_redo"], true);

    // A one-shot command line is a new session: nothing is open yet.
    let (ok, alone) = cli(&dir, &["analysis.trace", "threads:=1"], "");
    assert!(!ok);
    assert_eq!(alone[0]["error"]["code"], "no_document");
    // The previous short names keep working.
    let (ok, old) = cli(&dir, &["version"], "");
    assert!(ok);
    assert_eq!(old[0]["command"], "version");
    assert_eq!(old[0]["result"]["version"], env!("CARGO_PKG_VERSION"));
    let (ok, described) = cli(&dir, &["command.describe", "id=doc.set"], "");
    assert!(ok);
    assert_eq!(described[0]["result"]["kind"], "mutation");
}

fn set_name(session: &mut Session, name: &str, group: Option<&str>) -> Value {
    let mut params = json!({"path": "/name", "value": name});
    if let Some(group) = group {
        params["group"] = json!(group);
    }
    ok(session, "doc.set", params)
}

fn undo_depth(session: &mut Session) -> u64 {
    ok(session, "session.status", json!({}))["undo_depth"]
        .as_u64()
        .unwrap()
}

#[test]
fn changes_of_one_group_in_a_row_share_an_undo_step() {
    let mut session = open_lens();
    let original = document(&mut session);
    let revision = session.revision();
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        set_name(&mut session, name, Some("drag:1"));
        assert_eq!(session.revision(), revision + 1 + i as u64, "per change");
    }
    assert_eq!(undo_depth(&mut session), 1);
    // A different group, or none, starts a new step.
    set_name(&mut session, "d", Some("drag:2"));
    set_name(&mut session, "e", None);
    set_name(&mut session, "f", None);
    set_name(&mut session, "g", Some("drag:2"));
    assert_eq!(undo_depth(&mut session), 5);

    // Undo goes back to the state before the first change of the step.
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "f");
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "e");
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "d");
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "c");
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session), original);
    assert_eq!(
        error_code(&mut session, "session.undo", json!({})),
        "history"
    );
    ok(&mut session, "session.redo", json!({}));
    assert_eq!(document(&mut session)["name"], "c");
}

#[test]
fn undo_redo_open_and_other_commands_end_a_group() {
    let mut session = open_lens();
    set_name(&mut session, "a", Some("g"));
    set_name(&mut session, "b", Some("g"));
    ok(&mut session, "session.undo", json!({}));
    ok(&mut session, "session.redo", json!({}));
    // The step was closed by the undo and redo: the same group name starts a new one.
    set_name(&mut session, "c", Some("g"));
    assert_eq!(undo_depth(&mut session), 2);
    // A structural change in the same group joins the step.
    ok(
        &mut session,
        "component.add",
        json!({"kind": "detector", "group": "g"}),
    );
    assert_eq!(undo_depth(&mut session), 2);
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "b");
    assert_eq!(
        ok(&mut session, "session.status", json!({}))["document"]["occurrences"],
        1
    );
    // A failed change does not end the group, a query does not either.
    set_name(&mut session, "d", Some("h"));
    assert!(
        !exec(
            &mut session,
            "doc.set",
            json!({"path": "/nope", "value": 1, "group": "h"})
        )
        .ok
    );
    document(&mut session);
    set_name(&mut session, "e", Some("h"));
    assert_eq!(undo_depth(&mut session), 2);
    // Opening a document starts over.
    ok(&mut session, "doc.open", json!({"path": LENS}));
    assert_eq!(undo_depth(&mut session), 0);
    set_name(&mut session, "x", Some("h"));
    set_name(&mut session, "y", Some("h"));
    assert_eq!(undo_depth(&mut session), 1);
}

#[test]
fn the_session_keeps_at_most_64_undo_steps_and_a_byte_budget() {
    use raytrace_rs::session::HistoryLimits;
    let mut session = open_lens();
    assert_eq!(session.history_limits(), HistoryLimits::default());
    assert_eq!(HistoryLimits::default().steps, 64);
    assert_eq!(HistoryLimits::default().bytes, 128 * 1024 * 1024);
    for i in 0..70 {
        set_name(&mut session, &format!("n{i}"), None);
    }
    assert_eq!(undo_depth(&mut session), 64);
    assert_eq!(
        session.log().entries.len(),
        70,
        "the log keeps dropped steps"
    );
    for _ in 0..64 {
        ok(&mut session, "session.undo", json!({}));
    }
    assert_eq!(
        document(&mut session)["name"],
        "n5",
        "the oldest 6 are gone"
    );

    // The byte budget drops the oldest steps first but keeps the newest.
    let mut session = open_lens();
    for i in 0..5 {
        set_name(&mut session, &format!("n{i}"), None);
    }
    let one = session.undo_bytes() / 5;
    assert!(one > 0);
    session.set_history_limits(HistoryLimits {
        steps: 64,
        bytes: one * 2 + one / 2,
    });
    assert_eq!(undo_depth(&mut session), 2);
    session.set_history_limits(HistoryLimits {
        steps: 64,
        bytes: 0,
    });
    assert_eq!(undo_depth(&mut session), 1);
    assert!(session.undo_bytes() > 0);
}

#[test]
fn the_log_lists_committed_changes_without_the_undone_ones() {
    let mut session = Session::new();
    assert_eq!(session.log().base, None);
    assert!(session.log().entries.is_empty());
    let log = ok(&mut session, "session.log", json!({}));
    assert_eq!(log["base"], Value::Null);
    assert_eq!(log["entries"], json!([]));

    ok(&mut session, "doc.open", json!({"path": LENS}));
    let log = ok(&mut session, "session.log", json!({}));
    assert_eq!(
        log["base"],
        json!({"command": "doc.open", "params": {"path": LENS}})
    );
    assert_eq!(log["entries"], json!([]));
    assert_eq!(log["revision"], session.revision());

    let revision = session.revision();
    set_name(&mut session, "a", None);
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/mode", "value": "non_sequential", "expected_revision": revision + 1}),
    );
    let log = ok(&mut session, "session.log", json!({}));
    assert_eq!(
        log["entries"],
        json!([
            {"command": "doc.set", "params": {"path": "/name", "value": "a"}},
            {"command": "doc.set", "params": {"path": "/mode", "value": "non_sequential"}},
        ]),
        "expected_revision is not part of the log"
    );

    // A failed change is not logged; an undo takes the last step out and redo brings it back.
    assert!(
        !exec(
            &mut session,
            "doc.set",
            json!({"path": "/nope", "value": 1})
        )
        .ok
    );
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(session.log().entries.len(), 1);
    ok(&mut session, "session.redo", json!({}));
    assert_eq!(session.log().entries.len(), 2);
    // A new change after an undo replaces the undone one.
    ok(&mut session, "session.undo", json!({}));
    set_name(&mut session, "b", None);
    let entries = session.log().entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].params["value"], "b");
    assert_eq!(
        error_code(&mut session, "session.redo", json!({})),
        "history"
    );

    // Rewriting one place in a group keeps the last value only.
    for value in [1.0, 2.0, 3.0] {
        ok(
            &mut session,
            "doc.set",
            json!({"path": "/parts/@system-1/scene/surfaces/2/pose/translation_mm/2", "value": value, "group": "drag"}),
        );
    }
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "c", "group": "drag"}),
    );
    let entries = session.log().entries;
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[2].params["value"], 3.0);
    assert_eq!(entries[2].params["group"], "drag");

    // Opening or creating a document starts a new log.
    ok(&mut session, "doc.new", json!({"name": "n"}));
    let log = session.log();
    assert_eq!(log.base.unwrap().command, "doc.new");
    assert!(log.entries.is_empty());
    // A session can also start with doc.replace, which is then the first entry.
    let design = document(&mut session);
    let mut started = Session::new();
    ok(&mut started, "doc.replace", json!({"design": design}));
    let log = started.log();
    assert!(log.base.is_none());
    assert_eq!(log.entries[0].command, "doc.replace");
}

/// Run the log of `session` on a new session and return that session.
fn replay(session: &Session) -> Session {
    let log = session.log();
    let mut fresh = Session::new();
    if let Some(base) = &log.base {
        ok(&mut fresh, &base.command, base.params.clone());
    }
    for entry in &log.entries {
        ok(&mut fresh, &entry.command, entry.params.clone());
    }
    fresh
}

fn design_text(session: &Session) -> String {
    serde_json::to_string_pretty(session.design().unwrap().as_ref()).unwrap()
}

#[test]
fn replaying_the_log_in_a_new_session_gives_the_same_document() {
    use raytrace_rs::session::HistoryLimits;
    let mut session = open_lens();
    set_name(&mut session, "edited", None);
    let rod = ok(
        &mut session,
        "component.add",
        json!({"kind": "rod", "name": "R", "group": "add"}),
    );
    let rod = rod["occurrence"].as_str().unwrap().to_string();
    ok(
        &mut session,
        "doc.set",
        json!({"path": format!("/occurrences/@{rod}/pose/translation_mm/2"), "value": 12.5, "group": "add"}),
    );
    for z in [1.0, 2.0, 3.5] {
        ok(
            &mut session,
            "doc.set",
            json!({"path": format!("/occurrences/@{rod}/pose/translation_mm"), "value": [0.0, 0.1, z], "group": "drag:1"}),
        );
    }
    let copy = ok(
        &mut session,
        "component.duplicate",
        json!({"occurrence": rod}),
    );
    ok(&mut session, "component.add", json!({"kind": "detector"}));
    ok(
        &mut session,
        "component.remove",
        json!({"occurrence": copy["occurrence"]}),
    );
    // Undone and redone changes, and a step dropped by the limits.
    ok(&mut session, "session.undo", json!({}));
    ok(&mut session, "session.undo", json!({}));
    ok(&mut session, "session.redo", json!({}));
    ok(
        &mut session,
        "doc.set",
        json!({"path": "/name", "value": "last"}),
    );
    session.set_history_limits(HistoryLimits {
        steps: 3,
        bytes: usize::MAX,
    });
    assert_eq!(undo_depth(&mut session), 3);

    let fresh = replay(&session);
    assert_eq!(design_text(&fresh), design_text(&session));
    assert_eq!(fresh.log().entries, session.log().entries);
}

#[test]
fn a_new_document_can_start_from_a_design_and_is_not_dirty() {
    let mut session = open_lens();
    let design = document(&mut session);
    let mut other = Session::new();
    ok(
        &mut other,
        "doc.new",
        json!({"design": design.clone(), "name": "copy"}),
    );
    assert!(!other.is_dirty());
    assert_eq!(document(&mut other)["name"], "copy");
    assert_eq!(undo_depth(&mut other), 0);
    let fresh = replay(&other);
    assert_eq!(design_text(&fresh), design_text(&other));
    set_name(&mut other, "changed", None);
    assert!(other.is_dirty());
}

#[test]
fn a_session_adopts_the_result_of_work_done_on_another_thread() {
    let mut session = open_lens();
    set_name(&mut session, "a", None);
    let before = session.revision();

    // Edit a copy elsewhere (a worker thread), then take it over.
    let mut copy = session.clone();
    let worker = std::thread::spawn(move || {
        set_name(&mut copy, "b", None);
        copy
    });
    session.adopt(worker.join().unwrap());
    assert_eq!(document(&mut session)["name"], "b");
    assert!(session.revision() > before);
    ok(&mut session, "session.undo", json!({}));
    assert_eq!(document(&mut session)["name"], "a");

    // A session that opened a file takes over its document, and the revision keeps rising.
    let revision = session.revision();
    let mut opened = Session::new();
    ok(&mut opened, "doc.open", json!({"path": LENS}));
    session.adopt(opened);
    assert!(session.revision() > revision);
    assert_eq!(undo_depth(&mut session), 0);
    assert_eq!(session.log().base.unwrap().command, "doc.open");
}

#[test]
fn an_edit_inside_one_element_gives_the_document_the_whole_json_would() {
    let build = || {
        let mut session = Session::new();
        ok(&mut session, "doc.new", json!({}));
        ok(&mut session, "component.add", json!({"kind": "rod"}));
        ok(&mut session, "component.add", json!({"kind": "detector"}));
        session
    };
    let mut edited = build();
    let mut replaced = build();
    let mut json = document(&mut edited);
    let occurrence = json["occurrences"][0]["id"].as_str().unwrap().to_string();
    let part = json["parts"][0]["id"].as_str().unwrap().to_string();
    json["parts"][0]["rod"]["width_mm"] = json!(7.5);
    json["occurrences"][0]["pose"]["translation_mm"] = json!([1.0, 2.0, 3.0]);
    json["occurrences"][1]["name"] = json!("moved");
    for (path, value) in [
        (format!("/parts/@{part}/rod/width_mm"), json!(7.5)),
        (
            format!("/occurrences/@{occurrence}/pose/translation_mm"),
            json!([1.0, 2.0, 3.0]),
        ),
        ("/occurrences/1/name".to_string(), json!("moved")),
    ] {
        let result = ok(
            &mut edited,
            "doc.set",
            json!({"path": path, "value": value}),
        );
        let applied = result["path"].as_str().unwrap();
        assert!(
            applied.starts_with("/parts/0/") || applied.starts_with("/occurrences/"),
            "{applied}"
        );
    }
    ok(&mut replaced, "doc.replace", json!({"design": json}));
    assert_eq!(design_text(&edited), design_text(&replaced));
    // An edit that makes the element unreadable is refused and leaves the document alone.
    let before = design_text(&edited);
    assert_eq!(
        error_code(
            &mut edited,
            "doc.set",
            json!({"path": "/parts/0/rod/width_mm", "value": "wide"})
        ),
        "invalid_request"
    );
    assert_eq!(design_text(&edited), before);
}
