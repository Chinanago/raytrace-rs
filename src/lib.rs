//! A small geometric-optics kernel. Lengths are mm; wavelengths are micrometres.
pub use raytrace_app::{convergence, scan, validation};
pub use raytrace_core::{
    coating, dispersion, geometry, illumination, math, mesh, model, sampling, screen, spectrum,
    trace,
};
pub use raytrace_io::{assembly, cad, compatibility, export, project, record, storage, zemax};
