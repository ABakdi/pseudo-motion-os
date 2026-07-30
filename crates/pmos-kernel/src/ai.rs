//! AI agent manager (Architecture spec §4.5, AI System spec).
//!
//! Agents are kernel objects with provider bindings and conversation state.
//! The kernel builds complete HTTP requests (so API keys never leave it) and
//! hands them to the platform as directives; streamed chunks come back via
//! `Kernel::ai_chunk` and flow to the requesting process as `AiChunk` events.

use pmos_abi::{AgentId, AiProviderConfig, Pid, AGENT_APP_SMITH, AGENT_ASSISTANT};
use std::collections::HashMap;

/// A fully-built HTTP request for the platform to execute (fetch + SSE).
pub struct LlmRequest {
    pub agent: u32,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// 0 = Anthropic SSE framing, 1 = OpenAI SSE framing.
    pub kind: u8,
}

#[derive(Default)]
pub struct AiState {
    pub config: Option<AiProviderConfig>,
    /// Set when the config changed and should be persisted by the platform.
    pub config_dirty: bool,
    /// Requests waiting for the platform to pick up.
    pub pending: Vec<LlmRequest>,
    /// agent id → (requesting process, accumulated reply so far).
    pub streams: HashMap<u32, (Pid, String)>,
    history: HashMap<u32, Vec<(String, String)>>,
}

impl AiState {
    pub fn set_config(&mut self, cfg: AiProviderConfig, persist: bool) {
        self.config = Some(cfg);
        self.config_dirty = persist;
    }

    pub fn busy(&self, agent: AgentId) -> bool {
        self.streams.contains_key(&agent.0)
    }

    /// Queue a prompt for `agent` on behalf of `requester`.
    /// Returns an error string suited for direct display when impossible.
    pub fn prompt(&mut self, agent: AgentId, requester: Pid, msg: String) -> Result<(), String> {
        let Some(cfg) = self.config.clone() else {
            return Err("no AI provider configured — open Settings → AI".into());
        };
        if cfg.api_key.trim().is_empty() && cfg.kind == 0 {
            return Err("the Anthropic provider needs an API key — open Settings → AI".into());
        }
        if self.busy(agent) {
            return Err("that agent is still responding".into());
        }

        let history = self.history.entry(agent.0).or_default();
        history.push(("user".into(), msg));
        // Keep prompts bounded: last 12 turns.
        let tail: Vec<(String, String)> = history.iter().rev().take(12).rev().cloned().collect();
        let system = system_prompt(agent);

        let req = match cfg.kind {
            0 => {
                let url = if cfg.base_url.trim().is_empty() {
                    "https://api.anthropic.com/v1/messages".to_string()
                } else {
                    format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'))
                };
                let messages: Vec<serde_json::Value> = tail
                    .iter()
                    .map(|(role, content)| serde_json::json!({"role": role, "content": content}))
                    .collect();
                let body = serde_json::json!({
                    "model": cfg.model,
                    "max_tokens": 4096,
                    "system": system,
                    "messages": messages,
                    "stream": true,
                });
                LlmRequest {
                    agent: agent.0,
                    url,
                    headers: vec![
                        ("content-type".into(), "application/json".into()),
                        ("x-api-key".into(), cfg.api_key.clone()),
                        ("anthropic-version".into(), "2023-06-01".into()),
                        (
                            "anthropic-dangerous-direct-browser-access".into(),
                            "true".into(),
                        ),
                    ],
                    body: body.to_string(),
                    kind: 0,
                }
            }
            _ => {
                let base = if cfg.base_url.trim().is_empty() {
                    "https://api.openai.com".to_string()
                } else {
                    cfg.base_url.trim_end_matches('/').to_string()
                };
                let mut messages = vec![serde_json::json!({"role": "system", "content": system})];
                messages.extend(
                    tail.iter().map(
                        |(role, content)| serde_json::json!({"role": role, "content": content}),
                    ),
                );
                let body = serde_json::json!({
                    "model": cfg.model,
                    "messages": messages,
                    "stream": true,
                });
                let mut headers = vec![("content-type".into(), "application/json".into())];
                if !cfg.api_key.trim().is_empty() {
                    headers.push(("authorization".into(), format!("Bearer {}", cfg.api_key)));
                }
                LlmRequest {
                    agent: agent.0,
                    url: format!("{base}/v1/chat/completions"),
                    headers,
                    body: body.to_string(),
                    kind: 1,
                }
            }
        };

        self.streams.insert(agent.0, (requester, String::new()));
        self.pending.push(req);
        Ok(())
    }

    /// Platform delivered a streamed delta. Returns the requester to notify.
    pub fn chunk(&mut self, agent: u32, delta: &str, done: bool) -> Option<Pid> {
        let (requester, acc) = self.streams.get_mut(&agent)?;
        acc.push_str(delta);
        let requester = *requester;
        if done {
            let (_, full) = self.streams.remove(&agent).unwrap();
            self.history
                .entry(agent)
                .or_default()
                .push(("assistant".into(), full));
        }
        Some(requester)
    }

    /// App Smith conversations reset per conjuring (each app is fresh).
    pub fn reset_history(&mut self, agent: AgentId) {
        self.history.remove(&agent.0);
    }
}

fn system_prompt(agent: AgentId) -> String {
    if agent == AGENT_ASSISTANT {
        return "You are the Pseudo Motion OS System Assistant, living inside a \
                browser-based 3D desktop controlled by hand gestures. Answer \
                concisely and helpfully. You cannot yet run system commands; \
                if asked to create an app, tell the user to phrase it as \
                'make …' or 'create …' so the App Smith handles it."
            .to_string();
    }
    if agent == AGENT_APP_SMITH {
        // The compact Conjure contract (App DSL spec, v1 subset).
        return r##"You are the App Smith of Pseudo Motion OS. You create small apps as Conjure JSON documents. Reply with EXACTLY ONE JSON object and NOTHING else — no prose, no markdown fences.

Schema:
{"conjure":"1.0","manifest":{"id":"kebab-case-id","name":"Name","icon":"one emoji","description":"...","window":{"size":[W,H],"resizable":true}},"state":{"name":{"type":"number|string|bool|list","value":...}},"ui":<node>,"handlers":{"name":[<action>...]},"timers":[{"id":"t","every_ms":1000,"handler":"tick","autostart":true}]}

A <node> is {"w":"<widget>",...props,"children":[<node>...]}.
Widgets: column,row (props: spacing,align) · group (title) · scroll · separator · spacer (size) · label (text,size:"small|body|heading|title",color:"#rrggbb",bold) · button (text,on_click) · text_input (bind,placeholder,multiline) · slider (bind,min,max,step,on_change) · checkbox/toggle (bind,text,on_change) · progress (value 0..1) · if (cond, then-children in "children").

An <action> is {"do":"<verb>",...}: set{target,value} · toggle{target} · inc{target,by} · push{target,value} · remove_at{target,index} · clear{target} · if{cond,then:[...],else:[...]} · emit{handler} (no cycles) · notify{title,body} · window{op:"close"|"set_title",title} · timer{op:"start"|"stop"|"reset",id}.

Expressions go in "${...}" inside strings: state names as identifiers, numbers, 'strings', arithmetic + - * / %, comparisons, && || !, ternary c ? a : b, lists [..], indexing x[i]. Functions: math.abs/min/max/floor/ceil/round/sqrt/pow/clamp · str.len/upper/lower/trim/contains · list.len/get/sum/contains · map.get/has · fmt.mmss(secs)/fmt.num(x,decimals) · time.now(). Text props may mix text and ${...}. bind must name a /state entry; on_* must name a /handlers entry.

Rules: every state entry used must be declared; every handler referenced must exist; timers ≥100ms; keep apps small and delightful; window size fits the content. If the user reports validation errors, return the FULL corrected document."##
            .to_string();
    }
    "You are a PMOS agent.".to_string()
}
