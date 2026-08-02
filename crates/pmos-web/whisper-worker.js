// whisper-worker.js — in-browser speech-to-text with Whisper (AI System §5 v2).
//
// Runs OpenAI's Whisper (tiny) via transformers.js: works in ANY browser
// (Brave, Firefox, Safari — no Google speech backend needed), audio never
// leaves the machine, and the ~40 MB model downloads once and is cached by
// the browser. WebGPU when available, WASM otherwise.
//
// Protocol (speech.js ↔ this worker):
//   in : {type:'init', lang}                       → load the pipeline
//   in : {type:'transcribe', audio, rate, final}   → Float32 PCM at `rate` Hz
//   out: {type:'progress', pct}                    → model download progress
//   out: {type:'ready', device}
//   out: {type:'result', text, final}
//   out: {type:'error', error}

let asr = null;
let lang = "en";
let multilingual = false;

/// Linear resample to Whisper's expected 16 kHz.
function to16k(audio, rate) {
  if (rate === 16000) return audio;
  const n = Math.round((audio.length * 16000) / rate);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const x = (i * rate) / 16000;
    const j = Math.floor(x);
    const f = x - j;
    out[i] = audio[j] * (1 - f) + (audio[Math.min(j + 1, audio.length - 1)] || 0) * f;
  }
  return out;
}

/// Whisper emits artifacts on near-silence — strip them.
function clean(text) {
  return text
    .replace(/\[[^\]]*\]|\([^)]*\)/g, "") // [BLANK_AUDIO], (wind blowing)…
    .replace(/\s+/g, " ")
    .trim();
}

async function init(navLang, size) {
  const { pipeline } = await import(
    "https://cdn.jsdelivr.net/npm/@huggingface/transformers@3.7.1"
  );
  lang = (navLang || "en").split("-")[0].toLowerCase();
  multilingual = lang !== "en";
  // Model size from Settings → Voice: tiny (~40 MB) / base (~80 MB) /
  // small (~250 MB) — bigger is more accurate, downloads once.
  const sz = ["tiny", "base", "small"].includes(size) ? size : "tiny";
  const model = multilingual
    ? `onnx-community/whisper-${sz}`
    : `onnx-community/whisper-${sz}.en`;

  let lastPct = -1;
  const progress_callback = (p) => {
    if (p.status === "progress" && p.total > 8_000_000) {
      const pct = Math.floor((p.loaded / p.total) * 100);
      if (pct !== lastPct) {
        lastPct = pct;
        postMessage({ type: "progress", pct });
      }
    }
  };

  try {
    asr = await pipeline("automatic-speech-recognition", model, {
      device: "webgpu",
      dtype: { encoder_model: "fp32", decoder_model_merged: "q4" },
      progress_callback,
    });
    postMessage({ type: "ready", device: "webgpu" });
  } catch (err) {
    console.warn("[pmos-whisper] webgpu failed, falling back to wasm:", err);
    asr = await pipeline("automatic-speech-recognition", model, {
      device: "wasm",
      dtype: "q8",
      progress_callback,
    });
    postMessage({ type: "ready", device: "wasm" });
  }
}

self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    try {
      await init(m.lang, m.size);
    } catch (err) {
      postMessage({ type: "error", error: String(err) });
    }
    return;
  }
  if (m.type === "transcribe") {
    if (!asr) return;
    try {
      const audio = to16k(new Float32Array(m.audio), m.rate);
      const opts = multilingual ? { language: lang, task: "transcribe" } : {};
      const out = await asr(audio, opts);
      postMessage({ type: "result", text: clean(out.text || ""), final: m.final });
    } catch (err) {
      postMessage({ type: "error", error: String(err) });
    }
  }
};
