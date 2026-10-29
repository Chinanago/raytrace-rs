//! Runtime data belongs to the user's writable data directory, independent of the build tree.
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("RAYTRACE_DATA_DIR") {
        return p.into();
    }
    if let Some(p) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(p).join("Raytrace");
    }
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(p).join("raytrace");
    }
    if let Some(p) = std::env::var_os("HOME") {
        return PathBuf::from(p).join(".local/share/raytrace");
    }
    PathBuf::from("results/raytrace_data")
}
