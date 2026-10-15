//! A small geometric-optics kernel. Lengths are mm; wavelengths are micrometres.
pub mod assembly;
pub mod cad;
pub mod geometry;
#[cfg(feature = "gui")]
pub mod gui;
pub mod math;
pub mod mesh;
pub mod model;
pub mod project;
pub mod sampling;
pub mod trace;
pub mod zemax;
