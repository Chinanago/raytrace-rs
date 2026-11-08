//! Native ZMX/AGF parsing. Unknown data stays in the original document.
use crate::Result;
use crate::design::{Design, Part, PartId, PartKind, Provenance};
use raytrace_core::model::Material;
pub use raytrace_formats::zemax::{TextEncoding, ZmxDocument, decode};
use std::path::{Path, PathBuf};

/// Provenance format name of imported ZMX documents.
pub const PROVENANCE_FORMAT: &str = "zmx";

/// Read one glass through the native AGF parser without importing geometry.
pub fn read_catalog_material(directory: &Path, catalog: &str, name: &str) -> Result<Material> {
    raytrace_formats::zemax::read_catalog_material(directory, catalog, name).map_err(Into::into)
}

pub fn read_spcd(path: &Path) -> Result<raytrace_core::spectrum::Spectrum> {
    raytrace_formats::zemax::read_spcd(path).map_err(Into::into)
}

pub fn read_zmx(path: &Path, extra: &[PathBuf]) -> Result<Design> {
    read_zmx_with_cad_tolerance(path, extra, 0.02)
}
pub fn read_zmx_with_cad_tolerance(
    path: &Path,
    extra: &[PathBuf],
    deflection_mm: f64,
) -> Result<Design> {
    let model = raytrace_formats::zemax::read_zmx_with_cad_tolerance(path, extra, deflection_mm)?;
    let provenance = model
        .zemax
        .as_ref()
        .map(|z| {
            Ok::<_, crate::Error>(Provenance {
                format: PROVENANCE_FORMAT.into(),
                document: serde_json::to_value(z).map_err(|e| crate::Error::json(None, e))?,
            })
        })
        .transpose()?;
    let part = Part {
        id: PartId::new("system-0")?,
        name: model.scene.name.clone(),
        kind: PartKind::System,
        scene: model.scene,
        material_names: model.material_names,
        issues: model.issues,
        conditions: Default::default(),
        rod: None,
        cad_deflection_mm: model.cad_deflection_mm,
        provenance,
    };
    Ok(Design::from_system_part(part))
}

/// The original ZMX document kept in the part's provenance.
pub fn document(part: &Part) -> Result<Option<ZmxDocument>> {
    part.provenance
        .as_ref()
        .filter(|p| p.format == PROVENANCE_FORMAT)
        .map(|p| {
            serde_json::from_value(p.document.clone()).map_err(|e| crate::Error::json(None, e))
        })
        .transpose()
}

/// Preserve the complete source text on an unchanged document. Geometry edits
/// are patched only for supported coaxial sequential surfaces.
pub fn export(design: &Design, path: &Path) -> Result<()> {
    let Some(part) = design.lone_system() else {
        return Err(crate::Error::zmx(
            "組立光学系は設計JSONで保存できます。部品を含むZMX書出しは追加予定です".into(),
        ));
    };
    let document = document(part)?;
    raytrace_formats::zemax::export(
        &raytrace_formats::zemax::ZmxView {
            scene: &part.scene,
            material_names: &part.material_names,
            issues: &part.issues,
            zemax: document.as_ref(),
        },
        path,
    )
    .map_err(Into::into)
}
