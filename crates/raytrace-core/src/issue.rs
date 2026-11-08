//! Import and definition issues that travel with a part of the design document.
use serde::{Deserialize, Serialize};

/// A condition found while importing or editing a definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Issue {
    pub code: String,
    pub message: String,
    /// An unsupported physical definition prevents tracing.
    pub blocking: bool,
}
