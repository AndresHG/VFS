//! Tauri command and event contract (VFS-1 spec, "Tauri commands and events"): the error
//! and reveal types the desktop commands return, and the event names. Kept in the core so
//! the contract compiles and is tested without a webview; `vfs-desktop` re-exports them.

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Event `index-progress`, payload `IndexProgress`, at most every 500 ms.
pub const EVENT_INDEX_PROGRESS: &str = "index-progress";
/// Event `index-changed`, payload `null`, at most every 2 s.
pub const EVENT_INDEX_CHANGED: &str = "index-changed";

/// Rejection value of every command; `code` is [`Error::code`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl From<Error> for ApiError {
    fn from(e: Error) -> ApiError {
        ApiError {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

/// Result of "Mostrar en carpeta" (VFS-1 AC-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealOutcome {
    Revealed,
    OpenedParent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reveal outcomes and errors serialize as the command contract specifies.
    #[test]
    fn contract_shapes() {
        assert_eq!(
            serde_json::to_value(RevealOutcome::OpenedParent).unwrap(),
            json!("opened_parent")
        );
        assert_eq!(
            serde_json::to_value(RevealOutcome::Revealed).unwrap(),
            json!("revealed")
        );
        let e: ApiError = Error::OverlappingRoot {
            relation: crate::OverlapRelation::Inside,
            existing: "/a".into(),
        }
        .into();
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({ "code": "overlapping_root", "message": "overlaps an indexed folder: inside /a" })
        );
    }

    /// Every error variant maps to one of the contract's error codes.
    #[test]
    fn error_codes() {
        let cases = [
            (Error::NotFound("x".into()), "not_found"),
            (Error::NotADirectory("x".into()), "not_a_directory"),
            (
                Error::OverlappingRoot {
                    relation: crate::OverlapRelation::Same,
                    existing: "x".into(),
                },
                "overlapping_root",
            ),
            (Error::Unavailable("x".into()), "unavailable"),
            (Error::SchemaTooNew, "schema_too_new"),
            (Error::Db(rusqlite::Error::InvalidQuery), "db"),
            (Error::Io(std::io::Error::other("x")), "io"),
        ];
        for (e, code) in cases {
            assert_eq!(ApiError::from(e).code, code);
        }
    }
}
