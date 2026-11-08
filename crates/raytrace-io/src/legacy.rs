//! Migration of the project files that preceded the design document
//! (`raytrace-project-v1` to `v3`) into [`Design`]s.
use crate::design::{
    Component, Conditions, Design, Occurrence, OccurrenceId, Part, PartId, PartKind, Provenance,
    RodSize,
};
use crate::{Result, zemax};
use raytrace_core::{
    issue::Issue,
    math::{Pose, V3},
    model::Scene,
};
use raytrace_formats::zemax::ZmxDocument;
use serde::Deserialize;
use std::path::Path;

/// Whether `format` names a legacy project file.
pub fn is_project_format(format: &serde_json::Value) -> bool {
    [
        "raytrace-project-v1",
        "raytrace-project-v2",
        "raytrace-project-v3",
    ]
    .iter()
    .any(|f| format == f)
}

#[derive(Deserialize)]
struct LegacyProject {
    #[serde(default)]
    assembly: Option<LegacyAssembly>,
    scene: Scene,
    material_names: Vec<String>,
    #[serde(default)]
    issues: Vec<Issue>,
    #[serde(default)]
    zemax: Option<ZmxDocument>,
    #[serde(default)]
    allow_uncoated: bool,
    #[serde(default)]
    monochromatic_override_um: Option<f64>,
    #[serde(default)]
    cad_deflection_mm: Option<f64>,
}
#[derive(Deserialize)]
struct LegacyAssembly {
    components: Vec<LegacyComponent>,
}
#[derive(Deserialize)]
struct LegacyComponent {
    name: String,
    enabled: bool,
    kind: PartKind,
    pose: Pose,
    anchor_mm: V3,
    project: Box<LegacyProject>,
    rod: Option<RodSize>,
}

fn conditions(project: &LegacyProject) -> Conditions {
    Conditions {
        allow_uncoated: project.allow_uncoated,
        monochromatic_override_um: project.monochromatic_override_um,
    }
}

fn part(
    id: &str,
    name: &str,
    kind: PartKind,
    project: LegacyProject,
    conditions: Conditions,
    rod: Option<RodSize>,
) -> Result<Part> {
    let provenance = project
        .zemax
        .as_ref()
        .map(|z| {
            Ok::<_, crate::Error>(Provenance {
                format: zemax::PROVENANCE_FORMAT.into(),
                document: serde_json::to_value(z).map_err(|e| crate::Error::json(None, e))?,
            })
        })
        .transpose()?;
    Ok(Part {
        id: PartId::new(id)?,
        name: name.into(),
        kind,
        scene: project.scene,
        material_names: project.material_names,
        issues: project.issues,
        conditions,
        rod,
        cad_deflection_mm: project.cad_deflection_mm,
        provenance,
    })
}

/// Convert the JSON of a legacy project. A standalone project becomes one `system` part
/// placed without transform; an assembly becomes one part and one occurrence per
/// component, numbered in component order.
pub(crate) fn migrate(value: serde_json::Value, path: &Path) -> Result<Design> {
    let project: LegacyProject = serde_json::from_value(value)
        .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
    migrate_project(project)
}

/// Same as [`migrate`] for a project embedded in a record, without a file path.
pub(crate) fn migrate_embedded(value: serde_json::Value) -> Result<Design> {
    let project: LegacyProject =
        serde_json::from_value(value).map_err(|e| crate::Error::json(None, e))?;
    migrate_project(project)
}

fn migrate_project(mut project: LegacyProject) -> Result<Design> {
    let Some(assembly) = project.assembly.take() else {
        let name = project.scene.name.clone();
        let flags = conditions(&project);
        let part = part(
            "system-0",
            &name,
            PartKind::System,
            project,
            Conditions::default(),
            None,
        )?;
        let mut design = Design::from_system_part(part);
        design.conditions = flags;
        return Ok(design);
    };
    let mut design = Design::empty();
    design.name = project.scene.name.clone();
    design.reference_plane = project.scene.reference_plane.clone();
    design.settings = project.scene.settings.clone();
    design.mode = project.scene.mode;
    design.conditions = conditions(&project);
    for c in assembly.components {
        if c.project.assembly.is_some() {
            return Err(crate::Error::Project(
                "入れ子の組立プロジェクトには対応していません".into(),
            ));
        }
        let own = conditions(&c.project);
        let part = part("system-0", &c.name, c.kind, *c.project, own, c.rod)?;
        let occurrence = Occurrence {
            id: OccurrenceId::new("system-0")?,
            part: part.id.clone(),
            name: c.name,
            enabled: c.enabled,
            pose: c.pose,
            anchor_mm: c.anchor_mm,
        };
        design.push_component(Component { part, occurrence });
    }
    Ok(design)
}

/// Point the occurrence references of a legacy compatibility report (zero-based component
/// indices under `component`) at the occurrences of the migrated design.
pub(crate) fn migrate_compatibility(value: &mut serde_json::Value, design: &Design) {
    let Some(components) = value
        .get_mut("components")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for entry in components {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        if let Some(index) = object.remove("component") {
            let id = index
                .as_u64()
                .and_then(|i| design.occurrences.get(i as usize))
                .map_or(serde_json::Value::Null, |o| o.id.as_str().into());
            object.insert("occurrence".into(), id);
        }
    }
}
