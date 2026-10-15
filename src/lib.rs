//! A small geometric-optics kernel. Lengths are mm; wavelengths are micrometres.
pub mod assembly;
pub mod cad;
pub mod compatibility;
pub mod convergence;
pub mod geometry;
#[cfg(feature = "gui")]
pub mod gui;
pub mod math;
pub mod mesh;
pub mod model;
pub mod project;
pub mod record;
pub mod sampling;
pub mod screen;
pub mod trace;
pub mod validation;
pub mod zemax;
