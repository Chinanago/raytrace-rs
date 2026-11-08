//! The analysis registry: `analysis.budget` and `analysis.screen` work on the latest
//! calculation record of the session or on a record given as `run`, and their numbers are the
//! ones the pipeline and the screen post-processing compute from the same record.
#[path = "support/cli.rs"]
mod cli_support;
use raytrace_rs::{
    analysis::{
        ANALYSIS_BUDGET, ANALYSIS_SCREEN, AnalysisRequest, BudgetSettings, DetectorRef,
        ScreenSettings,
    },
    command::{Context, registry},
    engine::{RecordInput, Response, RunOutput},
    pipeline,
    record::RunRecord,
    screen::{ScreenAnalysis, ScreenRoi},
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
const MOVE_SCREEN: &str = "/parts/@system-1/scene/surfaces/2/pose/translation_mm/2";

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/analysis_tests")
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

fn trace_request(rays: usize, output: Option<PathBuf>) -> Value {
    json!({"overrides": {"rays_per_source": rays}, "threads": 1, "include_record": true,
           "output": output})
}

/// A session that opened the lens and traced it; also the record the trace returned.
fn traced() -> (Session, RunOutput) {
    let mut session = Session::new();
    ok(&mut session, "doc.open", json!({"path": LENS}));
    let response = exec(&mut session, "analysis.trace", trace_request(256, None));
    assert!(response.ok, "{:?}", response.error);
    let record = *response
        .typed::<raytrace_rs::engine::TraceResult>()
        .unwrap()
        .record
        .as_ref()
        .unwrap()
        .clone();
    (session, record)
}

fn screen(session: &mut Session, settings: ScreenSettings) -> raytrace_rs::analysis::ScreenOutput {
    session
        .run(ANALYSIS_SCREEN, AnalysisRequest::new(settings))
        .unwrap()
        .result
}

#[test]
fn budget_equals_the_pipeline_numbers_of_the_trace() {
    let (mut session, record) = traced();
    let response = session
        .run(ANALYSIS_BUDGET, AnalysisRequest::new(BudgetSettings {}))
        .unwrap();
    assert_eq!(response.run_revision, Some(1));
    assert!(!response.stale);
    let budget = response.result;
    let report = &record.report;

    assert_eq!(budget.reference_weight, record.reference_weight);
    assert_eq!(budget.launched_rays, record.benchmark.input_rays);
    let percent = pipeline::budget_percent(report, record.reference_weight).unwrap();
    let weights = pipeline::budget_weights(report).unwrap();
    assert_eq!(budget.entries.len(), percent.len());
    for (name, entry) in &budget.entries {
        assert_eq!(entry.percent_of_reference, percent[name], "{name}");
        assert_eq!(entry.weight, weights[name], "{name}");
        assert_eq!(
            entry.percent_of_reference,
            record.budget_percent_of_reference[name]
        );
    }
    assert_eq!(budget.entries["launched"].weight, report.budget.launched);
    assert_eq!(
        budget.relative_budget_closure_error,
        record.relative_budget_closure_error
    );
    assert_eq!(
        budget.unresolved_percent_of_reference,
        pipeline::unresolved_percent(report, record.reference_weight)
    );
    assert_eq!(budget.detectors.len(), report.detectors.len());
    for (i, detector) in budget.detectors.iter().enumerate() {
        assert_eq!(detector.index, i);
        assert_eq!(detector.name, report.detectors[i].name);
        assert_eq!(
            detector.percent_of_reference,
            pipeline::received_percent(&report.detectors[i], record.reference_weight)
        );
        assert_eq!(
            detector.percent_of_reference,
            record.detector_percent_of_reference[&detector.name]
        );
    }
    assert_eq!(
        serde_json::to_value(&budget.surfaces).unwrap(),
        serde_json::to_value(&record.surface_encounter_percent_of_reference).unwrap()
    );
}

#[test]
fn screen_equals_the_screen_analysis_of_the_same_record() {
    let (mut session, record) = traced();
    let map = &record.report.detectors[0];
    let reference = record.reference_weight;

    // Without a cursor the slices pass through the brightest pixel.
    let output = screen(&mut session, ScreenSettings::default());
    let direct = ScreenAnalysis::from_detector(map).unwrap();
    assert_eq!(output.detector, 0);
    assert_eq!(output.name, map.name);
    assert_eq!(output.pixels, map.pixels);
    assert_eq!(output.half_extent_mm, map.half_extent_mm);
    assert_eq!(output.pixel_pitch_mm, direct.pixel_pitch_mm);
    assert_eq!(output.reference_weight, reference);
    assert_eq!(
        output.received_percent_of_reference,
        pipeline::received_percent(map, reference)
    );
    assert_eq!(output.peak_weight, direct.peak_weight);
    assert_eq!(output.peak_pixel, direct.peak_pixel);
    assert_eq!(output.peak_position_mm, direct.peak_position_mm);
    assert_eq!(output.slice_position_mm, direct.slice_position_mm);
    assert_eq!(
        output.peak_percent_of_reference,
        100.0 * direct.peak_weight / reference
    );
    let (x, y) = (
        output.x_slice.as_ref().unwrap(),
        output.y_slice.as_ref().unwrap(),
    );
    let (dx, dy) = (
        direct.x_profile.as_ref().unwrap(),
        direct.y_profile.as_ref().unwrap(),
    );
    for (slice, profile) in [(x, dx), (y, dy)] {
        assert_eq!(slice.fwhm_mm, profile.half_maximum.width_mm());
        assert_eq!(slice.lower_mm, profile.half_maximum.lower_mm);
        assert_eq!(slice.upper_mm, profile.half_maximum.upper_mm);
        assert_eq!(slice.multiple_lobes, profile.half_maximum.multiple_lobes);
        assert_eq!(slice.peak_weight, profile.peak_weight());
        assert_eq!(slice.first_center_mm, profile.first_center_mm);
        assert_eq!(slice.pitch_mm, profile.pitch_mm);
        assert!(slice.weights.is_none(), "profiles only on request");
    }
    assert!(x.fwhm_mm.is_some() && y.fwhm_mm.is_some());
    assert!(output.roi.is_none());
    // The detector stats the studies report are the same numbers.
    let stats = pipeline::detector_stats(map, reference).unwrap();
    assert_eq!(stats.fwhm_mm, [x.fwhm_mm, y.fwhm_mm]);
    assert_eq!(stats.peak_position_mm, output.peak_position_mm);

    // A cursor, the profile samples and an ROI.
    let cursor = [0.05, -0.04];
    let roi = ScreenRoi {
        minimum_mm: [-0.1, -0.05],
        maximum_mm: [0.15, 0.1],
    };
    let output = screen(
        &mut session,
        ScreenSettings {
            detector: Some(DetectorRef::Index(0)),
            cursor_mm: Some(cursor),
            roi: Some(roi),
            include_profiles: true,
        },
    );
    let direct = ScreenAnalysis::at(map, Some(cursor)).unwrap();
    assert_eq!(output.slice_position_mm, direct.slice_position_mm);
    assert_eq!(output.peak_weight, direct.peak_weight);
    assert_eq!(output.roi, Some(roi.integrate(map, reference).unwrap()));
    for (slice, profile) in [
        (output.x_slice.as_ref(), direct.x_profile.as_ref()),
        (output.y_slice.as_ref(), direct.y_profile.as_ref()),
    ] {
        let (slice, profile) = (slice.unwrap(), profile.unwrap());
        assert_eq!(slice.weights.as_ref(), Some(&profile.weights));
        assert_eq!(slice.fwhm_mm, profile.half_maximum.width_mm());
        for i in [0, 7, profile.weights.len() - 1] {
            assert_eq!(slice.coordinate_mm(i), profile.coordinate_mm(i));
        }
    }

    // The JSON form carries the same numbers, with the optional parts left out.
    let json = ok(
        &mut session,
        "analysis.screen",
        json!({"cursor_mm": cursor, "roi": roi, "include_profiles": true}),
    );
    assert_eq!(
        json["x_slice"]["weights"],
        json!(output.x_slice.unwrap().weights)
    );
    assert_eq!(
        json["roi"]["percent_of_reference"],
        output.roi.unwrap().percent_of_reference
    );
    assert_eq!(json["run_revision"], 1);
    assert_eq!(json["stale"], false);
    let plain = ok(&mut session, "analysis.screen", json!({}));
    assert!(plain["x_slice"].get("weights").is_none());
    assert!(plain["roi"].is_null());
}

#[test]
fn a_screen_is_chosen_by_index_or_name_and_bad_settings_fail() {
    let (mut session, record) = traced();
    let name = record.report.detectors[0].name.clone();
    let by_name = screen(
        &mut session,
        ScreenSettings {
            detector: Some(DetectorRef::Name(name.clone())),
            ..ScreenSettings::default()
        },
    );
    assert_eq!(by_name, screen(&mut session, ScreenSettings::default()));
    // Text that is no name but a number is an index (`detector=0` on the command line).
    let numeric = ok(&mut session, "analysis.screen", json!({"detector": "0"}));
    assert_eq!(numeric["name"], name);
    assert_eq!(
        ok(&mut session, "analysis.screen", json!({"detector": 0}))["name"],
        name
    );

    assert_eq!(
        error_code(&mut session, "analysis.screen", json!({"detector": 5})),
        "invalid_request"
    );
    assert_eq!(
        error_code(&mut session, "analysis.screen", json!({"detector": "nope"})),
        "invalid_request"
    );
    assert_eq!(
        error_code(&mut session, "analysis.screen", json!({"bogus": 1})),
        "invalid_request"
    );
    assert_eq!(
        error_code(&mut session, "analysis.budget", json!({"detector": 0})),
        "invalid_request"
    );
    // A cursor off the screen and an empty ROI are the errors of the post-processing.
    assert_eq!(
        error_code(
            &mut session,
            "analysis.screen",
            json!({"cursor_mm": [9.0, 0.0]})
        ),
        "invalid_scene"
    );
    assert_eq!(
        error_code(
            &mut session,
            "analysis.screen",
            json!({"roi": {"minimum_mm": [0.1, 0.0], "maximum_mm": [0.0, 0.1]}})
        ),
        "invalid_scene"
    );
}

#[test]
fn stale_flips_when_the_document_changes() {
    let (mut session, _) = traced();
    let status = ok(&mut session, "session.status", json!({}));
    assert_eq!(
        status["run"],
        json!({"revision": 1, "stale": false, "detectors": 1})
    );
    assert_eq!(
        ok(&mut session, "analysis.budget", json!({}))["stale"],
        false
    );

    ok(
        &mut session,
        "doc.set",
        json!({"path": MOVE_SCREEN, "value": 70.0}),
    );
    let budget = ok(&mut session, "analysis.budget", json!({}));
    assert_eq!(budget["stale"], true);
    assert_eq!(budget["run_revision"], 1);
    let screen = ok(&mut session, "analysis.screen", json!({}));
    assert_eq!(
        (screen["stale"].as_bool(), screen["run_revision"].as_u64()),
        (Some(true), Some(1))
    );
    let status = ok(&mut session, "session.status", json!({}));
    assert_eq!(
        status["run"],
        json!({"revision": 1, "stale": true, "detectors": 1})
    );

    // Tracing again makes the record current; the old numbers are replaced.
    let before = ok(&mut session, "analysis.screen", json!({}))["peak_weight"].clone();
    ok(&mut session, "analysis.trace", trace_request(256, None));
    let after = ok(&mut session, "analysis.screen", json!({}));
    assert_eq!(after["stale"], false);
    assert_eq!(after["run_revision"], 2);
    assert_ne!(
        after["peak_weight"], before,
        "the moved screen sees other light"
    );
}

#[test]
fn without_a_run_the_analyses_report_no_run() {
    let mut session = Session::new();
    for command in ["analysis.budget", "analysis.screen"] {
        let response = exec(&mut session, command, json!({}));
        assert_eq!(response.error.as_ref().unwrap().code, "no_run", "{command}");
        assert!(response.error.unwrap().message.contains("計算結果"));
    }
    assert!(
        ok(&mut session, "session.status", json!({}))
            .get("run")
            .is_none()
    );
    // A document alone is not a calculation.
    ok(&mut session, "doc.open", json!({"path": LENS}));
    assert_eq!(
        error_code(&mut session, "analysis.budget", json!({})),
        "no_run"
    );
    // A one-shot trace of a given model also leaves its record, but not for the document.
    let mut other = Session::new();
    let mut request = trace_request(64, None);
    request["model"] = json!({"path": LENS});
    ok(&mut other, "analysis.trace", request);
    let budget = ok(&mut other, "analysis.budget", json!({}));
    assert!(budget["run_revision"].is_null());
    assert_eq!(budget["stale"], false);
}

#[test]
fn a_saved_record_is_analysed_in_a_fresh_session() {
    let dir = folder();
    let path = dir.join("lens.json");
    let (mut session, record) = traced();
    ok(
        &mut session,
        "analysis.trace",
        trace_request(256, Some(path.clone())),
    );
    let from_session = ok(
        &mut session,
        "analysis.screen",
        json!({"include_profiles": true}),
    );

    let mut fresh = Session::new();
    let from_file = ok(
        &mut fresh,
        "analysis.screen",
        json!({"run": {"path": path}, "include_profiles": true}),
    );
    assert!(from_file["run_revision"].is_null());
    assert_eq!(from_file["stale"], false);
    for key in [
        "name",
        "peak_weight",
        "x_slice",
        "y_slice",
        "received_percent_of_reference",
    ] {
        assert_eq!(from_file[key], from_session[key], "{key}");
    }
    // `run` does not replace the latest record of the session.
    assert_eq!(
        error_code(&mut fresh, "analysis.budget", json!({})),
        "no_run"
    );

    // An inline record, and a shared one, give the same numbers.
    let stored = RunRecord::open(&path).unwrap();
    let inline = ok(
        &mut fresh,
        "analysis.budget",
        json!({"run": {"record": stored}}),
    );
    let shared = fresh
        .run(
            ANALYSIS_BUDGET,
            AnalysisRequest::on(
                RecordInput::Shared(std::sync::Arc::new(stored)),
                BudgetSettings {},
            ),
        )
        .unwrap();
    assert_eq!(
        inline["unresolved_percent_of_reference"],
        shared.result.unresolved_percent_of_reference
    );
    assert_eq!(
        inline["entries"]["detected"]["percent_of_reference"],
        record.budget_percent_of_reference["detected"]
    );

    // A run given to the command wins over the latest record.
    let (mut session, _) = traced();
    ok(
        &mut session,
        "doc.set",
        json!({"path": MOVE_SCREEN, "value": 70.0}),
    );
    let given = ok(
        &mut session,
        "analysis.screen",
        json!({"run": {"path": path}}),
    );
    assert!(given["run_revision"].is_null());
    assert_eq!(given["stale"], false);
    assert_eq!(given["peak_weight"], from_session["peak_weight"]);

    let missing = error_code(
        &mut fresh,
        "analysis.budget",
        json!({"run": {"path": dir.join("none.json")}}),
    );
    assert_eq!(missing, "io");
}

#[test]
fn replay_and_run_open_become_the_latest_run() {
    let dir = folder();
    let path = dir.join("lens.json");
    let (mut session, _) = traced();
    ok(
        &mut session,
        "analysis.trace",
        trace_request(128, Some(path.clone())),
    );
    let expected = ok(&mut session, "analysis.screen", json!({}));

    let mut opened = Session::new();
    ok(&mut opened, "run.open", json!({"path": path}));
    let status = ok(&mut opened, "session.status", json!({}));
    assert_eq!(status["run"], json!({"stale": false, "detectors": 1}));
    let screen = ok(&mut opened, "analysis.screen", json!({}));
    assert!(screen["run_revision"].is_null());
    assert_eq!(screen["peak_weight"], expected["peak_weight"]);

    let mut replayed = Session::new();
    ok(
        &mut replayed,
        "run.replay",
        json!({"record": {"path": path}, "threads": 1}),
    );
    let screen = ok(&mut replayed, "analysis.screen", json!({}));
    assert!(screen["run_revision"].is_null());
    assert_eq!(screen["peak_weight"], expected["peak_weight"]);
}

#[test]
fn the_trace_summary_is_the_same_with_and_without_the_record() {
    let mut plain = Session::new();
    ok(&mut plain, "doc.open", json!({"path": LENS}));
    let mut full = plain.clone();
    let mut request = trace_request(64, None);
    request["include_record"] = json!(false);
    let without = ok(&mut plain, "analysis.trace", request);
    assert!(without.get("record").is_none());
    let with = ok(&mut full, "analysis.trace", trace_request(64, None));
    assert!(with["record"].is_object());
    for key in [
        "launched_rays",
        "detected_percent_of_reference",
        "budget_percent_of_reference",
        "detector_percent_of_reference",
        "relative_budget_closure_error",
        "warnings",
    ] {
        assert_eq!(without[key], with[key], "{key}");
    }
    // The record the analyses use is the same either way.
    assert_eq!(
        ok(&mut plain, "analysis.budget", json!({}))["entries"],
        ok(&mut full, "analysis.budget", json!({}))["entries"]
    );
}

#[test]
fn analyses_are_commands_with_schemas_and_cli_arguments() {
    for id in ["analysis.budget", "analysis.screen"] {
        let spec = registry().get(id).unwrap();
        assert_eq!(spec.kind, raytrace_rs::command::Kind::Query);
        let input = spec.input_schema().to_value();
        assert!(input["properties"]["run"].is_object(), "{id}: run");
        assert_eq!(input["additionalProperties"], false, "{id}");
        let output = spec.output_schema().to_value();
        for key in ["run_revision", "stale"] {
            assert!(output["properties"][key].is_object(), "{id}: {key}");
        }
    }
    let screen = registry()
        .get("analysis.screen")
        .unwrap()
        .input_schema()
        .to_value();
    for key in ["detector", "cursor_mm", "roi", "include_profiles"] {
        assert!(screen["properties"][key].is_object(), "{key}");
    }
    let mut session = Session::new();
    let listed = ok(&mut session, "command.list", json!({}));
    let ids: Vec<_> = listed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"analysis.budget") && ids.contains(&"analysis.screen"));
    let described = ok(
        &mut session,
        "command.describe",
        json!({"id": "analysis.screen"}),
    );
    assert_eq!(described["kind"], "query");

    // The generated request schema has one entry for each, with `run` among its fields.
    let request = ok(&mut session, "schema.get", json!({"name": "request"}));
    let variant = request["schema"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["title"] == "analysis.screen")
        .unwrap();
    assert!(variant["properties"]["run"].is_object());
    assert!(variant["properties"]["roi"].is_object());
}

#[test]
fn the_cli_runs_the_analyses_on_the_latest_run() {
    let dir = folder();
    let requests = [
        json!({"command": "doc.open", "path": LENS}),
        json!({"command": "analysis.trace", "overrides": {"rays_per_source": 64}, "threads": 1}),
        json!({"command": "analysis.budget"}),
        json!({"command": "analysis.screen", "detector": "0", "cursor_mm": [0.0, 0.0]}),
        json!({"command": "doc.set", "path": MOVE_SCREEN, "value": 70.0}),
        json!({"command": "analysis.screen"}),
    ]
    .map(|r| r.to_string())
    .join("\n");
    let (ok, responses) = cli(&dir, &["run", "-"], &requests);
    assert!(ok, "{responses:?}");
    assert_eq!(responses[2]["result"]["stale"], false);
    assert_eq!(
        responses[3]["result"]["slice_position_mm"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(responses[5]["result"]["stale"], true);

    // On the command line a one-shot command is a new session; `run` names the record.
    let record = dir.join("record.json");
    let (ok, saved) = cli(
        &dir,
        &[
            "analysis.trace",
            &format!("model.path={LENS}"),
            "overrides.rays_per_source:=64",
            "threads:=1",
            &format!("output={}", record.display()),
        ],
        "",
    );
    assert!(ok, "{saved:?}");
    let (ok, shown) = cli(
        &dir,
        &[
            "analysis.screen",
            &format!("run.path={}", record.display()),
            "detector=0",
            "roi:={\"minimum_mm\":[-0.1,-0.1],\"maximum_mm\":[0.1,0.1]}",
        ],
        "",
    );
    assert!(ok, "{shown:?}");
    assert!(shown[0]["result"]["roi"]["percent_of_reference"].is_number());
    let (ok, none) = cli(&dir, &["analysis.budget"], "");
    assert!(!ok);
    assert_eq!(none[0]["error"]["code"], "no_run");
}

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
