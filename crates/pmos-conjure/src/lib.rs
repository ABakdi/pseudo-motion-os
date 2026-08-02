//! Conjure — the PMOS application DSL.
//!
//! Host-independent on purpose: this crate must build natively so documents
//! can be validated and interpreted in tests and CLI tools, not only in the
//! browser. Spec: docs/App DSL.md. Validation errors are machine-readable
//! because they are fed back to the model in the App Smith repair loop
//! (AI System §4).

pub mod ast;
pub mod expr;
pub mod interp;

pub use ast::Document;
pub use expr::Value;
pub use interp::{AppInstance, Effect};

use serde::{Deserialize, Serialize};

/// Format version accepted by this interpreter (same major, ≤ this minor).
pub const CONJURE_VERSION: (u16, u16) = (1, 0);

/// Widgets the App Host can render (App DSL spec §5, v1 subset).
pub const KNOWN_WIDGETS: &[&str] = &[
    "column",
    "row",
    "group",
    "scroll",
    "separator",
    "spacer",
    "label",
    "button",
    "text_input",
    "slider",
    "checkbox",
    "toggle",
    "dropdown",
    "list_view",
    "canvas",
    "progress",
    "if",
];

/// Action verbs the interpreter executes (App DSL spec §7, v1 subset).
pub const KNOWN_ACTIONS: &[&str] = &[
    "set",
    "toggle",
    "inc",
    "push",
    "remove_at",
    "clear",
    "if",
    "emit",
    "notify",
    "window",
    "timer",
];

const MAX_DOC_BYTES: usize = 256 * 1024;
const MAX_NODES: usize = 512;
const MAX_STATE: usize = 64;
const MAX_TIMERS: usize = 4;

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

fn err(path: &str, code: &str, message: String, hint: &str) -> ValidationError {
    ValidationError {
        path: path.to_string(),
        code: code.to_string(),
        message,
        hint: hint.to_string(),
    }
}

/// Validate a Conjure document through all stages (App DSL spec §12).
/// Returns the parsed [`Document`] ready for the interpreter.
pub fn validate(doc_src: &str) -> Result<Document, Vec<ValidationError>> {
    if doc_src.len() > MAX_DOC_BYTES {
        return Err(vec![err(
            "",
            "LIM001",
            format!("document is {} bytes (max {MAX_DOC_BYTES})", doc_src.len()),
            "Generate a smaller app.",
        )]);
    }

    // Stage 1+2: JSON well-formedness and schema.
    let doc: Document = serde_json::from_str(doc_src).map_err(|e| {
        vec![err(
            "",
            "JSON001",
            e.to_string(),
            "Emit exactly one well-formed JSON object matching the Conjure schema.",
        )]
    })?;

    let mut errors = Vec::new();

    // Version gate.
    if !doc.conjure.starts_with("1.") {
        errors.push(err(
            "/conjure",
            "VER001",
            format!("unsupported format version {}", doc.conjure),
            "Use \"conjure\": \"1.0\".",
        ));
    }

    // Stage 6 first (cheap limits).
    if doc.state.len() > MAX_STATE {
        errors.push(err(
            "/state",
            "LIM002",
            format!("{} state entries (max {MAX_STATE})", doc.state.len()),
            "Reduce state entries.",
        ));
    }
    if doc.timers.len() > MAX_TIMERS {
        errors.push(err(
            "/timers",
            "LIM003",
            format!("{} timers (max {MAX_TIMERS})", doc.timers.len()),
            "Use at most 4 timers.",
        ));
    }

    // Stage 3+4 over the UI tree: known widgets, handler refs, expressions.
    let mut node_count = 0usize;
    validate_node(&doc, &doc.ui, "/ui", &mut node_count, &mut errors);
    if node_count > MAX_NODES {
        errors.push(err(
            "/ui",
            "LIM004",
            format!("{node_count} UI nodes (max {MAX_NODES})"),
            "Simplify the UI tree.",
        ));
    }

    // Stage 4 over handlers: known verbs, expression fields, handler refs.
    for (name, actions) in &doc.handlers {
        validate_actions(&doc, actions, &format!("/handlers/{name}"), &mut errors);
    }

    // Timers reference real handlers.
    for (i, t) in doc.timers.iter().enumerate() {
        if !doc.handlers.contains_key(&t.handler) {
            errors.push(err(
                &format!("/timers/{i}/handler"),
                "REF002",
                format!("timer references unknown handler `{}`", t.handler),
                "Point the timer at a handler defined in /handlers.",
            ));
        }
    }

    // Stage 5: emit call graph must be acyclic.
    for name in doc.handlers.keys() {
        let mut stack = vec![name.clone()];
        if has_cycle(&doc, name, &mut stack) {
            errors.push(err(
                &format!("/handlers/{name}"),
                "CYC001",
                format!("emit cycle involving `{name}`"),
                "Handlers may emit other handlers but never form a cycle.",
            ));
        }
    }

    if errors.is_empty() {
        Ok(doc)
    } else {
        errors.truncate(12); // keep repair prompts small
        Err(errors)
    }
}

fn expr_ok(src: &str) -> Result<(), expr::ExprError> {
    let inner = src
        .strip_prefix("${")
        .and_then(|x| x.strip_suffix('}'))
        .unwrap_or(src);
    if src.contains("${") && !(src.starts_with("${") && src.ends_with('}')) {
        // Mixed template text.
        return expr::parse_template(src).map(|_| ());
    }
    if inner.contains("${") {
        return expr::parse_template(inner).map(|_| ());
    }
    // Bare strings are allowed as literals; only reject if it *looks* like
    // an expression (starts with ${) but fails to parse.
    if src.starts_with("${") {
        expr::parse(inner).map(|_| ())
    } else {
        Ok(())
    }
}

fn validate_node(
    doc: &Document,
    node: &ast::Node,
    path: &str,
    count: &mut usize,
    errors: &mut Vec<ValidationError>,
) {
    *count += 1;
    if !KNOWN_WIDGETS.contains(&node.w.as_str()) {
        errors.push(err(
            &format!("{path}/w"),
            "WID001",
            format!("unknown widget `{}`", node.w),
            "Use one of: column,row,group,scroll,separator,spacer,label,button,text_input,slider,checkbox,toggle,progress,if.",
        ));
    }
    for (key, val) in &node.props {
        if let Some(handler) = val.as_str() {
            if key.starts_with("on_") && !doc.handlers.contains_key(handler) {
                errors.push(err(
                    &format!("{path}/{key}"),
                    "REF001",
                    format!("references unknown handler `{handler}`"),
                    "Every on_* prop must name a handler defined in /handlers.",
                ));
            }
            if key == "bind" && !doc.state.contains_key(handler) {
                errors.push(err(
                    &format!("{path}/bind"),
                    "REF003",
                    format!("binds unknown state `{handler}`"),
                    "bind must name an entry in /state.",
                ));
            }
            if !key.starts_with("on_") && key != "bind" {
                if let Err(e) = expr_ok(handler) {
                    errors.push(err(
                        &format!("{path}/{key}"),
                        "EXP001",
                        format!("bad expression: {e}"),
                        "Fix the ${…} expression syntax.",
                    ));
                }
            }
        }
    }
    for (i, child) in node.children.iter().enumerate() {
        validate_node(doc, child, &format!("{path}/children/{i}"), count, errors);
    }
}

fn validate_actions(
    doc: &Document,
    actions: &[ast::Action],
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    for (i, a) in actions.iter().enumerate() {
        let apath = format!("{path}/{i}");
        if !KNOWN_ACTIONS.contains(&a.verb.as_str()) {
            errors.push(err(
                &format!("{apath}/do"),
                "ACT001",
                format!("unknown action `{}`", a.verb),
                "Use one of: set,toggle,inc,push,remove_at,clear,if,emit,notify,window,timer.",
            ));
        }
        if a.verb == "emit" {
            if let Some(h) = a.str_field("handler") {
                if !doc.handlers.contains_key(h) {
                    errors.push(err(
                        &format!("{apath}/handler"),
                        "REF004",
                        format!("emit references unknown handler `{h}`"),
                        "Emit must target a defined handler.",
                    ));
                }
            }
        }
        for key in ["cond", "value", "by", "index"] {
            if let Some(serde_json::Value::String(s)) = a.fields.get(key) {
                if let Err(e) = expr_ok(s) {
                    errors.push(err(
                        &format!("{apath}/{key}"),
                        "EXP002",
                        format!("bad expression: {e}"),
                        "Fix the expression syntax.",
                    ));
                }
            }
        }
        for branch in ["then", "else"] {
            if let Some(nested) = a.actions_field(branch) {
                validate_actions(doc, &nested, &format!("{apath}/{branch}"), errors);
            }
        }
    }
}

fn has_cycle(doc: &Document, current: &str, stack: &mut Vec<String>) -> bool {
    let Some(actions) = doc.handlers.get(current) else {
        return false;
    };
    fn walk(doc: &Document, actions: &[ast::Action], stack: &mut Vec<String>) -> bool {
        for a in actions {
            if a.verb == "emit" {
                if let Some(target) = a.str_field("handler") {
                    if stack.iter().any(|s| s == target) {
                        return true;
                    }
                    stack.push(target.to_string());
                    if has_cycle(doc, target, stack) {
                        return true;
                    }
                    stack.pop();
                }
            }
            for branch in ["then", "else"] {
                if let Some(nested) = a.actions_field(branch) {
                    if walk(doc, &nested, stack) {
                        return true;
                    }
                }
            }
        }
        false
    }
    walk(doc, actions, stack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_pomodoro_example() {
        let src = include_str!("../examples/pomodoro.conjure.json");
        assert!(validate(src).is_ok());
    }

    #[test]
    fn rejects_missing_version() {
        let errs =
            validate(r#"{ "manifest": {"id":"x","name":"x"}, "ui": {"w":"column"} }"#).unwrap_err();
        assert_eq!(errs[0].code, "JSON001"); // missing required field
    }

    #[test]
    fn rejects_malformed_json() {
        let errs = validate("not json").unwrap_err();
        assert_eq!(errs[0].code, "JSON001");
    }

    #[test]
    fn rejects_unknown_widget_and_handler() {
        let src = r#"{
            "conjure": "1.0",
            "manifest": { "id": "x", "name": "x" },
            "ui": { "w": "blink", "on_click": "nope" }
        }"#;
        let errs = validate(src).unwrap_err();
        let codes: Vec<&str> = errs.iter().map(|e| e.code.as_str()).collect();
        assert!(codes.contains(&"WID001"));
        assert!(codes.contains(&"REF001"));
    }

    #[test]
    fn rejects_emit_cycle() {
        let src = r#"{
            "conjure": "1.0",
            "manifest": { "id": "x", "name": "x" },
            "ui": { "w": "column" },
            "handlers": {
                "a": [ { "do": "emit", "handler": "b" } ],
                "b": [ { "do": "emit", "handler": "a" } ]
            }
        }"#;
        let errs = validate(src).unwrap_err();
        assert!(errs.iter().any(|e| e.code == "CYC001"));
    }
}
