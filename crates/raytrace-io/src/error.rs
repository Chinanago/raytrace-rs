//! Errors from optical file formats and persistence.
use std::path::PathBuf;
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Format(raytrace_formats::Error),
    #[error(transparent)]
    Core(#[from] raytrace_core::Error),
    #[error("{path} の入出力に失敗しました: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JSONの処理に失敗しました（{path}）: {source}", path = .path.as_deref().map_or_else(|| "パス未指定".into(), |p| p.display().to_string()))]
    Json {
        path: Option<PathBuf>,
        #[source]
        source: serde_json::Error,
    },
    #[error("{detail}")]
    Zmx { line: Option<usize>, detail: String },
    #[error("{0}")]
    Agf(String),
    #[error("{0}")]
    Spcd(String),
    #[error("{detail}")]
    Step { detail: String },
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Project(String),
    #[error("{0}")]
    Record(String),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;
impl Error {
    pub(crate) fn zmx(detail: String) -> Self {
        Self::Zmx { line: None, detail }
    }
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
    pub fn json(path: Option<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json { path, source }
    }
}

impl From<raytrace_formats::Error> for Error {
    fn from(error: raytrace_formats::Error) -> Self {
        match error {
            raytrace_formats::Error::Core(source) => Self::Core(source),
            raytrace_formats::Error::Io { path, source } => Self::Io { path, source },
            raytrace_formats::Error::Json { path, source } => Self::Json { path, source },
            raytrace_formats::Error::Zmx { line, detail } => Self::Zmx { line, detail },
            raytrace_formats::Error::Agf(detail) => Self::Agf(detail),
            raytrace_formats::Error::Spcd(detail) => Self::Spcd(detail),
            raytrace_formats::Error::Step { detail } => Self::Step { detail },
            raytrace_formats::Error::Cad(detail) => Self::Project(detail),
            error => Self::Format(error),
        }
    }
}
