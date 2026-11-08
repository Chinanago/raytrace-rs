//! `component.*`: structural edits that hand out ids and keep references consistent.
use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    engine::{
        ComponentAddRequest, ComponentAdded, ComponentChanged, ComponentRef, DocSummary,
        EngineError,
    },
    session::Session,
};
use raytrace_core::model::Scene;
use raytrace_io::design::{self, Component, Design, PartKind};

pub const COMPONENT_ADD: Cmd<ComponentAddRequest, ComponentAdded> = Cmd::new("component.add");
pub const COMPONENT_REMOVE: Cmd<ComponentRef, ComponentChanged> = Cmd::new("component.remove");
pub const COMPONENT_DUPLICATE: Cmd<ComponentRef, ComponentChanged> =
    Cmd::new("component.duplicate");

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            COMPONENT_ADD,
            "ファイルを部品として取り込む、またはロッド・スクリーンを追加する",
            Kind::Mutation,
            component_add,
        ),
        command(
            COMPONENT_REMOVE,
            "配置を削除する（他に使われない部品も削除）",
            Kind::Mutation,
            component_remove,
        ),
        command(
            COMPONENT_DUPLICATE,
            "配置とその部品を複製する",
            Kind::Mutation,
            component_duplicate,
        ),
    ]
}

/// Fused silica of `examples/integrator_rod.json`, the default material of a rod.
fn silica() -> Result<raytrace_core::model::Material, EngineError> {
    let scene: Scene = serde_json::from_str(include_str!(
        "../../../../examples/integrator_rod.json"
    ))
    .map_err(|e| EngineError::new("invalid_json", format!("ロッドの既定材料を読めません: {e}")))?;
    scene
        .materials
        .get(1)
        .cloned()
        .ok_or_else(|| EngineError::new("invalid_scene", "ロッドの既定材料がありません"))
}

fn component_add(
    session: &mut Session,
    r: ComponentAddRequest,
    _: &mut Context<'_>,
) -> Result<ComponentAdded, EngineError> {
    session.check_revision(r.expected_revision)?;
    session.require_design()?;
    let mut component = match (r.path, r.kind) {
        (Some(path), kind) => {
            let kind = kind.unwrap_or(PartKind::Optic);
            if matches!(kind, PartKind::Rod | PartKind::Detector) {
                return Err(EngineError::invalid(
                    "ファイルから取り込む部品の kind は source・optic・system のいずれかです",
                ));
            }
            let catalogs = if r.catalogs.is_empty() {
                session.catalogs()
            } else {
                &r.catalogs
            };
            design::imported_component(design::open(&path, catalogs)?, kind)?
        }
        (None, Some(PartKind::Rod)) => Component::rod(
            match r.material {
                Some(material) => material,
                None => silica()?,
            },
            r.material_name.unwrap_or_else(|| "Silica".into()),
        ),
        (None, Some(PartKind::Detector)) => Component::detector(),
        (None, _) => {
            return Err(EngineError::invalid(
                "path にファイルを指定するか、kind に rod か detector を指定してください",
            ));
        }
    };
    if let Some(name) = r.name {
        component.occurrence.name = name;
    }
    session.edit(r.expected_revision, |current| {
        let mut next = current.clone();
        let id = next.add_component(component)?;
        let occurrence = next
            .occurrence(&id)
            .ok_or_else(|| EngineError::new("internal", "追加した配置が見つかりません"))?;
        let added = ComponentAdded {
            part: occurrence.part.as_str().into(),
            occurrence: id.as_str().into(),
            summary: DocSummary::of(&next),
        };
        Ok((next, added))
    })
}

fn occurrence_index(design: &Design, id: &str) -> Result<usize, EngineError> {
    design
        .occurrences
        .iter()
        .position(|o| o.id.as_str() == id)
        .ok_or_else(|| EngineError::invalid(format!("配置 {id} がありません")))
}

fn component_remove(
    session: &mut Session,
    r: ComponentRef,
    _: &mut Context<'_>,
) -> Result<ComponentChanged, EngineError> {
    session.edit(r.expected_revision, |current| {
        let mut next = current.clone();
        let index = occurrence_index(current, &r.occurrence)?;
        next.remove_occurrence(index);
        let changed = ComponentChanged {
            occurrence: r.occurrence.clone(),
            summary: DocSummary::of(&next),
        };
        Ok((next, changed))
    })
}

fn component_duplicate(
    session: &mut Session,
    r: ComponentRef,
    _: &mut Context<'_>,
) -> Result<ComponentChanged, EngineError> {
    session.edit(r.expected_revision, |current| {
        let mut next = current.clone();
        let index = occurrence_index(current, &r.occurrence)?;
        let id = next.duplicate_occurrence(index)?;
        let changed = ComponentChanged {
            occurrence: id.as_str().into(),
            summary: DocSummary::of(&next),
        };
        Ok((next, changed))
    })
}
