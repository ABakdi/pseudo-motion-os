//! The App Host: renders Conjure documents with egui and routes widget
//! events into the interpreter (App DSL spec §5; Architecture §3.3).
//! One host per conjured app; the interpreter's step budgets contain any
//! runaway document without affecting the rest of the shell.

use crate::theme;
use pmos_conjure::ast::Node;
use pmos_conjure::expr::{self, Env, Value};
use pmos_conjure::{AppInstance, Effect};
use std::collections::HashMap;

/// Render one frame of a conjured app; returns drained effects.
pub fn ui(app: &mut AppInstance, ui: &mut egui::Ui, now_ms: f64) -> Vec<Effect> {
    app.tick(now_ms);
    if app.misbehaving {
        ui.colored_label(
            egui::Color32::from_rgb(0xff, 0x9d, 0x6b),
            "⚠ this app exceeded its step budget and was stopped",
        );
        ui.separator();
    }
    let root = app.doc.ui.clone();
    render_node(app, &root, ui, now_ms);
    std::mem::take(&mut app.effects)
}

fn eval_str(app: &AppInstance, raw: &str, now_ms: f64) -> String {
    let locals = HashMap::new();
    let env = Env {
        state: &app.state,
        locals: &locals,
        now_ms,
    };
    expr::parse_template(raw)
        .and_then(|t| expr::eval_template(&t, &env))
        .unwrap_or_else(|_| raw.to_string())
}

fn eval_num(app: &AppInstance, node: &Node, key: &str, default: f64, now_ms: f64) -> f64 {
    match node.props.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(serde_json::Value::String(s)) => {
            let locals = HashMap::new();
            let env = Env {
                state: &app.state,
                locals: &locals,
                now_ms,
            };
            let src = s
                .strip_prefix("${")
                .and_then(|x| x.strip_suffix('}'))
                .unwrap_or(s);
            expr::parse(src)
                .and_then(|e| expr::eval(&e, &env))
                .map(|v| v.as_num())
                .unwrap_or(default)
        }
        _ => default,
    }
}

fn text_prop(app: &AppInstance, node: &Node, key: &str, now_ms: f64) -> String {
    node.props
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| eval_str(app, s, now_ms))
        .unwrap_or_default()
}

fn str_prop<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    node.props.get(key).and_then(|v| v.as_str())
}

fn fire(app: &mut AppInstance, node: &Node, event: &str, now_ms: f64) {
    if let Some(handler) = str_prop(node, event) {
        app.run_handler(&handler.to_string(), &HashMap::new(), now_ms);
    }
}

fn render_children(app: &mut AppInstance, node: &Node, ui: &mut egui::Ui, now_ms: f64) {
    let children = node.children.clone();
    for child in &children {
        render_node(app, child, ui, now_ms);
    }
}

fn render_node(app: &mut AppInstance, node: &Node, ui: &mut egui::Ui, now_ms: f64) {
    match node.w.as_str() {
        "column" => {
            let spacing = eval_num(app, node, "spacing", 6.0, now_ms) as f32;
            ui.spacing_mut().item_spacing.y = spacing;
            if str_prop(node, "align") == Some("center") {
                ui.vertical_centered(|ui| render_children(app, node, ui, now_ms));
            } else {
                ui.vertical(|ui| render_children(app, node, ui, now_ms));
            }
        }
        "row" => {
            let spacing = eval_num(app, node, "spacing", 6.0, now_ms) as f32;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                render_children(app, node, ui, now_ms);
            });
        }
        "group" => {
            let title = text_prop(app, node, "title", now_ms);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                if !title.is_empty() {
                    ui.strong(title);
                }
                render_children(app, node, ui, now_ms);
            });
        }
        "scroll" => {
            egui::ScrollArea::vertical()
                .max_height(ui.available_height())
                .show(ui, |ui| render_children(app, node, ui, now_ms));
        }
        "separator" => {
            ui.separator();
        }
        "spacer" => {
            ui.add_space(eval_num(app, node, "size", 8.0, now_ms) as f32);
        }
        "label" => {
            let text = text_prop(app, node, "text", now_ms);
            let mut rich = egui::RichText::new(text);
            match str_prop(node, "size") {
                Some("small") => rich = rich.size(11.0),
                Some("heading") => rich = rich.size(18.0),
                Some("title") => rich = rich.size(26.0),
                _ => {}
            }
            if let Some(color) = str_prop(node, "color") {
                let c = eval_str(app, color, now_ms);
                if let Ok(rgb) = u32::from_str_radix(c.trim_start_matches('#'), 16) {
                    rich = rich.color(egui::Color32::from_rgb(
                        (rgb >> 16) as u8,
                        (rgb >> 8) as u8,
                        rgb as u8,
                    ));
                }
            }
            if node.props.get("bold").and_then(|v| v.as_bool()) == Some(true) {
                rich = rich.strong();
            }
            ui.label(rich);
        }
        "button" => {
            let text = text_prop(app, node, "text", now_ms);
            if ui.button(text).clicked() {
                fire(app, node, "on_click", now_ms);
            }
        }
        "text_input" => {
            let Some(bind) = str_prop(node, "bind").map(|s| s.to_string()) else {
                return;
            };
            let mut value = app
                .state
                .get(&bind)
                .map(|v| v.display())
                .unwrap_or_default();
            let hint = text_prop(app, node, "placeholder", now_ms);
            let multiline = node.props.get("multiline").and_then(|v| v.as_bool()) == Some(true);
            let resp = if multiline {
                ui.add(
                    egui::TextEdit::multiline(&mut value)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
            } else {
                ui.add(
                    egui::TextEdit::singleline(&mut value)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
            };
            if resp.changed() {
                app.state.insert(bind, Value::Str(value));
                fire(app, node, "on_change", now_ms);
            }
        }
        "slider" => {
            let Some(bind) = str_prop(node, "bind").map(|s| s.to_string()) else {
                return;
            };
            let min = eval_num(app, node, "min", 0.0, now_ms);
            let max = eval_num(app, node, "max", 100.0, now_ms);
            let mut value = app.state.get(&bind).map(|v| v.as_num()).unwrap_or(min);
            let resp = ui.add(egui::Slider::new(&mut value, min..=max));
            if resp.changed() {
                app.state.insert(bind, Value::Num(value));
                fire(app, node, "on_change", now_ms);
            }
        }
        "checkbox" | "toggle" => {
            let Some(bind) = str_prop(node, "bind").map(|s| s.to_string()) else {
                return;
            };
            let mut value = app.state.get(&bind).map(|v| v.truthy()).unwrap_or(false);
            let text = text_prop(app, node, "text", now_ms);
            if ui.checkbox(&mut value, text).changed() {
                app.state.insert(bind, Value::Bool(value));
                fire(app, node, "on_change", now_ms);
            }
        }
        "progress" => {
            let v = eval_num(app, node, "value", 0.0, now_ms).clamp(0.0, 1.0) as f32;
            ui.add(
                egui::ProgressBar::new(v)
                    .fill(theme::accent_a().gamma_multiply(0.8))
                    .desired_width(ui.available_width().min(280.0)),
            );
        }
        "if" => {
            let cond = eval_num(app, node, "cond", 0.0, now_ms) != 0.0
                || node
                    .props
                    .get("cond")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        let locals = HashMap::new();
                        let env = Env {
                            state: &app.state,
                            locals: &locals,
                            now_ms,
                        };
                        let src = s
                            .strip_prefix("${")
                            .and_then(|x| x.strip_suffix('}'))
                            .unwrap_or(s);
                        expr::parse(src)
                            .and_then(|e| expr::eval(&e, &env))
                            .map(|v| v.truthy())
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
            if cond {
                render_children(app, node, ui, now_ms);
            }
        }
        _ => {}
    }
}
