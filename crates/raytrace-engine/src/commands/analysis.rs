//! `analysis.trace` and the commands on calculation records (`run.*`).
use super::resolve_model;
use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    engine::{
        EngineError, ExportScreenRequest, ReplayRequest, RunOpenRequest, RunOpened, SaveRunRequest,
        Saved, TraceRequest, TraceResult, load_record, run,
    },
    session::Session,
};
use raytrace_io::record::RunRecord;
use std::{fs, path::PathBuf, sync::Arc};

pub const ANALYSIS_TRACE: Cmd<TraceRequest, TraceResult> = Cmd::new("analysis.trace");
pub const RUN_REPLAY: Cmd<ReplayRequest, TraceResult> = Cmd::new("run.replay");
pub const RUN_OPEN: Cmd<RunOpenRequest, RunOpened> = Cmd::new("run.open");
pub const RUN_SAVE: Cmd<SaveRunRequest, Saved> = Cmd::new("run.save");
pub const RUN_EXPORT_SCREEN: Cmd<ExportScreenRequest, Saved> = Cmd::new("run.export_screen");

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            ANALYSIS_TRACE,
            "モデルを追跡し、計算記録を返す（output で保存）",
            Kind::Job,
            analysis_trace,
        ),
        command(
            RUN_REPLAY,
            "計算記録を記録された光学条件で再計算する",
            Kind::Job,
            run_replay,
        ),
        command(RUN_OPEN, "計算記録を読み込んで返す", Kind::Query, run_open),
        command(
            RUN_SAVE,
            "計算記録をJSONとして保存する",
            Kind::Query,
            run_save,
        ),
        command(
            RUN_EXPORT_SCREEN,
            "計算記録の1つのスクリーンをCSV・PNG・条件ファイルに書き出す",
            Kind::Query,
            run_export_screen,
        ),
    ]
}

fn analysis_trace(
    session: &mut Session,
    mut r: TraceRequest,
    context: &mut Context<'_>,
) -> Result<TraceResult, EngineError> {
    let model = resolve_model(session, r.model.take(), &r.catalogs)?;
    run::trace(
        r,
        Arc::unwrap_or_clone(model.design),
        model.path,
        context.control,
    )
}

fn run_replay(
    _: &mut Session,
    r: ReplayRequest,
    context: &mut Context<'_>,
) -> Result<TraceResult, EngineError> {
    run::replay(r, context.control)
}

fn run_open(
    _: &mut Session,
    r: RunOpenRequest,
    _: &mut Context<'_>,
) -> Result<RunOpened, EngineError> {
    let mut record = RunRecord::open(&r.path)?;
    if r.viewer_paths {
        record.report.paths = run::viewer_paths(&record)?;
    }
    Ok(RunOpened {
        record: Box::new(record),
    })
}

fn run_save(_: &mut Session, r: SaveRunRequest, _: &mut Context<'_>) -> Result<Saved, EngineError> {
    r.record
        .calculation
        .save(&r.record.report, r.record.trace_seconds, &r.output)?;
    Ok(Saved { output: r.output })
}

fn run_export_screen(
    _: &mut Session,
    r: ExportScreenRequest,
    _: &mut Context<'_>,
) -> Result<Saved, EngineError> {
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
    let path: PathBuf = r.output.join("conditions.json");
    let bytes = serde_json::to_vec_pretty(&conditions)
        .map_err(|e| raytrace_io::Error::json(Some(path.clone()), e))?;
    fs::write(&path, bytes).map_err(|e| EngineError::io(&path, e))?;
    Ok(Saved { output: r.output })
}
