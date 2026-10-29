use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub code: String,
    pub message: String,
    /// An unsupported physical definition prevents tracing.
    pub blocking: bool,
}
