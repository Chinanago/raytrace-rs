//! Read-only STEP geometry and optical tessellation, implemented in Rust.
use crate::Result;
use raytrace_core::model::Shape;
use std::path::Path;

/// Convert afresh, bypassing the conversion cache (also useful for convergence checks).
pub fn load_step_native_with_tolerance(path: &Path, deflection_mm: f64) -> Result<Shape> {
    raytrace_formats::cad::load_step_native_with_tolerance(path, deflection_mm).map_err(Into::into)
}
pub fn load_step(path: &Path) -> Result<Shape> {
    raytrace_formats::cad::load_step(path).map_err(Into::into)
}
pub fn load_step_with_tolerance(path: &Path, deflection_mm: f64) -> Result<Shape> {
    raytrace_formats::cad::load_step_with_tolerance(path, deflection_mm).map_err(Into::into)
}
