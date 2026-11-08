//! A small geometric-optics kernel. Lengths are mm; wavelengths are micrometres.
pub use raytrace_core::{
    coating, dispersion, geometry, illumination, math, mesh, model, sampling, screen, spectrum,
    trace,
};
pub use raytrace_doc as doc;
pub use raytrace_engine::{
    command, commands, convergence, engine, pipeline, scan, session, validation,
};
pub use raytrace_io::{cad, compatibility, design, export, record, storage, zemax};

pub mod error {
    pub use raytrace_core::Error as CoreError;
    pub use raytrace_engine::Error as AppError;
    pub use raytrace_formats::Error as FormatError;
    pub use raytrace_io::Error as IoError;
}
