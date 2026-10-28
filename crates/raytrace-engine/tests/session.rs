use raytrace_core::{
    math::V3,
    trace::{TraceOptions, trace_generated},
};
use raytrace_engine::{
    Request, Session,
    script::{self, Script},
};
use raytrace_io::{
    assembly::{self},
    project::Project,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

fn request(v: Value) -> Request {
    serde_json::from_value(v).unwrap()
}
fn apply(s: &mut Session, v: Value) -> raytrace_engine::Outcome {
    s.apply(request(v)).unwrap()
}
fn state(s: &Session) -> Value {
    serde_json::to_value(s.project()).unwrap()
}
fn folder() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/command_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[test]
fn schemas_and_json_types_describe_actual_requests() {
    let schema = raytrace_engine::schema();
    assert!(schema["$defs"].is_object());
    let r = request(
        json!({"expected_revision":0,"command":"set_property","target":"project","change":{"property":"name","value":"test"}}),
    );
    let text = serde_json::to_string(&r).unwrap();
    let _: Request = serde_json::from_str(&text).unwrap();
    for v in [
        json!({"command":"undo","unexpected":true}),
        json!({"command":"set_property","target":"project","change":{"property":"max_depth","value":"16"}}),
        json!({"command":"set_property","target":"project","change":{"property":"position_mm","value":[0,0]}}),
        json!({"command":"set_property","target":"project","change":{"property":"unknown","value":1}}),
    ] {
        assert!(
            serde_json::from_value::<Request>(v.clone()).is_err(),
            "accepted invalid request: {v}"
        );
    }
}

#[test]
fn failed_transactions_and_revision_conflicts_leave_everything_unchanged() {
    let mut s = Session::new(Project::new_assembly()).unwrap();
    let before = state(&s);
    let bad = json!({"command":"transaction","label":"bad","edits":[
        {"command":"add_detector","size_mm":[20,20],"pixels":[8,8],"alias":"screen"},
        {"command":"set_property","target":"screen","change":{"property":"pixels","value":[0,8]}}
    ]});
    assert!(s.apply(request(bad)).is_err());
    assert_eq!(state(&s), before);
    assert_eq!(s.revision(), 0);
    assert!(!s.can_undo());
    assert!(s.apply(request(json!({"expected_revision":1,"command":"set_property","target":"project","change":{"property":"name","value":"oops"}}))).is_err());
    assert_eq!(state(&s), before);
    let side_effect = folder().join("must_not_exist.json");
    assert!(s.apply(request(json!({"command":"transaction","label":"effects","edits":[{"command":"save","path":side_effect}]}))).is_err());
    assert!(!side_effect.exists());
}

#[test]
fn empty_assembly_can_be_built_and_ids_survive_duplicate_remove_disable_save_undo() {
    let mut s = Session::new(Project::new_assembly()).unwrap();
    let id = apply(
        &mut s,
        json!({"command":"add_detector","size_mm":[20,20],"pixels":[8,8],"alias":"screen"}),
    )
    .created[0]
        .clone();
    let first = s.project().bindings().unwrap();
    let copy = apply(
        &mut s,
        json!({"command":"duplicate_component","target":"screen","alias":"copy"}),
    )
    .created[0]
        .clone();
    assert_ne!(id, copy);
    apply(
        &mut s,
        json!({"command":"set_property","target":"screen","change":{"property":"enabled","value":false}}),
    );
    assert_eq!(s.project().bindings().unwrap().detectors.len(), 1);
    apply(
        &mut s,
        json!({"command":"remove_component","target":"copy"}),
    );
    assert!(s.project().bindings().unwrap().detectors.is_empty());
    apply(&mut s, json!({"command":"undo"}));
    apply(&mut s, json!({"command":"undo"}));
    assert_eq!(
        s.project().bindings().unwrap().detectors[0],
        first.detectors[0]
    );
    let path = folder().join("assembly.json");
    apply(&mut s, json!({"command":"save","path":path}));
    let loaded = Project::open(&path, &[]).unwrap();
    assert_eq!(loaded.bindings().unwrap(), s.project().bindings().unwrap());
    apply(&mut s, json!({"command":"undo"}));
    let next = apply(
        &mut s,
        json!({"command":"add_detector","size_mm":[10,10],"pixels":[4,4]}),
    )
    .created[0]
        .clone();
    assert_ne!(next, copy, "discarded IDs must not be reused");
}

#[test]
fn coalesced_gui_commits_and_undo_restore_matching_revisions() {
    let mut s = Session::new(assembly::demo_project().unwrap()).unwrap();
    let before = state(&s);
    for z in [1., 2., 3.] {
        let mut p = s.project().clone();
        p.assembly.as_mut().unwrap().components[1]
            .pose
            .translation_mm = V3([0., 0., z]);
        s.commit_project(p, "drag", None, Some("drag")).unwrap();
    }
    s.end_gesture();
    let after = state(&s);
    let revision = s.revision();
    apply(&mut s, json!({"command":"undo"}));
    assert_eq!(state(&s), before);
    assert_eq!(s.revision(), 0);
    assert!(!s.can_undo());
    apply(&mut s, json!({"command":"redo"}));
    assert_eq!(state(&s), after);
    assert_eq!(s.revision(), revision);
}

#[test]
fn all_legacy_formats_migrate_and_corrupt_v4_ids_are_rejected() {
    let base = assembly::demo_project().unwrap();
    for format in [
        "raytrace-project-v1",
        "raytrace-project-v2",
        "raytrace-project-v3",
    ] {
        let mut value = serde_json::to_value(&base).unwrap();
        value["format"] = format.into();
        value.as_object_mut().unwrap().remove("identity");
        for c in value["assembly"]["components"].as_array_mut().unwrap() {
            c.as_object_mut().unwrap().remove("id");
            c["project"].as_object_mut().unwrap().remove("identity");
        }
        let path = folder().join("old.json");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let p = Project::open(&path, &[]).unwrap();
        assert_eq!(p.format, "raytrace-project-v4");
        p.validate_identity().unwrap();
        assert_eq!(
            serde_json::to_value(p.trace_scene().unwrap()).unwrap(),
            serde_json::to_value(base.trace_scene().unwrap()).unwrap()
        );
        let mut bad = serde_json::to_value(&p).unwrap();
        bad["assembly"]["components"][1]["id"] = bad["assembly"]["components"][0]["id"].clone();
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(Project::open(&path, &[]).is_err());
    }
}

#[test]
fn session_trace_matches_existing_core_with_all_pixels_paths_and_terminals() {
    for mut p in [
        assembly::demo_project().unwrap(),
        Project::new(
            serde_json::from_str(include_str!("../../../examples/sequential_lens.json")).unwrap(),
        ),
    ] {
        for source in &mut p.scene.sources {
            source.count = 256;
        }
        if let Some(a) = &mut p.assembly {
            for c in &mut a.components {
                for source in &mut c.project.scene.sources {
                    source.count = 256;
                }
            }
        }
        p.refresh_assembly().unwrap();
        let prepared = p.trace_scene().unwrap().prepare().unwrap();
        let options = TraceOptions {
            keep_terminals: true,
            path_launch_count: 16,
        };
        let expected = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap()
            .install(|| trace_generated(&prepared, options))
            .unwrap();
        let mut s = Session::new(p).unwrap();
        let path = folder().join("run.json");
        apply(
            &mut s,
            json!({"command":"trace","threads":2,"paths":16,"terminals":true,"output":path,"alias":"run"}),
        );
        let complete = apply(&mut s, json!({"command":"wait","job":"run"}));
        assert_eq!(complete.data["status"], "completed");
        let actual = apply(
            &mut s,
            json!({"command":"query","query":{"query":"report","job":"run"}}),
        );
        assert_eq!(
            actual.data["report"],
            serde_json::to_value(&expected).unwrap()
        );
        let saved = raytrace_io::record::RunRecord::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(saved.replay().unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        apply(
            &mut s,
            json!({"command":"set_property","target":"project","change":{"property":"name","value":"new"}}),
        );
        assert_eq!(
            apply(
                &mut s,
                json!({"command":"query","query":{"query":"job","job":"run"}})
            )
            .data["stale"],
            true
        );
        apply(&mut s, json!({"command":"undo"}));
        assert_eq!(
            apply(
                &mut s,
                json!({"command":"query","query":{"query":"job","job":"run"}})
            )
            .data["stale"],
            false
        );
    }
}

#[test]
fn cancelling_before_completion_and_bad_optics_have_structured_job_outcomes() {
    let mut s = Session::new(assembly::demo_project().unwrap()).unwrap();
    apply(
        &mut s,
        json!({"command":"trace","rays_per_source":200_000,"alias":"cancel"}),
    );
    apply(&mut s, json!({"command":"cancel","job":"cancel"}));
    assert_eq!(
        apply(&mut s, json!({"command":"wait","job":"cancel"})).data["status"],
        "cancelled"
    );
    assert!(
        s.apply(request(json!({"command":"trace","threads":0})))
            .is_err()
    );
    let mut bad = Session::new(Project::new_assembly()).unwrap();
    assert!(bad.apply(request(json!({"command":"trace"}))).is_err());
}

#[test]
fn persistent_json_transport_survives_bad_requests_and_returns_revisions() {
    let input=b"{\"command\":\"nonsense\"}\n{\"command\":\"set_property\",\"target\":\"project\",\"change\":{\"property\":\"name\",\"value\":\"ok\"}}\n{\"command\":\"query\",\"query\":{\"query\":\"project\"}}\n";
    let mut out = Vec::new();
    script::serve(&input[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["ok"], false);
    assert_eq!(lines[2]["result"]["data"]["name"], "ok");
}

#[test]
fn transactions_restore_all_aliases_and_changes_as_one_edit() {
    let mut s = Session::new(Project::new_assembly()).unwrap();
    let before = state(&s);
    let commands = vec![request(
        json!({"command":"transaction","label":"assembly","edits":[
            {"command":"add_rod","material":{"type":"constant","n":1.5},"size_mm":[2,2,10],"alias":"rod"},
            {"command":"set_property","target":"rod","change":{"property":"position_mm","value":[0,0,20]}},
            {"command":"add_detector","size_mm":[10,10],"pixels":[8,8],"alias":"screen"}
        ]}),
    )];
    assert!(
        script::execute(
            &mut s,
            Script {
                format: script::SCRIPT_FORMAT.into(),
                commands
            }
        )
        .unwrap()
        .success
    );
    apply(&mut s, json!({"command":"undo"}));
    assert_eq!(state(&s), before);
    assert!(!s.can_undo());
    apply(&mut s, json!({"command":"redo"}));
    assert!(s.apply(request(json!({"command":"set_property","target":"rod","change":{"property":"rod_size_mm","value":[1,2,20]}}))).is_ok());
}

#[test]
fn relative_zmx_script_retains_dispersion_and_imported_definitions_after_save() {
    let dir = folder();
    std::fs::write(dir.join("source.zmx"),"MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_SDIO 0 0 Test\n NSOP 0 0 -5 0 0 0\n NSOD 2 128\n NSOD 3 1\n NSOD 7 2\n NSOD 9 2\n").unwrap();
    std::fs::write(dir.join("lens.zmx"),"MODE SEQ\nUNIT MM\nGCAT TEST\nWAVM 1 0.55 1\nSURF 0\n TYPE STANDARD\n DISZ INFINITY\nSURF 1\n TYPE STANDARD\n CURV 0.02\n GLAS TESTGLASS 0 0\n DISZ 3\n DIAM 2\nSURF 2\n TYPE STANDARD\n CURV -0.02\n DISZ 30\n DIAM 2\nSURF 3\n TYPE STANDARD\n DISZ 0\n DIAM 10\n").unwrap();
    std::fs::write(
        dir.join("TEST.AGF"),
        "NM TESTGLASS 1 0 1.5 50 0 0\nCD 2.25 0 0.01 0 0 0\nLD 0.4 0.7\n",
    )
    .unwrap();
    let script = json!({"format":"raytrace-script-v1","commands":[
        {"command":"add_component","path":"source.zmx","kind":"source","alias":"source"},
        {"command":"add_component","path":"lens.zmx","kind":"optic","alias":"lens"},
        {"command":"set_property","target":"source","change":{"property":"position_mm","value":[0,0,-5]}},
        {"command":"add_rod","material":{"type":"catalog","directory":".","catalog":"TEST","name":"TESTGLASS"},"size_mm":[3,3,5],"alias":"rod"},
        {"command":"set_property","target":"rod","change":{"property":"position_mm","value":[0,0,8]}},
        {"command":"add_detector","size_mm":[20,20],"pixels":[8,8],"alias":"screen"},
        {"command":"set_property","target":"screen","change":{"property":"position_mm","value":[0,0,20]}},
        {"command":"save","path":"saved.json"},
        {"command":"trace","threads":2,"output":"trace.json","alias":"run"},
        {"command":"wait","job":"run"}
    ]});
    let path = dir.join("script.json");
    std::fs::write(&path, serde_json::to_vec(&script).unwrap()).unwrap();
    let result = script::run_file(&path).unwrap();
    assert!(
        result.success,
        "{}",
        serde_json::to_string(&result).unwrap()
    );
    let saved = Project::open(&dir.join("saved.json"), &[]).unwrap();
    let rod = &saved.assembly.as_ref().unwrap().components[2];
    let m = &rod.project.scene.materials[1];
    assert_ne!(m.index(0.45), m.index(0.65));
    std::fs::remove_file(dir.join("source.zmx")).unwrap();
    std::fs::remove_file(dir.join("lens.zmx")).unwrap();
    std::fs::remove_file(dir.join("TEST.AGF")).unwrap();
    let record = raytrace_io::record::RunRecord::open(&dir.join("trace.json")).unwrap();
    assert_eq!(
        serde_json::to_value(record.replay().unwrap()).unwrap(),
        serde_json::to_value(record.report).unwrap()
    );
}

#[test]
fn scan_ids_follow_parts_after_reorder_and_reject_a_removed_target() {
    use raytrace_app::scan::{Parameter, ScanManifest, ScanPlan};
    let mut p = assembly::demo_project().unwrap();
    let rod = p.assembly.as_ref().unwrap().components[2].id.clone();
    let plan = ScanPlan {
        parameter: Parameter::ComponentPosition {
            component: 2,
            axis: 2,
        },
        values: vec![68., 69.],
        ..Default::default()
    };
    let manifest = ScanManifest::new(&p, plan).unwrap();
    let parameter = manifest.plan.parameter;
    assert!(matches!(parameter, Parameter::ComponentPositionId { .. }));
    p.assembly.as_mut().unwrap().components.swap(0, 2);
    p.refresh_assembly().unwrap();
    parameter.apply(&mut p, 77.).unwrap();
    assert_eq!(
        p.assembly
            .as_ref()
            .unwrap()
            .components
            .iter()
            .find(|c| c.id == rod)
            .unwrap()
            .pose
            .translation_mm
            .0[2],
        77.
    );
    let path = folder().join("reordered.json");
    p.save(&path).unwrap();
    let mut saved = Project::open(&path, &[]).unwrap();
    assert_eq!(parameter.value(&saved).unwrap(), 77.);
    saved
        .assembly
        .as_mut()
        .unwrap()
        .components
        .retain(|c| c.id != rod);
    saved.refresh_assembly().unwrap();
    assert!(parameter.apply(&mut saved, 88.).is_err());
}

#[test]
fn prepared_import_edits_commit_atomically_and_reject_edits_even_after_undo() {
    let mut session = Session::new(Project::new_assembly()).unwrap();
    let command =
        json!({"command":"add_detector","size_mm":[20,20],"pixels":[8,8],"alias":"screen"});
    let preparation = session.prepare_edit(request(command.clone())).unwrap();
    let before = state(&session);
    let prepared = preparation.run().unwrap();
    assert_eq!(state(&session), before);
    session.apply_prepared(prepared).unwrap();
    assert_eq!(
        session
            .project()
            .assembly
            .as_ref()
            .unwrap()
            .components
            .len(),
        1
    );
    let prepared=session.prepare_edit(request(json!({"command":"set_property","target":"screen","change":{"property":"position_mm","value":[0,0,20]}}))).unwrap().run().unwrap();
    apply(
        &mut session,
        json!({"command":"set_property","target":"project","change":{"property":"name","value":"changed"}}),
    );
    apply(&mut session, json!({"command":"undo"}));
    assert_eq!(
        session.apply_prepared(prepared).unwrap_err().code,
        "revision_conflict"
    );
    assert_eq!(
        session.project().assembly.as_ref().unwrap().components[0]
            .pose
            .translation_mm
            .0,
        [0., 0., 0.]
    );
}

#[test]
fn discarded_ids_stay_reserved_after_undo_save_and_reopen() {
    let mut session = Session::new(Project::new_assembly()).unwrap();
    let old = apply(
        &mut session,
        json!({"command":"add_detector","size_mm":[2,2],"pixels":[4,4]}),
    )
    .created[0]
        .clone();
    apply(&mut session, json!({"command":"undo"}));
    let path = folder().join("reserved.json");
    apply(&mut session, json!({"command":"save","path":path}));
    let mut reopened = Session::new(Project::open(&path, &[]).unwrap()).unwrap();
    let next = apply(
        &mut reopened,
        json!({"command":"add_detector","size_mm":[2,2],"pixels":[4,4]}),
    )
    .created[0]
        .clone();
    assert_ne!(old, next);
}

#[test]
fn json_study_jobs_keep_the_current_model_and_save_replayable_cases() {
    let mut session = Session::new(assembly::demo_project().unwrap()).unwrap();
    let before = state(&session);
    let dir = folder();
    let rod = session.project().assembly.as_ref().unwrap().components[2]
        .id
        .to_string();
    let scan = dir.join("scan_plan.json");
    std::fs::write(&scan,serde_json::to_vec(&json!({"parameter":{"type":"component_position_id","component":rod,"axis":2},"values":[68.5,69.],"threads":2,"rays_per_source":32})).unwrap()).unwrap();
    apply(
        &mut session,
        json!({"command":"scan","plan":scan,"output":dir.join("scan"),"alias":"scan"}),
    );
    let scan = apply(&mut session, json!({"command":"wait","job":"scan"}));
    assert_eq!(scan.data["status"], "completed");
    assert_eq!(scan.data["study"]["cases"].as_array().unwrap().len(), 2);
    let manifest = raytrace_app::scan::ScanManifest::open(&dir.join("scan")).unwrap();
    assert_eq!(manifest.cases.len(), 2);
    let convergence = dir.join("convergence_plan.json");
    std::fs::write(&convergence,serde_json::to_vec(&json!({"ray_counts":[16,32],"seeds":[42],"pixels":[[8,8]],"cad_deflections_mm":[0.02],"threads":2,"save_runs":false})).unwrap()).unwrap();
    apply(
        &mut session,
        json!({"command":"convergence","plan":convergence,"output":dir.join("convergence"),"alias":"convergence"}),
    );
    let c = apply(&mut session, json!({"command":"wait","job":"convergence"}));
    assert_eq!(c.data["status"], "completed");
    assert_eq!(
        c.data["study"]["summary"]["cases"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(state(&session), before);
    let control = raytrace_core::trace::TraceControl::default();
    control.cancel.store(true, Ordering::Relaxed);
    assert!(
        raytrace_app::convergence::run_project_controlled(
            session.project(),
            &Default::default(),
            &dir.join("cancelled"),
            &control
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn legacy_scan_can_be_reopened_twice_without_rewriting_completed_run_files() {
    use raytrace_app::scan::{Parameter, ScanManifest, ScanPlan};
    let scene =
        serde_json::from_str(include_str!("../../../examples/sequential_lens.json")).unwrap();
    let p = Project::new(scene);
    let dir = folder();
    let plan = ScanPlan {
        parameter: Parameter::SurfacePosition {
            surface: 2,
            axis: 2,
        },
        values: vec![68., 69.],
        rays_per_source: Some(16),
        threads: 2,
        ..Default::default()
    };
    let mut manifest = ScanManifest::new(&p, plan.clone()).unwrap();
    manifest.execute(&dir, &Default::default(), |_| {}).unwrap();
    fn legacy(value: &mut Value) {
        value["format"] = if value["assembly"].is_object() {
            "raytrace-project-v2".into()
        } else {
            "raytrace-project-v1".into()
        };
        value.as_object_mut().unwrap().remove("identity");
        if let Some(parts) = value
            .get_mut("assembly")
            .and_then(|a| a.get_mut("components"))
            .and_then(Value::as_array_mut)
        {
            for c in parts {
                c.as_object_mut().unwrap().remove("id");
                legacy(&mut c["project"]);
            }
        }
    }
    let mut value = serde_json::to_value(&manifest).unwrap();
    legacy(&mut value["base"]);
    value["plan"] = serde_json::to_value(plan).unwrap();
    std::fs::write(dir.join("scan.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    let mut original = Vec::new();
    for i in 0..2 {
        let path = dir.join(format!("case_{i:04}.raytrace-run.json"));
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        legacy(&mut value["calculation"]["project"]);
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        original.push((path, bytes));
    }
    let loaded = ScanManifest::open(&dir).unwrap();
    loaded.save(&dir).unwrap();
    let loaded = ScanManifest::open(&dir).unwrap();
    assert_eq!(loaded.cases.len(), 2);
    for (path, bytes) in original {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
