// webllm-worker.js — WebLLM inference off the main thread (AI System §2).
//
// The MLCEngine used to run on the main thread, so every token generated
// janked the whole OS (camera loop, cursor, physics). This worker hosts the
// engine; webllm.js drives it through WebLLM's own message protocol via
// CreateWebWorkerMLCEngine. WebGPU-in-workers is available in Chromium; on
// browsers where it isn't, webllm.js falls back to the main-thread engine.
import { WebWorkerMLCEngineHandler } from "https://esm.run/@mlc-ai/web-llm@0.2.79";

const handler = new WebWorkerMLCEngineHandler();
onmessage = (msg) => handler.onmessage(msg);
