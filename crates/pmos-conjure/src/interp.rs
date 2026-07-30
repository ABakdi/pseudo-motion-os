//! The Conjure interpreter — the App Host's core (App DSL spec §7–§9, §11).
//!
//! Handlers run under a hard step budget; effects exist only as catalog
//! actions. System-touching actions surface as [`Effect`]s for the host to
//! route through capability-checked syscalls — the interpreter itself can
//! reach nothing.

use crate::ast::{Action, Document};
use crate::expr::{self, Env, Value};
use std::collections::HashMap;

pub const MAX_STEPS: u32 = 4096;
const MAX_IF_DEPTH: u32 = 8;
/// Emit chains are validated acyclic, but depth is bounded anyway so a bad
/// document can never overflow the native stack.
const MAX_EMIT_DEPTH: u32 = 16;

/// Side effects requested by handlers, drained by the host each frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Notify { title: String, body: String },
    SetTitle(String),
    CloseWindow,
}

struct TimerState {
    id: String,
    every_ms: f64,
    handler: String,
    running: bool,
    next_due: f64,
}

pub struct AppInstance {
    pub doc: Document,
    pub state: HashMap<String, Value>,
    timers: Vec<TimerState>,
    pub effects: Vec<Effect>,
    /// Set when a handler blew its step budget — shown in the titlebar and
    /// the app stops running handlers (spec §9: contained, not fatal).
    pub misbehaving: bool,
}

impl AppInstance {
    pub fn new(doc: Document, now_ms: f64) -> Self {
        let state = doc
            .state
            .iter()
            .map(|(k, decl)| {
                let v =
                    serde_json::from_value::<Value>(decl.value.clone()).unwrap_or(Value::Num(0.0));
                (k.clone(), v)
            })
            .collect();
        let timers = doc
            .timers
            .iter()
            .map(|t| TimerState {
                id: t.id.clone(),
                every_ms: t.every_ms.max(100.0),
                handler: t.handler.clone(),
                running: t.autostart,
                next_due: now_ms + t.every_ms.max(100.0),
            })
            .collect();
        let mut app = Self {
            doc,
            state,
            timers,
            effects: Vec::new(),
            misbehaving: false,
        };
        app.run_handler("on_init", &HashMap::new(), now_ms);
        app
    }

    /// Fire due timers. Called by the host every frame.
    pub fn tick(&mut self, now_ms: f64) {
        let due: Vec<(String, f64)> = self
            .timers
            .iter_mut()
            .filter(|t| t.running && now_ms >= t.next_due)
            .map(|t| {
                t.next_due = now_ms + t.every_ms;
                (t.handler.clone(), t.every_ms)
            })
            .collect();
        for (handler, _) in due {
            self.run_handler(&handler, &HashMap::new(), now_ms);
        }
    }

    /// Run a named handler with local bindings (e.g. widget event payloads).
    pub fn run_handler(&mut self, name: &str, locals: &HashMap<String, Value>, now_ms: f64) {
        if self.misbehaving {
            return;
        }
        let Some(actions) = self.doc.handlers.get(name).cloned() else {
            return;
        };
        let mut steps = 0u32;
        if self
            .exec_actions(&actions, locals, now_ms, &mut steps, 0, 0)
            .is_err()
        {
            self.misbehaving = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_actions(
        &mut self,
        actions: &[Action],
        locals: &HashMap<String, Value>,
        now_ms: f64,
        steps: &mut u32,
        if_depth: u32,
        emit_depth: u32,
    ) -> Result<(), ()> {
        for a in actions {
            *steps += 1;
            if *steps > MAX_STEPS || if_depth > MAX_IF_DEPTH || emit_depth > MAX_EMIT_DEPTH {
                return Err(());
            }
            self.exec_action(a, locals, now_ms, steps, if_depth, emit_depth)?;
        }
        Ok(())
    }

    fn eval_field(
        &self,
        a: &Action,
        key: &str,
        locals: &HashMap<String, Value>,
        now_ms: f64,
    ) -> Option<Value> {
        let raw = a.fields.get(key)?;
        let env = Env {
            state: &self.state,
            locals,
            now_ms,
        };
        match raw {
            serde_json::Value::String(s) => {
                // String fields are expressions ("${…}" optional sugar).
                let src = s
                    .strip_prefix("${")
                    .and_then(|x| x.strip_suffix('}'))
                    .unwrap_or(s);
                match expr::parse(src).and_then(|e| expr::eval(&e, &env)) {
                    Ok(v) => Some(v),
                    // A plain word that isn't a valid expression is a literal.
                    Err(_) => Some(Value::Str(s.clone())),
                }
            }
            other => serde_json::from_value(other.clone()).ok(),
        }
    }

    /// Text fields that should be treated as templates, not raw expressions.
    fn eval_text_field(
        &self,
        a: &Action,
        key: &str,
        locals: &HashMap<String, Value>,
        now_ms: f64,
    ) -> String {
        let Some(raw) = a.str_field(key) else {
            return String::new();
        };
        let env = Env {
            state: &self.state,
            locals,
            now_ms,
        };
        expr::parse_template(raw)
            .and_then(|t| expr::eval_template(&t, &env))
            .unwrap_or_else(|_| raw.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_action(
        &mut self,
        a: &Action,
        locals: &HashMap<String, Value>,
        now_ms: f64,
        steps: &mut u32,
        if_depth: u32,
        emit_depth: u32,
    ) -> Result<(), ()> {
        match a.verb.as_str() {
            "set" => {
                let target = a.str_field("target").unwrap_or_default().to_string();
                if let Some(v) = self.eval_field(a, "value", locals, now_ms) {
                    self.state.insert(target, v);
                }
            }
            "toggle" => {
                let target = a.str_field("target").unwrap_or_default();
                let cur = self.state.get(target).map(|v| v.truthy()).unwrap_or(false);
                self.state.insert(target.to_string(), Value::Bool(!cur));
            }
            "inc" => {
                let target = a.str_field("target").unwrap_or_default().to_string();
                let by = self
                    .eval_field(a, "by", locals, now_ms)
                    .map(|v| v.as_num())
                    .unwrap_or(1.0);
                let cur = self.state.get(&target).map(|v| v.as_num()).unwrap_or(0.0);
                self.state.insert(target, Value::Num(cur + by));
            }
            "push" => {
                let target = a.str_field("target").unwrap_or_default().to_string();
                if let Some(v) = self.eval_field(a, "value", locals, now_ms) {
                    if let Some(Value::List(l)) = self.state.get_mut(&target) {
                        if l.len() < 4096 {
                            l.push(v);
                        }
                    }
                }
            }
            "remove_at" => {
                let target = a.str_field("target").unwrap_or_default().to_string();
                let idx = self
                    .eval_field(a, "index", locals, now_ms)
                    .map(|v| v.as_num() as usize)
                    .unwrap_or(usize::MAX);
                if let Some(Value::List(l)) = self.state.get_mut(&target) {
                    if idx < l.len() {
                        l.remove(idx);
                    }
                }
            }
            "clear" => {
                let target = a.str_field("target").unwrap_or_default().to_string();
                match self.state.get_mut(&target) {
                    Some(Value::List(l)) => l.clear(),
                    Some(Value::Str(s)) => s.clear(),
                    _ => {}
                }
            }
            "if" => {
                let cond = self
                    .eval_field(a, "cond", locals, now_ms)
                    .map(|v| v.truthy())
                    .unwrap_or(false);
                let branch = if cond { "then" } else { "else" };
                if let Some(actions) = a.actions_field(branch) {
                    self.exec_actions(&actions, locals, now_ms, steps, if_depth + 1, emit_depth)?;
                }
            }
            "emit" => {
                // Call-graph acyclicity is validated up front, so this cannot
                // recurse; the shared step budget still bounds total work.
                if let Some(handler) = a.str_field("handler") {
                    if let Some(actions) = self.doc.handlers.get(handler).cloned() {
                        self.exec_actions(
                            &actions,
                            locals,
                            now_ms,
                            steps,
                            if_depth,
                            emit_depth + 1,
                        )?;
                    }
                }
            }
            "notify" => {
                let title = self.eval_text_field(a, "title", locals, now_ms);
                let body = self.eval_text_field(a, "body", locals, now_ms);
                if self.effects.len() < 16 {
                    self.effects.push(Effect::Notify { title, body });
                }
            }
            "window" => match a.str_field("op") {
                Some("close") => self.effects.push(Effect::CloseWindow),
                Some("set_title") => {
                    let t = self.eval_text_field(a, "title", locals, now_ms);
                    self.effects.push(Effect::SetTitle(t));
                }
                _ => {}
            },
            "timer" => {
                let id = a.str_field("id").unwrap_or_default();
                let op = a.str_field("op").unwrap_or_default();
                for t in &mut self.timers {
                    if t.id == id {
                        match op {
                            "start" => {
                                t.running = true;
                                t.next_due = now_ms + t.every_ms;
                            }
                            "stop" => t.running = false,
                            "reset" => t.next_due = now_ms + t.every_ms,
                            _ => {}
                        }
                    }
                }
            }
            // Capability-gated system actions (fs/ai/notes) arrive with M6+.
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate;

    const TIMER_APP: &str = include_str!("../examples/pomodoro.conjure.json");

    #[test]
    fn pomodoro_ticks_and_notifies() {
        let doc = validate(TIMER_APP).expect("valid example");
        let mut app = AppInstance::new(doc, 0.0);
        // Start out running=false; press the start button.
        app.run_handler("start_pause", &HashMap::new(), 0.0);
        assert_eq!(app.state.get("running"), Some(&Value::Bool(true)));
        // Fast-forward the timer to one second before the end.
        app.state.insert("remaining".into(), Value::Num(1.0));
        app.tick(1_001.0); // fires `tick` → remaining 0 → done branch next tick
        app.tick(2_002.0);
        assert_eq!(app.state.get("running"), Some(&Value::Bool(false)));
        assert!(app
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Notify { .. })));
        assert!(!app.misbehaving);
    }

    #[test]
    fn step_budget_contains_runaway_emit() {
        // Two handlers emitting each other would recurse forever — the
        // validator rejects the cycle, but even if it slipped through, the
        // step budget must contain it.
        let doc_json = serde_json::json!({
            "conjure": "1.0",
            "manifest": { "id": "x", "name": "x" },
            "ui": { "w": "column" },
            "handlers": {
                "a": [ { "do": "emit", "handler": "b" } ],
                "b": [ { "do": "emit", "handler": "a" } ]
            }
        });
        let doc: crate::ast::Document = serde_json::from_value(doc_json).unwrap();
        let mut app = AppInstance::new(doc, 0.0);
        app.run_handler("a", &HashMap::new(), 0.0);
        assert!(app.misbehaving);
    }
}
