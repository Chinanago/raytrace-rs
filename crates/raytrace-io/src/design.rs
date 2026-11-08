//! Design files (`raytrace-design-v1`) and the import of other documents into designs.
use crate::{Result, legacy, zemax};
pub use raytrace_core::issue::Issue;
use raytrace_core::model::{Mode, Scene};
pub use raytrace_doc::{
    Compiled, Component, Conditions, Design, FORMAT, Occurrence, OccurrenceId, Part, PartId,
    PartKind, Provenance, RodSize, air, default_material_names, demo_design, empty_scene,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Read a design from `.zmx`, a design file, a legacy project file (`raytrace-project-v1`
/// to `v3`) or a scene JSON such as `examples/*.json`.
pub fn open(path: &Path, catalogs: &[PathBuf]) -> Result<Design> {
    open_with_cad_tolerance(path, catalogs, 0.02)
}

pub fn open_with_cad_tolerance(
    path: &Path,
    catalogs: &[PathBuf],
    deflection_mm: f64,
) -> Result<Design> {
    if !deflection_mm.is_finite() || deflection_mm <= 0.0 {
        return Err(crate::Error::Project(
            "CAD分割精度は正の有限値で指定してください".into(),
        ));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zmx"))
    {
        return zemax::read_zmx_with_cad_tolerance(path, catalogs, deflection_mm);
    }
    let bytes = fs::read(path).map_err(|e| crate::Error::io(path, e))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?;
    let json_error = |e| crate::Error::json(Some(path.to_path_buf()), e);
    let Some(format) = value.get("format") else {
        return Ok(Design::from_scene(
            serde_json::from_value::<Scene>(value).map_err(json_error)?,
        ));
    };
    if format == crate::record::RUN_FORMAT {
        return Err(crate::Error::Project(
            "計算記録は replay コマンドで再計算できます。配置の編集には設計JSONを開いてください"
                .into(),
        ));
    }
    let design = if format == FORMAT {
        serde_json::from_value(value).map_err(json_error)?
    } else if legacy::is_project_format(format) {
        legacy::migrate(value, path)?
    } else {
        return Err(crate::Error::Project("未対応のプロジェクト形式です".into()));
    };
    // Opening checks that the design compiles, like composing did for assemblies.
    check(&design)?;
    Ok(design)
}

/// Write the design as pretty JSON after checking that it compiles.
pub fn save(design: &Design, path: &Path) -> Result<()> {
    check(design)?;
    fs::write(
        path,
        serde_json::to_vec_pretty(design)
            .map_err(|e| crate::Error::json(Some(path.to_path_buf()), e))?,
    )
    .map_err(|e| crate::Error::io(path, e))
}

/// Check ids, references and composition.
pub fn check(design: &Design) -> Result<()> {
    design.compile()?;
    Ok(())
}

/// Turn a standalone design into a component of `kind`. A source part cannot come from a
/// sequential ZMX, because that file has no light-source definition.
pub fn imported_component(design: Design, kind: PartKind) -> Result<Component> {
    if kind == PartKind::Source
        && let Some(part) = design.lone_system()
        && zemax::document(part)?.is_some_and(|z| z.baseline_scene.mode == Mode::Sequential)
    {
        return Err(crate::Error::Project(
            "光源部品にはNSC光源ZMX、または光源を定義したJSONを指定してください".into(),
        ));
    }
    Component::imported(design, kind).map_err(Into::into)
}
