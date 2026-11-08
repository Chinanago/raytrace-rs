//! Session, command registry, calculation pipeline, parameter studies and validation.
//!
//! ```
//! use raytrace_engine::{Session, commands};
//! let mut session = Session::new();
//! let version = session.run(commands::APP_VERSION, Default::default()).unwrap();
//! assert!(!version.version.is_empty());
//! ```
pub mod analysis;
pub mod command;
pub mod commands;
pub mod convergence;
pub mod engine;
pub mod footprint;
pub mod nsc_reference;
pub mod pipeline;
pub mod scan;
pub mod session;
pub mod validation;

mod pointer;

pub mod error;
pub use command::{Cmd, CommandSpec, Context, Kind, Registry, registry};
pub use engine::{EngineError, Progress, Response};
pub use error::{Error, Result};
pub use session::Session;
