// webllm.js — in-browser LLM inference via WebLLM/MLC (AI System §2).
//
// The DEFAULT AI provider: free, no API key, runs on the user's GPU through
// WebGPU (which PMOS already requires). The model downloads once (~0.6–2 GB
// depending on the performance tier picked in Settings → AI) and is cached
// by the browser; after that it works offline. Prompts never leave the
// machine.
//
// The WebLLM library itself is imported lazily from the CDN on first use, so
// it costs nothing at boot. Progress lines are streamed as '\r'-prefixed
// AiChunk deltas — the kernel and UIs REPLACE accumulated text on '\r', so
// download progress never pollutes the conversation.
(function () {
  const log = (...a) => console.log("[pmos-webllm]", ...a);

  let enginePromise = null;
  let engineModel = null;
  let chain = Promise.resolve(); // serialize requests — one inference at a time

  function getEngine(model, onProgress) {
    if (enginePromise && engineModel === model) return enginePromise;
    engineModel = model;
    enginePromise = (async () => {
      const { CreateMLCEngine } = await import(
        "https://esm.run/@mlc-ai/web-llm@0.2.79"
      );
      log("loading", model);
      return CreateMLCEngine(model, {
        initProgressCallback: (p) => onProgress(p),
      });
    })();
    enginePromise.catch(() => {
      // Failed loads must not poison future attempts.
      enginePromise = null;
      engineModel = null;
    });
    return enginePromise;
  }

  async function run(agent, bodyJson) {
    const chunk = (delta, done) => window.wasmBindings.pmos_ai_chunk(agent, delta, done);
    let body;
    try {
      body = JSON.parse(bodyJson);
    } catch {
      chunk("⚠ bad in-browser AI request", true);
      return;
    }
    if (!navigator.gpu) {
      chunk("⚠ in-browser AI needs WebGPU", true);
      return;
    }
    try {
      const engine = await getEngine(body.model, (p) => {
        const pct = Math.round((p.progress || 0) * 100);
        chunk("\r⏳ " + (p.text || `loading model… ${pct}%`), false);
      });
      chunk("\r", false); // model ready — clear the progress line
      const stream = await engine.chat.completions.create({
        messages: body.messages,
        stream: true,
        temperature: 0.7,
        max_tokens: 2048,
      });
      for await (const c of stream) {
        const delta = c.choices?.[0]?.delta?.content;
        if (delta) chunk(delta, false);
      }
      chunk("", true);
    } catch (err) {
      log("error:", err);
      chunk(
        "\r⚠ in-browser model failed: " +
          (err?.message || err) +
          " — try a smaller tier in Settings → AI",
        true
      );
    }
  }

  window.pmosWebLlm = {
    request(agent, body) {
      chain = chain.then(() => run(agent, body));
      return chain;
    },
  };
})();
