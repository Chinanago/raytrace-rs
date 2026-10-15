//! Optional STEP tessellation bridge. ZMX parsing and all tracing stay in Rust.
use crate::model::Shape;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn load_step(path: &Path) -> Result<Shape, String> {
    if !path.is_file() {
        return Err(format!("参照CADがありません: {}", path.display()));
    }
    let cache_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results/cad_cache");
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .hash(&mut hash);
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    metadata.len().hash(&mut hash);
    metadata.modified().ok().hash(&mut hash);
    let cache = cache_dir.join(format!("{:016x}_d002_v1.json", hash.finish()));
    if !cache.is_file() {
        let python = std::env::var_os("RAYTRACE_FREECAD_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join("Programs/FreeCAD 1.1/bin/python.exe")
            });
        if !python.is_file() {
            return Err(
                "STEP読込にはFreeCADのPythonが必要です。RAYTRACE_FREECAD_PYTHONで指定できます"
                    .into(),
            );
        }
        let script = cache_dir.join("step_mesh_v1.py");
        fs::write(&script, include_str!("../scripts/step_mesh.py")).map_err(|e| e.to_string())?;
        let mut command = Command::new(python);
        command.arg(script).arg(path).arg(&cache);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "STEP読込に失敗しました: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    serde_json::from_slice(&fs::read(cache).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
