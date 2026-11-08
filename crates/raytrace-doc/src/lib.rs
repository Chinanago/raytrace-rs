//! The design document (docs/DESIGN.md layer L2): parts, occurrences and their compilation.
//!
//! A [`Design`] is a list of [`Part`] definitions, each in its own local coordinates,
//! and [`Occurrence`]s that place them. [`Design::compile`] turns the document into the
//! scene that the tracer consumes. Nothing here touches files, clocks or the
//! environment.
pub mod compatibility;
mod compile;
mod component;
mod design;
mod error;
mod id;

pub use compile::Compiled;
pub use component::{Component, demo_design};
pub use design::{
    Conditions, Design, FORMAT, Occurrence, Part, PartKind, Provenance, RodSize, air,
    default_material_names, empty_scene,
};
pub use error::{Error, Result};
pub use id::{OccurrenceId, PartId, is_valid_id};
