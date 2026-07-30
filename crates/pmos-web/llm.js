// LLM streaming bridge (AI System spec §2).
// The kernel builds complete requests (URL + headers + body) so API keys
// never appear here as configuration — this file only executes the fetch and
// parses the SSE stream, forwarding deltas into the WASM kernel.
(() => {
  async function request(agent, url, headersJson, body, kind) {
    const chunk = (delta, done) => window.wasmBindings.pmos_ai_chunk(agent, delta, done);
    let headers;
    try {
      headers = JSON.parse(headersJson);
    } catch {
      chunk("⚠ bad request headers", true);
      return;
    }
    let resp;
    try {
      resp = await fetch(url, { method: "POST", headers, body });
    } catch (err) {
      chunk("⚠ network error: " + err.message + " (check the base URL and your connection)", true);
      return;
    }
    if (!resp.ok) {
      let detail = "";
      try {
        detail = (await resp.text()).slice(0, 400);
      } catch {}
      chunk(`⚠ provider returned ${resp.status}: ${detail}`, true);
      return;
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buf = "";
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        // SSE frames are separated by blank lines.
        const frames = buf.split("\n\n");
        buf = frames.pop();
        for (const frame of frames) {
          for (const line of frame.split("\n")) {
            if (!line.startsWith("data:")) continue;
            const data = line.slice(5).trim();
            if (kind === 1) {
              // OpenAI framing.
              if (data === "[DONE]") {
                chunk("", true);
                return;
              }
              try {
                const j = JSON.parse(data);
                const delta = j.choices?.[0]?.delta?.content;
                if (delta) chunk(delta, false);
              } catch {}
            } else {
              // Anthropic framing.
              try {
                const j = JSON.parse(data);
                if (j.type === "content_block_delta" && j.delta?.text) {
                  chunk(j.delta.text, false);
                } else if (j.type === "message_stop") {
                  chunk("", true);
                  return;
                } else if (j.type === "error") {
                  chunk("⚠ " + (j.error?.message || "provider error"), true);
                  return;
                }
              } catch {}
            }
          }
        }
      }
      chunk("", true); // stream ended without an explicit stop marker
    } catch (err) {
      chunk("⚠ stream interrupted: " + err.message, true);
    }
  }

  window.pmosLlm = { request };
})();
