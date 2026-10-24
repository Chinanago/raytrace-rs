//! A small geometric-optics kernel. Lengths are mm; wavelengths are micrometres.
pub use raytrace_app::{convergence, scan, validation};
pub use raytrace_core::{
    coating, dispersion, geometry, illumination, math, mesh, model, sampling, screen, spectrum,
    trace,
};
pub use raytrace_io::{assembly, cad, compatibility, export, project, record, storage, zemax};

pub mod error {
    pub use raytrace_app::Error as AppError;
    pub use raytrace_core::Error as CoreError;
    pub use raytrace_io::Error as IoError;
}
