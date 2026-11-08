//! Errors from validating and compiling the design document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The document is inconsistent or cannot be traced. The message is Japanese.
    #[error("{0}")]
    Design(String),
    #[error(transparent)]
    Core(#[from] raytrace_core::Error),
    #[error("JSONの処理に失敗しました: {0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub(crate) fn design(message: impl Into<String>) -> Self {
        Self::Design(message.into())
    }
    /// Stable snake_case identifier for scripts and agents; the message stays Japanese.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Design(_) => "design",
            Self::Core(error) => error.code(),
            Self::Json(_) => "invalid_json",
        }
    }
}
