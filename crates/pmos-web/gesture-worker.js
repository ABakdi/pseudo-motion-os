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
  const cfg = (delegate) => ({
    baseOptions: {
      modelAssetPath:
        "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task",
      delegate,
    },
    runningMode: "VIDEO",
    numFaces: 1,
    outputFaceBlendshapes: true,
  });
  try {
    try {
      faceLandmarker = await FaceLandmarkerClass.createFromOptions(fileset, cfg("GPU"));
    } catch (gpuErr) {
      // Same fallback the hand model gets — some machines can't create a
      // second GPU delegate context.
      console.warn("[pmos gesture-worker] face GPU delegate failed, using CPU:", gpuErr);
      faceLandmarker = await FaceLandmarkerClass.createFromOptions(fileset, cfg("CPU"));
    }
    console.log("[pmos gesture-worker] face landmarker ready");
    postMessage({ type: "faceStatus", ok: true, msg: "" });
  } catch (err) {
    // Never fail silently — the main thread surfaces this to the user.
    console.warn("[pmos gesture-worker] face model failed:", err);
    postMessage({ type: "faceStatus", ok: false, msg: String(err) });
    faceWanted = false;
  }
  faceBuilding = false;
}

// Coarse gaze estimate (CSL spec §6, honest limits): head pose dominates,
// eye-in-head refines. Iris centers (468/473) between the eye corners give
// the eye component; the nose tip against the eye line gives yaw/pitch.
// Output is camera-image-normalized [0,1] — the kernel mirrors and smooths.
// This is REGION accuracy (thirds/quadrants), never a cursor.
function gazeEstimate(lm, get) {
  const irisR = lm[468], irisL = lm[473];
  const rOut = lm[33], rIn = lm[133]; // right eye corners (image-left)
  const lIn = lm[362], lOut = lm[263]; // left eye corners (image-right)
  const nose = lm[1];
  if (!irisR || !irisL || !rOut || !lOut || !nose) return [-1, -1];
  // Eye-in-head horizontal: iris position across each eye's corner span.
  const ratio = (p, a, b) => (b.x - a.x !== 0 ? (p.x - a.x) / (b.x - a.x) : 0.5);
  const hx = (ratio(irisR, rOut, rIn) + ratio(irisL, lIn, lOut)) / 2;
  // Head pose proxy: nose tip against the inter-ocular line.
  const midX = (rOut.x + lOut.x) / 2;
  const midY = (rOut.y + lOut.y) / 2;
  const eyeDist = Math.hypot(lOut.x - rOut.x, lOut.y - rOut.y) || 1;
  const yaw = (nose.x - midX) / eyeDist; // ~0 facing camera
  const pitch = (nose.y - midY) / eyeDist; // ~0.55 neutral (nose below eyes)
  // Eye-in-head vertical: blendshapes are steadier than iris-vs-eyelid.
  const lookV =
    (get("eyeLookDownLeft") + get("eyeLookDownRight")) / 2 -
    (get("eyeLookUpLeft") + get("eyeLookUpRight")) / 2;
  const clamp = (v) => Math.min(1, Math.max(0, v));
  // Eye-in-head weighted heavily: iris travel is only ~±0.15 of the corner
  // span, but "move your eyes across the screen" must sweep the estimate.
  const gx = clamp(0.5 + yaw * 1.4 + (hx - 0.5) * 3.0);
  const gy = clamp(0.5 + (pitch - 0.55) * 1.3 + lookV * 1.0);
  return [gx, gy];
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
        const lm = fr.faceLandmarks?.[0];
        const chin = lm?.[152]; // canonical chin point
        const [gx, gy] = lm ? gazeEstimate(lm, get) : [-1, -1];
        // Full mesh for the viewer overlay (landmarks only — never pixels);
        // the kernel drops it unless the Hand Tracker viewer is open.
        let mesh = null;
        if (lm) {
          mesh = new Float32Array(lm.length * 3);
          lm.forEach((p, i) => {
            mesh[i * 3] = p.x;
            mesh[i * 3 + 1] = p.y;
            mesh[i * 3 + 2] = p.z;
          });
        }
        postMessage(
          {
            type: "face",
            blinkL: get("eyeBlinkLeft"),
            blinkR: get("eyeBlinkRight"),
            jaw: get("jawOpen"),
            brow: get("browInnerUp"),
            chinX: chin ? chin.x : -1,
            chinY: chin ? chin.y : -1,
            gazeX: gx,
            gazeY: gy,
            mesh,
          },
          mesh ? [mesh.buffer] : []
        );
      }
    }
  } catch (err) {
    console.warn("[pmos gesture-worker] detect failed:", err);
  } finally {
    m.bitmap?.close?.();
  }
  postMessage({ type: "hands", hands, data }, [data.buffer]);
};
