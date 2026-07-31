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

  // q4f16 model builds need the WebGPU `shader-f16` feature; many Linux
  // Vulkan drivers (and some browsers) lack or mishandle it, failing with
  // "Invalid ShaderModule" at pipeline creation. The q4f32 variants of the
  // same models run everywhere — detect up front, and also fall back when a
  // driver *claims* f16 support but still rejects the shaders.
  let f16Support = null;
  async function supportsF16() {
    if (f16Support !== null) return f16Support;
    try {
      const adapter = await navigator.gpu.requestAdapter();
      f16Support = !!adapter?.features?.has("shader-f16");
    } catch {
      f16Support = false;
    }
    log("shader-f16 support:", f16Support);
    return f16Support;
  }

  async function infer(chunk, model, messages) {
    const engine = await getEngine(model, (p) => {
      const pct = Math.round((p.progress || 0) * 100);
      chunk("\r⏳ " + (p.text || `loading model… ${pct}%`), false);
    });
    chunk("\r", false); // model ready — clear the progress line
    const stream = await engine.chat.completions.create({
      messages,
      stream: true,
      temperature: 0.7,
      max_tokens: 2048,
    });
    for await (const c of stream) {
      const delta = c.choices?.[0]?.delta?.content;
      if (delta) chunk(delta, false);
    }
    chunk("", true);
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
    let model = body.model;
    if (model.includes("q4f16") && !(await supportsF16())) {
      model = model.replace("q4f16", "q4f32");
      log("GPU lacks shader-f16 — using", model);
    }
    try {
      await infer(chunk, model, body.messages);
    } catch (err) {
      log("error:", err);
      // Driver claimed f16 but rejected the shaders → one retry on f32.
      if (model.includes("q4f16")) {
        const alt = model.replace("q4f16", "q4f32");
        log("retrying with", alt);
        chunk("\r⏳ your GPU rejected the fp16 model — switching to the fp32 variant…", false);
        try {
          await infer(chunk, alt, body.messages);
          f16Support = false; // stop trying f16 this session
          return;
        } catch (err2) {
          log("fallback error:", err2);
          err = err2;
        }
      }
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
