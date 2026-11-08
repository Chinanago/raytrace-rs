//! `study.*`: parameter scans, convergence studies and scan directories.
use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    commands::resolve_model,
    convergence,
    engine::{
        ConvergenceRequest, ConvergenceResult, EngineError, ModelInput, Progress,
        ResumeScanRequest, ScanRequest, ScanResult, StudyOpenRequest, StudyOpened,
    },
    scan,
    session::Session,
};
use std::path::PathBuf;

pub const STUDY_SCAN: Cmd<ScanRequest, ScanResult> = Cmd::new("study.scan");
pub const STUDY_RESUME: Cmd<ResumeScanRequest, ScanResult> = Cmd::new("study.resume");
pub const STUDY_CONVERGENCE: Cmd<ConvergenceRequest, ConvergenceResult> =
    Cmd::new("study.convergence");
pub const STUDY_OPEN: Cmd<StudyOpenRequest, StudyOpened> = Cmd::new("study.open");

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            STUDY_SCAN,
            "パラメーター走査を新しいディレクトリに実行する",
            Kind::Job,
            study_scan,
        )
        .with_failure(scan_failure),
        command(
            STUDY_RESUME,
            "走査ディレクトリの未完了ケースを続行する",
            Kind::Job,
            study_resume,
        )
        .with_failure(scan_failure),
        command(
            STUDY_CONVERGENCE,
            "光線数・seed・画素数・CAD分割精度を変えて追跡し、収束を確認する",
            Kind::Job,
            study_convergence,
        )
        .with_failure(|r: &ConvergenceResult| {
            (r.failed > 0).then(|| {
                format!(
                    "収束確認で{}ケースが失敗しました。{}を確認してください",
                    r.failed,
                    r.output.display()
                )
            })
        }),
        command(
            STUDY_OPEN,
            "走査ディレクトリのscan.jsonを読み込んで返す",
            Kind::Query,
            study_open,
        ),
    ]
}

fn scan_failure(r: &ScanResult) -> Option<String> {
    (r.failed > 0).then(|| {
        format!(
            "走査で{}ケースが失敗しました。{}を確認してください",
            r.failed,
            r.output.display()
        )
    })
}

fn study_scan(
    session: &mut Session,
    mut r: ScanRequest,
    context: &mut Context<'_>,
) -> Result<ScanResult, EngineError> {
    if r.output.join("scan.json").exists() {
        return Err(EngineError::invalid(
            "走査の出力先は既に存在します。study.resume で再開するか、新しいディレクトリを指定してください",
        ));
    }
    let model = resolve_model(session, r.model.take(), &r.catalogs)?;
    let manifest = scan::ScanManifest::new(&model.design, r.plan)?;
    run_scan(manifest, r.output, r.include_manifest, context)
}

fn study_resume(
    _: &mut Session,
    r: ResumeScanRequest,
    context: &mut Context<'_>,
) -> Result<ScanResult, EngineError> {
    let manifest = scan::ScanManifest::open(&r.directory)?;
    run_scan(manifest, r.directory, r.include_manifest, context)
}

fn run_scan(
    mut manifest: scan::ScanManifest,
    directory: PathBuf,
    include_manifest: bool,
    context: &mut Context<'_>,
) -> Result<ScanResult, EngineError> {
    let control = context.control;
    manifest.execute(&directory, control, |m| context.progress(Progress::Scan(m)))?;
    let count = |state: &str| manifest.cases.iter().filter(|c| c.state == state).count();
    Ok(ScanResult {
        output: directory.join("scan.json"),
        cases: manifest.cases.len(),
        completed: count("completed"),
        failed: count("failed"),
        directory,
        manifest: include_manifest.then(|| Box::new(manifest)),
    })
}

fn study_convergence(
    session: &mut Session,
    r: ConvergenceRequest,
    _: &mut Context<'_>,
) -> Result<ConvergenceResult, EngineError> {
    let model = match r.model {
        Some(ModelInput::Path(path)) => path,
        Some(_) => {
            return Err(EngineError::invalid(
                "収束確認のモデルはファイル（model.path）で指定してください",
            ));
        }
        None => {
            session.require_design()?;
            match session.path() {
                Some(path) if !session.is_dirty() => path.to_path_buf(),
                Some(_) => {
                    return Err(EngineError::invalid(
                        "収束確認はファイルを読み直して比較します。未保存の変更があるため、先に doc.save で保存するか、model.path を指定してください",
                    ));
                }
                None => {
                    return Err(EngineError::invalid(
                        "収束確認はファイルを読み直して比較します。文書を doc.save で保存するか、model.path を指定してください",
                    ));
                }
            }
        }
    };
    let report = convergence::run(&model, &r.plan, &r.output)?;
    Ok(ConvergenceResult {
        output: r.output.join("convergence.json"),
        cases: report.cases.len(),
        comparisons: report.comparisons.len(),
        failed: report.cases.iter().filter(|c| c.error.is_some()).count(),
        report: r.include_report.then(|| Box::new(report)),
    })
}

fn study_open(
    _: &mut Session,
    r: StudyOpenRequest,
    _: &mut Context<'_>,
) -> Result<StudyOpened, EngineError> {
    Ok(StudyOpened {
        manifest: Box::new(scan::ScanManifest::open(&r.path)?),
    })
}
