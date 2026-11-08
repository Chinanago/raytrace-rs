//! Stable identifiers of parts and occurrences (docs/DESIGN.md §5).
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Whether `text` matches `[a-z][a-z0-9_-]*`.
pub fn is_valid_id(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

macro_rules! identifier {
    ($(#[$meta:meta])* $name:ident, $label:literal, $english:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(pub(crate) String);
        impl $name {
            /// Validate `[a-z][a-z0-9_-]*`.
            pub fn new(text: impl Into<String>) -> Result<Self> {
                let text = text.into();
                if is_valid_id(&text) {
                    Ok(Self(text))
                } else {
                    Err(Error::design(format!(
                        concat!($label, "のIDは英小文字で始まり、英小文字・数字・_・-だけで指定してください: {}"),
                        text
                    )))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(text: String) -> Result<Self> {
                Self::new(text)
            }
        }
        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
        impl std::str::FromStr for $name {
            type Err = Error;
            fn from_str(text: &str) -> Result<Self> {
                Self::new(text)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }
        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
        #[cfg(feature = "schema")]
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9_-]*$",
                    "description": concat!("Identifier of a ", $english, ", unique within the design document.")
                })
            }
        }
    };
}

identifier!(
    /// Identifier of a part definition.
    PartId, "部品", "part"
);
identifier!(
    /// Identifier of an occurrence (a placed instance of a part).
    OccurrenceId, "配置", "occurrence"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_follow_the_documented_pattern() {
        for ok in ["a", "optic-3", "rod_1", "system-12"] {
            assert!(PartId::new(ok).is_ok(), "{ok}");
        }
        for bad in ["", "3a", "Optic", "a b", "a/b", "ロッド", "-a"] {
            assert!(OccurrenceId::new(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<PartId>("\"Bad\"").is_err());
        assert_eq!(
            serde_json::to_string(&PartId::new("rod-5").unwrap()).unwrap(),
            "\"rod-5\""
        );
    }
}
