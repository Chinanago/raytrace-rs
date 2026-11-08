//! A cheap estimate of the memory a design holds, for the limits of the undo history.
//!
//! Geometry vectors dominate the size of a design. The estimate counts them without
//! serialising or walking every triangle.
use raytrace_core::{
    coating::{Coating, CoatingModel},
    math::V3,
    model::{Scene, Shape, Source, Surface},
};
use raytrace_io::design::Design;
use serde_json::Value;
use std::mem::size_of;

fn coating_bytes(c: &Coating) -> usize {
    let size = |m: &CoatingModel| match m {
        CoatingModel::Table {
            wavelengths_um,
            angles_deg,
            coefficients,
            ..
        } => (wavelengths_um.len() + angles_deg.len()) * 8 + coefficients.len() * 16,
        _ => 0,
    };
    c.name.len()
        + c.zemax_definition.as_ref().map_or(0, |s| s.len())
        + size(&c.front)
        + c.back.as_ref().map_or(0, size)
}

fn scene_bytes(scene: &Scene) -> usize {
    scene.sources.len() * size_of::<Source>()
        + scene
            .sources
            .iter()
            .filter_map(|s| s.spectrum.as_ref())
            .map(|s| s.samples.len() * 16)
            .sum::<usize>()
        + scene.materials.len() * 1024
        + scene
            .surfaces
            .iter()
            .map(|s| {
                size_of::<Surface>()
                    + s.name.len()
                    + s.coating.as_ref().map_or(0, coating_bytes)
                    + match &s.shape {
                        Shape::TriangleMesh {
                            vertices,
                            triangles,
                            normals,
                        } => {
                            (vertices.len() + normals.len()) * size_of::<V3>()
                                + triangles.len() * 12
                        }
                        Shape::EvenAsphere { coefficients, .. }
                        | Shape::FlatEvenAsphere { coefficients } => coefficients.len() * 8,
                        _ => 0,
                    }
            })
            .sum::<usize>()
}

/// The provenance is opaque JSON; arrays are assumed to hold similar entries so that a
/// large mesh costs one step per nesting level instead of one per triangle.
fn value_bytes(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Array(a) => a
            .first()
            .map_or(0, |first| a.len() * (value_bytes(first) + 16)),
        Value::Object(o) => o.iter().map(|(k, v)| k.len() + value_bytes(v) + 16).sum(),
        _ => 16,
    }
}

/// Estimated bytes held by a design.
pub fn design_bytes(d: &Design) -> usize {
    d.parts
        .iter()
        .map(|p| {
            scene_bytes(&p.scene)
                + p.provenance
                    .as_ref()
                    .map_or(0, |o| value_bytes(&o.document))
                + p.rod
                    .as_ref()
                    .map_or(0, |r| r.coatings.values().map(coating_bytes).sum::<usize>())
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_design_is_free_and_parts_add_up() {
        let empty = Design::empty();
        assert_eq!(design_bytes(&empty), 0);
        let mut one = empty.clone();
        one.add_component(raytrace_io::design::Component::detector())
            .unwrap();
        assert!(design_bytes(&one) > 0);
    }
}
