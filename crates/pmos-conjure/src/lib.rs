//! Conjure — the PMOS application DSL.
//!
//! Host-independent on purpose: this crate must build natively so documents
//! can be validated in tests and CLI tools, not only in the browser.
//! Spec: docs/App DSL.md. Validation errors are machine-readable because they
//! are fed back to the model in the App Smith repair loop (AI System §4).

use serde::{Deserialize, Serialize};

/// Format version accepted by this interpreter (same major, ≤ this minor).
pub const CONJURE_VERSION: (u16, u16) = (1, 0);

/// A machine-readable validation error (App DSL spec §12).
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code} at {path}: {message}")]
pub struct ValidationError {
    /// JSON-pointer-ish location, e.g. `/ui/children/2/on_click`.
    pub path: String,
    /// Stable error code, e.g. `CAP001`.
    pub code: String,
    pub message: String,
    /// Actionable fix hint for the repair loop.
    pub hint: String,
}

/// Validate a Conjure document. Stages per spec §12; only stage 1 (JSON
/// well-formedness) and the version gate exist yet — the rest lands with
/// milestone 5 (see docs/Todo.md).
pub fn validate(doc: &str) -> Result<(), Vec<ValidationError>> {
    let json: serde_json::Value = serde_json::from_str(doc).map_err(|e| {
        vec![ValidationError {
            path: String::new(),
            code: "JSON001".into(),
            message: e.to_string(),
            hint: "Emit a single well-formed JSON object.".into(),
        }]
    })?;

    let version = json.get("conjure").and_then(|v| v.as_str());
    match version {
        Some(v) if v.starts_with("1.") => Ok(()),
        Some(v) => Err(vec![ValidationError {
            path: "/conjure".into(),
            code: "VER001".into(),
            message: format!("unsupported format version {v}"),
            hint: "Use \"conjure\": \"1.0\".".into(),
        }]),
        None => Err(vec![ValidationError {
            path: "/conjure".into(),
            code: "VER002".into(),
            message: "missing required \"conjure\" version field".into(),
            hint: "Add \"conjure\": \"1.0\" at the top level.".into(),
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_v1_document() {
        assert!(validate(r#"{ "conjure": "1.0" }"#).is_ok());
    }

    #[test]
    fn rejects_missing_version() {
        let errs = validate("{}").unwrap_err();
        assert_eq!(errs[0].code, "VER002");
    }

    #[test]
    fn rejects_malformed_json() {
        let errs = validate("not json").unwrap_err();
        assert_eq!(errs[0].code, "JSON001");
    }
}
