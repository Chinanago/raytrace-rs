//! A native transport for the same session commands used by every front end.
//! The GUI keeps ownership of its session and accepts prepared work on its UI thread.
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};

/// Connection capabilities for one running GUI. Keep this file private: its token grants
/// the same file and command access as the GUI process. Never publish it or log its token.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInfo {
    pub format: String,
    pub software_version: String,
    pub address: SocketAddr,
    pub token: String,
    /// Relative command paths are resolved in the GUI's working directory.
    pub working_directory: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{Client, Pending, Prepared, Server};
