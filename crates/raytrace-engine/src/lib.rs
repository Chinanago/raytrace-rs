//! The editor boundary shared by scripted and interactive frontends.
pub mod bridge;
mod command;
mod error;
pub mod mcp;
mod properties;
pub mod script;
mod session;
pub use command::*;
pub use error::{Error, Result};
pub use session::{EditPreparation, PreparedEdit, Session};

pub fn schema() -> serde_json::Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(Request)).expect("schema serializes");
    // Serde flatten merges command fields with the request revision. In JSON Schema
    // 2020-12, this closes the composed object without rejecting either field set.
    schema["unevaluatedProperties"] = false.into();
    schema
}
