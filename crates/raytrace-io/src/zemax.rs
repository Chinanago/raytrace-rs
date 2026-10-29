//! Native ZMX/AGF parsing. Unknown data stays in the original document.
use crate::Result;
use crate::project::Project;
use raytrace_core::model::Material;
pub use raytrace_formats::zemax::{TextEncoding, ZmxDocument, decode};
use std::path::{Path, PathBuf};

/// Read one glass through the native AGF parser without importing geometry.
pub fn read_catalog_material(directory: &Path, catalog: &str, name: &str) -> Result<Material> {
    raytrace_formats::zemax::read_catalog_material(directory, catalog, name).map_err(Into::into)
}

pub fn read_spcd(path: &Path) -> Result<raytrace_core::spectrum::Spectrum> {
    raytrace_formats::zemax::read_spcd(path).map_err(Into::into)
}

pub fn read_zmx(path: &Path, extra: &[PathBuf]) -> Result<Project> {
    read_zmx_with_cad_tolerance(path, extra, 0.02)
}
pub fn read_zmx_with_cad_tolerance(
    path: &Path,
    extra: &[PathBuf],
    deflection_mm: f64,
) -> Result<Project> {
    let model = raytrace_formats::zemax::read_zmx_with_cad_tolerance(path, extra, deflection_mm)?;
    let mut project = Project::new(model.scene);
    project.material_names = model.material_names;
    project.issues = model.issues;
    project.zemax = model.zemax;
    project.cad_deflection_mm = model.cad_deflection_mm;
    project.refresh_assembly()?;
    Ok(project)
}

/// Preserve the complete source text on an unchanged document. Geometry edits
/// are patched only for supported coaxial sequential surfaces.
pub fn export(project: &Project, path: &Path) -> Result<()> {
    if project.assembly.is_some() {
        return Err(crate::Error::zmx(
            "組立光学系はJSONプロジェクトで保存できます。部品を含むZMX書出しは追加予定です".into(),
        ));
    }
    raytrace_formats::zemax::export(
        &raytrace_formats::zemax::ZmxView {
            scene: &project.scene,
            material_names: &project.material_names,
            issues: &project.issues,
            zemax: project.zemax.as_ref(),
        },
        path,
    )
    .map_err(Into::into)
}
