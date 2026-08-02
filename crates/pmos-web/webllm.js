// webllm.js — in-browser LLM inference via WebLLM/MLC (AI System §2).
//
// The DEFAULT AI provider: free, no API key, runs on the user's GPU through
// WebGPU (which PMOS already requires). The model downloads once (~0.6–2 GB
// depending on the performance tier picked in Settings → AI) and is cached
// by the browser; after that it works offline. Prompts never leave the
// machine.
//
// Hardware fit (user-requested): at boot the machine is probed (RAM via
// navigator.deviceMemory, GPU headroom via adapter.limits.maxBufferSize) and
// the recommended tier is reported to the kernel (/sys/llm_tier — Settings
// marks it "fits this machine"). At run time, a model that fails to load or
// infer steps down automatically: fp16→fp32, then tier by tier, with the
// progress line explaining each step.
//
// One MLCEngine instance, switched with engine.reload() — creating a second
// engine over a live one corrupts the TVM runtime ("PackedFunc has already
// been disposed", user-reported when switching to the Quality tier).
(function () {
  const log = (...a) => console.log("[pmos-webllm]", ...a);

  // Tier order matches Settings → AI (Fast, Balanced, Quality).
  const TIER_PREFIX = [
    "Qwen2.5-0.5B-Instruct",
    "Llama-3.2-1B-Instruct",
    "Qwen2.5-3B-Instruct",
  ];
  const tierOf = (model) => TIER_PREFIX.findIndex((p) => model.startsWith(p));
  const modelFor = (tier, f16) =>
    TIER_PREFIX[tier] + (f16 ? "-q4f16_1-MLC" : "-q4f32_1-MLC");

  // ---------- machine probe → recommended tier ----------
  async function probeTier() {
    if (!navigator.gpu) return 0;
    const memGB = navigator.deviceMemory || 4; // Chrome caps the answer at 8
    let maxBuf = 0;
    try {
      const adapter = await navigator.gpu.requestAdapter();
      maxBuf = Number(adapter?.limits?.maxBufferSize || 0);
    } catch {}
    log(`probe: ram>=${memGB}GB, maxBuffer=${(maxBuf / 2 ** 30).toFixed(1)}GB`);
    if (memGB >= 8 && maxBuf >= 3.5 * 2 ** 30) return 2;
    if (memGB >= 6 || maxBuf >= 1.5 * 2 ** 30) return 1;
    return 0;
  }
  probeTier().then((tier) => {
    log("recommended tier:", tier);
    const push = () => {
      if (window.wasmBindings?.pmos_llm_tier) window.wasmBindings.pmos_llm_tier(tier);
      else setTimeout(push, 500);
    };
    push();
  });

  // ---------- f16 capability ----------
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

  // ---------- single engine, reload to switch ----------
  let engine = null;
  let engineModel = null;
  let progressCb = null; // registered once; retargeted per request
  let chain = Promise.resolve(); // serialize requests

  async function getEngine(model, onProgress) {
    progressCb = onProgress;
    if (engine && engineModel === model) return engine;
    const lib = await import("https://esm.run/@mlc-ai/web-llm@0.2.79");
    if (engine) {
      log("reloading engine:", engineModel, "→", model);
      await engine.reload(model);
    } else {
      engine = await lib.CreateMLCEngine(model, {
        initProgressCallback: (p) => progressCb?.(p),
      });
    }
    engineModel = model;
    return engine;
  }

  async function teardown() {
    const e = engine;
    engine = null;
    engineModel = null;
    if (e) {
      try {
        await e.unload();
      } catch {}
    }
  }

  async function infer(chunk, model, messages) {
    const eng = await getEngine(model, (p) => {
      const pct = Math.round((p.progress || 0) * 100);
      chunk("\r⏳ " + (p.text || `loading model… ${pct}%`), false);
    });
    chunk("\r", false); // model ready — clear the progress line
    const stream = await eng.chat.completions.create({
      messages,
      stream: true,
      temperature: 0.7,
      max_tokens: 3500,
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

    // Candidate ladder: requested model (dtype-adjusted) → its fp32 variant
    // → each smaller tier on fp32. A '\r' before each retry resets any
    // partial output so fallbacks never duplicate text.
    const f16ok = await supportsF16();
    const wantF16 = f16ok && !body.model.includes("q4f32");
    const reqTier = tierOf(body.model);
    const candidates = [];
    if (reqTier >= 0) {
      candidates.push(modelFor(reqTier, wantF16));
      if (wantF16) candidates.push(modelFor(reqTier, false));
      for (let t = reqTier - 1; t >= 0; t--) candidates.push(modelFor(t, false));
    } else {
      // Unknown/custom model id: honor it, with only the dtype fallback.
      const adjusted =
        !f16ok && body.model.includes("q4f16")
          ? body.model.replace("q4f16", "q4f32")
          : body.model;
      candidates.push(adjusted);
      if (adjusted.includes("q4f16")) candidates.push(adjusted.replace("q4f16", "q4f32"));
    }

    let lastErr = null;
    for (const model of candidates) {
      if (lastErr !== null) {
        chunk(`\r⏳ that model doesn't fit this machine — trying ${model}…`, false);
        await teardown(); // a failed runtime can't be reloaded safely
      }
      try {
        await infer(chunk, model, body.messages);
        if (lastErr !== null) log("settled on", model);
        return;
      } catch (err) {
        log("failed on", model, "—", err);
        lastErr = err;
      }
    }
    await teardown();
    chunk(
      "\r⚠ in-browser AI failed on every model this machine could try: " +
        (lastErr?.message || lastErr),
      true
    );
  }

  window.pmosWebLlm = {
    request(agent, body) {
      chain = chain.then(() => run(agent, body));
      return chain;
    },
  };
})();
