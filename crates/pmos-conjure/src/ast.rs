//! The Conjure document model (App DSL spec §2–§5, §7–§8).
//! Strict parsing: unknown top-level/manifest fields are errors — strictness
//! keeps the App Smith repair loop honest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Format version, e.g. "1.0".
    pub conjure: String,
    pub manifest: Manifest,
    #[serde(default)]
    pub state: BTreeMap<String, StateDecl>,
    pub ui: Node,
    #[serde(default)]
    pub handlers: BTreeMap<String, Vec<Action>>,
    #[serde(default)]
    pub timers: Vec<TimerDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub window: WindowCfg,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn default_icon() -> String {
    "✨".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCfg {
    #[serde(default = "default_size")]
    pub size: [f32; 2],
    #[serde(default = "default_true")]
    pub resizable: bool,
    #[serde(default)]
    pub min_size: Option<[f32; 2]>,
}

fn default_size() -> [f32; 2] {
    [360.0, 320.0]
}

fn default_true() -> bool {
    true
}

impl Default for WindowCfg {
    fn default() -> Self {
        Self {
            size: default_size(),
            resizable: true,
            min_size: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateDecl {
    #[serde(rename = "type")]
    pub ty: String,
    pub value: serde_json::Value,
    /// Element type for lists.
    #[serde(default)]
    pub item: Option<String>,
}

/// A UI node: `{"w": "column", …props, "children": […]}`. Props stay loosely
/// typed JSON; the validator parses every expression-bearing prop up front.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub w: String,
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(flatten)]
    pub props: BTreeMap<String, serde_json::Value>,
}

/// An action: `{"do": "set", …fields}`. Field validation happens in the
/// validator so error messages carry paths and hints for the repair loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    #[serde(rename = "do")]
    pub verb: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerDecl {
    pub id: String,
    pub every_ms: f64,
    pub handler: String,
    #[serde(default = "default_true")]
    pub autostart: bool,
}

impl Action {
    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(|v| v.as_str())
    }

    pub fn actions_field(&self, key: &str) -> Option<Vec<Action>> {
        self.fields
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}
