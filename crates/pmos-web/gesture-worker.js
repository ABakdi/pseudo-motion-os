// The gesture worker (Architecture §3): MediaPipe HandLandmarker inference,
// GPU-delegated, off the main thread. Receives ImageBitmaps, posts flat
// landmark arrays (hands × 21 × [x,y,z], normalized camera space).
const VISION = "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.14";
const MODEL =
  "https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task";

// MediaPipe's wasm loader still calls importScripts(), which module workers
// forbid — shim it with synchronous XHR + indirect eval (global scope).
globalThis.importScripts = (...urls) => {
  for (const url of urls) {
    const xhr = new XMLHttpRequest();
    xhr.open("GET", url, false);
    xhr.send();
    if (xhr.status < 200 || xhr.status >= 300) {
      throw new Error("importScripts shim failed for " + url);
    }
    (0, eval)(xhr.responseText);
  }
};

let landmarker = null;
let fileset = null;
let makeLandmarker = null;
// Builds must be strictly serialized: two concurrent createFromOptions calls
// in one worker race and can leave the landmarker permanently broken. While
// a build runs, the newest requested config waits its turn.
let building = false;
let pendingCfg = null;

async function build(opts = {}) {
  const cfg = (delegate) => ({
    baseOptions: { modelAssetPath: MODEL, delegate },
    runningMode: "VIDEO",
    numHands: opts.numHands ?? 2,
    minHandDetectionConfidence: opts.detConf ?? 0.5,
    minTrackingConfidence: opts.trackConf ?? 0.5,
  });
  try {
    landmarker = await makeLandmarker(fileset, cfg("GPU"));
  } catch (e) {
    // Some machines/browsers can't create the GPU delegate — fall back.
    console.warn("[pmos gesture-worker] GPU delegate failed, using CPU:", e);
    landmarker = await makeLandmarker(fileset, cfg("CPU"));
  }
}

async function reconfigure(opts) {
  if (building) {
    pendingCfg = opts;
    return;
  }
  building = true;
  const old = landmarker;
  landmarker = null;
  old?.close?.();
  try {
    await build(opts);
  } catch (err) {
    postMessage({ type: "error", message: String(err) });
  }
  building = false;
  if (pendingCfg) {
    const next = pendingCfg;
    pendingCfg = null;
    reconfigure(next);
  }
}

(async () => {
  building = true;
  try {
    const mod = await import(`${VISION}/vision_bundle.mjs`);
    const { FilesetResolver, HandLandmarker } = mod;
    FaceLandmarkerClass = mod.FaceLandmarker;
    fileset = await FilesetResolver.forVisionTasks(`${VISION}/wasm`);
    makeLandmarker = (f, o) => HandLandmarker.createFromOptions(f, o);
    await build();
    postMessage({ type: "ready" });
  } catch (e) {
    postMessage({ type: "error", message: String(e) });
  }
  building = false;
  if (pendingCfg) {
    const next = pendingCfg;
    pendingCfg = null;
    reconfigure(next);
  }
})();

// ---- face layer (M10, opt-in from Settings → Face) ----
// Blendshapes only ever leave this worker — never pixels.
let FaceLandmarkerClass = null;
let faceLandmarker = null;
let faceWanted = false;
let faceBuilding = false;

async function ensureFace() {
  if (faceLandmarker || faceBuilding || !faceWanted || !fileset || !FaceLandmarkerClass) return;
  faceBuilding = true;
  try {
    faceLandmarker = await FaceLandmarkerClass.createFromOptions(fileset, {
      baseOptions: {
        modelAssetPath:
          "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task",
        delegate: "GPU",
      },
      runningMode: "VIDEO",
      numFaces: 1,
      outputFaceBlendshapes: true,
    });
    console.log("[pmos gesture-worker] face landmarker ready");
  } catch (err) {
    console.warn("[pmos gesture-worker] face model failed:", err);
    faceWanted = false;
  }
  faceBuilding = false;
}

onmessage = async (e) => {
  const m = e.data;
  if (m.type === "face") {
    faceWanted = !!m.enable;
    if (!faceWanted) faceLandmarker = null;
    else ensureFace();
    return;
  }
  if (m.type === "configure") {
    if (fileset || building) reconfigure(m);
    return;
  }
  if (m.type !== "frame") {
    m.bitmap?.close?.();
    return;
  }
  // Contract: EVERY frame message gets a reply — the main thread's busy
  // flag depends on it. Dropped/failed frames reply with zero hands.
  let hands = 0;
  let data = new Float32Array(0);
  try {
    if (landmarker) {
      const result = landmarker.detectForVideo(m.bitmap, m.ts);
      hands = result.landmarks.length;
      data = new Float32Array(hands * 63);
      result.landmarks.forEach((hand, h) =>
        hand.forEach((p, i) => {
          const o = (h * 21 + i) * 3;
          data[o] = p.x;
          data[o + 1] = p.y;
          data[o + 2] = p.z;
        })
      );
    }
    if (faceWanted && faceLandmarker) {
      const fr = faceLandmarker.detectForVideo(m.bitmap, m.ts);
      const cats = fr.faceBlendshapes?.[0]?.categories;
      if (cats) {
        const get = (n) => cats.find((c) => c.categoryName === n)?.score ?? 0;
        postMessage({
          type: "face",
          blinkL: get("eyeBlinkLeft"),
          blinkR: get("eyeBlinkRight"),
          jaw: get("jawOpen"),
        });
      }
    }
  } catch (err) {
    console.warn("[pmos gesture-worker] detect failed:", err);
  } finally {
    m.bitmap?.close?.();
  }
  postMessage({ type: "hands", hands, data }, [data.buffer]);
};
