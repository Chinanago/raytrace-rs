//! Errors from optical model validation and tracing.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{0}")]
    InvalidScene(String),
    #[error("{detail}")]
    /// `source_index` is the one-based source number used in messages.
    InvalidSource { source_index: usize, detail: String },
    #[error("{detail}")]
    InvalidRay { id: u64, detail: String },
    #[error(
        "材料 {material} は波長 {wavelength_um} um（光線 {launch}）で定義されていません。有効な波長範囲を確認してください"
    )]
    Wavelength {
        material: usize,
        wavelength_um: f64,
        launch: u64,
    },
    #[error("光源 {source_index} / 光線 {ray} / 波長 {wavelength_um} um: {detail}")]
    /// `source_index` is the one-based source number used in messages.
    RayAiming {
        source_index: usize,
        ray: u64,
        wavelength_um: f64,
        detail: String,
    },
    #[error("{0}")]
    Coating(String),
    #[error("{0}")]
    Limit(String),
    /// An internal invariant did not hold; reported instead of panicking.
    #[error("内部エラー: {0}")]
    Internal(String),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Stable snake_case identifier for scripts and agents; the message stays Japanese.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidScene(_) => "invalid_scene",
            Self::InvalidSource { .. } => "invalid_source",
            Self::InvalidRay { .. } => "invalid_ray",
            Self::Wavelength { .. } => "wavelength_out_of_range",
            Self::RayAiming { .. } => "ray_aiming",
            Self::Coating(_) => "coating",
            Self::Limit(_) => "limit",
            Self::Internal(_) => "internal",
        }
    }
}
