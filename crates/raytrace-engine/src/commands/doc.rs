//! `doc.*`: opening, reading, editing, saving and inspecting the document.
use super::resolve_model;
use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    engine::{
        DocGetRequest, DocGetResult, DocNewRequest, DocOpenRequest, DocOpened, DocReplaceRequest,
        DocSaveRequest, DocSetRequest, DocSetResult, DocSummary, EngineError, ExportZmxRequest,
        InspectRequest, InspectResult, Saved, create_parent,
    },
    pointer,
    session::Session,
};
use raytrace_io::{
    design::{self, Design},
    zemax,
};
use serde_json::Value;
use std::sync::Arc;

pub const DOC_NEW: Cmd<DocNewRequest, DocSummary> = Cmd::new("doc.new");
pub const DOC_OPEN: Cmd<DocOpenRequest, DocOpened> = Cmd::new("doc.open");
pub const DOC_SAVE: Cmd<DocSaveRequest, Saved> = Cmd::new("doc.save");
pub const DOC_GET: Cmd<DocGetRequest, DocGetResult> = Cmd::new("doc.get");
pub const DOC_INSPECT: Cmd<InspectRequest, InspectResult> = Cmd::new("doc.inspect");
pub const DOC_EXPORT_ZMX: Cmd<ExportZmxRequest, Saved> = Cmd::new("doc.export_zmx");
pub const DOC_SET: Cmd<DocSetRequest, DocSetResult> = Cmd::new("doc.set");
pub const DOC_REPLACE: Cmd<DocReplaceRequest, DocSummary> = Cmd::new("doc.replace");

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            DOC_NEW,
            "空の組立光学系をセッションの文書にする",
            Kind::Mutation,
            doc_new,
        ),
        command(
            DOC_OPEN,
            "ZMX・設計JSON・光学系JSONを開いてセッションの文書にする",
            Kind::Mutation,
            doc_open,
        ),
        command(
            DOC_SAVE,
            "設計文書をJSONとして保存する",
            Kind::Query,
            doc_save,
        ),
        command(
            DOC_GET,
            "文書、またはJSON Pointerで指定した一部を返す",
            Kind::Query,
            doc_get,
        ),
        command(
            DOC_INSPECT,
            "モデルを読み込み、互換性レポートを返す",
            Kind::Query,
            doc_inspect,
        ),
        command(
            DOC_EXPORT_ZMX,
            "単体のモデルをZMXとして書き出す",
            Kind::Query,
            doc_export_zmx,
        ),
        command(
            DOC_SET,
            "文書の1か所をJSON Pointerで指定して書き換える",
            Kind::Mutation,
            doc_set,
        ),
        command(
            DOC_REPLACE,
            "文書全体を置き換える",
            Kind::Mutation,
            doc_replace,
        ),
    ]
}

fn doc_new(
    session: &mut Session,
    r: DocNewRequest,
    _: &mut Context<'_>,
) -> Result<DocSummary, EngineError> {
    session.check_revision(r.expected_revision)?;
    let mut design = Design::empty();
    if let Some(name) = r.name {
        design.set_display_name(name);
    }
    let summary = DocSummary::of(&design);
    session.open_design(design, None, r.catalogs)?;
    Ok(summary)
}

fn doc_open(
    session: &mut Session,
    r: DocOpenRequest,
    _: &mut Context<'_>,
) -> Result<DocOpened, EngineError> {
    session.check_revision(r.expected_revision)?;
    if r.path.is_dir() {
        return Err(EngineError::new(
            "study_directory",
            "走査ディレクトリです。study.open で開いてください",
        ));
    }
    let catalogs = if r.catalogs.is_empty() {
        session.catalogs().to_vec()
    } else {
        r.catalogs
    };
    let design = match design::open(&r.path, &catalogs) {
        Ok(design) => design,
        // design::open recognises calculation records and refers them to replay.
        Err(raytrace_io::Error::Project(message)) if message.starts_with("計算記録は") => {
            return Err(EngineError::new(
                "run_record",
                "計算記録です。run.open で開くか、run.replay で再計算してください",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let summary = DocSummary::of(&design);
    let copy = r.include_design.then(|| Box::new(design.clone()));
    session.open_design(design, Some(r.path.clone()), catalogs)?;
    Ok(DocOpened {
        path: r.path,
        summary,
        design: copy,
    })
}

fn doc_save(
    session: &mut Session,
    r: DocSaveRequest,
    _: &mut Context<'_>,
) -> Result<Saved, EngineError> {
    let (design, of_session) = match r.design {
        Some(design) => (Arc::new(*design), false),
        None => (session.require_design()?.clone(), true),
    };
    let output = match r.output {
        Some(output) => output,
        None => session
            .path()
            .map(std::path::PathBuf::from)
            .filter(|_| of_session)
            .ok_or_else(|| {
                EngineError::invalid(
                    "output に保存先を指定してください（セッションの文書にはまだ保存先がありません）",
                )
            })?,
    };
    create_parent(&output)?;
    design::save(&design, &output)?;
    if of_session {
        session.mark_saved(output.clone());
    }
    Ok(Saved { output })
}

fn doc_get(
    session: &mut Session,
    r: DocGetRequest,
    _: &mut Context<'_>,
) -> Result<DocGetResult, EngineError> {
    let model = resolve_model(session, r.model, &[])?;
    let mut json = to_json(&model.design)?;
    let segments = pointer::parse(&r.path)?;
    let (value, path) = pointer::resolve(&mut json, &segments, false)?;
    Ok(DocGetResult {
        path,
        value: value.clone(),
    })
}

fn to_json(design: &Design) -> Result<Value, EngineError> {
    serde_json::to_value(design)
        .map_err(|e| EngineError::new("invalid_json", format!("文書をJSONにできません: {e}")))
}

fn doc_inspect(
    session: &mut Session,
    r: InspectRequest,
    _: &mut Context<'_>,
) -> Result<InspectResult, EngineError> {
    let model = resolve_model(session, r.model, &r.catalogs)?;
    let mut design = Arc::unwrap_or_clone(model.design);
    design.conditions.allow_uncoated |= r.ignore_coatings;
    if let Some(w) = r.wavelength_um {
        design.conditions.monochromatic_override_um = Some(w);
    }
    if let Some(path) = &r.export_zmx {
        zemax::export(&design, path)?;
    }
    if let Some(path) = &r.output {
        create_parent(path)?;
        design::save(&design, path)?;
    }
    let compatibility = design.compatibility_report();
    let compiled = design.compile().map_err(raytrace_io::Error::from)?;
    Ok(InspectResult {
        output: r.output,
        exported_zmx: r.export_zmx,
        sources: compiled.scene.sources.len(),
        surfaces: compiled.scene.surfaces.len(),
        materials: compiled.scene.materials.len(),
        traceable: compatibility.traceable,
        compatibility,
        design: r.include_design.then(|| Box::new(design)),
    })
}

fn doc_export_zmx(
    session: &mut Session,
    r: ExportZmxRequest,
    _: &mut Context<'_>,
) -> Result<Saved, EngineError> {
    let model = resolve_model(session, r.model, &r.catalogs)?;
    zemax::export(&model.design, &r.output)?;
    Ok(Saved { output: r.output })
}

fn protected(segments: &[String]) -> Option<&'static str> {
    match segments {
        [first, ..] if first == "format" => Some("format"),
        [first, ..] if first == "next_id" => Some("next_id"),
        [list, _, last] if (list == "parts" || list == "occurrences") && last == "id" => Some("id"),
        _ => None,
    }
}

fn ids(design: &Design) -> (Vec<&str>, Vec<&str>) {
    let mut parts: Vec<_> = design.parts.iter().map(|p| p.id.as_str()).collect();
    let mut occurrences: Vec<_> = design.occurrences.iter().map(|o| o.id.as_str()).collect();
    parts.sort_unstable();
    occurrences.sort_unstable();
    (parts, occurrences)
}

fn doc_set(
    session: &mut Session,
    r: DocSetRequest,
    _: &mut Context<'_>,
) -> Result<DocSetResult, EngineError> {
    let segments = pointer::parse(&r.path)?;
    if segments.is_empty() {
        return Err(EngineError::invalid(
            "パスが空です。文書全体の置き換えには doc.replace を使ってください",
        ));
    }
    if let Some(field) = protected(&segments) {
        return Err(EngineError::invalid(format!(
            "{field} は doc.set で変更できません。部品や配置の追加・削除・複製には component.add・component.remove・component.duplicate を使ってください"
        )));
    }
    let (path, summary) = session.edit(r.expected_revision, |design| {
        let mut json = to_json(design)?;
        let (target, path) = pointer::resolve(&mut json, &segments, true)?;
        *target = r.value;
        let next: Design = serde_json::from_value(json).map_err(|e| {
            EngineError::invalid(format!(
                "{path} を書き換えた文書が設計文書として読めません（schema.get name=design を確認してください）: {e}"
            ))
        })?;
        if next.format != design.format
            || next.next_id != design.next_id
            || ids(&next) != ids(design)
        {
            return Err(EngineError::invalid(
                "id・format・next_id は変更できません。部品や配置の追加・削除・複製には component.add・component.remove・component.duplicate を使ってください",
            ));
        }
        let summary = DocSummary::of(&next);
        Ok((next, (path, summary)))
    })?;
    Ok(DocSetResult { path, summary })
}

fn doc_replace(
    session: &mut Session,
    r: DocReplaceRequest,
    _: &mut Context<'_>,
) -> Result<DocSummary, EngineError> {
    let summary = DocSummary::of(&r.design);
    session.replace(r.expected_revision, *r.design)?;
    Ok(summary)
}
