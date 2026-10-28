use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}
impl Error {
    pub fn new(code: &str, message: impl std::fmt::Display) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<raytrace_io::Error> for Error {
    fn from(e: raytrace_io::Error) -> Self {
        Self::new("model", e)
    }
}
impl From<raytrace_core::Error> for Error {
    fn from(e: raytrace_core::Error) -> Self {
        Self::new("optics", e)
    }
}
impl From<raytrace_app::Error> for Error {
    fn from(e: raytrace_app::Error) -> Self {
        Self::new("study", e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("json", e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
