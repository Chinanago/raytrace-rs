//! Optical file formats, persistence and project data.
pub mod assembly;
pub mod cad;
pub mod compatibility;
pub mod export;
pub mod identity;
pub mod project;
pub mod record;
pub mod storage;
pub mod zemax;

pub mod error;
pub use error::{Error, Result};
