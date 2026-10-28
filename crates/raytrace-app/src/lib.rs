//! Optical project workflows, parameter studies and validation.
pub mod convergence;
pub mod nsc_reference;
pub mod scan;
pub mod validation;

pub mod error;
pub use error::{Error, Result};
