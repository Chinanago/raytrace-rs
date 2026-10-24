//! Errors from simulation workflows.
use std::path::PathBuf;
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] raytrace_core::Error),
    #[error(transparent)]
    Io(#[from] raytrace_io::Error),
    #[error("{0}")]
    Plan(String),
    #[error("{total} ケース中 {failed} ケースが失敗しました。概要を確認してください: {summary}", summary = .summary.display())]
    CasesFailed {
        failed: usize,
        total: usize,
        summary: PathBuf,
    },
    #[error("計算を中止しました")]
    Cancelled,
}
pub type Result<T, E = Error> = std::result::Result<T, E>;
