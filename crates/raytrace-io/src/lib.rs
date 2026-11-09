//! Design files, calculation records, external-format imports and result exports.
pub mod cad;
pub use raytrace_doc::compatibility;
pub mod design;
pub mod export;
pub mod record;
pub mod storage;
pub mod zemax;

pub mod error;
pub use error::{Error, Result};
