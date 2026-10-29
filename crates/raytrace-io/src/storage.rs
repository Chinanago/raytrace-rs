//! Runtime data belongs to the user's writable data directory, independent of the build tree.
use crate::Result;
pub use raytrace_formats::storage::data_dir;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub recent_files: Vec<PathBuf>,
}
impl Preferences {
    pub fn load(dir: &Path) -> Self {
        fs::read(dir.join("preferences.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn remember(&mut self, path: &Path) {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent_files.retain(|p| {
            if cfg!(windows) {
                !p.to_string_lossy()
                    .eq_ignore_ascii_case(&path.to_string_lossy())
            } else {
                *p != path
            }
        });
        self.recent_files.insert(0, path);
        self.recent_files.truncate(10);
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).map_err(|e| crate::Error::io(dir, e))?;
        fs::write(
            dir.join("preferences.json.tmp"),
            serde_json::to_vec_pretty(self)
                .map_err(|e| crate::Error::json(Some(dir.join("preferences.json.tmp")), e))?,
        )
        .map_err(|e| crate::Error::io(dir.join("preferences.json.tmp"), e))?;
        fs::rename(
            dir.join("preferences.json.tmp"),
            dir.join("preferences.json"),
        )
        .map_err(|e| crate::Error::io(dir.join("preferences.json"), e))
    }
}
