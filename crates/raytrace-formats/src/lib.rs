//! Optical file formats independent of the editable project model.
pub mod cad;
pub mod storage;
pub mod zemax;

mod issue;
pub use issue::Issue;

pub mod error;
pub use error::{Error, Result};
