//! Read-only STEP geometry and optical tessellation, implemented in Rust.
use crate::Result;
use raytrace_core::model::Shape;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
mod geometry;
mod nurbs;
mod placement;
mod step;
mod tessellate;

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const CACHE_FORMAT: &str = "raytrace-step-native-v1";
fn check_tolerance(value: f64) -> Result<()> {
    if !value.is_finite() || value <= 0. {
        return Err(crate::Error::Cad(
            "CAD分割精度は正の有限値で指定してください".into(),
        ));
    }
    Ok(())
}
fn read_step(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|e| crate::Error::io(path, e))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| crate::Error::io(path, e))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(crate::Error::Cad(
            "STEP: ファイルの読込上限128 MiBを超えています".into(),
        ));
    }
    Ok(bytes)
}
fn mesh(bytes: &[u8], deflection_mm: f64) -> Result<Shape> {
    let table = step::Table::read(bytes)?;
    let scale = table.length_scale_mm()?;
    let (solid, transform) = placement::solid_placement(&table)?;
    let lengths = transform.axes.map(|a| a.norm());
    let orthogonal = (0..3).all(|i| {
        (i + 1..3).all(|j| {
            transform.axes[i].dot(transform.axes[j]).abs() <= lengths[i] * lengths[j] * 1e-12
        })
    });
    let magnification = if orthogonal {
        lengths.into_iter().fold(0., f64::max)
    } else {
        lengths.iter().map(|v| v * v).sum::<f64>().sqrt()
    };
    let tolerance = deflection_mm / (scale * magnification);
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(crate::Error::Cad(
            "STEP: 単位・配置倍率と分割精度の組合せが範囲外です".into(),
        ));
    }
    tessellate::Mesher::new(&table, tolerance).solid(solid, transform, scale)
}
/// Convert afresh, bypassing the conversion cache (also useful for convergence checks).
pub fn load_step_native_with_tolerance(path: &Path, deflection_mm: f64) -> Result<Shape> {
    check_tolerance(deflection_mm)?;
    mesh(&read_step(path)?, deflection_mm)
}
pub fn load_step(path: &Path) -> Result<Shape> {
    load_step_with_tolerance(path, 0.02)
}
pub fn load_step_with_tolerance(path: &Path, deflection_mm: f64) -> Result<Shape> {
    load_cached(
        path,
        deflection_mm,
        &crate::storage::data_dir().join("cad_cache"),
    )
}
fn cache_key(bytes: &[u8], deflection_mm: f64) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    CACHE_FORMAT.hash(&mut hash);
    env!("CARGO_PKG_VERSION").hash(&mut hash);
    bytes.hash(&mut hash);
    deflection_mm.to_bits().hash(&mut hash);
    format!("{:016x}_native_v1.json", hash.finish())
}
#[derive(Deserialize)]
struct CachedMesh {
    format: String,
    shape: Shape,
}
fn usable(shape: &Shape) -> bool {
    let Shape::TriangleMesh {
        vertices,
        normals,
        triangles,
    } = shape
    else {
        return false;
    };
    !vertices.is_empty()
        && vertices.len() <= 3_000_000
        && !triangles.is_empty()
        && triangles.len() <= 1_000_000
        && normals.len() == vertices.len()
        && vertices.iter().all(|v| v.finite())
        && normals
            .iter()
            .all(|n| n.finite() && (n.norm() - 1.).abs() < 1e-6)
        && triangles.iter().all(|t| {
            t.iter().all(|&i| (i as usize) < vertices.len())
                && t[0] != t[1]
                && t[1] != t[2]
                && t[2] != t[0]
        })
}
fn load_cached(path: &Path, deflection_mm: f64, cache_dir: &Path) -> Result<Shape> {
    check_tolerance(deflection_mm)?;
    let bytes = read_step(path)?;
    let cache = cache_dir.join(cache_key(&bytes, deflection_mm));
    if fs::metadata(&cache).is_ok_and(|m| m.len() <= 512 * 1024 * 1024)
        && let Ok(bytes) = fs::read(&cache)
        && let Ok(saved) = serde_json::from_slice::<CachedMesh>(&bytes)
        && saved.format == CACHE_FORMAT
        && usable(&saved.shape)
    {
        return Ok(saved.shape);
    }
    let shape = mesh(&bytes, deflection_mm)?;
    // An unwritable/corrupt optional cache must not prevent geometry import.
    let _ = save_cache(&cache, &shape);
    Ok(shape)
}
fn save_cache(path: &Path, shape: &Shape) -> Result<()> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        format: &'static str,
        shape: &'a Shape,
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| crate::Error::step("STEP: キャッシュ保存先がありません".into()))?,
    )
    .map_err(|e| crate::Error::io(path, e))?;
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::io::BufWriter::new(
            fs::File::create(&temporary).map_err(|e| crate::Error::io(path, e))?,
        );
        serde_json::to_writer(
            &mut file,
            &Borrowed {
                format: CACHE_FORMAT,
                shape,
            },
        )
        .map_err(|e| crate::Error::json(None, e))?;
        file.flush().map_err(|e| crate::Error::io(path, e))?;
        drop(file);
        fs::rename(&temporary, path).map_err(|e| crate::Error::io(path, e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_keys_follow_content_and_tolerance_and_invalid_meshes_are_rejected() {
        assert_eq!(cache_key(b"abc", 0.02), cache_key(b"abc", 0.02));
        assert_ne!(cache_key(b"abc", 0.02), cache_key(b"abd", 0.02));
        assert_ne!(cache_key(b"abc", 0.02), cache_key(b"abc", 0.01));
        assert!(!usable(&Shape::TriangleMesh {
            vertices: vec![],
            normals: vec![],
            triangles: vec![]
        }));
        assert!(!usable(&Shape::Plane));
    }
}
