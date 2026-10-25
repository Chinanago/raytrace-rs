//! Pure geometric optics kernel and ray propagation.
mod acceleration;
pub mod coating;
pub mod dispersion;
pub mod geometry;
pub mod illumination;
pub mod math;
pub mod mesh;
pub mod model;
mod packet;
pub mod sampling;
pub mod screen;
pub mod spectrum;
pub mod trace;

pub mod error;
pub use error::{Error, Result};
