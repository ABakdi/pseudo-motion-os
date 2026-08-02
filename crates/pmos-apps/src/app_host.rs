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

/// A list prop: a literal JSON array, or a `${…}` expression yielding a list.
fn list_prop(app: &AppInstance, node: &Node, key: &str, now_ms: f64) -> Vec<Value> {
    match node.props.get(key) {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .map(|v| match v {
                serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
                serde_json::Value::Bool(b) => Value::Bool(*b),
                other => Value::Str(other.as_str().unwrap_or_default().to_string()),
            })
            .collect(),
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
            match expr::parse(src).and_then(|e| expr::eval(&e, &env)) {
                Ok(Value::List(items)) => items,
                Ok(other) => vec![other],
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
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
        "dropdown" => {
            let Some(bind) = str_prop(node, "bind").map(|s| s.to_string()) else {
                return;
            };
            let options = list_prop(app, node, "options", now_ms)
                .into_iter()
                .map(|v| v.display())
                .collect::<Vec<_>>();
            let mut current = app
                .state
                .get(&bind)
                .map(|v| v.display())
                .unwrap_or_default();
            let mut changed = false;
            egui::ComboBox::from_id_salt(("conjure-dropdown", &bind))
                .selected_text(current.clone())
                .show_ui(ui, |ui| {
                    for opt in &options {
                        if ui
                            .selectable_value(&mut current, opt.clone(), opt)
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
            if changed {
                app.state.insert(bind, Value::Str(current));
                fire(app, node, "on_change", now_ms);
            }
        }
        "list_view" => {
            // v1 subset (App DSL §5): `items` list + `template` string with
            // `item`/`i` locals; optional `on_remove` handler gets the same
            // locals. (The full widget-template `item_ui` form is future.)
            let items = list_prop(app, node, "items", now_ms);
            let template = str_prop(node, "template").unwrap_or("${item}").to_string();
            let on_remove = str_prop(node, "on_remove").map(|s| s.to_string());
            for (i, item) in items.into_iter().enumerate().take(4096) {
                let mut locals = HashMap::new();
                locals.insert("item".to_string(), item);
                locals.insert("i".to_string(), Value::Num(i as f64));
                let env = Env {
                    state: &app.state,
                    locals: &locals,
                    now_ms,
                };
                let line = expr::parse_template(&template)
                    .and_then(|t| expr::eval_template(&t, &env))
                    .unwrap_or_else(|_| template.clone());
                ui.horizontal(|ui| {
                    ui.label(line);
                    if let Some(handler) = &on_remove {
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("✕").clicked() {
                                    app.run_handler(handler, &locals, now_ms);
                                }
                            },
                        );
                    }
                });
            }
        }
        "canvas" => {
            // v1 canvas (App DSL §5): a fixed-size surface drawn from a list
            // of draw-op maps — {"kind":"circle","x":..,"y":..,"r":..,
            // "color":"#hex"} / rect(w,h) / line(x2,y2) / text(text,size).
            // `on_pointer` fires with locals px/py on click.
            let w = eval_num(app, node, "width", 240.0, now_ms) as f32;
            let h = eval_num(app, node, "height", 160.0, now_ms) as f32;
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 6.0, egui::Color32::from_black_alpha(120));
            let ops = list_prop(app, node, "draw", now_ms);
            for op in ops.iter().take(512) {
                let Value::Map(m) = op else { continue };
                let num = |k: &str| m.get(k).map(|v| v.as_num() as f32).unwrap_or(0.0);
                let color = m
                    .get("color")
                    .map(|v| v.display())
                    .and_then(|c| {
                        u32::from_str_radix(c.trim_start_matches('#'), 16).ok()
                    })
                    .map(|rgb| {
                        egui::Color32::from_rgb(
                            (rgb >> 16) as u8,
                            (rgb >> 8) as u8,
                            rgb as u8,
                        )
                    })
                    .unwrap_or(theme::accent_a());
                let o = rect.min;
                match m.get("kind").map(|v| v.display()).as_deref() {
                    Some("circle") => {
                        painter.circle_filled(
                            o + egui::vec2(num("x"), num("y")),
                            num("r").max(0.5),
                            color,
                        );
                    }
                    Some("rect") => {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                o + egui::vec2(num("x"), num("y")),
                                egui::vec2(num("w").max(0.0), num("h").max(0.0)),
                            ),
                            2.0,
                            color,
                        );
                    }
                    Some("line") => {
                        painter.line_segment(
                            [
                                o + egui::vec2(num("x"), num("y")),
                                o + egui::vec2(num("x2"), num("y2")),
                            ],
                            egui::Stroke::new(num("width").max(1.0), color),
                        );
                    }
                    Some("text") => {
                        painter.text(
                            o + egui::vec2(num("x"), num("y")),
                            egui::Align2::LEFT_TOP,
                            m.get("text").map(|v| v.display()).unwrap_or_default(),
                            egui::FontId::proportional(num("size").max(8.0)),
                            color,
                        );
                    }
                    _ => {}
                }
            }
            if resp.clicked() {
                if let (Some(pos), Some(handler)) =
                    (resp.interact_pointer_pos(), str_prop(node, "on_pointer"))
                {
                    let mut locals = HashMap::new();
                    locals.insert("px".into(), Value::Num((pos.x - rect.min.x) as f64));
                    locals.insert("py".into(), Value::Num((pos.y - rect.min.y) as f64));
                    app.run_handler(&handler.to_string(), &locals, now_ms);
                }
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
