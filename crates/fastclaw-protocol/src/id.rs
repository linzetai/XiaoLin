use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export))]
        #[cfg_attr(feature = "ts", ts(as = "String"))]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn generate() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
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

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                self.0 == *other
            }
        }
    };
}

define_id!(
    /// Type-safe wrapper for agent identifiers.
    AgentId
);

define_id!(
    /// Identifies a conversation session (persists across turns).
    SessionId
);

define_id!(
    /// Identifies a single agent turn within a session.
    TurnId
);

define_id!(
    /// Identifies a single client submission (request-response correlation).
    SubmissionId
);

/// Backwards-compatible type aliases used by `fastclaw-core`.
pub type MessageId = String;
pub type ToolCallId = String;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_serde_roundtrip() {
        let id = SessionId::new("sess-123");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"sess-123\"");
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn id_generate_is_unique() {
        let a = TurnId::generate();
        let b = TurnId::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn id_deref_and_eq() {
        let id = AgentId::new("main");
        assert_eq!(&*id, "main");
        assert_eq!(id, "main");
        assert_eq!(id, *"main");
        assert_eq!(id, "main".to_string());
    }
}
